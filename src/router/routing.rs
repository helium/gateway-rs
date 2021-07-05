use crate::*;
use helium_proto::routing_information::Data as RoutingData;
use router::filter::{DevAddrFilter, EuiFilter};
use service::router::Service as RouterService;
use slog::{warn, Logger};

pub struct Routing {
    pub(crate) filters: Vec<EuiFilter>,
    pub(crate) subnets: Vec<DevAddrFilter>,
    pub(crate) clients: Vec<RouterService>,
}

impl Routing {
    pub fn matches_routing_data(&self, routing_data: &RoutingData) -> bool {
        match routing_data {
            RoutingData::Eui(eui) => self.filters.iter().any(|filter| filter.contains(eui)),
            RoutingData::Devaddr(dev_addr) => {
                self.subnets.iter().any(|filter| filter.contains(dev_addr))
            }
        }
    }

    pub fn from_proto(logger: &Logger, r: &helium_proto::Routing) -> Result<Self> {
        let filters = r.filters.iter().map(|f| EuiFilter::from_bin(f)).collect();
        let subnets = r
            .subnets
            .iter()
            .map(|s| DevAddrFilter::from_bin(s))
            .collect();
        let mut clients = vec![];
        for address in r.addresses.iter().filter(|a| !a.uri.is_empty()) {
            let uri_str = String::from_utf8_lossy(&address.uri);
            match uri_str.parse() {
                Ok(uri) => clients.push(RouterService::new(uri, None)?),
                Err(err) => warn!(
                    logger,
                    "ignoring invalid uri: \"{}\": {:?}", uri_str, err;
                    "oui" => r.oui
                ),
            }
        }
        Ok(Self {
            filters,
            subnets,
            clients,
        })
    }
}
