use crate::error::Result;
use ed25519_dalek::Signer;
use rand::rngs::OsRng;
use std::{fmt, fs, path};

/// A Key for gateways. The key is used to identify the gateway with remote
/// connections through certs used for both authentication and encryption of
/// those connections.
#[derive(Debug)]
pub struct Keypair(ed25519_dalek::Keypair);

#[derive(Debug)]
pub struct PublicKey(ed25519_dalek::PublicKey);

#[derive(Debug)]
pub struct Signature(ed25519_dalek::Signature);

pub use ed25519_dalek::{SECRET_KEY_LENGTH, SIGNATURE_LENGTH};
pub const KEYPAIR_LENGTH: usize = ed25519_dalek::KEYPAIR_LENGTH + 1;
pub const PUBLIC_KEY_LENGTH: usize = ed25519_dalek::PUBLIC_KEY_LENGTH + 1;
pub const KEYTYPE_ED25519: u8 = 1;

impl Keypair {
    /// Generate a new key. Keys are ed25519 for compatibility with other Helium
    /// systems.
    pub fn generate() -> Result<Self> {
        let mut csprng = OsRng {};
        let keypair = ed25519_dalek::Keypair::generate(&mut csprng);
        Ok(Self(keypair))
    }

    /// Load a key from a PEM file at a given path.
    pub fn load(path: &str) -> Result<Self> {
        let data = fs::read(path)?;
        Self::from_bytes(&data)
    }

    /// Save the key to a given path. If a parent folder is part of the given
    /// path it is created if absent.
    pub fn save(&self, path: &str) -> Result {
        if let Some(parent) = path::PathBuf::from(path).parent() {
            fs::create_dir_all(parent)?
        };
        fs::write(path, self.to_bytes())?;
        Ok(())
    }

    /// Signs a given messase with this key.
    pub fn sign(&self, data: &[u8]) -> Result<Signature> {
        Ok(Signature(self.0.sign(data)))
    }

    /// Verifies a given messase against a signature with this key.
    pub fn verify(&self, signature: &Signature, data: &[u8]) -> Result {
        Ok(self.0.verify(data, &signature.0)?)
    }

    /// Converts a keypair to a binary form. This format is compatible with
    /// the helium wallet format.
    pub fn to_bytes(&self) -> [u8; KEYPAIR_LENGTH] {
        let mut dest = [0u8; KEYPAIR_LENGTH];
        dest[0] = KEYTYPE_ED25519;
        dest[1..ed25519_dalek::SECRET_KEY_LENGTH + 1].copy_from_slice(self.0.secret.as_bytes());
        dest[ed25519_dalek::SECRET_KEY_LENGTH + 1..].copy_from_slice(self.0.public.as_bytes());
        dest
    }

    /// Constructs a keypair from a given binary slice.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Ok(Self(ed25519_dalek::Keypair::from_bytes(&bytes[1..])?))
    }

    /// Returns the public key of this keypair
    pub fn public(&self) -> PublicKey {
        PublicKey(self.0.public)
    }
}

impl fmt::Display for Keypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::result::Result<(), fmt::Error> {
        self.public().fmt(f)
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::result::Result<(), fmt::Error> {
        write!(
            f,
            "{}",
            bs58::encode(self.0.as_bytes()).with_check().into_string()
        )
    }
}

impl PublicKey {
    pub fn to_bytes(&self) -> [u8; PUBLIC_KEY_LENGTH] {
        let mut data = [0u8; PUBLIC_KEY_LENGTH];
        data[0] = KEYTYPE_ED25519;
        data[1..].copy_from_slice(self.0.as_bytes());
        data
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.to_bytes().to_vec()
    }

    pub fn to_b58(&self) -> Result<String> {
        // First 0 value is the "version" number defined for addresses
        // in libp2p, 2nd byte is keytype
        let mut data = [0u8; PUBLIC_KEY_LENGTH + 1];
        data[1] = KEYTYPE_ED25519;
        data[2..].copy_from_slice(self.0.as_bytes());
        Ok(bs58::encode(data.as_ref()).with_check().into_string())
    }
}

impl PartialEq for PublicKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl Eq for PublicKey {}

impl Signature {
    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_bytes().to_vec()
    }

    pub fn to_bytes(&self) -> [u8; SIGNATURE_LENGTH] {
        self.0.to_bytes()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn coding() {
        let key = Keypair::generate().expect("key");
        let data = key.to_bytes();
        assert_eq!(key.public(), Keypair::from_bytes(&data).unwrap().public())
    }

    #[test]
    fn sign() {
        let key = Keypair::generate().expect("key");
        let signature = key.sign(b"hello world").unwrap();
        assert!(key.verify(&signature, b"hello world").is_ok());
    }
}
