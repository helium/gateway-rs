use crate::result::Result;
use foreign_types::ForeignType;
use openssl::{pkey, sign::Signer, sign::Verifier};
use openssl_sys::EVP_PKEY;
use std::{fmt, fs, path, result};

/// A Key for gateways. The key is used to identify the gateway with remote
/// connections through certs used for both authentication and encryption of
/// those connections.
#[derive(Debug, Clone)]
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

    /// Signs a given messase with this key.
    pub fn sign(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut signer = Signer::new_without_digest(&self.0)?;
        Ok(signer.sign_oneshot_to_vec(&data)?)
    }

    /// Verifies a given messase against a signature with this key.
    pub fn verify(&self, signature: &[u8], data: &[u8]) -> Result<bool> {
        let mut verifier = Verifier::new_without_digest(&self.0)?;
        Ok(verifier.verify_oneshot(&signature, &data)?)
    }

    pub fn to_key_bin(&self) -> Vec<u8> {
        let mut dest = vec![0u8; 34];
        dest[0] = KEY_BIN_VERSION;
        dest[1] = KEY_TYPE_ED25519;
        self.get_raw_public_key(&mut dest[2..]);
        dest
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

const KEY_BIN_VERSION: u8 = 0;
const KEY_TYPE_ED25519: u8 = 1;

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> result::Result<(), fmt::Error> {
        write!(
            f,
            "{}",
            bs58::encode(&self.to_key_bin()).with_check().into_string()
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
        let key = Key::generate().expect("key");
        let data = key.to_pem().expect("pem encode");
        assert_eq!(key, Key::from_pem(&data).unwrap())
    }

    #[test]
    fn sign() {
        let key = Key::generate().expect("key");
        let signature = key.sign(b"hello world").unwrap();
        assert!(key.verify(&signature, b"hello world").unwrap());
    }
}
