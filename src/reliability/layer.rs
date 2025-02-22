use std::{
    cmp::{Ordering, Reverse},
    collections::{BinaryHeap, HashMap, VecDeque},
    net::SocketAddr,
};

use bytes::{Bytes, BytesMut};
use tokio::net::UdpSocket;

use crate::{
    error::LayerError,
    proto::{
        bytes::{RukBuf, RukBufMut},
        datagram::{Datagram, Fragment, OrderingInfo, SplitInfo},
        DEFAULT_TIMEOUT, IP_UDP_HEADER_SIZE,
    },
};

use super::{
    congestion_controller::CongestionController,
    fragment::{Priority, Reliability},
    retransmit_cache::RetransmitCache,
    window::Window,
};

const SEQ_WINDOW_SIZE: usize = 128;
const RELIABLE_WINDOW_SIZE: usize = 512;
const ORDERING_CHANNEL_SIZE: usize = 32;

#[derive(Debug)]
struct OutgoingFragment {
    weight: usize,
    priority: Priority,
    fragment: Fragment,
}

impl PartialEq for OutgoingFragment {
    fn eq(&self, other: &Self) -> bool {
        self.weight == other.weight
    }
}

impl Eq for OutgoingFragment {}

impl PartialOrd for OutgoingFragment {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.weight.cmp(&other.weight))
    }
}

impl Ord for OutgoingFragment {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.weight.cmp(&other.weight)
    }
}

#[derive(Debug)]
pub struct Layer<T: CongestionController> {
    max_size_of_packet: usize,
    bandwidth_exceeded: bool,
    needs_arrival_rate: bool,
    is_dead: bool,
    timeout: u64,
    last_update: u64,
    time_last_datagram_recved: u64,
    total_bytes_acked: usize,
    acks: Vec<u32>,
    naks: Vec<u32>,
    outgoing_queue: BinaryHeap<Reverse<OutgoingFragment>>,
    outgoing_next_weight: [usize; Priority::COUNT],
    output_queue: Vec<Bytes>,
    retransmit_cache: RetransmitCache,
    seq_window: Window<u64, SEQ_WINDOW_SIZE>,
    reliable_window: Window<(), RELIABLE_WINDOW_SIZE>,
    split_fragments: HashMap<u16, Vec<Option<Bytes>>>,
    ordered_fragments: [VecDeque<Option<Bytes>>; ORDERING_CHANNEL_SIZE],
    send_split_id: u16,
    send_reliable_index: u32,
    send_sequence_index: u32,
    send_ordering_index: [u32; ORDERING_CHANNEL_SIZE],
    recv_sequence_index: [u32; ORDERING_CHANNEL_SIZE],
    recv_ordering_index: [u32; ORDERING_CHANNEL_SIZE],
    congestion_controller: T,
}

impl<T: CongestionController> Layer<T> {
    pub fn new(now: u64, mtu_size: usize) -> Self {
        Self {
            max_size_of_packet: mtu_size
                - IP_UDP_HEADER_SIZE
                - Datagram::HEADER_SIZE
                - Datagram::PACKET_HEADER_SIZE,
            bandwidth_exceeded: false,
            needs_arrival_rate: false,
            is_dead: false,
            timeout: DEFAULT_TIMEOUT,
            last_update: now,
            time_last_datagram_recved: now,
            total_bytes_acked: 0,
            acks: Vec::with_capacity(64),
            naks: Vec::with_capacity(64),
            outgoing_queue: BinaryHeap::new(),
            outgoing_next_weight: [0; Priority::COUNT],
            output_queue: Vec::with_capacity(64),
            retransmit_cache: RetransmitCache::new(),
            seq_window: Window::new(),
            reliable_window: Window::new(),
            split_fragments: HashMap::new(),
            ordered_fragments: std::array::from_fn(|_| VecDeque::new()),
            send_split_id: 0,
            send_reliable_index: 0,
            send_sequence_index: 0,
            send_ordering_index: [0; ORDERING_CHANNEL_SIZE],
            recv_sequence_index: [0; ORDERING_CHANNEL_SIZE],
            recv_ordering_index: [0; ORDERING_CHANNEL_SIZE],
            congestion_controller: T::new(mtu_size - IP_UDP_HEADER_SIZE),
        }
    }

    pub fn is_dead(&self) -> bool {
        self.is_dead
    }

    pub fn set_timeout(&mut self, timeout: u64) {
        self.timeout = timeout;
    }

    pub fn get_timeout(&self) -> u64 {
        self.timeout
    }

    pub async fn update(
        &mut self,
        now: u64,
        reply_buf: &mut BytesMut,
        socket: &UdpSocket,
        addr: &SocketAddr,
    ) -> Result<(), LayerError> {
        let time_since_last_update = now - self.last_update;
        self.last_update = now;

        if !self.retransmit_cache.is_empty() && now - self.time_last_datagram_recved > self.timeout
        {
            self.is_dead = true;
        }

        if self
            .congestion_controller
            .should_send_ack(now, time_since_last_update)
            && !self.acks.is_empty()
        {
            let mut arrival_rate = None;
            self.congestion_controller
                .on_send_ack(self.needs_arrival_rate, &mut arrival_rate);

            let mut datagram = Datagram::Ack {
                arrival_rate,
                seqs: self.acks.drain(..).collect(),
            };

            self.send_datagram(reply_buf, socket, addr, &mut datagram)
                .await?;
        }

        if !self.naks.is_empty() {
            let mut datagram = Datagram::Nak {
                seqs: self.naks.drain(..).collect(),
            };

            self.send_datagram(reply_buf, socket, addr, &mut datagram)
                .await?;
        }

        let needs_arrival_rate = self.congestion_controller.is_in_slow_start();
        let is_continuous_send = self.bandwidth_exceeded;

        self.bandwidth_exceeded = !self.outgoing_queue.is_empty();

        if !self.retransmit_cache.is_empty() || self.bandwidth_exceeded {
            let transmission_bandwidth = self.congestion_controller.get_transmission_bandwidth(
                now,
                time_since_last_update,
                is_continuous_send,
            );

            let mut all_datagram_sizes = 0;

            let retransmit_fragments = self
                .retransmit_cache
                .get_retransmit_fragments(now, self.congestion_controller.get_rto());

            let mut packet_sizes = Vec::with_capacity(8);
            let mut packets = Vec::with_capacity(8);
            let mut current_packet_size = 0;
            let mut current_packet = Vec::with_capacity(16);

            while all_datagram_sizes < transmission_bandwidth {
                if let Some(fragment) = retransmit_fragments.pop_front() {
                    let size = fragment.calc_full_size();

                    if all_datagram_sizes + size > transmission_bandwidth {
                        retransmit_fragments.push_front(fragment);
                        break;
                    }

                    if current_packet_size + size > self.max_size_of_packet {
                        packet_sizes.push(current_packet_size);
                        packets.push(current_packet);
                        current_packet = Vec::with_capacity(16);
                        current_packet_size = 0;
                    }

                    self.congestion_controller.on_resend(now);

                    current_packet.push(fragment);
                    current_packet_size += size;
                    all_datagram_sizes += size;
                } else {
                    break;
                }
            }

            while all_datagram_sizes < transmission_bandwidth {
                if let Some(Reverse(outgoing_fragment)) = self.outgoing_queue.peek() {
                    let size = outgoing_fragment.fragment.calc_full_size();

                    if all_datagram_sizes + size > transmission_bandwidth {
                        break;
                    }

                    let mut outgoing_fragment = self.outgoing_queue.pop().unwrap().0;

                    if current_packet_size + size > self.max_size_of_packet {
                        packet_sizes.push(current_packet_size);
                        packets.push(current_packet);
                        current_packet = Vec::with_capacity(16);
                        current_packet_size = 0;
                    }

                    if outgoing_fragment.fragment.reliability.is_reliable() {
                        outgoing_fragment.fragment.reliable_index = Some(self.send_reliable_index);
                        self.send_reliable_index += 1;
                    }

                    current_packet.push(outgoing_fragment.fragment);
                    current_packet_size += size;
                    all_datagram_sizes += size;
                } else {
                    break;
                }
            }

            if !current_packet.is_empty() {
                packet_sizes.push(current_packet_size);
                packets.push(current_packet);
            }

            for (i, fragments) in packets.into_iter().enumerate() {
                let seq = self.congestion_controller.get_seq_and_increment();

                let mut packet = Datagram::Packet {
                    is_packet_pair: false,
                    is_continuous_send: true,
                    // is_continuous_send: if i == 0 { is_continuous_send } else { true },
                    needs_arrival_rate,
                    seq,
                    fragments,
                };

                self.congestion_controller.on_send_packet(packet_sizes[i]);

                self.send_datagram(reply_buf, socket, addr, &mut packet)
                    .await?;

                if let Datagram::Packet { seq, fragments, .. } = packet {
                    self.retransmit_cache.insert(
                        seq,
                        fragments
                            .into_iter()
                            .filter(|f| f.reliability.is_reliable())
                            .collect(),
                        now,
                    )?;
                }
            }

            self.bandwidth_exceeded = !self.outgoing_queue.is_empty();
        }

        Ok(())
    }

    pub fn handle_datagram(&mut self, now: u64, mut data: Bytes) -> Result<(), LayerError> {
        let size = data.len();
        let datagram = data.try_get_datagram()?;

        self.time_last_datagram_recved = now;

        match datagram {
            Datagram::Ack { arrival_rate, seqs } => {
                self.handle_ack(now, arrival_rate, seqs)?;
            }
            Datagram::Nak { seqs } => {
                self.hanlde_nak(seqs)?;
            }
            #[allow(unused_variables)]
            Datagram::Packet {
                // is_packet_pair is not used in original RakNet.
                is_packet_pair,
                is_continuous_send,
                needs_arrival_rate,
                seq,
                fragments,
            } => {
                self.handle_packet(
                    now,
                    size,
                    is_continuous_send,
                    needs_arrival_rate,
                    seq,
                    fragments,
                )?;
            }
        }

        Ok(())
    }

    fn handle_ack(
        &mut self,
        now: u64,
        arrival_rate: Option<f32>,
        seqs: Vec<u32>,
    ) -> Result<(), LayerError> {
        for seq in seqs {
            if let Some((fragments, time_sent)) = self.retransmit_cache.mark_as_used(seq) {
                let rtt = now - time_sent;
                self.congestion_controller.on_recv_ack(
                    now,
                    rtt,
                    arrival_rate,
                    self.total_bytes_acked,
                    seq,
                    self.bandwidth_exceeded,
                );

                self.total_bytes_acked +=
                    fragments.iter().map(|f| f.calc_full_size()).sum::<usize>();
            }
        }

        self.retransmit_cache.shift();

        Ok(())
    }

    fn hanlde_nak(&mut self, seqs: Vec<u32>) -> Result<(), LayerError> {
        for seq in seqs {
            if let Some((fragments, _)) = self.retransmit_cache.mark_as_used(seq) {
                self.congestion_controller.on_recv_nak();
                self.retransmit_cache.extend_retransmit_fragments(fragments);
            }
        }

        self.retransmit_cache.shift();

        Ok(())
    }

    fn handle_packet(
        &mut self,
        now: u64,
        size: usize,
        is_continuous_send: bool,
        needs_arrival_rate: bool,
        seq: u32,
        fragments: Vec<Fragment>,
    ) -> Result<(), LayerError> {
        self.seq_window.insert(seq, now)?;

        if self.seq_window.shift() == 0 {
            let missing = self
                .seq_window
                .collect_missing(now, self.congestion_controller.get_rto());
            self.naks.extend(missing);
        }

        self.congestion_controller
            .on_recv_packet(now, size, is_continuous_send);
        self.needs_arrival_rate = needs_arrival_rate;
        self.acks.push(seq);

        for fragment in fragments {
            self.handle_fragment(fragment)?;
        }

        Ok(())
    }

    fn handle_fragment(&mut self, mut fragment: Fragment) -> Result<(), LayerError> {
        if let Some(reliable_index) = fragment.reliable_index {
            if !self.reliable_window.insert(reliable_index, ())? {
                return Ok(());
            }
            self.reliable_window.shift();
        }

        if fragment.split_info.is_some() {
            if let Some(split_fragment) = self.build_split_fragment(fragment) {
                fragment = split_fragment;
            } else {
                return Ok(());
            }
        }

        if let Some(ordering_info) = fragment.ordering_info {
            let channel = ordering_info.channel as usize;
            let current_order_index = self.recv_ordering_index[channel];

            match ordering_info.index.cmp(&current_order_index) {
                Ordering::Less => {
                    return Ok(());
                }
                Ordering::Greater => {
                    let index = ordering_info.index - current_order_index - 1;
                    let channel_queue = &mut self.ordered_fragments[channel];

                    if channel_queue.len() > index as usize {
                        channel_queue[index as usize] = Some(fragment.data);
                    } else {
                        channel_queue.resize_with(index as usize + 1, || None);
                        channel_queue[index as usize] = Some(fragment.data);
                    }
                }
                Ordering::Equal => match fragment.sequence_index {
                    Some(sequence_index) if sequence_index >= self.recv_sequence_index[channel] => {
                        self.recv_sequence_index[channel] = sequence_index + 1;
                        self.output_queue.push(fragment.data);
                    }
                    None => {
                        self.recv_ordering_index[channel] += 1;
                        self.recv_sequence_index[channel] = 0;
                        self.output_queue.push(fragment.data);

                        while let Some(Some(data)) = self.ordered_fragments[channel]
                            .front_mut()
                            .map(|v| v.take())
                        {
                            self.ordered_fragments[channel].pop_front();
                            self.recv_ordering_index[channel] += 1;
                            self.output_queue.push(data);
                        }
                    }
                    _ => {}
                },
            }

            return Ok(());
        }

        self.output_queue.push(fragment.data);

        Ok(())
    }

    fn build_split_fragment(&mut self, mut fragment: Fragment) -> Option<Fragment> {
        let split_info = fragment.split_info.as_ref().unwrap();

        if split_info.index >= split_info.size {
            return None;
        }

        if let Some(fragments) = self.split_fragments.get_mut(&split_info.id) {
            fragments[split_info.index as usize] = Some(fragment.data);

            if fragments.iter().all(|f| f.is_some()) {
                let mut data = BytesMut::new();
                for fragment in fragments.iter() {
                    data.extend(fragment.as_ref().unwrap());
                }
                fragment.data = data.freeze();
                self.split_fragments.remove(&split_info.id);
                return Some(fragment);
            }
        } else {
            let mut fragments = vec![None; split_info.size as usize];
            fragments[split_info.index as usize] = Some(fragment.data);
            self.split_fragments.insert(split_info.id, fragments);
        }

        None
    }

    async fn send_datagram(
        &mut self,
        reply_buf: &mut BytesMut,
        socket: &UdpSocket,
        addr: &SocketAddr,
        datagram: &mut Datagram,
    ) -> Result<(), LayerError> {
        reply_buf.clear();
        reply_buf.put_datagram(datagram);
        socket.send_to(reply_buf, addr).await?;
        Ok(())
    }

    pub fn recv(&mut self) -> Vec<Bytes> {
        std::mem::take(&mut self.output_queue)
    }

    pub fn send(
        &mut self,
        priority: Priority,
        reliability: Reliability,
        ordering_channel: u8,
        data: Bytes,
    ) {
        let mut fragment = Fragment {
            reliability,
            reliable_index: None,
            sequence_index: None,
            ordering_info: None,
            split_info: None,
            data,
        };

        if fragment.reliability.is_sequenced() {
            fragment.sequence_index = Some(self.send_sequence_index);
            self.send_sequence_index += 1;
            fragment.ordering_info = Some(OrderingInfo {
                index: self.send_ordering_index[ordering_channel as usize],
                channel: ordering_channel,
            });
        } else if fragment.reliability.is_ordered() {
            self.send_sequence_index = 0;
            fragment.ordering_info = Some(OrderingInfo {
                index: self.send_ordering_index[ordering_channel as usize],
                channel: ordering_channel,
            });
            self.send_ordering_index[ordering_channel as usize] += 1;
        }

        let size = fragment.calc_full_size();

        if size > self.max_size_of_packet {
            fragment.reliability.reliablize();
            self.split_fragment(priority, fragment);
        } else {
            let weight = self.get_next_weight(priority);
            self.outgoing_queue.push(Reverse(OutgoingFragment {
                weight,
                priority,
                fragment,
            }));
        }
    }

    fn split_fragment(&mut self, priority: Priority, mut base_fragment: Fragment) {
        let mut data = std::mem::take(&mut base_fragment.data);
        let allowed_size =
            self.max_size_of_packet - base_fragment.calc_header_size() - Fragment::SPLIT_INFO_SIZE;
        let split_size = (data.len() - 1) / allowed_size + 1;

        for i in 0..split_size {
            let mut fragment = base_fragment.clone();

            let split_info = SplitInfo {
                size: split_size as u32,
                id: self.send_split_id,
                index: i as u32,
            };

            let data = data.split_to(data.len().min(allowed_size));
            fragment.split_info = Some(split_info);
            fragment.data = data;

            let weight = self.get_next_weight(priority);
            self.outgoing_queue.push(Reverse(OutgoingFragment {
                weight,
                priority,
                fragment,
            }));
        }

        self.send_split_id += 1;
    }

    fn priority_factor(p: usize) -> usize {
        (1usize << p).wrapping_mul(p) + p
    }

    fn init_next_weight(&mut self) {
        for i in 0..Priority::COUNT {
            self.outgoing_next_weight[i] = (1 << i) * i + i;
        }
    }

    fn get_next_weight(&mut self, priority: Priority) -> usize {
        let priority_idx = priority as usize;

        if self.outgoing_queue.is_empty() {
            self.init_next_weight();
            return self.outgoing_next_weight[priority_idx];
        }

        let current_weight = self.outgoing_next_weight[priority_idx];
        let peek = &self.outgoing_queue.peek().unwrap().0;

        let min_weight = peek.weight - Self::priority_factor(peek.priority as usize);

        let next_weight = if current_weight < min_weight {
            min_weight + Self::priority_factor(priority_idx)
        } else {
            current_weight
        };

        self.outgoing_next_weight[priority_idx] =
            next_weight + Self::priority_factor(priority_idx + 1);
        next_weight
    }

    pub fn is_retransmit_cache_empty(&self) -> bool {
        self.retransmit_cache.is_empty()
    }
}
