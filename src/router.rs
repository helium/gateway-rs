use crate::{key, result::Result, settings::Settings};
use helium_proto::{
    blockchain_state_channel_message_v1::Msg, routing_information::Data as RoutingData,
    BlockchainStateChannelMessageV1, BlockchainStateChannelPacketV1,
    BlockchainStateChannelResponseV1, Eui, Message as ProstMessage, Packet, RoutingInformation,
};
use prost::bytes::{Buf, BufMut};
use std::time::Duration;

#[derive(Debug)]
pub struct Client(reqwest::Client);

static USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
static AGENT_ID_HEADER: &str = "x-gateway-id";
static DEV_ADDR_HEADER: &str = "x-devaddr";

#[derive(Debug, Clone)]
pub struct Message(BlockchainStateChannelMessageV1);

#[derive(Debug, Clone)]
pub struct Response(BlockchainStateChannelResponseV1);

#[derive(Debug, Clone)]
pub struct Routing(RoutingInformation);

pub use helium_proto::Region;

pub use reqwest::Url;

pub use reqwest::Certificate;

impl Client {
    pub fn new(settings: &Settings) -> Result<Self> {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut default_headers = HeaderMap::new();
        default_headers.insert(
            HeaderName::from_static(&AGENT_ID_HEADER),
            HeaderValue::from_str(&settings.key.to_string()).expect("public key not available"),
        );
        let mut builder = reqwest::Client::builder()
            .danger_accept_invalid_hostnames(true)
            .default_headers(default_headers)
            .user_agent(USER_AGENT)
            .http2_prior_knowledge();
        // .identity(Self::mk_identity(settings)?);
        for cert in &settings.root_certs {
            builder = builder.add_root_certificate(cert.clone());
        }
        Ok(Self(builder.build()?))
    }

    pub async fn send(
        &self,
        router: &reqwest::Url,
        message: &Message,
        timeout: Duration,
    ) -> Result<Option<Response>> {
        let mut encoded = vec![];
        message.encode(&mut encoded)?;
        let mut request = self.0.post(router.clone()).body(encoded).timeout(timeout);

        if let Some(dev_addr) = message.dev_addr() {
            request = request.header(DEV_ADDR_HEADER, format!("{:#04x}", dev_addr));
        }
        match request.send().await?.error_for_status() {
            Ok(response) => {
                let mut bytes = response.bytes().await?;
                Ok(Some(Response::decode(&mut bytes)?))
            }
            Err(err) => Err(err.into()),
        }
    }
}

impl Message {
    pub fn encode(&self, mut buf: &mut dyn BufMut) -> Result {
        Ok(self.0.encode(&mut buf)?)
    }

    pub fn from_packet(packet: Packet, key: &key::Key, region: Region) -> Result<Self> {
        let mut router_packet = BlockchainStateChannelPacketV1 {
            packet: Some(packet),
            signature: vec![],
            hotspot: key.to_key_bin(),
            region: region.into(),
        };
        let mut encoded = vec![];
        router_packet.encode(&mut encoded)?;
        router_packet.signature = key.sign(&encoded)?;
        let message = BlockchainStateChannelMessageV1 {
            msg: Some(Msg::Packet(router_packet)),
        };
        Ok(Self(message))
    }

    pub fn dev_addr(&self) -> Option<u32> {
        match &self.0 {
            BlockchainStateChannelMessageV1 {
                msg:
                    Some(Msg::Packet(BlockchainStateChannelPacketV1 {
                        packet:
                            Some(Packet {
                                routing:
                                    Some(RoutingInformation {
                                        data: Some(RoutingData::Devaddr(dev_addr)),
                                    }),
                                ..
                            }),
                        ..
                    })),
            } => Some(*dev_addr),
            _ => None,
        }
    }
}

impl Response {
    pub fn decode(mut buf: &mut dyn Buf) -> Result<Self> {
        Ok(Self(BlockchainStateChannelResponseV1::decode(&mut buf)?))
    }

    pub fn downlink(&self) -> Option<&Packet> {
        self.0.downlink.as_ref()
    }
}

impl Routing {
    pub fn from_data(payload: &[u8]) -> Result<Option<Self>> {
        use lorawan::{Direction, PHYPayload, PHYPayloadFrame};
        use std::io::Cursor;
        match PHYPayload::read(Direction::Uplink, &mut Cursor::new(payload)) {
            Ok(packet) => {
                let routing_data = match &packet.payload {
                    PHYPayloadFrame::JoinRequest(request) => Some(RoutingData::Eui(Eui {
                        deveui: request.dev_eui,
                        appeui: request.app_eui,
                    })),
                    PHYPayloadFrame::MACPayload(mac_payload) => {
                        Some(RoutingData::Devaddr(mac_payload.dev_addr()))
                    }
                    _ => return Ok(None),
                };
                Ok(routing_data.map(|r| Self(RoutingInformation { data: Some(r) })))
            }
            Err(err) => Err(err.into()),
        }
    }
}

impl From<Routing> for RoutingInformation {
    fn from(v: Routing) -> Self {
        v.0
    }
}
