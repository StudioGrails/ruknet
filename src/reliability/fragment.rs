#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reliability {
    Unreliable,
    UnreliableSequenced,
    Reliable,
    ReliableOrdered,
    ReliableSequenced,
    UnreliableWithAckReceipt,
    ReliableWithAckReceipt,
    ReliableOrderedWithAckReceipt,
}

impl Reliability {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Unreliable),
            1 => Some(Self::UnreliableSequenced),
            2 => Some(Self::Reliable),
            3 => Some(Self::ReliableOrdered),
            4 => Some(Self::ReliableSequenced),
            5 => Some(Self::UnreliableWithAckReceipt),
            6 => Some(Self::ReliableWithAckReceipt),
            7 => Some(Self::ReliableOrderedWithAckReceipt),
            _ => None,
        }
    }

    pub fn to_u8(&self) -> u8 {
        match self {
            Self::Unreliable => 0,
            Self::UnreliableSequenced => 1,
            Self::Reliable => 2,
            Self::ReliableOrdered => 3,
            Self::ReliableSequenced => 4,
            Self::UnreliableWithAckReceipt => 5,
            Self::ReliableWithAckReceipt => 6,
            Self::ReliableOrderedWithAckReceipt => 7,
        }
    }

    pub fn is_reliable(&self) -> bool {
        matches!(
            self,
            Self::Reliable
                | Self::ReliableOrdered
                | Self::ReliableSequenced
                | Self::ReliableWithAckReceipt
                | Self::ReliableOrderedWithAckReceipt
        )
    }

    pub fn is_sequenced(&self) -> bool {
        matches!(self, Self::UnreliableSequenced | Self::ReliableSequenced)
    }

    pub fn is_ordered(&self) -> bool {
        matches!(
            self,
            Self::ReliableOrdered | Self::ReliableOrderedWithAckReceipt
        )
    }

    pub fn reliablize(&self) -> Self {
        match self {
            Self::Unreliable => Self::Reliable,
            Self::UnreliableSequenced => Self::ReliableSequenced,
            Self::UnreliableWithAckReceipt => Self::ReliableWithAckReceipt,
            _ => *self,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Immediate,
    High,
    Medium,
    Low,
}

impl Priority {
    pub const COUNT: usize = 4;
}
