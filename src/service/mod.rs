use std::time::Duration;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const RPC_TIMEOUT: Duration = Duration::from_secs(5);

pub mod config;
pub mod entropy;
pub mod packet_router;
pub mod poc;
