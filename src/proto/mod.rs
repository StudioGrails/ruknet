pub mod bytes;
pub mod datagram;
pub mod message;

pub const PROTOCOL_VERSION: u8 = 11;

pub const MAX_MTU_SIZE: usize = 1492;
pub const MIN_MTU_SIZE: usize = 576;
pub const IP_UDP_HEADER_SIZE: usize = 28;
pub const MTU_SIZES: [usize; 3] = [MAX_MTU_SIZE, 1200, MIN_MTU_SIZE];

pub const MIN_PACKET_SIZE: usize = MIN_ONLINE_PACKET_SIZE;
pub const MIN_ONLINE_PACKET_SIZE: usize = 6;
pub const MIN_OFFLINE_PACKET_SIZE: usize = 25;
pub const OFFLINE_MESSAGE_DATA_ID: [u8; 16] = [
    0x00, 0xFF, 0xFF, 0x00, 0xFE, 0xFE, 0xFE, 0xFE, 0xFD, 0xFD, 0xFD, 0xFD, 0x12, 0x34, 0x56, 0x78,
];

pub const DEFAULT_TIMEOUT: u64 = 10000;
