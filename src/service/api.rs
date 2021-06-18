use crate::{curl, Result};
use helium_crypto::Network;
use serde::{de::DeserializeOwned, Deserialize};

#[derive(Clone, Deserialize, Debug)]
pub(crate) struct Data<T> {
    pub data: T,
}

#[derive(Debug, Clone)]
pub(crate) struct Service {
    pub base_uri: http::Uri,
}

const DEFAULT_TESTNET_BASE_URL: &str = "https://testnet-api.helium.wtf/v1";
const DEFAULT_BASE_URL: &str = "https://api.helium.io/v1";

impl Service {
    pub fn blockchain(network: Network) -> Self {
        let base_uri = match network {
            Network::MainNet => DEFAULT_BASE_URL,
            Network::TestNet => DEFAULT_TESTNET_BASE_URL,
        };
        Self::new(base_uri.parse::<http::Uri>().unwrap())
    }

    pub fn new(base_uri: http::Uri) -> Self {
        Self { base_uri }
    }

    pub async fn get<T>(&self, path: &str) -> Result<T>
    where
        T: 'static + DeserializeOwned + std::marker::Send,
    {
        let request_url = format!("{}{}", self.base_uri, path);

        let result = curl::get(
            request_url,
            &["-H", "Accept: application/json"],
            move |output| {
                let data: Data<T> = serde_json::from_slice(output)?;
                Ok(data)
            },
        )
        .await?;
        Ok(result.data)
    }
}
