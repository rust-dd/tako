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

use crate::extractors::FromRequest;
use crate::extractors::FromRequestParts;
use crate::responder::Responder;
use crate::state::get_state;
use crate::types::Request;

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
  fn into_response(self) -> crate::types::Response {
    (
      http::StatusCode::INTERNAL_SERVER_ERROR,
      "missing application state",
    )
      .into_response()
  }
}

impl<'a, T> FromRequest<'a> for State<T>
where
  T: Send + Sync + 'static,
{
  type Error = MissingState;

  fn from_request(
    _req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(match get_state::<T>() {
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
    _parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(match get_state::<T>() {
      Some(arc) => Ok(Self(arc)),
      None => Err(MissingState),
    })
  }
}
