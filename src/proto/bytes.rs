use std::net::SocketAddr;

use bytes::{Buf, BufMut, Bytes};

use crate::{error::RukBytesError, reliability::fragment::Reliability};

use super::datagram::{Datagram, Fragment, OrderingInfo, SplitInfo};

pub trait RukBuf {
    fn try_advance(&mut self, n: usize) -> Result<(), RukBytesError>;
    fn try_get_u24_le(&mut self) -> Result<u32, RukBytesError>;
    fn try_get_string(&mut self) -> Result<String, RukBytesError>;
    fn try_get_addr(&mut self) -> Result<SocketAddr, RukBytesError>;
    fn try_get_datagram(&mut self) -> Result<Datagram, RukBytesError>;
    fn try_get_seqs(&mut self) -> Result<Vec<u32>, RukBytesError>;
    fn try_get_fragment(&mut self) -> Result<Fragment, RukBytesError>;
}

impl RukBuf for Bytes {
    fn try_advance(&mut self, n: usize) -> Result<(), RukBytesError> {
        if self.remaining() < n {
            return Err(RukBytesError::InvalidData("Not enough data".to_string()));
        }
        self.advance(n);
        Ok(())
    }

    fn try_get_u24_le(&mut self) -> Result<u32, RukBytesError> {
        Ok(self.try_get_uint_le(3).map(|v| v as u32)?)
    }

    fn try_get_string(&mut self) -> Result<String, RukBytesError> {
        let len = self.try_get_u16()? as usize;
        if self.remaining() < len {
            return Err(RukBytesError::InvalidData("Not enough data".to_string()));
        }
        let data = self.split_to(len);
        Ok(String::from_utf8_lossy(&data).to_string())
    }

    fn try_get_addr(&mut self) -> Result<SocketAddr, RukBytesError> {
        match self.try_get_u8()? {
            4 => {
                let mut ip = [0u8; 4];
                for e in &mut ip {
                    *e = self.try_get_u8()?;
                }
                let port = self.try_get_u16()?;
                Ok(SocketAddr::new(ip.into(), port))
            }
            6 => {
                self.try_get_u16()?;
                let port = self.try_get_u16()?;
                self.try_get_u32()?;
                let mut ip = [0u8; 16];
                for e in &mut ip {
                    *e = self.try_get_u8()?;
                }
                self.try_get_u32()?;
                Ok(SocketAddr::new(ip.into(), port))
            }
            _ => Err(RukBytesError::InvalidData(
                "Invalid address type".to_string(),
            )),
        }
    }

    fn try_get_datagram(&mut self) -> Result<Datagram, RukBytesError> {
        let bit_flags = self.try_get_u8()?;

        let is_valid = bit_flags & Datagram::IS_VALID != 0;
        if !is_valid {
            return Err(RukBytesError::InvalidData("Invalid datagram".to_string()));
        }

        let is_ack = bit_flags & Datagram::IS_ACK != 0;
        if is_ack {
            let has_arrival_rate = bit_flags & Datagram::HAS_ARRIVAL_RATE != 0;
            let arrival_rate = if has_arrival_rate {
                Some(self.try_get_f32()?)
            } else {
                None
            };
            let seqs = self.try_get_seqs()?;
            return Ok(Datagram::Ack { arrival_rate, seqs });
        }

        let is_nak = bit_flags & Datagram::IS_NAK != 0;
        if is_nak {
            let seqs = self.try_get_seqs()?;
            return Ok(Datagram::Nak { seqs });
        }

        let is_packet_pair = bit_flags & Datagram::IS_PACKET_PAIR != 0;
        let is_continuous_send = bit_flags & Datagram::IS_CONTINUOUS_SEND != 0;
        let needs_arrival_rate = bit_flags & Datagram::NEEDS_ARRIVAL_RATE != 0;
        let seq = self.try_get_u24_le()?;
        let mut fragments = vec![];
        while self.has_remaining() {
            fragments.push(self.try_get_fragment()?);
        }

        Ok(Datagram::Packet {
            is_packet_pair,
            is_continuous_send,
            needs_arrival_rate,
            seq,
            fragments,
        })
    }

    fn try_get_seqs(&mut self) -> Result<Vec<u32>, RukBytesError> {
        let size = self.try_get_u16()?;

        let mut seqs = vec![];
        for _ in 0..size {
            match self.try_get_u8()? {
                0 => {
                    let start = self.try_get_u24_le()?;
                    let end = self.try_get_u24_le()?;
                    seqs.extend(start..=end);
                }
                1 => {
                    let seq = self.try_get_u24_le()?;
                    seqs.push(seq);
                }
                _ => return Err(RukBytesError::InvalidData("Invalid seq type".to_string())),
            }
        }

        Ok(seqs)
    }

    fn try_get_fragment(&mut self) -> Result<Fragment, RukBytesError> {
        let bit_flags = self.try_get_u8()?;
        let reliability = Reliability::from_u8((bit_flags & Fragment::RELIABILITY_MASK) >> 5)
            .ok_or(RukBytesError::InvalidData(
                "Invalid reliability".to_string(),
            ))?;
        let is_split = bit_flags & Fragment::IS_SPLIT != 0;
        let len = self.try_get_u16()? / 8;

        let reliable_index = if reliability.is_reliable() {
            Some(self.try_get_u24_le()?)
        } else {
            None
        };
        let sequence_index = if reliability.is_sequenced() {
            Some(self.try_get_u24_le()?)
        } else {
            None
        };
        let ordering_info = if reliability.is_sequenced() || reliability.is_ordered() {
            Some(OrderingInfo {
                index: self.try_get_u24_le()?,
                channel: self.try_get_u8()?,
            })
        } else {
            None
        };
        let split_info = if is_split {
            Some(SplitInfo {
                size: self.try_get_u32()?,
                id: self.try_get_u16()?,
                index: self.try_get_u32()?,
            })
        } else {
            None
        };

        if self.remaining() < len as usize {
            return Err(RukBytesError::InvalidData("Not enough data".to_string()));
        }

        let data = self.split_to(len as usize);

        Ok(Fragment {
            reliability,
            reliable_index,
            sequence_index,
            ordering_info,
            split_info,
            data,
        })
    }
}

pub trait RukBufMut: BufMut {
    fn put_u24_le(&mut self, n: u32) {
        self.put_uint_le(n as u64, 3);
    }

    fn put_string(&mut self, s: &String) {
        self.put_u16(s.len() as u16);
        self.put_slice(s.as_bytes());
    }

    fn put_addr(&mut self, addr: SocketAddr) {
        match addr {
            SocketAddr::V4(addr) => {
                self.put_u8(4);
                self.put_slice(&addr.ip().octets());
                self.put_u16(addr.port());
            }
            SocketAddr::V6(addr) => {
                self.put_u8(6);
                self.put_u16(0);
                self.put_u16(addr.port());
                self.put_u32(0);
                self.put_slice(&addr.ip().octets());
                self.put_u32(0);
            }
        }
    }

    fn put_datagram(&mut self, datagram: &mut Datagram) {
        let mut bit_flags = Datagram::IS_VALID;
        match datagram {
            Datagram::Ack { arrival_rate, seqs } => {
                bit_flags |= Datagram::IS_ACK;
                if arrival_rate.is_some() {
                    bit_flags |= Datagram::HAS_ARRIVAL_RATE;
                }
                self.put_u8(bit_flags);
                if let Some(arrival_rate) = arrival_rate {
                    self.put_f32(*arrival_rate);
                }
                self.put_seqs(seqs);
            }
            Datagram::Nak { seqs } => {
                bit_flags |= Datagram::IS_NAK;
                self.put_u8(bit_flags);
                self.put_seqs(seqs);
            }
            Datagram::Packet {
                is_packet_pair,
                is_continuous_send,
                needs_arrival_rate,
                seq,
                fragments,
            } => {
                if *is_packet_pair {
                    bit_flags |= Datagram::IS_PACKET_PAIR;
                }
                if *is_continuous_send {
                    bit_flags |= Datagram::IS_CONTINUOUS_SEND;
                }
                if *needs_arrival_rate {
                    bit_flags |= Datagram::NEEDS_ARRIVAL_RATE;
                }
                self.put_u8(bit_flags);
                self.put_u24_le(*seq);
                for fragment in fragments {
                    self.put_fragment(fragment);
                }
            }
        }
    }

    fn put_seqs(&mut self, seqs: &mut Vec<u32>) {
        seqs.sort_unstable();

        let mut ranges = vec![];
        let mut start = seqs[0];
        let mut end = seqs[0];
        for seq in seqs.iter().skip(1) {
            if *seq == end + 1 {
                end = *seq
            } else {
                ranges.push((start, end));
                start = *seq;
                end = *seq;
            }
        }
        ranges.push((start, end));

        self.put_u16(ranges.len() as u16);
        for (start, end) in ranges {
            if start == end {
                self.put_u8(1);
                self.put_u24_le(start);
            } else {
                self.put_u8(0);
                self.put_u24_le(start);
                self.put_u24_le(end);
            }
        }
    }

    fn put_fragment(&mut self, fragment: &Fragment) {
        let mut bit_flags = 0;
        bit_flags |= (fragment.reliability.to_u8()) << 5;
        if fragment.split_info.is_some() {
            bit_flags |= Fragment::IS_SPLIT;
        }
        self.put_u8(bit_flags);
        self.put_u16((fragment.data.len() * 8) as u16);

        if let Some(reliable_index) = fragment.reliable_index {
            self.put_u24_le(reliable_index);
        }
        if let Some(sequence_index) = fragment.sequence_index {
            self.put_u24_le(sequence_index);
        }
        if let Some(ordering_info) = &fragment.ordering_info {
            self.put_u24_le(ordering_info.index);
            self.put_u8(ordering_info.channel);
        }
        if let Some(split_info) = &fragment.split_info {
            self.put_u32(split_info.size);
            self.put_u16(split_info.id);
            self.put_u32(split_info.index);
        }

        self.put_slice(&fragment.data);
    }
}

impl<T: BufMut> RukBufMut for T {}
