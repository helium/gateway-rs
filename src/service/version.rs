use serde::Serialize;
use std::fmt;

#[derive(Debug, Serialize)]
pub struct GatewayVersion {
    major: u16,
    minor: u16,
    patch: u16,
}

impl From<u64> for GatewayVersion {
    fn from(v: u64) -> Self {
        let patch = (v % 10000) as u16;
        let minor = ((v / 10000) % 1000) as u16;
        let major = ((v / 10_000_000) % 1000) as u16;
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for GatewayVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("{}.{}.{}", self.major, self.minor, self.patch))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn version() {
        let version = GatewayVersion::from(10110000u64);
        assert_eq!("1.11.0", version.to_string());
    }
}
