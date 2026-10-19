//! The [`Router`] type definition, its fields, and constructors.

use std::sync::Arc;
#[cfg(feature = "plugins")]
use std::sync::OnceLock;
use std::sync::Weak;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use arc_swap::ArcSwap;

use super::ErrorHandler;
use super::method_map::MethodMap;
use crate::handler::BoxHandler;
#[cfg(feature = "plugins")]
use crate::plugins::TakoPlugin;
use crate::route::Route;
use crate::router_state::RouterState;
#[cfg(feature = "signals")]
use crate::signals::SignalArbiter;
use crate::types::BoxMiddleware;

/// Matches requests by method and path, then runs them through middleware to a handler.
///
/// # Examples
///
/// ```rust
/// use tako::{router::Router, Method, responder::Responder, types::Request};
///
/// async fn index(_req: Request) -> impl Responder {
///     "Welcome to the home page!"
/// }
///
/// async fn user_profile(_req: Request) -> impl Responder {
///     "User profile page"
/// }
///
/// let mut router = Router::new();
/// router.route(Method::GET, "/", index);
/// router.route(Method::GET, "/users/{id}", user_profile);
/// router.with_state("MyApp".to_string());
/// ```
#[doc(alias = "router")]
pub struct Router {
  pub(crate) inner: MethodMap<matchit::Router<Arc<Route>>>,
  /// Routes in registration order for introspection; each one is also held by `inner`.
  pub(crate) routes: MethodMap<Vec<Weak<Route>>>,
  /// Prefix that [`Router::scope`] and [`Router::mount_all_into`] add while they run.
  pub(crate) pending_prefix: Option<String>,
  pub(crate) nested_routers: Vec<Arc<Router>>,
  pub(crate) middlewares: ArcSwap<Vec<BoxMiddleware>>,
  /// Lets dispatch skip the `ArcSwap` load while no global middleware is registered.
  pub(crate) has_global_middleware: AtomicBool,
  pub(crate) fallback: Option<BoxHandler>,
  #[cfg(feature = "plugins")]
  pub(crate) plugins: Vec<Box<dyn TakoPlugin>>,
  /// Publishes plugin initialization before concurrent callers can proceed.
  #[cfg(feature = "plugins")]
  pub(crate) plugins_initialized: OnceLock<Result<(), String>>,
  #[cfg(feature = "signals")]
  pub(crate) signals: SignalArbiter,
  pub(crate) timeout: Option<Duration>,
  pub(crate) timeout_status: http::StatusCode,
  pub(crate) body_limit: Option<crate::extractors::body::BodyLimit>,
  pub(crate) timeout_fallback: Option<BoxHandler>,
  /// Runs on 5xx responses.
  pub(crate) error_handler: Option<ErrorHandler>,
  pub(crate) error_handler_with_parts: Option<super::ErrorHandlerWithParts>,
  /// Runs on 4xx responses.
  pub(crate) client_error_handler: Option<ErrorHandler>,
  /// Shared with each request as an extension so `State<T>` can read it.
  pub(crate) router_state: Arc<RouterState>,
  /// Lets dispatch skip attaching `router_state` until [`Router::with_state`] is called.
  pub(crate) has_router_state: AtomicBool,
}

impl Default for Router {
  #[inline]
  fn default() -> Self {
    Self::new()
  }
}

impl Router {
  #[must_use]
  pub fn new() -> Self {
    let router = Self {
      inner: MethodMap::new(),
      routes: MethodMap::new(),
      pending_prefix: None,
      nested_routers: Vec::new(),
      middlewares: ArcSwap::new(Arc::default()),
      has_global_middleware: AtomicBool::new(false),
      fallback: None,
      #[cfg(feature = "plugins")]
      plugins: Vec::new(),
      #[cfg(feature = "plugins")]
      plugins_initialized: OnceLock::new(),
      #[cfg(feature = "signals")]
      signals: SignalArbiter::new(),
      timeout: None,
      timeout_status: http::StatusCode::GATEWAY_TIMEOUT,
      body_limit: None,
      timeout_fallback: None,
      error_handler: None,
      error_handler_with_parts: None,
      client_error_handler: None,
      router_state: Arc::new(RouterState::new()),
      has_router_state: AtomicBool::new(false),
    };

    #[cfg(feature = "signals")]
    {
      // The first router's arbiter is the global one `State<SignalArbiter>` falls back to.
      let arbiter_clone = router.signals.clone();
      let _ = crate::state::get_or_init_state::<SignalArbiter, _>(move || arbiter_clone);
    }

    router
  }
}
