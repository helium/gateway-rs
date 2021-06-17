use crate::*;
use helium_proto::routing_information::Data as RoutingData;
use router::filter::{DevAddrFilter, EuiFilter};
use service::router::Service as RouterService;

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

    pub fn from_proto(r: &helium_proto::Routing) -> Result<Self> {
        let filters = r.filters.iter().map(|f| EuiFilter::from_bin(f)).collect();
        let subnets = r
            .subnets
            .iter()
            .map(|s| DevAddrFilter::from_bin(s))
            .collect();
        let mut clients = vec![];
        for address in r.addresses.iter() {
            let uri = String::from_utf8_lossy(&address.uri).parse()?;
            clients.push(RouterService::new(uri, None)?);
        }
        Ok(Self {
            filters,
            subnets,
            clients,
        })
    }
}
