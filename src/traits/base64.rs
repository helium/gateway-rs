pub trait Base64 {
    fn to_b64url(&self) -> String
    where
        Self: AsRef<[u8]>,
    {
        base64::encode_config(self.as_ref(), base64::URL_SAFE_NO_PAD)
    }

    fn to_b64(&self) -> String
    where
        Self: AsRef<[u8]>,
    {
        base64::encode_config(self.as_ref(), base64::STANDARD)
    }
}

impl Base64 for &[u8] {}
impl Base64 for Vec<u8> {}
