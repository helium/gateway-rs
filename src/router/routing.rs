use super::{DevAddrFilter, EuiFilter};
use crate::{KeyedUri, PublicKey, Result};
use helium_proto::{routing_information::Data as RoutingData, RoutingInformation};
use slog::{warn, Logger};
use std::{convert::TryFrom, sync::Arc};

#[derive(Clone, Debug)]
pub struct Routing {
    pub(crate) oui: u32,
    pub(crate) uris: Vec<KeyedUri>,
    filters: Vec<EuiFilter>,
    subnets: Vec<DevAddrFilter>,
}

impl Routing {
    pub fn contains_uri(&self, uri: &KeyedUri) -> bool {
        self.uris.iter().any(|keyed_uri| keyed_uri == uri)
    }

    pub fn matches_routing_info(&self, routing_info: &Option<RoutingInformation>) -> bool {
        match routing_info {
            Some(RoutingInformation { ref data }) => self.matches_routing_data(data),
            None => false,
        }
    }

    fn matches_routing_data(&self, routing_data: &Option<RoutingData>) -> bool {
        match routing_data {
            None => false,
            Some(RoutingData::Eui(eui)) => self.filters.iter().any(|filter| filter.contains(eui)),
            Some(RoutingData::Devaddr(dev_addr)) => {
                self.subnets.iter().any(|filter| filter.contains(dev_addr))
            }
        }
    }

    pub fn from_proto(logger: &Logger, r: &helium_proto::Routing) -> Result<Self> {
        let filters = r.filters.iter().map(EuiFilter::from_bin).collect();
        let subnets = r.subnets.iter().map(DevAddrFilter::from_bin).collect();
        let oui = r.oui;
        let uris = r
            .addresses
            .iter()
            .filter_map(|address| {
                if address.uri.is_empty() {
                    return None;
                }
                let uri_str = String::from_utf8_lossy(&address.uri);
                uri_str
                    .parse()
                    .map_err(|err| {
                        warn!(
                            logger,
                            "ignoring invalid uri: \"{}\": {:?}", uri_str, err;
                            "oui" => r.oui
                        );
                    })
                    .and_then(|uri| {
                        PublicKey::try_from(address.pub_key.as_ref())
                            .map(|public_key| KeyedUri {
                                uri,
                                pubkey: Arc::new(public_key),
                            })
                            .map_err(|err| {
                                warn!(
                                    logger,
                                    "ignoring public key: {:?}", err;
                                    "oui" => r.oui
                                )
                            })
                    })
                    .ok()
            })
            .collect();
        Ok(Self {
            oui,
            filters,
            subnets,
            uris,
        })
    }
}
