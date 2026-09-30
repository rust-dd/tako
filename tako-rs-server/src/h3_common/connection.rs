use bytes::Bytes;
use h3::error::ConnectionError;
use h3::quic;
use h3::server::Connection;

/// The QUIC connection type h3 runs on for this runtime.
#[cfg(not(feature = "compio"))]
pub(crate) type QuicConnection = h3_quinn::Connection;
#[cfg(feature = "compio")]
pub(crate) type QuicConnection = compio::quic::Connection;

/// Starts HTTP/3 on an accepted QUIC connection.
///
/// With `webtransport`, the SETTINGS frame also advertises extended CONNECT,
/// HTTP/3 datagrams, and one WebTransport session per connection, which is
/// what browsers require before they open a session.
pub(crate) async fn accept<C>(conn: C) -> Result<Connection<C, Bytes>, ConnectionError>
where
  C: quic::Connection<Bytes>,
{
  #[cfg(feature = "webtransport")]
  let conn = h3::server::builder()
    .enable_webtransport(true)
    .enable_extended_connect(true)
    .enable_datagram(true)
    .max_webtransport_sessions(1)
    .send_grease(true)
    .build(conn)
    .await;
  #[cfg(not(feature = "webtransport"))]
  let conn = Connection::new(conn).await;
  conn
}
