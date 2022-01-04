mod client;
mod server;

pub const LISTEN_ADDR: &str = "127.0.0.1:4467";
pub const CONNECT_URI: &str = "http://127.0.0.1:4467";

pub use client::LocalClient;
pub use helium_proto::services::local::{
    ConfigReq, ConfigRes, ConfigValue, EcdhReq, EcdhRes, HeightReq, HeightRes, PubkeyReq,
    PubkeyRes, SignReq, SignRes,
};
pub use server::LocalServer;
