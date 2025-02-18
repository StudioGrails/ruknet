use bytes::Bytes;

use crate::reliability::fragment::Reliability;

#[derive(Debug)]
pub enum Datagram {
    Ack {
        arrival_rate: Option<f32>,
        seqs: Vec<u32>,
    },
    Nak {
        seqs: Vec<u32>,
    },
    Packet {
        is_packet_pair: bool,
        is_continuous_send: bool,
        needs_arrival_rate: bool,
        seq: u32,
        fragments: Vec<Fragment>,
    },
}

impl Datagram {
    pub const HEADER_SIZE: usize = 1;
    pub const PACKET_HEADER_SIZE: usize = 3;

    pub const IS_VALID: u8 = 0b1000_0000;
    pub const IS_ACK: u8 = 0b0100_0000;
    pub const IS_NAK: u8 = 0b0010_0000;
    pub const HAS_ARRIVAL_RATE: u8 = 0b0010_0000;
    pub const IS_PACKET_PAIR: u8 = 0b0001_0000;
    pub const IS_CONTINUOUS_SEND: u8 = 0b0000_1000;
    pub const NEEDS_ARRIVAL_RATE: u8 = 0b0000_0100;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fragment {
    pub reliability: Reliability,
    pub reliable_index: Option<u32>,
    pub sequence_index: Option<u32>,
    pub ordering_info: Option<OrderingInfo>,
    pub split_info: Option<SplitInfo>,
    pub data: Bytes,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderingInfo {
    pub index: u32,
    pub channel: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SplitInfo {
    pub size: u32,
    pub id: u16,
    pub index: u32,
}

impl Fragment {
    pub const SPLIT_INFO_SIZE: usize = 10;

    pub const RELIABILITY_MASK: u8 = 0b1110_0000;
    pub const IS_SPLIT: u8 = 0b0001_0000;

    pub fn calc_header_size(&self) -> usize {
        let mut size = 1 + 2;

        if self.reliability.is_reliable() {
            size += 3;
        }
        if self.reliability.is_sequenced() {
            size += 3;
            size += 4;
        } else if self.reliability.is_ordered() {
            size += 4;
        }
        if self.split_info.is_some() {
            size += 10;
        }

        size
    }

    pub fn calc_full_size(&self) -> usize {
        self.calc_header_size() + self.data.len()
    }
}
