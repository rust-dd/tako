//! `ConnectInfo<T>` extractor — typed access to per-connection transport data.
//!
//! `T` may be `SocketAddr` (peer address from any IP transport),
//! `tako_core::conn_info::ConnInfo` (full transport snapshot), or any other
//! type that implements [`FromConnInfo`](crate::connect_info::FromConnInfo).

use std::net::SocketAddr;

use http::StatusCode;
use http::request::Parts;
use tako_core::conn_info::ConnInfo;
use tako_core::conn_info::PeerAddr;
use tako_core::extractors::FromRequest;
use tako_core::extractors::FromRequestParts;
use tako_core::responder::Responder;
use tako_core::types::Request;

/// Trait derived from `ConnInfo` to produce the `T` exposed by `ConnectInfo<T>`.
pub trait FromConnInfo: Sized {
  /// Build the typed view from the connection metadata, or `None` if unsupported.
  fn from_conn_info(info: &ConnInfo) -> Option<Self>;
}

impl FromConnInfo for ConnInfo {
  fn from_conn_info(info: &ConnInfo) -> Option<Self> {
    Some(info.clone())
  }
}

impl FromConnInfo for SocketAddr {
  fn from_conn_info(info: &ConnInfo) -> Option<Self> {
    match &info.peer {
      PeerAddr::Ip(addr) => Some(*addr),
      _ => None,
    }
  }
}

impl FromConnInfo for PeerAddr {
  fn from_conn_info(info: &ConnInfo) -> Option<Self> {
    Some(info.peer.clone())
  }
}

/// Extracts a typed view of the per-connection metadata.
pub struct ConnectInfo<T>(pub T);

/// Rejection emitted when no `ConnInfo` is on the request, or the conversion fails.
#[derive(Debug)]
pub struct ConnectInfoMissing;

impl Responder for ConnectInfoMissing {
  fn into_response(self) -> tako_core::types::Response {
    (
      StatusCode::INTERNAL_SERVER_ERROR,
      "connection info unavailable for this request",
    )
      .into_response()
  }
}

fn extract<T: FromConnInfo>(ext: &http::Extensions) -> Result<T, ConnectInfoMissing> {
  let info = ext.get::<ConnInfo>().ok_or(ConnectInfoMissing)?;
  T::from_conn_info(info).ok_or(ConnectInfoMissing)
}

impl<'a, T> FromRequest<'a> for ConnectInfo<T>
where
  T: FromConnInfo + Send + 'a,
{
  type Error = ConnectInfoMissing;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(extract::<T>(req.extensions()).map(ConnectInfo))
  }
}

impl<'a, T> FromRequestParts<'a> for ConnectInfo<T>
where
  T: FromConnInfo + Send + 'a,
{
  type Error = ConnectInfoMissing;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(extract::<T>(&parts.extensions).map(ConnectInfo))
  }
}
