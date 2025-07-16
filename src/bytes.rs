//! Byte stream and buffer manipulation utilities for efficient data handling.
//!
//! This module provides `TakoBytes`, a wrapper around the `Bytes` type that offers
//! additional conversion capabilities and integrates seamlessly with Tako's type system.
//! `TakoBytes` enables efficient handling of byte data in web applications, particularly
//! for request/response bodies, streaming data, and Server-Sent Events without
//! unnecessary allocations or copies.
//!
//! # Examples
//!
//! ```rust
//! use tako::bytes::TakoBytes;
//! use bytes::Bytes;
//!
//! // Create from string
//! let from_string = TakoBytes::from("Hello, World!".to_string());
//!
//! // Create from bytes
//! let original_bytes = Bytes::from_static(b"Binary data");
//! let tako_bytes = TakoBytes::from(original_bytes);
//!
//! // Access inner bytes
//! let inner: &Bytes = &tako_bytes.0;
//! assert_eq!(inner.len(), 11);
//! ```

use bytes::Bytes;

/// Efficient byte buffer wrapper with enhanced conversion capabilities.
///
/// `TakoBytes` wraps the `Bytes` type from the bytes crate, providing a unified
/// interface for handling byte data throughout the Tako framework. It offers
/// zero-copy conversions from various sources while maintaining the performance
/// characteristics of the underlying `Bytes` implementation.
///
/// # Examples
///
/// ```rust
/// use tako::bytes::TakoBytes;
/// use bytes::Bytes;
///
/// // Create from static data
/// let static_data = Bytes::from_static(b"Static content");
/// let tako_bytes = TakoBytes::from(static_data);
///
/// // Create from owned string
/// let message = "Dynamic content".to_string();
/// let dynamic_bytes = TakoBytes::from(message);
///
/// // Access the underlying bytes
/// assert_eq!(tako_bytes.0.len(), 14);
/// assert_eq!(dynamic_bytes.0.len(), 15);
/// ```
pub struct TakoBytes(pub Bytes);

/// Converts `Bytes` into `TakoBytes` with zero-copy efficiency.
///
/// This conversion wraps the existing `Bytes` instance without copying the
/// underlying data, maintaining the performance benefits of the bytes crate's
/// reference-counted implementation.
///
/// # Examples
///
/// ```rust
/// use tako::bytes::TakoBytes;
/// use bytes::Bytes;
///
/// let original = Bytes::from_static(b"Hello, bytes!");
/// let wrapped = TakoBytes::from(original);
///
/// assert_eq!(wrapped.0.len(), 14);
/// assert_eq!(&wrapped.0[..], b"Hello, bytes!");
/// ```
impl From<Bytes> for TakoBytes {
    fn from(b: Bytes) -> Self {
        TakoBytes(b)
    }
}

/// Converts `String` into `TakoBytes` by consuming the string data.
///
/// This conversion takes ownership of the string and converts it into bytes
/// efficiently. The string's buffer is reused when possible, avoiding
/// unnecessary allocations.
///
/// # Examples
///
/// ```rust
/// use tako::bytes::TakoBytes;
///
/// let message = "Hello, Tako!".to_string();
/// let bytes = TakoBytes::from(message);
///
/// assert_eq!(bytes.0.len(), 12);
/// assert_eq!(&bytes.0[..], b"Hello, Tako!");
///
/// // Demonstrate with formatted string
/// let formatted = format!("User ID: {}", 42);
/// let formatted_bytes = TakoBytes::from(formatted);
/// assert_eq!(&formatted_bytes.0[..], b"User ID: 42");
/// ```
impl From<String> for TakoBytes {
    fn from(s: String) -> Self {
        TakoBytes(Bytes::from(s))
    }
}
