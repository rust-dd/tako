//! `State<T>`: the router's `T` from `Router::with_state`, else the process-global `T` from
//! `get_or_init_state`.
//!
//! # Examples
//!
//! ```rust
//! use tako::{extractors::state::State, responder::Responder, router::Router, Method};
//!
//! #[derive(Clone)]
//! struct AppConfig { name: String }
//!
//! async fn handler(State(cfg): State<AppConfig>) -> impl Responder { cfg.name.clone() }
//!
//! let mut router = Router::new();
//! router.with_state(AppConfig { name: "demo".into() });
//! router.route(Method::GET, "/", handler);
//! ```

use std::sync::Arc;

use http::request::Parts;
use tako_rs_core::extractors::Entries;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::router_state::RouterState;
use tako_rs_core::state::get_state;
use tako_rs_core::types::Request;

/// Rejects with 500 when neither the router nor the process holds a `T`.
pub struct State<T>(pub Arc<T>);

impl<T> Clone for State<T> {
  fn clone(&self) -> Self {
    Self(self.0.clone())
  }
}

#[derive(Debug)]
pub struct MissingState;

impl Responder for MissingState {
  fn into_response(self) -> tako_rs_core::types::Response {
    (
      http::StatusCode::INTERNAL_SERVER_ERROR,
      "missing application state",
    )
      .into_response()
  }
}

fn lookup<T: Send + Sync + 'static>(extensions: &http::Extensions) -> Option<Arc<T>> {
  if let Some(rs) = extensions.get::<Arc<RouterState>>()
    && let Some(arc) = rs.get::<T>()
  {
    return Some(arc);
  }
  get_state::<T>()
}

impl<'a, T> FromRequest<'a> for State<T>
where
  T: Send + Sync + 'static,
{
  type Error = MissingState;
  const ENTRIES: Entries = Entries::STATE;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(match lookup::<T>(req.extensions()) {
      Some(arc) => Ok(Self(arc)),
      None => Err(MissingState),
    })
  }
}

impl<'a, T> FromRequestParts<'a> for State<T>
where
  T: Send + Sync + 'static,
{
  type Error = MissingState;
  const ENTRIES: Entries = Entries::STATE;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(match lookup::<T>(&parts.extensions) {
      Some(arc) => Ok(Self(arc)),
      None => Err(MissingState),
    })
  }
}
