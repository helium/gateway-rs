const URL_SAFE_ENGINE: base64::engine::fast_portable::FastPortable =
    base64::engine::fast_portable::FastPortable::from(
        &base64::alphabet::URL_SAFE,
        base64::engine::fast_portable::NO_PAD,
    );
pub trait Base64 {
    fn to_b64url(&self) -> String
    where
        Self: AsRef<[u8]>,
    {
        base64::encode_engine(self.as_ref(), &URL_SAFE_ENGINE)
    }

    fn to_b64(&self) -> String
    where
        Self: AsRef<[u8]>,
    {
        base64::encode(self.as_ref())
    }
}

impl Base64 for &[u8] {}
impl Base64 for Vec<u8> {}
