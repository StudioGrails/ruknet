use std::{
    collections::HashMap,
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use crossbeam_queue::ArrayQueue;
use dashmap::DashMap;
use std::time::Instant;
use tokio::{
    net::UdpSocket,
    sync::{Notify, RwLock},
};

use crate::{
    app::{RukMessage, RukMessageContext},
    conn::{Conn, ConnectMode, ReqConn},
    error::{PeerError, RukBytesError},
    proto::{
        bytes::{RukBuf, RukBufMut},
        message::{OfflineMessageIDs, OnlineMessageIDs},
        DEFAULT_TIMEOUT, IP_UDP_HEADER_SIZE, MAX_MTU_SIZE, MIN_OFFLINE_PACKET_SIZE,
        MIN_PACKET_SIZE, MTU_SIZES, OFFLINE_MESSAGE_DATA_ID, PROTOCOL_VERSION,
    },
    reliability::fragment::{Priority, Reliability},
    ruknet_debug,
};

#[derive(Debug)]
enum RukCommand {
    Send {
        addr: SocketAddr,
        priority: Priority,
        reliability: Reliability,
        ordering_channel: u8,
        data: Bytes,
    },
    Close {
        addr: SocketAddr,
    },
}

#[derive(Debug)]
pub struct Peer {
    pub addr: SocketAddr,
    pub guid: u64,
    is_listening: bool,
    instant: Arc<tokio::time::Instant>,
    socket: Arc<UdpSocket>,
    req_conns: Arc<DashMap<SocketAddr, ReqConn>>,
    ban_list: Arc<DashMap<SocketAddr, u64>>,
    ping_res: Arc<RwLock<String>>,
    command_queue: Arc<ArrayQueue<RukCommand>>,
    message_queue: Arc<ArrayQueue<RukMessage>>,
    command_notify: Arc<Notify>,
    message_notify: Arc<Notify>,
    close_notify: Arc<Notify>,
}

impl Peer {
    const PACKET_QUEUE_SIZE: usize = 8192;
    const COMMAND_QUEUE_SIZE: usize = 8192;
    const MESSAGE_QUEUE_SIZE: usize = 8192;

    pub async fn new<A: ToSocketAddrs>(addr: A, ping_res: &str) -> Result<Self, PeerError> {
        let addr = addr
            .to_socket_addrs()
            .map_err(|_| PeerError::InvalidAddress)?
            .next()
            .ok_or(PeerError::InvalidAddress)?;
        let socket = UdpSocket::bind(addr)
            .await
            .map_err(PeerError::FailedToBind)?;

        Ok(Self {
            addr,
            guid: rand::random(),
            is_listening: false,
            instant: Arc::new(tokio::time::Instant::now()),
            socket: Arc::new(socket),
            req_conns: Arc::new(DashMap::new()),
            ban_list: Arc::new(DashMap::new()),
            ping_res: Arc::new(RwLock::new(ping_res.to_string())),
            command_queue: Arc::new(ArrayQueue::new(Self::COMMAND_QUEUE_SIZE)),
            message_queue: Arc::new(ArrayQueue::new(Self::MESSAGE_QUEUE_SIZE)),
            command_notify: Arc::new(Notify::new()),
            message_notify: Arc::new(Notify::new()),
            close_notify: Arc::new(Notify::new()),
        })
    }

    pub async fn listen(&mut self, max_conn_count: usize) -> Result<(), PeerError> {
        if self.is_listening {
            return Err(PeerError::AlreadyListening);
        }

        self.is_listening = true;

        let packet_queue = Arc::new(ArrayQueue::new(Self::PACKET_QUEUE_SIZE));
        let packet_notify = Arc::new(Notify::new());

        self.start_recv_task(&packet_queue, &packet_notify);
        self.start_update_task(&packet_queue, &packet_notify, max_conn_count);

        Ok(())
    }

    pub fn connect<A: ToSocketAddrs>(
        &self,
        addr: A,
        max_req_count: usize,
        req_interval: u64,
        timeout: Option<u64>,
    ) -> Result<(), PeerError> {
        let addr = addr
            .to_socket_addrs()
            .map_err(|_| PeerError::InvalidAddress)?
            .next()
            .ok_or(PeerError::InvalidAddress)?;

        if self.req_conns.contains_key(&addr) {
            return Err(PeerError::AlreadyConnecting);
        }

        let req_conn = ReqConn::new(
            self.instant.elapsed().as_millis() as u64,
            addr,
            req_interval,
            max_req_count,
            timeout,
        );

        self.req_conns.insert(addr, req_conn);

        Ok(())
    }

    pub async fn ping<A: ToSocketAddrs>(&self, addr: A) -> Result<(), PeerError> {
        let addr = addr
            .to_socket_addrs()
            .map_err(|_| PeerError::InvalidAddress)?
            .next()
            .ok_or(PeerError::InvalidAddress)?;

        let mut ping = BytesMut::with_capacity(17);
        ping.put_u8(OfflineMessageIDs::UnconnectedPing.to_u8());
        ping.put_u64(self.instant.elapsed().as_millis() as u64);
        ping.put_slice(&OFFLINE_MESSAGE_DATA_ID);
        ping.put_u64(self.guid);

        if let Err(e) = self.socket.send_to(&ping, addr).await {
            ruknet_debug!("Failed to send unconnected ping: {:?}", e);
        }

        Ok(())
    }

    pub fn recv(&self) -> Option<RukMessage> {
        if self.message_queue.is_empty() {
            return None;
        }

        self.message_queue.pop().inspect(|_| {
            self.message_notify.notify_waiters();
        })
    }

    pub fn recv_many(&self, n: usize) -> Vec<RukMessage> {
        if self.message_queue.is_empty() {
            return Vec::new();
        }

        let mut msgs = Vec::with_capacity(n);

        for _ in 0..n {
            if let Some(msg) = self.recv() {
                msgs.push(msg);
            } else {
                break;
            }
        }

        if !msgs.is_empty() {
            self.message_notify.notify_waiters();
        }

        msgs
    }

    pub async fn send(
        &self,
        addr: SocketAddr,
        priority: Priority,
        reliability: Reliability,
        ordering_channel: u8,
        data: Vec<u8>,
    ) {
        Self::send_command_send(
            &self.command_queue,
            &self.command_notify,
            addr,
            priority,
            reliability,
            ordering_channel,
            Bytes::from(data),
        )
        .await;
    }

    pub fn close(&self) {
        self.close_notify.notify_waiters();
    }

    pub async fn close_conn(&self, addr: SocketAddr) {
        Self::close_conn_internal(
            &self.command_queue,
            &self.command_notify,
            addr,
            Priority::Low,
            0,
        )
        .await;
    }

    pub fn ban_addr(&self, addr: SocketAddr, time: u64) {
        Self::ban_addr_internal(&self.ban_list, addr, time);
    }

    pub fn unban_addr(&self, addr: SocketAddr) {
        self.ban_list.remove(&addr);
    }

    pub async fn get_ping_res(&self) -> String {
        self.ping_res.read().await.clone()
    }

    pub async fn set_ping_res(&self, ping_res: &str) {
        *self.ping_res.write().await = ping_res.to_string();
    }

    fn start_recv_task(
        &self,
        packet_queue: &Arc<ArrayQueue<(SocketAddr, Bytes)>>,
        packet_notify: &Arc<Notify>,
    ) {
        let packet_queue = packet_queue.clone();
        let packet_notify = packet_notify.clone();
        let socket = self.socket.clone();
        let close_notify = self.close_notify.clone();

        tokio::spawn(async move {
            let mut buf = [0; MAX_MTU_SIZE - IP_UDP_HEADER_SIZE];

            loop {
                tokio::select! {
                    _ = close_notify.notified() => {
                        break;
                    }
                    result = socket.recv_from(&mut buf) => {
                        match result {
                            Ok((len, addr)) => {
                                if len < MIN_PACKET_SIZE {
                                    ruknet_debug!("Received packet is too small.");
                                    continue;
                                }

                                let mut packet = (addr, Bytes::copy_from_slice(&buf[..len]));
                                while let Err(returned) = packet_queue.push(packet) {
                                    ruknet_debug!("Packet queue is full, waiting for space...");
                                    packet_notify.notified().await;
                                    packet = returned;
                                }
                                // Notify update task immediately when packet arrives
                                packet_notify.notify_one();
                            }
                            Err(e) => {
                                ruknet_debug!("Failed to receive data: {:?}", e);
                            }
                        }
                    }
                }
            }
        });
    }

    fn start_update_task(
        &self,
        packet_queue: &Arc<ArrayQueue<(SocketAddr, Bytes)>>,
        packet_notify: &Arc<Notify>,
        max_conn_count: usize,
    ) {
        let packet_queue = packet_queue.clone();
        let packet_notify = packet_notify.clone();
        let mut conn_count = 0;
        let mut conns = HashMap::new();
        let mut offline_reply_buf = BytesMut::with_capacity(50);
        let mut online_reply_buf = BytesMut::with_capacity(MAX_MTU_SIZE - IP_UDP_HEADER_SIZE);
        let guid = self.guid;
        let instant = self.instant.clone();
        let socket = self.socket.clone();
        let req_conns = self.req_conns.clone();
        let ban_list = self.ban_list.clone();
        let ping_res = self.ping_res.clone();
        let command_queue = self.command_queue.clone();
        let message_queue = self.message_queue.clone();
        let command_notify = self.command_notify.clone();
        let message_notify = self.message_notify.clone();
        let close_notify = self.close_notify.clone();

        tokio::spawn(async move {
            // Use 10μs interval for maximum throughput
            let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(30));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            
            // Cache ping_res to avoid repeated reads
            let mut cached_ping_res = ping_res.read().await.clone();
            let mut last_ping_res_update = Instant::now();

            loop {
                tokio::select! {
                    biased;  // Check in order for better responsiveness
                    
                    _ = close_notify.notified() => {
                        for (addr, _) in conns.iter() {
                            Self::close_conn_internal(
                                &command_queue,
                                &command_notify,
                                *addr,
                                Priority::Immediate,
                                0
                            ).await;
                        }

                        Self::update_cycle(
                            &packet_queue,
                            &packet_notify,
                            max_conn_count,
                            &mut conn_count,
                            &mut conns,
                            &mut offline_reply_buf,
                            &mut online_reply_buf,
                            &guid,
                            &instant,
                            &socket,
                            &req_conns,
                            &ban_list,
                            &cached_ping_res,
                            &command_queue,
                            &message_queue,
                            &command_notify,
                            &message_notify
                        ).await;

                        break;
                    }
                    _ = command_notify.notified() => {
                        // Process immediately when new command arrives (high priority for throughput)
                        Self::update_cycle(
                            &packet_queue,
                            &packet_notify,
                            max_conn_count,
                            &mut conn_count,
                            &mut conns,
                            &mut offline_reply_buf,
                            &mut online_reply_buf,
                            &guid,
                            &instant,
                            &socket,
                            &req_conns,
                            &ban_list,
                            &cached_ping_res,
                            &command_queue,
                            &message_queue,
                            &command_notify,
                            &message_notify
                        ).await;
                    }
                    _ = packet_notify.notified() => {
                        // Process immediately when packet arrives
                        Self::update_cycle(
                            &packet_queue,
                            &packet_notify,
                            max_conn_count,
                            &mut conn_count,
                            &mut conns,
                            &mut offline_reply_buf,
                            &mut online_reply_buf,
                            &guid,
                            &instant,
                            &socket,
                            &req_conns,
                            &ban_list,
                            &cached_ping_res,
                            &command_queue,
                            &message_queue,
                            &command_notify,
                            &message_notify
                        ).await;
                    }
                    _ = interval.tick() => {
                        // Update cached ping_res periodically (every 100ms)
                        if last_ping_res_update.elapsed().as_millis() >= 100 {
                            cached_ping_res = ping_res.read().await.clone();
                            last_ping_res_update = Instant::now();
                        }
                        
                        Self::update_cycle(
                            &packet_queue,
                            &packet_notify,
                            max_conn_count,
                            &mut conn_count,
                            &mut conns,
                            &mut offline_reply_buf,
                            &mut online_reply_buf,
                            &guid,
                            &instant,
                            &socket,
                            &req_conns,
                            &ban_list,
                            &cached_ping_res,
                            &command_queue,
                            &message_queue,
                            &command_notify,
                            &message_notify
                        ).await;
                    }
                }
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_cycle(
        packet_queue: &ArrayQueue<(SocketAddr, Bytes)>,
        packet_notify: &Notify,
        max_conn_count: usize,
        conn_count: &mut usize,
        conns: &mut HashMap<SocketAddr, Conn>,
        offline_reply_buf: &mut BytesMut,
        online_reply_buf: &mut BytesMut,
        guid: &u64,
        instant: &tokio::time::Instant,
        socket: &UdpSocket,
        req_conns: &DashMap<SocketAddr, ReqConn>,
        ban_list: &DashMap<SocketAddr, u64>,
        ping_res: &String,
        command_queue: &ArrayQueue<RukCommand>,
        message_queue: &ArrayQueue<RukMessage>,
        command_notify: &Notify,
        message_notify: &Notify,
    ) {
        let now = instant.elapsed().as_millis() as u64;

        let mut poped = false;

        while let Some((addr, data)) = packet_queue.pop() {
            poped = true;
            Self::process_packet(
                max_conn_count,
                conn_count,
                conns,
                offline_reply_buf,
                guid,
                socket,
                req_conns,
                ban_list,
                ping_res,
                command_queue,
                message_queue,
                command_notify,
                message_notify,
                now,
                addr,
                data,
            )
            .await;
        }

        if poped {
            packet_notify.notify_waiters();
            poped = false;
        }

        while let Some(command) = command_queue.pop() {
            poped = true;
            Self::process_command(now, conn_count, conns, command);
        }

        if poped {
            command_notify.notify_waiters();
        }

        if !req_conns.is_empty() {
            for mut req_conn in req_conns.iter_mut() {
                let addr = *req_conn.key();
                if now > req_conn.value().next_req_time {
                    if req_conn.value().req_count >= req_conn.value().max_req_count {
                        Self::send_message(
                            message_queue,
                            message_notify,
                            addr,
                            *guid,
                            RukMessageContext::ConnectionAttemptFailed,
                        )
                        .await;

                        req_conns.remove(&addr);
                    } else {
                        let mtu_size_index = (req_conn.value().req_count
                            / (req_conn.value().max_req_count / MTU_SIZES.len()))
                        .min(MTU_SIZES.len() - 1);
                        req_conn.value_mut().req_count += 1;
                        req_conn.value_mut().next_req_time = now + req_conn.value().req_interval;

                        let mut req = BytesMut::with_capacity(MTU_SIZES[mtu_size_index]);
                        req.put_u8(OfflineMessageIDs::OpenConnectionRequest1.to_u8());
                        req.put_slice(&OFFLINE_MESSAGE_DATA_ID);
                        req.put_u8(PROTOCOL_VERSION);
                        req.put_bytes(
                            0,
                            MTU_SIZES[mtu_size_index] - IP_UDP_HEADER_SIZE - req.len(),
                        );

                        if let Err(e) = socket.send_to(&req, addr).await {
                            ruknet_debug!("Failed to send open connection request 1: {:?}", e);
                        }
                    }
                }
            }
        }

        for (addr, conn) in conns.iter_mut() {
            if conn.connect_mode == ConnectMode::Connected
                && now - conn.last_reliable_send > conn.layer.get_timeout() / 2
                && conn.layer.is_retransmit_cache_empty()
            {
                Self::send_ping_to_conn(now, conn, Reliability::Reliable);
            }

            if let Err(e) = conn.layer.update(now, online_reply_buf, socket, addr).await {
                ruknet_debug!("Failed to update connection: {:?}", e);
                Self::close_conn_internal(command_queue, command_notify, *addr, Priority::Low, 0)
                    .await;
                continue;
            }

            if conn.layer.is_dead()
                || ((conn.connect_mode == ConnectMode::RequestedConnection
                    || conn.connect_mode == ConnectMode::HandlingConnectionRequest
                    || conn.connect_mode == ConnectMode::UnverfiedSender)
                    && now - conn.create_time > 10000)
            {
                Self::close_conn_internal(command_queue, command_notify, *addr, Priority::Low, 0)
                    .await;
                continue;
            }

            if conn.connect_mode == ConnectMode::Connected && now > conn.next_ping_time {
                conn.next_ping_time = now + 5000;
                Self::send_ping_to_conn(now, conn, Reliability::Unreliable);
            }

            // Process any remaining messages (in case some were queued)
            let msgs = conn.layer.recv();

            for msg in msgs {
                if let Err(e) = Self::process_message(
                    now,
                    command_queue,
                    message_queue,
                    command_notify,
                    message_notify,
                    conn,
                    addr,
                    msg,
                )
                .await
                {
                    ruknet_debug!("Failed to process message: {:?}", e);
                    Self::ban_addr_internal(ban_list, *addr, conn.layer.get_timeout());
                    Self::close_conn_internal(
                        command_queue,
                        command_notify,
                        *addr,
                        Priority::Low,
                        0,
                    )
                    .await;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_packet(
        max_conn_count: usize,
        conn_count: &mut usize,
        conns: &mut HashMap<SocketAddr, Conn>,
        offline_reply_buf: &mut BytesMut,
        guid: &u64,
        socket: &UdpSocket,
        req_conns: &DashMap<SocketAddr, ReqConn>,
        ban_list: &DashMap<SocketAddr, u64>,
        ping_res: &String,
        command_queue: &ArrayQueue<RukCommand>,
        message_queue: &ArrayQueue<RukMessage>,
        command_notify: &Notify,
        message_notify: &Notify,
        now: u64,
        addr: SocketAddr,
        data: Bytes,
    ) {
        if let Some(conn) = conns.get_mut(&addr) {
            if let Err(e) = conn.layer.handle_datagram(now, data) {
                ruknet_debug!("Failed to handle datagram: {:?}", e);
                Self::close_conn_internal(command_queue, command_notify, addr, Priority::Low, 0)
                    .await;
                return;
            }
            
            // Process messages immediately after handling datagram
            let msgs = conn.layer.recv();
            for msg in msgs {
                if let Err(e) = Self::process_message(
                    now,
                    command_queue,
                    message_queue,
                    command_notify,
                    message_notify,
                    conn,
                    &addr,
                    msg,
                )
                .await
                {
                    ruknet_debug!("Failed to process message: {:?}", e);
                    Self::ban_addr_internal(ban_list, addr, conn.layer.get_timeout());
                    Self::close_conn_internal(command_queue, command_notify, addr, Priority::Low, 0)
                        .await;
                    return;
                }
            }
        } else {
            if let Some(ban_time) = ban_list.get(&addr) {
                if *ban_time < now {
                    ban_list.remove(&addr);
                } else {
                    let mut reply = BytesMut::with_capacity(25);
                    reply.put_u8(OfflineMessageIDs::ConnectionBanned.to_u8());
                    reply.put_slice(&OFFLINE_MESSAGE_DATA_ID);
                    reply.put_u64(*guid);

                    if let Err(e) = socket.send_to(&reply, addr).await {
                        ruknet_debug!("Failed to send connection banned message: {:?}", e);
                    }

                    return;
                }
            }

            if let Err(e) = Self::process_offline_packet(
                max_conn_count,
                conn_count,
                conns,
                offline_reply_buf,
                guid,
                socket,
                req_conns,
                ping_res,
                message_queue,
                message_notify,
                now,
                addr,
                data,
            )
            .await
            {
                ruknet_debug!("Failed to process offline packet: {:?}", e);
                Self::ban_addr_internal(ban_list, addr, DEFAULT_TIMEOUT);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_offline_packet(
        max_conn_count: usize,
        conn_count: &mut usize,
        conns: &mut HashMap<SocketAddr, Conn>,
        offline_reply_buf: &mut BytesMut,
        guid: &u64,
        socket: &UdpSocket,
        req_conns: &DashMap<SocketAddr, ReqConn>,
        ping_res: &String,
        message_queue: &ArrayQueue<RukMessage>,
        message_notify: &Notify,
        now: u64,
        addr: SocketAddr,
        mut data: Bytes,
    ) -> Result<(), PeerError> {
        if data.len() < MIN_OFFLINE_PACKET_SIZE {
            return Err(PeerError::InvalidPacket("packet is too small.".to_string()));
        }

        if let Some(id) = OfflineMessageIDs::from_u8(data[0]) {
            data.advance(1);

            match id {
                OfflineMessageIDs::UnconnectedPing => {
                    if *conn_count >= max_conn_count {
                        return Ok(());
                    }

                    let ping_time = data.try_get_u64()?;
                    data.try_advance(OFFLINE_MESSAGE_DATA_ID.len())?;
                    let remote_guid = data.try_get_u64()?;

                    offline_reply_buf.clear();
                    offline_reply_buf.put_u8(OfflineMessageIDs::UnconnectedPong.to_u8());
                    offline_reply_buf.put_u64(ping_time);
                    offline_reply_buf.put_u64(*guid);
                    offline_reply_buf.put_slice(&OFFLINE_MESSAGE_DATA_ID);
                    offline_reply_buf.put_string(ping_res);

                    socket.send_to(offline_reply_buf, addr).await?;

                    Self::send_message(
                        message_queue,
                        message_notify,
                        addr,
                        remote_guid,
                        RukMessageContext::UnconnectedPing,
                    )
                    .await;
                }
                OfflineMessageIDs::UnconnectedPong => {
                    let ping_time = data.try_get_u64()?;
                    let remote_guid = data.try_get_u64()?;
                    data.try_advance(OFFLINE_MESSAGE_DATA_ID.len())?;
                    let ping_res = data.try_get_string()?;

                    Self::send_message(
                        message_queue,
                        message_notify,
                        addr,
                        remote_guid,
                        RukMessageContext::UnconnectedPong {
                            ping: now.saturating_sub(ping_time),
                            ping_res,
                        },
                    )
                    .await;
                }
                OfflineMessageIDs::OpenConnectionRequest1 => {
                    data.try_advance(OFFLINE_MESSAGE_DATA_ID.len())?;
                    let proto_version = data.try_get_u8()?;

                    if proto_version != PROTOCOL_VERSION {
                        offline_reply_buf.clear();
                        offline_reply_buf
                            .put_u8(OfflineMessageIDs::IncompatibleProtocolVersion.to_u8());
                        offline_reply_buf.put_u8(PROTOCOL_VERSION);
                        offline_reply_buf.put_slice(&OFFLINE_MESSAGE_DATA_ID);
                        offline_reply_buf.put_u64(*guid);

                        socket.send_to(offline_reply_buf, addr).await?;

                        return Ok(());
                    }

                    offline_reply_buf.clear();
                    offline_reply_buf.put_u8(OfflineMessageIDs::OpenConnectionReply1.to_u8());
                    offline_reply_buf.put_slice(&OFFLINE_MESSAGE_DATA_ID);
                    offline_reply_buf.put_u64(*guid);

                    offline_reply_buf.put_u8(0);

                    offline_reply_buf
                        .put_u16((data.len() + 18 + IP_UDP_HEADER_SIZE).min(MAX_MTU_SIZE) as u16);

                    socket.send_to(offline_reply_buf, addr).await?;
                }
                OfflineMessageIDs::OpenConnectionReply1 => {
                    data.try_advance(OFFLINE_MESSAGE_DATA_ID.len())?;
                    // remote_guid is used in security.
                    let _remote_guid = data.try_get_u64()?;
                    let cookie = if data.try_get_u8()? == 1 {
                        Some(data.try_get_u32()?)
                    } else {
                        None
                    };

                    offline_reply_buf.clear();
                    offline_reply_buf.put_u8(OfflineMessageIDs::OpenConnectionRequest2.to_u8());
                    offline_reply_buf.put_slice(&OFFLINE_MESSAGE_DATA_ID);
                    if let Some(cookie) = cookie {
                        offline_reply_buf.put_u32(cookie);
                    }

                    if let Some(req_conn) = req_conns.get(&addr) {
                        if cookie.is_some() {
                            offline_reply_buf.put_u8(0);
                        }

                        let mtu_size = data.try_get_u16()?;

                        offline_reply_buf.put_addr(req_conn.addr);
                        offline_reply_buf.put_u16(mtu_size);
                        offline_reply_buf.put_u64(*guid);

                        socket.send_to(offline_reply_buf, addr).await?;
                    }
                }
                OfflineMessageIDs::OpenConnectionRequest2 => {
                    data.try_advance(OFFLINE_MESSAGE_DATA_ID.len())?;

                    let remote_needs_security = false;

                    let binding_addr = data.try_get_addr()?;
                    let mtu_size = data.try_get_u16()?;
                    let remote_guid = data.try_get_u64()?;

                    if *conn_count >= max_conn_count {
                        offline_reply_buf.clear();
                        offline_reply_buf
                            .put_u8(OfflineMessageIDs::NoFreeIncomingConnections.to_u8());
                        offline_reply_buf.put_slice(&OFFLINE_MESSAGE_DATA_ID);
                        offline_reply_buf.put_u64(*guid);

                        socket.send_to(offline_reply_buf, addr).await?;
                        return Ok(());
                    }

                    if let Some(conn) = Self::register_conn(
                        now,
                        addr,
                        remote_guid,
                        binding_addr,
                        ConnectMode::UnverfiedSender,
                        mtu_size as usize,
                        remote_needs_security,
                    ) {
                        *conn_count += 1;
                        conns.insert(addr, conn);

                        offline_reply_buf.clear();
                        offline_reply_buf.put_u8(OfflineMessageIDs::OpenConnectionReply2.to_u8());
                        offline_reply_buf.put_slice(&OFFLINE_MESSAGE_DATA_ID);
                        offline_reply_buf.put_u64(*guid);
                        offline_reply_buf.put_addr(addr);
                        offline_reply_buf.put_u16(mtu_size);
                        offline_reply_buf.put_u8(remote_needs_security as u8);

                        socket.send_to(offline_reply_buf, addr).await?;
                    } else {
                        offline_reply_buf.clear();
                        offline_reply_buf.put_u8(OfflineMessageIDs::IpRecentlyConnected.to_u8());
                        offline_reply_buf.put_slice(&OFFLINE_MESSAGE_DATA_ID);
                        offline_reply_buf.put_u64(*guid);

                        socket.send_to(offline_reply_buf, addr).await?;
                    }
                }
                OfflineMessageIDs::OpenConnectionReply2 => {
                    data.try_advance(OFFLINE_MESSAGE_DATA_ID.len())?;
                    let remote_guid = data.try_get_u64()?;
                    let binding_addr = data.try_get_addr()?;
                    let mtu_size = data.try_get_u16()?;
                    let remote_needs_security = data.try_get_u8()? == 1;

                    if let Some((addr, req_conn)) = req_conns.remove(&addr) {
                        if let Some(mut conn) = Self::register_conn(
                            now,
                            addr,
                            remote_guid,
                            binding_addr,
                            ConnectMode::UnverfiedSender,
                            mtu_size as usize,
                            remote_needs_security,
                        ) {
                            conn.connect_mode = ConnectMode::RequestedConnection;

                            if let Some(timeout) = req_conn.timeout {
                                conn.layer.set_timeout(timeout);
                            }

                            offline_reply_buf.clear();
                            offline_reply_buf.put_u8(OnlineMessageIDs::ConnectionRequest.to_u8());
                            offline_reply_buf.put_u64(*guid);
                            offline_reply_buf.put_u64(now);
                            offline_reply_buf.put_u8(0);

                            Self::send_to_conn(
                                now,
                                &mut conn,
                                Priority::Immediate,
                                Reliability::Reliable,
                                0,
                                Bytes::copy_from_slice(offline_reply_buf),
                            );

                            *conn_count += 1;
                            conns.insert(addr, conn);
                        }
                    }
                }
                OfflineMessageIDs::IncompatibleProtocolVersion => {
                    data.try_advance(OFFLINE_MESSAGE_DATA_ID.len())?;
                    let remote_proto_version = data.try_get_u8()?;
                    let remote_guid = data.try_get_u64()?;

                    Self::send_message(
                        message_queue,
                        message_notify,
                        addr,
                        remote_guid,
                        RukMessageContext::IncompatibleProtocolVersion {
                            remote_proto_version,
                        },
                    )
                    .await;
                }
                OfflineMessageIDs::AlreadyConnected => {
                    data.try_advance(OFFLINE_MESSAGE_DATA_ID.len())?;
                    let remote_guid = data.try_get_u64()?;

                    Self::send_message(
                        message_queue,
                        message_notify,
                        addr,
                        remote_guid,
                        RukMessageContext::AlreadyConnected,
                    )
                    .await;
                }
                OfflineMessageIDs::NoFreeIncomingConnections => {
                    data.try_advance(OFFLINE_MESSAGE_DATA_ID.len())?;
                    let remote_guid = data.try_get_u64()?;

                    Self::send_message(
                        message_queue,
                        message_notify,
                        addr,
                        remote_guid,
                        RukMessageContext::NoFreeIncomingConnections,
                    )
                    .await;
                }
                OfflineMessageIDs::ConnectionBanned => {
                    data.try_advance(OFFLINE_MESSAGE_DATA_ID.len())?;
                    let remote_guid = data.try_get_u64()?;

                    Self::send_message(
                        message_queue,
                        message_notify,
                        addr,
                        remote_guid,
                        RukMessageContext::ConnectionBanned,
                    )
                    .await;
                }
                OfflineMessageIDs::IpRecentlyConnected => {
                    data.try_advance(OFFLINE_MESSAGE_DATA_ID.len())?;
                    let remote_guid = data.try_get_u64()?;

                    Self::send_message(
                        message_queue,
                        message_notify,
                        addr,
                        remote_guid,
                        RukMessageContext::IpRecentlyConnected,
                    )
                    .await;
                }
            }

            Ok(())
        } else {
            Err(PeerError::InvalidPacket(
                "invalid offline message id.".to_string(),
            ))
        }
    }

    fn process_command(
        now: u64,
        conn_count: &mut usize,
        conns: &mut HashMap<SocketAddr, Conn>,
        command: RukCommand,
    ) {
        match command {
            RukCommand::Send {
                addr,
                priority,
                reliability,
                ordering_channel,
                data,
            } => {
                if let Some(conn) = conns.get_mut(&addr) {
                    Self::send_to_conn(now, conn, priority, reliability, ordering_channel, data)
                }
            }
            RukCommand::Close { addr } => {
                if conns.remove(&addr).is_some() {
                    *conn_count -= 1;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_message(
        now: u64,
        command_queue: &ArrayQueue<RukCommand>,
        message_queue: &ArrayQueue<RukMessage>,
        command_notify: &Notify,
        message_notify: &Notify,
        conn: &mut Conn,
        addr: &SocketAddr,
        mut msg: Bytes,
    ) -> Result<(), PeerError> {
        if msg.is_empty() {
            return Err(PeerError::InvalidPacket(
                "message is too small.".to_string(),
            ));
        }

        if conn.connect_mode == ConnectMode::UnverfiedSender
            && msg[0] != OnlineMessageIDs::ConnectionRequest.to_u8()
        {
            return Err(PeerError::InvalidPacket(
                "unverified sender can only send connection request.".to_string(),
            ));
        }

        if let Some(id) = OnlineMessageIDs::from_u8(msg[0]) {
            msg.advance(1);

            match id {
                OnlineMessageIDs::ConnectionRequest => {
                    let _remote_guid = msg.try_get_u64()?;
                    let ping_time = msg.try_get_u64()?;
                    let _needs_security = msg.try_get_u8()? == 1;

                    conn.connect_mode = ConnectMode::HandlingConnectionRequest;

                    let mut reply = BytesMut::with_capacity(300);
                    reply.put_u8(OnlineMessageIDs::ConnectionRequestAccepted.to_u8());
                    reply.put_addr(*addr);
                    reply.put_u16(0); // System index
                    [SocketAddr::new([0, 0, 0, 0].into(), 0); 10]
                        .iter()
                        .for_each(|addr| {
                            reply.put_addr(*addr);
                        });
                    reply.put_u64(ping_time);
                    reply.put_u64(now);

                    Self::send_to_conn(
                        now,
                        conn,
                        Priority::Immediate,
                        Reliability::ReliableOrdered,
                        0,
                        reply.freeze(),
                    );
                }
                OnlineMessageIDs::ConnectionRequestAccepted => {
                    if conn.connect_mode != ConnectMode::RequestedConnection {
                        return Err(PeerError::InvalidPacket(
                            "connection request accepted is not expected.".to_string(),
                        ));
                    }

                    let remote_addr = msg.try_get_addr()?;
                    let _system_index = msg.try_get_u16()?;
                    let _internal_addrs =
                        (0..10)
                            .map(|_| msg.try_get_addr())
                            .collect::<Result<Vec<SocketAddr>, RukBytesError>>()?;
                    let ping_time = msg.try_get_u64()?;
                    let pong_time = msg.try_get_u64()?;

                    conn.ping = now.saturating_sub(ping_time);
                    conn.connect_mode = ConnectMode::Connected;

                    Self::send_message(
                        message_queue,
                        message_notify,
                        *addr,
                        conn.guid,
                        RukMessageContext::ConnectionRequestAccepted,
                    )
                    .await;

                    let mut reply = BytesMut::with_capacity(300);
                    reply.put_u8(OnlineMessageIDs::NewIncomingConnection.to_u8());
                    reply.put_addr(remote_addr);
                    [SocketAddr::new([0, 0, 0, 0].into(), 0); 20]
                        .iter()
                        .for_each(|addr| {
                            reply.put_addr(*addr);
                        });
                    reply.put_u64(pong_time);
                    reply.put_u64(now);

                    Self::send_to_conn(
                        now,
                        conn,
                        Priority::Immediate,
                        Reliability::ReliableOrdered,
                        0,
                        reply.freeze(),
                    );

                    Self::send_ping_to_conn(now, conn, Reliability::Unreliable);
                }
                OnlineMessageIDs::NewIncomingConnection => {
                    if conn.connect_mode != ConnectMode::HandlingConnectionRequest {
                        return Err(PeerError::InvalidPacket(
                            "new incoming connection is not expected.".to_string(),
                        ));
                    }

                    conn.connect_mode = ConnectMode::Connected;

                    Self::send_ping_to_conn(now, conn, Reliability::Unreliable);
                    let _remote_addr = msg.try_get_addr()?;
                    let _internal_addrs =
                        (0..20)
                            .map(|_| msg.try_get_addr())
                            .collect::<Result<Vec<SocketAddr>, RukBytesError>>()?;
                    let ping_time = msg.try_get_u64()?;
                    let _pong_time = msg.try_get_u64()?;

                    conn.ping = now.saturating_sub(ping_time);

                    Self::send_message(
                        message_queue,
                        message_notify,
                        *addr,
                        conn.guid,
                        RukMessageContext::NewIncomingConnection,
                    )
                    .await;
                }
                OnlineMessageIDs::ConnectedPing => {
                    let ping_time = msg.try_get_u64()?;

                    let mut reply = BytesMut::with_capacity(17);
                    reply.put_u8(OnlineMessageIDs::ConnectedPong.to_u8());
                    reply.put_u64(ping_time);
                    reply.put_u64(now);

                    Self::send_to_conn(
                        now,
                        conn,
                        Priority::Immediate,
                        Reliability::Unreliable,
                        0,
                        reply.freeze(),
                    );
                }
                OnlineMessageIDs::ConnectedPong => {
                    let ping_time = msg.try_get_u64()?;
                    let _pong_time = msg.try_get_u64()?;

                    conn.ping = now.saturating_sub(ping_time);
                }
                OnlineMessageIDs::DisconnectionNotification => {
                    Self::close_conn_internal(
                        command_queue,
                        command_notify,
                        *addr,
                        Priority::Low,
                        0,
                    )
                    .await;
                }
            }
        } else {
            Self::send_message(
                message_queue,
                message_notify,
                *addr,
                conn.guid,
                RukMessageContext::App { data: Vec::from(msg) },
            )
            .await;
        }

        Ok(())
    }

    fn register_conn(
        now: u64,
        _addr: SocketAddr,
        guid: u64,
        // binding_addr is not used in original RakNet.
        _binding_addr: SocketAddr,
        connect_mode: ConnectMode,
        mtu_size: usize,
        _use_security: bool,
    ) -> Option<Conn> {
        let conn = Conn::new(now, guid, connect_mode, mtu_size);

        Some(conn)
    }

    async fn close_conn_internal(
        command_queue: &ArrayQueue<RukCommand>,
        command_notify: &Notify,
        addr: SocketAddr,
        priority: Priority,
        ordering_channel: u8,
    ) {
        Self::send_command_send(
            command_queue,
            command_notify,
            addr,
            priority,
            Reliability::ReliableOrdered,
            ordering_channel,
            Bytes::from(vec![OnlineMessageIDs::DisconnectionNotification.to_u8()]),
        )
        .await;
        Self::send_command_close(command_queue, command_notify, addr).await;
    }

    async fn send_command_send(
        command_queue: &ArrayQueue<RukCommand>,
        command_notify: &Notify,
        addr: SocketAddr,
        priority: Priority,
        reliability: Reliability,
        ordering_channel: u8,
        data: Bytes,
    ) {
        let mut command = RukCommand::Send {
            addr,
            priority,
            reliability,
            ordering_channel,
            data,
        };
        let mut retry_count = 0;
        while let Err(returned) = command_queue.push(command) {
            ruknet_debug!("Command queue is full, yielding and retrying...");
            // Yield to allow update_task to process commands and free up space
            tokio::task::yield_now().await;
            command = returned;
            retry_count += 1;
            // If we've retried many times, wait briefly on the notify
            if retry_count > 100 {
                tokio::select! {
                    _ = command_notify.notified() => {}
                    _ = tokio::time::sleep(tokio::time::Duration::from_micros(100)) => {}
                }
                retry_count = 0;
            }
        }
        // Notify update_task that there's a new command to process
        command_notify.notify_one();
    }

    async fn send_command_close(
        command_queue: &ArrayQueue<RukCommand>,
        command_notify: &Notify,
        addr: SocketAddr,
    ) {
        let mut command = RukCommand::Close { addr };
        let mut retry_count = 0;
        while let Err(returned) = command_queue.push(command) {
            ruknet_debug!("Command queue is full, yielding and retrying...");
            tokio::task::yield_now().await;
            command = returned;
            retry_count += 1;
            if retry_count > 100 {
                tokio::select! {
                    _ = command_notify.notified() => {}
                    _ = tokio::time::sleep(tokio::time::Duration::from_micros(100)) => {}
                }
                retry_count = 0;
            }
        }
    }

    async fn send_message(
        message_queue: &ArrayQueue<RukMessage>,
        message_notify: &Notify,
        addr: SocketAddr,
        guid: u64,
        context: RukMessageContext,
    ) {
        let mut message = RukMessage {
            addr,
            guid,
            context,
        };
        let mut retry_count = 0;
        while let Err(returned) = message_queue.push(message) {
            ruknet_debug!("Message queue is full, yielding and retrying...");
            tokio::task::yield_now().await;
            message = returned;
            retry_count += 1;
            if retry_count > 100 {
                tokio::select! {
                    _ = message_notify.notified() => {}
                    _ = tokio::time::sleep(tokio::time::Duration::from_micros(100)) => {}
                }
                retry_count = 0;
            }
        }
        // Notify receivers that a new message is available
        message_notify.notify_one();
    }

    fn send_to_conn(
        now: u64,
        conn: &mut Conn,
        priority: Priority,
        reliability: Reliability,
        ordering_channel: u8,
        data: Bytes,
    ) {
        if reliability.is_reliable() {
            conn.last_reliable_send = now;
        }

        conn.layer
            .send(priority, reliability, ordering_channel, data);
    }

    fn send_ping_to_conn(now: u64, conn: &mut Conn, reliability: Reliability) {
        let mut data = BytesMut::with_capacity(8);
        data.put_u8(OnlineMessageIDs::ConnectedPing.to_u8());
        data.put_u64(now);

        Self::send_to_conn(
            now,
            conn,
            Priority::Immediate,
            reliability,
            0,
            data.freeze(),
        );
    }

    fn ban_addr_internal(ban_list: &DashMap<SocketAddr, u64>, addr: SocketAddr, time: u64) {
        ban_list.insert(addr, time);
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.close();
    }
}
