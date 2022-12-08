use std::{error::Error, fmt, io};

#[derive(Debug)]
pub enum LoraWanError {
    InvalidPacketType(u8),
    InvalidFPortForFopts,
    InvalidPacketSize(super::MType, usize),
    Io(io::Error),
}

impl fmt::Display for LoraWanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoraWanError::InvalidPacketType(v) => write!(f, "Invalid packet type: {v:#02x}"),
            LoraWanError::InvalidFPortForFopts => write!(f, "Invalid: fport 0 with fopts"),
            LoraWanError::InvalidPacketSize(mtype, s) => {
                write!(f, "Invalid packet size {s} for type {mtype:?}")
            }
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
