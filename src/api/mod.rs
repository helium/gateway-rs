mod client;
mod server;

const LISTEN_ADDR: &str = "127.0.0.1:";
const URI_PREFIX: &str = "http://";

pub use client::LocalClient;
pub use helium_proto::services::local::{
    ConfigReq, ConfigRes, ConfigValue, EcdhReq, EcdhRes, HeightReq, HeightRes, PubkeyReq,
    PubkeyRes, SignReq, SignRes,
};
pub use server::LocalServer;

pub fn listen_addr(port: u16) -> String {
    let mut address = LISTEN_ADDR.to_string();
    address += &port.to_string();
    address
}

pub fn connect_uri(port: u16) -> String {
    let mut uri = URI_PREFIX.to_string();
    let addr = listen_addr(port);
    uri += &addr;
    uri
}
