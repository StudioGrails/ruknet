#[derive(Debug)]
pub enum OfflineMessageIDs {
    UnconnectedPing,
    UnconnectedPong,
    OpenConnectionRequest1,
    OpenConnectionReply1,
    OpenConnectionRequest2,
    OpenConnectionReply2,
    AlreadyConnected,
    NoFreeIncomingConnections,
    ConnectionBanned,
    IncompatibleProtocolVersion,
    IpRecentlyConnected,
}

impl OfflineMessageIDs {
    pub fn from_u8(id: u8) -> Option<Self> {
        match id {
            0x01 => Some(Self::UnconnectedPing),
            0x1c => Some(Self::UnconnectedPong),
            0x05 => Some(Self::OpenConnectionRequest1),
            0x06 => Some(Self::OpenConnectionReply1),
            0x07 => Some(Self::OpenConnectionRequest2),
            0x08 => Some(Self::OpenConnectionReply2),
            0x12 => Some(Self::AlreadyConnected),
            0x14 => Some(Self::NoFreeIncomingConnections),
            0x17 => Some(Self::ConnectionBanned),
            0x19 => Some(Self::IncompatibleProtocolVersion),
            0x1a => Some(Self::IpRecentlyConnected),
            _ => None,
        }
    }

    pub fn to_u8(&self) -> u8 {
        match self {
            Self::UnconnectedPing => 0x01,
            Self::UnconnectedPong => 0x1c,
            Self::OpenConnectionRequest1 => 0x05,
            Self::OpenConnectionReply1 => 0x06,
            Self::OpenConnectionRequest2 => 0x07,
            Self::OpenConnectionReply2 => 0x08,
            Self::AlreadyConnected => 0x12,
            Self::NoFreeIncomingConnections => 0x14,
            Self::ConnectionBanned => 0x17,
            Self::IncompatibleProtocolVersion => 0x19,
            Self::IpRecentlyConnected => 0x1a,
        }
    }
}

#[derive(Debug)]
pub enum OnlineMessageIDs {
    ConnectedPing,
    ConnectedPong,
    ConnectionRequest,
    ConnectionRequestAccepted,
    NewIncomingConnection,
    DisconnectionNotification,
}

impl OnlineMessageIDs {
    pub fn from_u8(id: u8) -> Option<Self> {
        match id {
            0x00 => Some(Self::ConnectedPing),
            0x03 => Some(Self::ConnectedPong),
            0x09 => Some(Self::ConnectionRequest),
            0x10 => Some(Self::ConnectionRequestAccepted),
            0x13 => Some(Self::NewIncomingConnection),
            0x15 => Some(Self::DisconnectionNotification),
            _ => None,
        }
    }

    pub fn to_u8(&self) -> u8 {
        match self {
            Self::ConnectedPing => 0x00,
            Self::ConnectedPong => 0x03,
            Self::ConnectionRequest => 0x09,
            Self::ConnectionRequestAccepted => 0x10,
            Self::NewIncomingConnection => 0x13,
            Self::DisconnectionNotification => 0x15,
        }
    }
}
