use crate::result::Result;
use foreign_types::ForeignType;
use openssl::pkey;
use openssl_sys::EVP_PKEY;
use std::{fmt, fs, path, result};

/// A Key for gateways. The key is used to identify the gateway with remote
/// connections through certs used for both authentication and encryption of
/// those connections.
#[derive(Debug)]
pub struct Key(pkey::PKey<pkey::Private>);

impl Key {
    /// Generate a new key. Keys are ed25519 for compatibility with other Helium
    /// systems.
    pub fn generate() -> Result<Self> {
        Ok(Self(pkey::PKey::generate_ed25519()?))
    }

    /// Load a key from a PEM file at a given path.
    pub fn load(path: &str) -> Result<Self> {
        let data = fs::read(path)?;
        Ok(Self::from_pem(&data)?)
    }

    /// Save the key to a given path. If a parent folder is part of the given
    /// path it is created if absent.
    pub fn save(&self, path: &str) -> Result {
        if let Some(parent) = path::PathBuf::from(path).parent() {
            fs::create_dir_all(parent)?
        };
        fs::write(path, self.to_pem()?)?;
        Ok(())
    }

    /// Convert the key to a PEM
    pub fn to_pem(&self) -> Result<Vec<u8>> {
        Ok(self.0.private_key_to_pem_pkcs8()?)
    }

    /// Read a key to a PEM
    pub fn from_pem(data: &[u8]) -> Result<Self> {
        let pkey = pkey::PKey::private_key_from_pem(data)?;
        Ok(Self(pkey))
    }

    fn get_raw_public_key(&self, dest: &mut [u8]) {
        let mut len: usize = 32;
        let _ = unsafe {
            openssl_sys::EVP_PKEY_get_raw_public_key(
                self.0.as_ptr() as *const EVP_PKEY,
                dest[..].as_mut_ptr() as *mut _,
                &mut len as *mut _,
            )
        };
    }
}

// impl string::ToString for Key {
//     fn to_string(&self) -> String {
//         // First 0 value is the "version" number defined for addresses in the
//         // classic helium addressing scheme. The '1' indicates the ed25519
//         // keytype.
//         let mut data = [0u8; 34];
//         data[0] = 0;
//         data[1] = 1;
//         self.get_raw_public_key(&mut data[2..]);
//         bs58::encode(data.as_ref()).with_check().into_string()
//     }
// }

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> result::Result<(), fmt::Error> {
        // First 0 value is the "version" number defined for addresses in the
        // classic helium addressing scheme. The '1' indicates the ed25519
        // keytype.
        let mut data = [0u8; 34];
        data[0] = 0;
        data[1] = 1;
        self.get_raw_public_key(&mut data[2..]);
        write!(
            f,
            "{}",
            bs58::encode(data.as_ref()).with_check().into_string()
        )
    }
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.0.public_eq(&other.0)
    }
}

impl Eq for Key {}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn pem() {
        let key = Key::generate().expect("key generated");
        let data = key.to_pem().expect("pem encode");
        assert_eq!(key, Key::from_pem(&data).unwrap())
    }
}
