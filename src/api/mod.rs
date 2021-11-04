mod client;
mod server;

mod service {
    pub const LISTEN_ADDR: &str = "127.0.0.1:4467";
    pub const CONNECT_URI: &str = "http://127.0.0.1:4467";

    pub use helium_proto::services::local::{
        Api, Client, PubkeyReq, PubkeyRes, Server, SignReq, SignRes,
    };
}

pub use client::GatewayClient;
pub use server::GatewayServer;
