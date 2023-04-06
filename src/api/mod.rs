mod client;
mod server;

pub use client::LocalClient;
pub use helium_proto::{
    services::local::{
        AddGatewayReq, AddGatewayRes, PubkeyReq, PubkeyRes, RegionReq, RegionRes, RouterReq,
        RouterRes,
    },
    GatewayStakingMode,
};
pub use server::LocalServer;

use crate::{Error, Result};

impl TryFrom<RouterRes> for crate::packet_router::RouterStatus {
    type Error = Error;
    fn try_from(value: RouterRes) -> Result<Self> {
        use std::str::FromStr;
        Ok(Self {
            uri: http::Uri::from_str(&value.uri)?,
            connected: value.connected,
        })
    }
}
