use std::fmt;
use std::ops::Deref;

use bytes::Bytes;

/// UTF-8 text backed by [`Bytes`], cheap to clone.
#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct Utf8Bytes(Bytes);

impl Utf8Bytes {
  /// Wraps a static string without copying.
  pub const fn from_static(text: &'static str) -> Self {
    Self(Bytes::from_static(text.as_bytes()))
  }

  pub fn as_str(&self) -> &str {
    // SAFETY: every constructor checks or guarantees UTF-8.
    unsafe { std::str::from_utf8_unchecked(&self.0) }
  }

  /// # Safety
  /// `bytes` must be valid UTF-8.
  pub(crate) unsafe fn from_bytes_unchecked(bytes: Bytes) -> Self {
    Self(bytes)
  }
}

impl Deref for Utf8Bytes {
  type Target = str;

  fn deref(&self) -> &str {
    self.as_str()
  }
}

impl AsRef<str> for Utf8Bytes {
  fn as_ref(&self) -> &str {
    self.as_str()
  }
}

impl AsRef<[u8]> for Utf8Bytes {
  fn as_ref(&self) -> &[u8] {
    &self.0
  }
}

impl fmt::Display for Utf8Bytes {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(self.as_str())
  }
}

impl fmt::Debug for Utf8Bytes {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    fmt::Debug::fmt(self.as_str(), f)
  }
}

impl From<String> for Utf8Bytes {
  fn from(text: String) -> Self {
    Self(Bytes::from(text))
  }
}

impl From<&'static str> for Utf8Bytes {
  fn from(text: &'static str) -> Self {
    Self::from_static(text)
  }
}

impl TryFrom<Bytes> for Utf8Bytes {
  type Error = std::str::Utf8Error;

  fn try_from(bytes: Bytes) -> Result<Self, Self::Error> {
    std::str::from_utf8(&bytes)?;
    Ok(Self(bytes))
  }
}

impl From<Utf8Bytes> for Bytes {
  fn from(text: Utf8Bytes) -> Self {
    text.0
  }
}

impl PartialEq<str> for Utf8Bytes {
  fn eq(&self, other: &str) -> bool {
    self.as_str() == other
  }
}

impl PartialEq<&str> for Utf8Bytes {
  fn eq(&self, other: &&str) -> bool {
    self.as_str() == *other
  }
}

/// A close status code (RFC 6455, section 7.4).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CloseCode(u16);

impl CloseCode {
  /// 1000: the purpose of the connection was fulfilled.
  pub const NORMAL: Self = Self(1000);
  /// 1001: the endpoint is going away.
  pub const GOING_AWAY: Self = Self(1001);
  /// 1002: the peer broke the protocol.
  pub const PROTOCOL_ERROR: Self = Self(1002);
  /// 1003: the endpoint cannot accept this kind of data.
  pub const UNSUPPORTED: Self = Self(1003);
  /// 1007: the payload did not match the message type, such as invalid UTF-8.
  pub const INVALID_PAYLOAD: Self = Self(1007);
  /// 1008: the message violated a policy.
  pub const POLICY: Self = Self(1008);
  /// 1009: the message was too big to process.
  pub const TOO_BIG: Self = Self(1009);
  /// 1010: the client expected an extension the server did not negotiate.
  pub const EXTENSION: Self = Self(1010);
  /// 1011: the server hit an unexpected condition.
  pub const INTERNAL: Self = Self(1011);

  pub const fn new(code: u16) -> Self {
    Self(code)
  }

  pub const fn as_u16(self) -> u16 {
    self.0
  }

  /// Whether a peer may send this code: 1000-1003, 1007-1014 and 3000-4999.
  pub const fn is_allowed(self) -> bool {
    matches!(self.0, 1000..=1003 | 1007..=1014 | 3000..=4999)
  }
}

impl fmt::Display for CloseCode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}", self.0)
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseFrame {
  pub code: CloseCode,
  /// UTF-8 reason for closing, at most 123 bytes.
  pub reason: Utf8Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
  Text(Utf8Bytes),
  Binary(Bytes),
  /// tako answers it with a pong while the socket is read.
  Ping(Bytes),
  Pong(Bytes),
  /// A close frame, with the peer's code and reason when it sent them.
  Close(Option<CloseFrame>),
}

impl Message {
  pub fn text(text: impl Into<Utf8Bytes>) -> Self {
    Self::Text(text.into())
  }

  pub fn binary(data: impl Into<Bytes>) -> Self {
    Self::Binary(data.into())
  }

  pub fn is_text(&self) -> bool {
    matches!(self, Self::Text(_))
  }

  pub fn is_binary(&self) -> bool {
    matches!(self, Self::Binary(_))
  }

  pub fn is_ping(&self) -> bool {
    matches!(self, Self::Ping(_))
  }

  pub fn is_pong(&self) -> bool {
    matches!(self, Self::Pong(_))
  }

  pub fn is_close(&self) -> bool {
    matches!(self, Self::Close(_))
  }

  /// The payload: text as bytes, or the close reason.
  pub fn into_data(self) -> Bytes {
    match self {
      Self::Text(text) => text.into(),
      Self::Binary(data) | Self::Ping(data) | Self::Pong(data) => data,
      Self::Close(Some(frame)) => frame.reason.into(),
      Self::Close(None) => Bytes::new(),
    }
  }
}

impl From<String> for Message {
  fn from(text: String) -> Self {
    Self::Text(text.into())
  }
}

impl From<&'static str> for Message {
  fn from(text: &'static str) -> Self {
    Self::Text(text.into())
  }
}

impl From<Bytes> for Message {
  fn from(data: Bytes) -> Self {
    Self::Binary(data)
  }
}

impl From<Vec<u8>> for Message {
  fn from(data: Vec<u8>) -> Self {
    Self::Binary(data.into())
  }
}

#[cfg(test)]
mod tests {
  use bytes::Bytes;

  use super::CloseCode;
  use super::CloseFrame;
  use super::Message;
  use super::Utf8Bytes;

  #[test]
  fn utf8_bytes_round_trip_through_bytes() {
    let text = Utf8Bytes::from_static("héllo");
    assert_eq!(text.as_str(), "héllo");
    let bytes = Bytes::from(text.clone());
    assert_eq!(Utf8Bytes::try_from(bytes).unwrap(), text);
    assert_eq!(Utf8Bytes::from(String::from("héllo")), "héllo");
  }

  #[test]
  fn utf8_bytes_reject_invalid_input() {
    assert!(Utf8Bytes::try_from(Bytes::from_static(&[0xff])).is_err());
  }

  #[test]
  fn only_codes_a_peer_may_send_are_allowed() {
    let allowed = [1000, 1001, 1002, 1003, 1007, 1011, 1013, 1014, 3000, 4999];
    let rejected = [0, 999, 1004, 1005, 1006, 1015, 1016, 2999, 5000];
    for code in allowed {
      assert!(CloseCode::new(code).is_allowed(), "{code}");
    }
    for code in rejected {
      assert!(!CloseCode::new(code).is_allowed(), "{code}");
    }
  }

  #[test]
  fn into_data_returns_each_payload() {
    assert_eq!(Message::text("hi").into_data(), "hi");
    assert_eq!(
      Message::binary(vec![1, 2]).into_data(),
      Bytes::from_static(&[1, 2])
    );
    assert_eq!(Message::Ping(Bytes::from_static(b"p")).into_data(), "p");
    assert_eq!(Message::Pong(Bytes::from_static(b"q")).into_data(), "q");
    let close = CloseFrame {
      code: CloseCode::NORMAL,
      reason: Utf8Bytes::from_static("bye"),
    };
    assert_eq!(Message::Close(Some(close)).into_data(), "bye");
    assert!(Message::Close(None).into_data().is_empty());
  }

  #[test]
  fn kind_predicates_match_the_variant() {
    assert!(Message::from("x").is_text());
    assert!(Message::from(vec![0u8]).is_binary());
    assert!(Message::Ping(Bytes::new()).is_ping());
    assert!(Message::Pong(Bytes::new()).is_pong());
    assert!(Message::Close(None).is_close());
  }
}
