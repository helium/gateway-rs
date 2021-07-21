use serde::{de, Deserialize, Deserializer};

pub use helium_proto::Region;

pub fn deserialize<'de, D>(d: D) -> std::result::Result<Region, D::Error>
where
    D: Deserializer<'de>,
{
    let region = match String::deserialize(d)?.as_str() {
        "US915" => Region::Us915,
        "EU868" => Region::Eu868,
        "EU433" => Region::Eu433,
        "CN470" => Region::Cn470,
        "CN779" => Region::Cn779,
        "AU915" => Region::Au915,
        "AS923_1" => Region::As9231,
        "AS923_2" => Region::As9232,
        "AS923_3" => Region::As9233,
        "AS923_4" => Region::As9234,
        "KR920" => Region::Kr920,
        "IN865" => Region::In865,
        unsupported => {
            return Err(de::Error::custom(format!(
                "unsupported region: {}",
                unsupported
            )))
        }
    };
    Ok(region)
}
