use std::{error::Error, fmt, io};

#[derive(Debug)]
pub enum LoraWanError {
    InvalidPacketType(u8),
    Io(io::Error),
}

impl fmt::Display for LoraWanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoraWanError::InvalidPacketType(v) => write!(f, "Invalid packet type: {:#02x}", v),
            LoraWanError::Io(err) => err.fmt(f),
        }
    }
}

impl Error for LoraWanError {}

impl From<io::Error> for LoraWanError {
    fn from(err: io::Error) -> Self {
        LoraWanError::Io(err)
    }
}
