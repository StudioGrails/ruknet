use std::{cmp::Ordering, collections::VecDeque};

use super::CongestionController;

const SYN: u64 = 10;

const MIN_CWND: usize = 2;
const MAX_CWND: usize = 512;

const DEFAULT_TRANSFER_RATE: f32 = 0.0036;
const DEFAULT_BYTE_INTERVAL: f32 = 1.0 / DEFAULT_TRANSFER_RATE;

const PACKET_ARRIVAL_HISTORY_SIZE: usize = 64;

const INTERVAL_SIZE: usize = 33;

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug)]
pub struct UDT {
    cwnd: usize,
    mtu_size_excluding_header: usize,
    next_seq: u32,
    arrival_rate: Option<f32>,
    snd: f32,
    bytes_can_send_this_tick: isize,
    packet_arrival_history_index: usize,
    packet_arrival_history: [f32; PACKET_ARRIVAL_HISTORY_SIZE],
    pings_last_interval: VecDeque<u64>,
    last_packet_arrival_time: u64,
    last_rtt_on_increase_send_rate: u64,
    oldest_unsent_ack: u64,
    next_congestion_control_block: u32,
    had_packet_loss_this_block: bool,
    is_in_slow_start: bool,
}

impl CongestionController for UDT {
    fn new(mtu_size_excluding_header: usize) -> Self {
        Self {
            cwnd: MIN_CWND,
            mtu_size_excluding_header,
            next_seq: 0,
            arrival_rate: Some(DEFAULT_TRANSFER_RATE),
            snd: DEFAULT_BYTE_INTERVAL,
            bytes_can_send_this_tick: 0,
            packet_arrival_history_index: 0,
            packet_arrival_history: [0.0; PACKET_ARRIVAL_HISTORY_SIZE],
            pings_last_interval: VecDeque::new(),
            last_packet_arrival_time: 0,
            last_rtt_on_increase_send_rate: 1000,
            oldest_unsent_ack: 0,
            next_congestion_control_block: 0,
            had_packet_loss_this_block: false,
            is_in_slow_start: true,
        }
    }

    fn should_send_ack(&self, now: u64, time_since_last_update: u64) -> bool {
        now - self.oldest_unsent_ack >= SYN
            || now + time_since_last_update < self.oldest_unsent_ack + SYN
    }

    fn is_in_slow_start(&self) -> bool {
        self.is_in_slow_start
    }

    fn get_seq_and_increment(&mut self) -> u32 {
        let current = self.next_seq;
        self.next_seq += 1;
        current
    }

    fn get_transmission_bandwidth(
        &mut self,
        _now: u64,
        mut time_since_last_update: u64,
        is_continuous_send: bool,
    ) -> usize {
        if self.is_in_slow_start {
            self.cwnd * self.mtu_size_excluding_header
        } else {
            if self.bytes_can_send_this_tick < 0 {
                self.bytes_can_send_this_tick = 0;
            }

            if !is_continuous_send && time_since_last_update > 100 {
                time_since_last_update = 100;
            }

            self.bytes_can_send_this_tick +=
                time_since_last_update as isize * (1.0 / self.snd) as isize;

            if self.bytes_can_send_this_tick < 0 {
                self.bytes_can_send_this_tick as usize
            } else {
                0
            }
        }
    }

    fn get_rto(&self) -> u64 {
        (self.last_rtt_on_increase_send_rate * 2).clamp(100, 10000)
    }

    fn on_resend(&mut self, _now: u64) {
        if self.is_in_slow_start && self.arrival_rate.is_some() {
            self.end_slow_start();
        } else if !self.had_packet_loss_this_block {
            self.increase_time_between_sends();
            self.had_packet_loss_this_block = true;
        }
    }

    fn on_recv_ack(
        &mut self,
        now: u64,
        rtt: u64,
        arrival_rate: Option<f32>,
        total_bytes_acked: usize,
        seq: u32,
        is_continuous_send: bool,
    ) {
        if let Some(arrival_rate) = arrival_rate {
            self.arrival_rate = Some(arrival_rate);
        }

        if self.oldest_unsent_ack == 0 {
            self.oldest_unsent_ack = now;
        }

        if self.is_in_slow_start {
            self.next_congestion_control_block = seq;
            self.last_rtt_on_increase_send_rate = rtt;
            self.update_window_size_slow_start(total_bytes_acked);
        } else {
            self.update_window_size_syn(rtt, seq, is_continuous_send);
        }
    }

    fn on_recv_nak(&mut self) {
        if self.is_in_slow_start && self.arrival_rate.is_some() {
            self.end_slow_start();
        } else if !self.had_packet_loss_this_block {
            self.increase_time_between_sends();
            self.had_packet_loss_this_block = true;
        }
    }

    fn on_recv_packet(&mut self, now: u64, size: usize, is_continuous_send: bool) {
        if now > self.last_packet_arrival_time {
            let interval = now - self.last_packet_arrival_time;

            if is_continuous_send {
                self.packet_arrival_history
                    [self.packet_arrival_history_index & (PACKET_ARRIVAL_HISTORY_SIZE - 1)] =
                    size as f32 / interval as f32;
                self.packet_arrival_history_index += 1;
            }

            self.last_packet_arrival_time = now;
        }
    }

    fn on_send_ack(&mut self, needs_arrival_rate: bool, arrival_rate: &mut Option<f32>) {
        self.oldest_unsent_ack = 0;

        if needs_arrival_rate {
            *arrival_rate = self.calc_arrival_rate();
        }
    }

    fn on_send_packet(&mut self, size: usize) {
        if !self.is_in_slow_start {
            self.bytes_can_send_this_tick -= size as isize;
        }
    }
}

impl UDT {
    fn end_slow_start(&mut self) {
        self.is_in_slow_start = false;
        self.snd = 1.0 / self.arrival_rate.unwrap();
        self.cap_min_snd();
    }

    fn increase_time_between_sends(&mut self) {
        let increment = 0.02 * (self.snd + 1.0).powf(2.0) / 501.0f32.powf(2.0);
        self.snd *= 1.02 - increment;
        self.cap_min_snd();
    }

    fn decrease_time_between_sends(&mut self) {
        let increment = 0.01 * (self.snd + 1.0).powf(2.0) / 501.0f32.powf(2.0);
        self.snd *= 0.99 + increment;
    }

    fn cap_min_snd(&mut self) {
        if self.snd > 500.0 {
            self.snd = 500.0;
        }
    }

    fn calc_arrival_rate(&self) -> Option<f32> {
        if self.packet_arrival_history_index < PACKET_ARRIVAL_HISTORY_SIZE {
            return None;
        }

        let median = self.find_median_arrival_rate();
        let one_eighth_median = median / 8.0;
        let eight_times_median = median * 8.0;
        let mut sum = 0.0;
        let mut count = 0;

        for i in 0..PACKET_ARRIVAL_HISTORY_SIZE {
            if self.packet_arrival_history[i] >= one_eighth_median
                && self.packet_arrival_history[i] < eight_times_median
            {
                sum += self.packet_arrival_history[i];
                count += 1;
            }
        }

        if count > 0 {
            Some(sum / count as f32)
        } else {
            None
        }
    }

    fn find_median_arrival_rate(&self) -> f32 {
        let mid_index = self.packet_arrival_history.len() / 2;
        Self::find_kth_element_recursive(&self.packet_arrival_history, mid_index)
    }

    fn find_kth_element_recursive(list: &[f32], k: usize) -> f32 {
        if list.len() == 1 {
            return list[0];
        }

        let pivot_index = list.len() / 2;
        let pivot = list[pivot_index];

        let mut less = Vec::new();
        let mut equal = Vec::new();
        let mut greater = Vec::new();

        for &e in list {
            match e.partial_cmp(&pivot).unwrap() {
                Ordering::Less => less.push(e),
                Ordering::Equal => equal.push(e),
                Ordering::Greater => greater.push(e),
            }
        }

        if k < less.len() {
            Self::find_kth_element_recursive(&less, k)
        } else if k < less.len() + equal.len() {
            pivot
        } else {
            Self::find_kth_element_recursive(&greater, k - less.len() - equal.len())
        }
    }

    fn update_window_size_slow_start(&mut self, total_bytes_acked: usize) {
        self.cwnd = total_bytes_acked / self.mtu_size_excluding_header;

        if self.cwnd < MIN_CWND {
            self.cwnd = MIN_CWND;
        } else if self.cwnd > MAX_CWND {
            self.cwnd = MAX_CWND;

            if self.arrival_rate.is_some() {
                self.end_slow_start();
            }
        }
    }

    fn update_window_size_syn(&mut self, rtt: u64, seq: u32, is_continuous_send: bool) {
        if !is_continuous_send {
            self.next_congestion_control_block = self.next_seq;
            self.pings_last_interval.clear();
            return;
        }

        self.pings_last_interval.push_back(rtt);

        if self.pings_last_interval.len() > INTERVAL_SIZE {
            self.pings_last_interval.pop_front();
        }

        if seq > self.next_congestion_control_block
            && seq - self.next_congestion_control_block >= INTERVAL_SIZE as u32
            && self.pings_last_interval.len() == INTERVAL_SIZE
        {
            let mut slope_sum = 0;
            let mut average = self.pings_last_interval[0];
            let size = self.pings_last_interval.len();

            for i in 1..size {
                slope_sum += self.pings_last_interval[i] - self.pings_last_interval[i - 1];
                average += self.pings_last_interval[i];
            }

            average /= size as u64;

            if slope_sum > average / 10 {
                self.increase_time_between_sends();
            } else {
                self.last_rtt_on_increase_send_rate = rtt;
                self.decrease_time_between_sends();
            }

            self.pings_last_interval.clear();
            self.had_packet_loss_this_block = false;
            self.next_congestion_control_block = seq;
        }
    }
}
