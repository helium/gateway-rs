use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};

pub trait Base64 {
    fn to_b64url(&self) -> String
    where
        Self: AsRef<[u8]>,
    {
        URL_SAFE_NO_PAD.encode(self.as_ref())
    }

    fn to_b64(&self) -> String
    where
        Self: AsRef<[u8]>,
    {
        STANDARD.encode(self.as_ref())
    }
}

impl Base64 for &[u8] {}
impl Base64 for Vec<u8> {}
