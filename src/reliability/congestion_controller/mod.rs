pub mod sliding_window;
pub mod udt;

pub trait CongestionController {
    fn new(mtu_size_excluding_header: usize) -> Self;

    fn should_send_ack(&self, now: u64, time_since_last_update: u64) -> bool;

    fn is_in_slow_start(&self) -> bool;

    fn get_seq_and_increment(&mut self) -> u32;

    fn get_transmission_bandwidth(
        &mut self,
        now: u64,
        time_since_last_update: u64,
        is_continuous_send: bool,
    ) -> usize;

    fn get_rto(&self) -> u64;

    fn on_resend(&mut self, now: u64);

    fn on_recv_ack(
        &mut self,
        now: u64,
        rtt: u64,
        arrival_rate: Option<f32>,
        total_bytes_acked: usize,
        seq: u32,
        is_continuous_send: bool,
    );

    fn on_recv_nak(&mut self);

    fn on_recv_packet(&mut self, now: u64, size: usize, is_continuous_send: bool);

    fn on_send_ack(&mut self, needs_arrival_rate: bool, arrival_rate: &mut Option<f32>);

    fn on_send_packet(&mut self, size: usize);
}
