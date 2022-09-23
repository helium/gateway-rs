use crate::{curl, Result};
use beacon::Entropy;
use http::Uri;

#[derive(Debug)]
pub struct EntropyService {
    uri: Uri,
}

impl EntropyService {
    pub fn new(uri: Uri) -> Self {
        Self { uri }
    }

    pub async fn get_entropy(&self) -> Result<Entropy> {
        curl::get(
            self.uri.to_string(),
            ["-H", "Accept: application/json"],
            move |output| {
                let entropy: Entropy = serde_json::from_slice(output)?;
                Ok(entropy)
            },
        )
        .await
    }
}
