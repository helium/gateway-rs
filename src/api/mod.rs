mod client;
mod server;

const LISTEN_ADDR: &str = "127.0.0.1";

pub use client::LocalClient;
pub use helium_proto::{
    services::local::{AddGatewayReq, AddGatewayRes, PubkeyReq, PubkeyRes, RegionReq, RegionRes},
    GatewayStakingMode,
};
pub use server::LocalServer;

pub fn listen_addr(port: u16) -> String {
    format!("{LISTEN_ADDR}:{port}")
}

pub fn connect_uri(port: u16) -> String {
    let listen_addr = listen_addr(port);
    format!("http://{listen_addr}")
}
