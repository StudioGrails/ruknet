use super::CongestionController;

const SYN: u64 = 10;

const MAX_RTO: u64 = 2000;
const ADDITIONAL_VAR: f64 = 30.0;
const U: f64 = 2.0;
const Q: f64 = 4.0;
const D: f64 = 0.05;

#[derive(Debug)]
pub struct SlidingWindow {
    cwnd: usize,
    ss_thresh: usize,
    mtu_size_excluding_header: usize,
    next_seq: u32,
    last_rtt: u64,
    estimated_rtt: f64,
    deviation_rtt: f64,
    oldest_unsent_ack: u64,
    next_congestion_control_block: u32,
    backoff_this_block: bool,
    speedup_this_block: bool,
    is_continuous_send: bool,
}

impl CongestionController for SlidingWindow {
    fn new(mtu_size_excluding_header: usize) -> Self {
        Self {
            cwnd: mtu_size_excluding_header,
            ss_thresh: 0,
            mtu_size_excluding_header,
            next_seq: 0,
            last_rtt: 0,
            estimated_rtt: 50.0,
            deviation_rtt: 6.25,
            oldest_unsent_ack: 0,
            next_congestion_control_block: 0,
            backoff_this_block: false,
            speedup_this_block: false,
            is_continuous_send: false,
        }
    }

    fn should_send_ack(&self, now: u64, _time_since_last_update: u64) -> bool {
        now - self.oldest_unsent_ack >= SYN
    }

    fn is_in_slow_start(&self) -> bool {
        self.cwnd <= self.ss_thresh || self.ss_thresh == 0
    }

    fn get_seq_and_increment(&mut self) -> u32 {
        let current = self.next_seq;
        self.next_seq += 1;
        current
    }

    fn get_transmission_bandwidth(
        &mut self,
        _now: u64,
        _time_since_last_update: u64,
        is_continuous_send: bool,
    ) -> usize {
        self.is_continuous_send = is_continuous_send;
        self.cwnd
    }

    fn get_rto(&self) -> u64 {
        ((U * self.estimated_rtt + Q * self.deviation_rtt + ADDITIONAL_VAR).round() as u64)
            .min(MAX_RTO)
    }

    fn on_resend(&mut self, _now: u64) {
        if self.is_continuous_send
            && !self.backoff_this_block
            && self.cwnd > self.mtu_size_excluding_header * 2
        {
            let mut ss_thresh = self.cwnd / 2;
            if ss_thresh < self.mtu_size_excluding_header {
                ss_thresh = self.mtu_size_excluding_header;
            }
            self.ss_thresh = ss_thresh;
            self.cwnd = self.mtu_size_excluding_header;
            self.next_congestion_control_block = self.next_seq;
            self.backoff_this_block = true;
        }
    }

    fn on_recv_ack(
        &mut self,
        _now: u64,
        rtt: u64,
        _arrival_rate: Option<f32>,
        _total_bytes_acked: usize,
        seq: u32,
        is_continuous_send: bool,
    ) {
        self.last_rtt = rtt;

        let diff = rtt as f64 - self.estimated_rtt;

        self.estimated_rtt = (self.estimated_rtt + D * diff).max(0.0);
        self.deviation_rtt = (self.deviation_rtt + D * (diff.abs() - self.deviation_rtt)).max(0.0);

        self.is_continuous_send = is_continuous_send;

        if is_continuous_send {
            let is_new_congestion_control_period = seq > self.next_congestion_control_block;

            if is_new_congestion_control_period {
                self.backoff_this_block = false;
                self.speedup_this_block = true;
                self.next_congestion_control_block = self.next_seq;
            }

            if self.is_in_slow_start() {
                let mut cwnd = self.cwnd + self.mtu_size_excluding_header;
                if cwnd > self.ss_thresh && self.ss_thresh != 0 {
                    cwnd = self.ss_thresh + (self.mtu_size_excluding_header.pow(2) / cwnd);
                }
                self.cwnd = cwnd;
            } else if is_new_congestion_control_period {
                self.cwnd = self.mtu_size_excluding_header.pow(2) / self.cwnd;
            }
        }
    }

    fn on_recv_nak(&mut self) {
        if self.is_continuous_send && !self.backoff_this_block {
            self.ss_thresh = self.cwnd / 2;
        }
    }

    fn on_recv_packet(&mut self, now: u64, _size: usize, _is_continuous_send: bool) {
        if self.oldest_unsent_ack == 0 {
            self.oldest_unsent_ack = now;
        }
    }

    fn on_send_ack(&mut self, _needs_arrival_rate: bool, _arrival_rate: &mut Option<f32>) {}

    fn on_send_packet(&mut self, _size: usize) {}
}
