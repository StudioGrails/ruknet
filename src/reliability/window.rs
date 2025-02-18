use std::fmt::Debug;

use crate::error::BufferOverflowError;

#[derive(Debug)]
pub struct Window<T: Debug, const BUFFER_SIZE: usize> {
    buf: [Option<T>; BUFFER_SIZE],
    start_index: u32,
    furthest_index: Option<u32>,
}

impl<T: Debug, const BUFFER_SIZE: usize> Window<T, BUFFER_SIZE> {
    pub fn new() -> Self {
        Self {
            buf: [const { None }; BUFFER_SIZE],
            start_index: 0,
            furthest_index: None,
        }
    }

    fn relative_index(index: u32) -> usize {
        index as usize & (BUFFER_SIZE - 1)
    }

    pub fn insert(&mut self, index: u32, value: T) -> Result<bool, BufferOverflowError> {
        if self.start_index as usize + BUFFER_SIZE <= index as usize {
            return Err(BufferOverflowError);
        }

        let relative_index = Self::relative_index(index);

        if self.buf[relative_index].is_some() {
            return Ok(false);
        }

        self.buf[relative_index] = Some(value);

        if let Some(furthest_index) = self.furthest_index {
            if index > furthest_index {
                self.furthest_index = Some(index);
            }
        } else {
            self.furthest_index = Some(index);
        }

        Ok(true)
    }

    pub fn shift(&mut self) -> usize {
        let mut count = 0;

        let range = if let Some(furthest_index) = self.furthest_index {
            (furthest_index - self.start_index + 1) as usize
        } else {
            // should never happen
            BUFFER_SIZE
        };

        for i in 0..range {
            let relative_index = Self::relative_index(self.start_index + i as u32);

            if self.buf[relative_index].is_none() {
                break;
            } else {
                self.buf[relative_index] = None;
            }

            count += 1;
        }

        if let Some(furthest_index) = self.furthest_index {
            if count > 0 && self.start_index + count as u32 - 1 == furthest_index {
                self.furthest_index = None;
            }
        }

        self.start_index += count as u32;

        count
    }
}

impl<const BUFFER_SIZE: usize> Window<u64, BUFFER_SIZE> {
    pub fn collect_missing(&mut self, now: u64, rto: u64) -> Vec<u32> {
        let mut missing = Vec::with_capacity(16);
        let mut timeout_index = None;

        let range = if let Some(furthest_index) = self.furthest_index {
            (furthest_index - self.start_index + 1) as usize
        } else {
            // should never happen
            BUFFER_SIZE
        };

        for i in (0..range).rev() {
            let relative_index = Self::relative_index(self.start_index + i as u32);

            match self.buf[relative_index] {
                Some(time_recved) => {
                    if timeout_index.is_some() {
                        self.buf[relative_index] = None;
                    } else if now - time_recved > rto {
                        timeout_index = Some(i);
                        self.buf[relative_index] = None;
                    }
                }
                None => {
                    if timeout_index.is_some() {
                        missing.push(self.start_index + i as u32);
                    }
                }
            }
        }

        if let Some(index) = timeout_index {
            self.start_index += index as u32 + 1;
        }

        missing
    }
}
