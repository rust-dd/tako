//! WebSocket (RFC 6455) server connections.

mod driver;
mod error;
mod frame;
mod handshake;
mod mask;
mod message;
mod protocol;
mod receiver;
mod runtime;
mod socket;
#[cfg(test)]
mod testing;
mod upgrade;
mod utf8;
mod wake;

pub use error::Error;
pub use error::ProtocolError;
#[cfg(all(feature = "async-graphql", not(feature = "compio")))]
pub(crate) use handshake::accept;
#[cfg(all(feature = "async-graphql", not(feature = "compio")))]
pub(crate) use handshake::rejection;
pub use message::CloseCode;
pub use message::CloseFrame;
pub use message::Message;
pub use message::Utf8Bytes;
pub use socket::WebSocket;
pub use upgrade::WebSocketUpgrade;
#[cfg(all(feature = "async-graphql", not(feature = "compio")))]
pub(crate) use upgrade::spawn_session;

/// Feeds `data` to the frame decoder in two pieces, as a fuzz target does.
#[cfg(fuzzing)]
#[doc(hidden)]
pub fn fuzz_decode(data: &[u8]) {
  let limits = receiver::Limits {
    max_frame_size: 1 << 16,
    max_message_size: 1 << 18,
  };
  let mut protocol = protocol::Protocol::new(limits);
  let mut input = bytes::BytesMut::new();
  let mut output = bytes::BytesMut::new();
  let (head, tail) = data.split_at(data.len() / 2);
  for piece in [head, tail] {
    input.extend_from_slice(piece);
    loop {
      match protocol.decode(&mut input) {
        Ok(Some(_)) => {
          protocol.drain_pong(&mut output);
          protocol.drain_close(&mut output);
        }
        Ok(None) => break,
        Err(error) => {
          protocol.fail(error);
          protocol.drain_close(&mut output);
          return;
        }
      }
    }
  }
}
