use std::time::Duration;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const RPC_TIMEOUT: Duration = Duration::from_secs(5);

pub mod entropy;
pub mod gateway;
pub mod poc;
pub mod router;
mod version;
