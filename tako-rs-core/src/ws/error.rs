use std::fmt;
use std::io;

use super::CloseCode;

#[derive(Debug)]
pub enum Error {
  Io(io::Error),
  /// Invalid incoming frames queue a close reply while the connection is open, except at EOF.
  /// Invalid outgoing messages are rejected without changing the connection.
  Protocol(ProtocolError),
  /// The closing handshake has begun; nothing more can be sent.
  Closed,
}

impl fmt::Display for Error {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Io(error) => write!(f, "WebSocket transport error: {error}"),
      Self::Protocol(error) => write!(f, "WebSocket protocol error: {error}"),
      Self::Closed => f.write_str("WebSocket connection is closed"),
    }
  }
}

impl std::error::Error for Error {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::Io(error) => Some(error),
      Self::Protocol(error) => Some(error),
      Self::Closed => None,
    }
  }
}

impl From<io::Error> for Error {
  fn from(error: io::Error) -> Self {
    Self::Io(error)
  }
}

impl From<ProtocolError> for Error {
  fn from(error: ProtocolError) -> Self {
    Self::Protocol(error)
  }
}

/// A received frame or outgoing message that violates RFC 6455.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProtocolError {
  /// A reserved header bit was set; tako negotiates no extensions.
  ReservedBits,
  UnmaskedFrame,
  /// The payload length did not use the shortest encoding.
  NonMinimalLength,
  /// The high bit of a 64-bit payload length was set.
  ReservedLengthBit,
  /// The frame used a reserved opcode.
  UnknownOpcode(u8),
  /// A control frame had FIN clear.
  FragmentedControlFrame,
  /// A control frame carried more than 125 bytes.
  ControlFrameTooLarge,
  /// A continuation frame arrived with no message in progress.
  UnexpectedContinuation,
  /// A new message started before the fragmented one finished.
  ExpectedContinuation,
  /// Text or a close reason was not valid UTF-8.
  InvalidUtf8,
  /// A close frame carried a one-byte payload or an outgoing close code was invalid.
  InvalidCloseFrame,
  /// A frame announced more than `max_frame_size` bytes.
  FrameTooLarge {
    size: u64,
    max: usize,
  },
  /// A message grew past `max_message_size` bytes.
  MessageTooLarge {
    /// Message length so far.
    size: usize,
    max: usize,
  },
  /// The peer closed the transport without a close frame.
  ResetWithoutClosingHandshake,
}

impl ProtocolError {
  pub(crate) fn close_code(self) -> CloseCode {
    match self {
      Self::InvalidUtf8 => CloseCode::INVALID_PAYLOAD,
      Self::FrameTooLarge { .. } | Self::MessageTooLarge { .. } => CloseCode::TOO_BIG,
      _ => CloseCode::PROTOCOL_ERROR,
    }
  }
}

impl fmt::Display for ProtocolError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::ReservedBits => f.write_str("reserved bits set"),
      Self::UnmaskedFrame => f.write_str("client frame is not masked"),
      Self::NonMinimalLength => f.write_str("payload length is not minimally encoded"),
      Self::ReservedLengthBit => f.write_str("reserved payload-length bit is set"),
      Self::UnknownOpcode(opcode) => write!(f, "unknown opcode {opcode}"),
      Self::FragmentedControlFrame => f.write_str("fragmented control frame"),
      Self::ControlFrameTooLarge => f.write_str("control frame longer than 125 bytes"),
      Self::UnexpectedContinuation => f.write_str("continuation frame without a message"),
      Self::ExpectedContinuation => f.write_str("new message inside a fragmented one"),
      Self::InvalidUtf8 => f.write_str("invalid UTF-8"),
      Self::InvalidCloseFrame => f.write_str("invalid close frame payload or status code"),
      Self::FrameTooLarge { size, max } => {
        write!(f, "frame of {size} bytes over the {max}-byte limit")
      }
      Self::MessageTooLarge { size, max } => {
        write!(f, "message of {size} bytes over the {max}-byte limit")
      }
      Self::ResetWithoutClosingHandshake => {
        f.write_str("connection reset without a closing handshake")
      }
    }
  }
}

impl std::error::Error for ProtocolError {}

#[cfg(test)]
mod tests {
  use super::CloseCode;
  use super::ProtocolError;

  #[test]
  fn each_error_maps_to_its_close_code() {
    assert_eq!(
      ProtocolError::InvalidUtf8.close_code(),
      CloseCode::INVALID_PAYLOAD
    );
    let frame = ProtocolError::FrameTooLarge { size: 2, max: 1 };
    assert_eq!(frame.close_code(), CloseCode::TOO_BIG);
    let message = ProtocolError::MessageTooLarge { size: 2, max: 1 };
    assert_eq!(message.close_code(), CloseCode::TOO_BIG);
    assert_eq!(
      ProtocolError::ReservedBits.close_code(),
      CloseCode::PROTOCOL_ERROR
    );
    assert_eq!(
      ProtocolError::UnmaskedFrame.close_code(),
      CloseCode::PROTOCOL_ERROR
    );
  }
}
