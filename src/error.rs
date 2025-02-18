use bytes::TryGetError;

#[derive(Debug)]
pub enum RukBytesError {
    TryGet(bytes::TryGetError),
    Utf8(std::str::Utf8Error),
    FromUtf8(std::string::FromUtf8Error),
    InvalidData(String),
}

#[derive(Debug)]
pub enum PeerError {
    RukBytes(RukBytesError),
    InvalidAddress,
    FailedToBind(std::io::Error),
    AlreadyListening,
    AlreadyConnecting,
    InvalidPacket(String),
    FailedToSend(std::io::Error),
}

#[derive(Debug)]
pub enum LayerError {
    RukBytes(RukBytesError),
    BufferOverflow(BufferOverflowError),
    FailedToSend(std::io::Error),
}

#[derive(Debug)]
pub struct BufferOverflowError;

impl From<bytes::TryGetError> for RukBytesError {
    fn from(e: bytes::TryGetError) -> Self {
        RukBytesError::TryGet(e)
    }
}

impl From<std::str::Utf8Error> for RukBytesError {
    fn from(e: std::str::Utf8Error) -> Self {
        RukBytesError::Utf8(e)
    }
}

impl From<std::string::FromUtf8Error> for RukBytesError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        RukBytesError::FromUtf8(e)
    }
}

impl From<TryGetError> for PeerError {
    fn from(e: TryGetError) -> Self {
        PeerError::RukBytes(RukBytesError::TryGet(e))
    }
}

impl From<RukBytesError> for PeerError {
    fn from(e: RukBytesError) -> Self {
        PeerError::RukBytes(e)
    }
}

impl From<std::io::Error> for PeerError {
    fn from(e: std::io::Error) -> Self {
        PeerError::FailedToSend(e)
    }
}

impl From<RukBytesError> for LayerError {
    fn from(e: RukBytesError) -> Self {
        LayerError::RukBytes(e)
    }
}

impl From<BufferOverflowError> for LayerError {
    fn from(e: BufferOverflowError) -> Self {
        LayerError::BufferOverflow(e)
    }
}

impl From<std::io::Error> for LayerError {
    fn from(e: std::io::Error) -> Self {
        LayerError::FailedToSend(e)
    }
}

impl std::fmt::Display for RukBytesError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            RukBytesError::TryGet(e) => write!(f, "TryGet error: {}", e),
            RukBytesError::Utf8(e) => write!(f, "Utf8 error: {}", e),
            RukBytesError::FromUtf8(e) => write!(f, "FromUtf8 error: {}", e),
            RukBytesError::InvalidData(e) => write!(f, "Invalid data: {}", e),
        }
    }
}

impl std::fmt::Display for PeerError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            PeerError::RukBytes(e) => write!(f, "RukBytes error: {}", e),
            PeerError::InvalidAddress => write!(f, "Invalid address"),
            PeerError::FailedToBind(e) => write!(f, "Failed to bind: {}", e),
            PeerError::AlreadyListening => write!(f, "Already listening"),
            PeerError::AlreadyConnecting => write!(f, "Already connecting"),
            PeerError::InvalidPacket(e) => write!(f, "Invalid packet: {}", e),
            PeerError::FailedToSend(e) => write!(f, "Failed to send: {}", e),
        }
    }
}

impl std::fmt::Display for LayerError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            LayerError::RukBytes(e) => write!(f, "RukBytes error: {}", e),
            LayerError::BufferOverflow(e) => write!(f, "Buffer overflow: {}", e),
            LayerError::FailedToSend(e) => write!(f, "Failed to send: {}", e),
        }
    }
}

impl std::fmt::Display for BufferOverflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Buffer overflow")
    }
}

impl std::error::Error for RukBytesError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RukBytesError::TryGet(e) => Some(e),
            RukBytesError::Utf8(e) => Some(e),
            RukBytesError::FromUtf8(e) => Some(e),
            RukBytesError::InvalidData(_) => None,
        }
    }
}

impl std::error::Error for PeerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PeerError::RukBytes(e) => Some(e),
            PeerError::InvalidAddress => None,
            PeerError::FailedToBind(e) => Some(e),
            PeerError::AlreadyListening => None,
            PeerError::AlreadyConnecting => None,
            PeerError::InvalidPacket(_) => None,
            PeerError::FailedToSend(e) => Some(e),
        }
    }
}

impl std::error::Error for LayerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LayerError::RukBytes(e) => Some(e),
            LayerError::BufferOverflow(e) => Some(e),
            LayerError::FailedToSend(e) => Some(e),
        }
    }
}

impl std::error::Error for BufferOverflowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}
