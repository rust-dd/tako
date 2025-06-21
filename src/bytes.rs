/// This module provides the `TakoBytes` struct, which is a wrapper around the `Bytes` type
/// from the `bytes` crate. It includes implementations for converting from `Bytes` and `String`.
use bytes::Bytes;

/// The `TakoBytes` struct is a simple wrapper around the `Bytes` type, providing
/// additional conversion capabilities.
///
/// # Example
///
/// ```rust
/// use bytes::Bytes;
/// use tako::bytes::TakoBytes;
///
/// let bytes = Bytes::from("example");
/// let tako_bytes = TakoBytes::from(bytes);
/// ```
pub struct TakoBytes(pub Bytes);

/// Converts a `Bytes` instance into a `TakoBytes` instance.
impl From<Bytes> for TakoBytes {
    fn from(b: Bytes) -> Self {
        TakoBytes(b)
    }
}

/// Converts a `String` instance into a `TakoBytes` instance by first converting
/// the string into `Bytes`.
impl From<String> for TakoBytes {
    fn from(s: String) -> Self {
        TakoBytes(Bytes::from(s))
    }
}
