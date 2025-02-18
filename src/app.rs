use std::net::SocketAddr;

#[derive(Debug)]
pub struct RukMessage {
    pub addr: SocketAddr,
    pub guid: u64,
    pub context: RukMessageContext,
}

#[derive(Debug)]
pub enum RukMessageContext {
    UnconnectedPing,
    UnconnectedPong { ping: u64, ping_res: String },
    IncompatibleProtocolVersion { remote_proto_version: u8 },
    ConnectionAttemptFailed,
    AlreadyConnected,
    NoFreeIncomingConnections,
    ConnectionBanned,
    IpRecentlyConnected,
    ConnectionRequestAccepted,
    NewIncomingConnection,
    App { data: Vec<u8> },
}
