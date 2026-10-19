//! `ConnectInfo<T>`: a typed view of the connection, for `T` = `SocketAddr`, `PeerAddr`,
//! `ConnInfo`, or any other [`FromConnInfo`] type.

use std::net::SocketAddr;

use http::StatusCode;
use http::request::Parts;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::conn_info::PeerAddr;
use tako_rs_core::extractors::Entries;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

/// A view that `ConnectInfo<T>` can build from the connection's `ConnInfo`.
pub trait FromConnInfo: Sized {
  /// `None` when the transport cannot supply this view, which rejects the request.
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

pub struct ConnectInfo<T>(pub T);

/// No `ConnInfo` on the request, or the transport cannot supply `T`; responds 500.
#[derive(Debug)]
pub struct ConnectInfoMissing;

impl Responder for ConnectInfoMissing {
  fn into_response(self) -> tako_rs_core::types::Response {
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
  const ENTRIES: Entries = Entries::CONN;

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
  const ENTRIES: Entries = Entries::CONN;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(extract::<T>(&parts.extensions).map(ConnectInfo))
  }
}
