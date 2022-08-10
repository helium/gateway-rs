use crate::{KeyedUri, PublicKey, Result};
use slog::{warn, Logger};
use std::{convert::TryFrom, sync::Arc};

#[derive(Clone, Debug)]
pub struct Routing {
    pub(crate) uris: Vec<KeyedUri>,
}

impl Routing {
    pub fn contains_uri(&self, uri: &KeyedUri) -> bool {
        self.uris.iter().any(|keyed_uri| keyed_uri == uri)
    }

    pub fn from_proto(logger: &Logger, r: &helium_proto::Routing) -> Result<Self> {
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
        Ok(Self { uris })
    }
}
