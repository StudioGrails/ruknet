use std::collections::VecDeque;

use crate::{error::BufferOverflowError, proto::datagram::Fragment};

const PACKET_HISTORY_SIZE: usize = 128;

#[derive(Debug, PartialEq)]
pub enum PacketHistory {
    Valid {
        fragments: Vec<Fragment>,
        time_sent: u64,
    },
    Invalid,
    Used,
}

#[derive(Debug)]
pub struct RetransmitCache {
    histories: [PacketHistory; PACKET_HISTORY_SIZE],
    start_seq: u32,
    retransmit_fragments: VecDeque<Fragment>,
}

impl RetransmitCache {
    fn relative_index(seq: u32) -> usize {
        seq as usize & (PACKET_HISTORY_SIZE - 1)
    }

    pub fn new() -> Self {
        Self {
            histories: [const { PacketHistory::Invalid }; PACKET_HISTORY_SIZE],
            start_seq: 0,
            retransmit_fragments: VecDeque::with_capacity(16),
        }
    }

    pub fn is_empty(&self) -> bool {
        let relative_index = Self::relative_index(self.start_seq);
        self.histories[relative_index] == PacketHistory::Invalid
            && self.retransmit_fragments.is_empty()
    }

    pub fn insert(
        &mut self,
        seq: u32,
        fragments: Vec<Fragment>,
        time_sent: u64,
    ) -> Result<(), BufferOverflowError> {
        if self.start_seq + PACKET_HISTORY_SIZE as u32 <= seq {
            return Err(BufferOverflowError);
        }

        let relative_index = Self::relative_index(seq);

        if let PacketHistory::Invalid = self.histories[relative_index] {
            self.histories[relative_index] = PacketHistory::Valid {
                fragments,
                time_sent,
            };
        }

        Ok(())
    }

    pub fn mark_as_used(&mut self, seq: u32) -> Option<(Vec<Fragment>, u64)> {
        let relative_index = Self::relative_index(seq);

        match &mut self.histories[relative_index] {
            PacketHistory::Valid {
                fragments,
                time_sent,
            } => {
                let fragments = std::mem::take(fragments);
                let time_sent = *time_sent;
                self.histories[relative_index] = PacketHistory::Used;
                Some((fragments, time_sent))
            }
            _ => None,
        }
    }

    pub fn shift(&mut self) -> usize {
        let mut count = 0;

        for i in 0..PACKET_HISTORY_SIZE {
            let relative_index = Self::relative_index(self.start_seq + i as u32);

            if let PacketHistory::Used = self.histories[relative_index] {
                self.histories[relative_index] = PacketHistory::Invalid;
                count += 1;
            } else {
                break;
            }
        }

        self.start_seq += count as u32;

        count
    }

    pub fn get_retransmit_fragments(&mut self, now: u64, rto: u64) -> &mut VecDeque<Fragment> {
        if self.is_empty() {
            return &mut self.retransmit_fragments;
        }

        let mut count = 0;

        for i in 0..PACKET_HISTORY_SIZE {
            let relative_index = Self::relative_index(self.start_seq + i as u32);

            match &mut self.histories[relative_index] {
                PacketHistory::Valid {
                    time_sent,
                    fragments,
                } => {
                    if now - *time_sent >= rto {
                        self.retransmit_fragments.extend(std::mem::take(fragments));
                        self.histories[relative_index] = PacketHistory::Invalid;
                        count += 1;
                    } else {
                        break;
                    }
                }
                PacketHistory::Invalid => break,
                PacketHistory::Used => {
                    self.histories[relative_index] = PacketHistory::Invalid;
                    count += 1;
                }
            }
        }

        self.start_seq += count as u32;

        &mut self.retransmit_fragments
    }

    pub fn extend_retransmit_fragments(&mut self, fragments: Vec<Fragment>) {
        self.retransmit_fragments.extend(fragments);
    }
}
