use std::net::SocketAddr;

use crate::reliability::{congestion_controller, layer::Layer};

#[derive(Debug)]
pub struct ReqConn {
    pub addr: SocketAddr,
    pub next_req_time: u64,
    pub req_count: usize,
    pub max_req_count: usize,
    pub req_interval: u64,
    pub timeout: Option<u64>,
}

impl ReqConn {
    pub fn new(
        now: u64,
        addr: SocketAddr,
        req_interval: u64,
        max_req_count: usize,
        timeout: Option<u64>,
    ) -> Self {
        Self {
            addr,
            next_req_time: now,
            req_count: 0,
            max_req_count,
            req_interval,
            timeout,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConnectMode {
    UnverfiedSender,
    RequestedConnection,
    HandlingConnectionRequest,
    Connected,
}

#[derive(Debug)]
pub struct Conn {
    pub guid: u64,
    pub create_time: u64,
    pub next_ping_time: u64,
    pub ping: u64,
    pub connect_mode: ConnectMode,
    #[cfg(feature = "udt")]
    pub layer: Layer<congestion_controller::udt::UDT>,
    #[cfg(not(feature = "udt"))]
    pub layer: Layer<congestion_controller::sliding_window::SlidingWindow>,
    pub last_reliable_send: u64,
}

impl Conn {
    pub fn new(now: u64, guid: u64, connect_mode: ConnectMode, mtu_size: usize) -> Self {
        Self {
            guid,
            create_time: now,
            next_ping_time: now,
            ping: 0,
            connect_mode,
            layer: Layer::new(now, mtu_size),
            last_reliable_send: now,
        }
    }
}
