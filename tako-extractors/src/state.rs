//! Global state extraction for retrieving shared application state.
//!
//! This module exposes `State<T>` to access Tako's global state store from handlers.
//! It retrieves a value by its concrete type (stored via `set_state`).
//!
//! # Examples
//!
//! ```rust
//! use tako::{extractors::state::State, responder::Responder, router::Router, Method, state::set_state};
//!
//! #[derive(Clone)]
//! struct AppConfig { name: String }
//!
//! async fn handler(State(cfg): State<AppConfig>) -> impl Responder { cfg.name }
//!
//! let mut router = Router::new();
//! set_state(AppConfig { name: "demo".into() });
//! router.route(Method::GET, "/", handler);
//! ```

use std::sync::Arc;

use http::request::Parts;

use tako_core::extractors::FromRequest;
use tako_core::extractors::FromRequestParts;
use tako_core::responder::Responder;
use tako_core::router_state::RouterState;
use tako_core::state::get_state;
use tako_core::types::Request;

/// Extractor for accessing a value stored in Tako's global state by type.
pub struct State<T>(pub Arc<T>);

impl<T> Clone for State<T> {
  fn clone(&self) -> Self {
    Self(self.0.clone())
  }
}

#[derive(Debug)]
pub struct MissingState;

impl Responder for MissingState {
  fn into_response(self) -> tako_core::types::Response {
    (
      http::StatusCode::INTERNAL_SERVER_ERROR,
      "missing application state",
    )
      .into_response()
  }
}

/// Reads `T` from the per-router typed state first (when the router was set
/// up via `Router::with_state`), falling back to the process-global registry.
fn lookup<T: Send + Sync + 'static>(extensions: &http::Extensions) -> Option<Arc<T>> {
  if let Some(rs) = extensions.get::<Arc<RouterState>>() {
    if let Some(arc) = rs.get::<T>() {
      return Some(arc);
    }
  }
  get_state::<T>()
}

impl<'a, T> FromRequest<'a> for State<T>
where
  T: Send + Sync + 'static,
{
  type Error = MissingState;

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

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(match lookup::<T>(&parts.extensions) {
      Some(arc) => Ok(Self(arc)),
      None => Err(MissingState),
    })
  }
}
