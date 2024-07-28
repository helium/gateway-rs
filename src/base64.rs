use base64::{engine::general_purpose::STANDARD, Engine};

pub trait Base64 {
    fn to_b64(&self) -> String
    where
        Self: AsRef<[u8]>,
    {
        STANDARD.encode(self.as_ref())
    }
}

impl Base64 for &[u8] {}
impl Base64 for Vec<u8> {}
