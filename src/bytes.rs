use bytes::Bytes;

pub struct TakoBytes(pub Bytes);

impl From<Bytes> for TakoBytes {
    fn from(b: Bytes) -> Self {
        TakoBytes(b)
    }
}

impl From<String> for TakoBytes {
    fn from(s: String) -> Self {
        TakoBytes(Bytes::from(s))
    }
}
