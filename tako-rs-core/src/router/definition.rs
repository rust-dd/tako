//! The [`Router`] type definition, its fields, and constructors.

use std::sync::Arc;
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

/// HTTP router for managing routes, middleware, and request dispatching.
///
/// The `Router` is the central component for routing HTTP requests to appropriate
/// handlers. It supports dynamic path parameters, middleware chains, plugin integration,
/// and global state management. Routes are matched based on HTTP method and path pattern,
/// with support for trailing slash redirection and parameter extraction.
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
/// router.state("app_name", "MyApp".to_string());
/// ```
#[doc(alias = "router")]
pub struct Router {
  /// Map of registered routes keyed by method (O(1) array lookup).
  pub(crate) inner: MethodMap<matchit::Router<Arc<Route>>>,
  /// An easy-to-iterate index of the same routes so we can access the `Arc<Route>` values.
  ///
  /// Holds `Weak<Route>` (not `Arc`) so an external holder of an `Arc<Route>`
  /// returned from [`Router::route`] can release it without keeping the router
  /// graph alive past its useful lifetime. All current code paths that store a
  /// `Weak` here also store the matching `Arc` in `inner`, so upgrades always
  /// succeed today; [`Router::compact_routes`] sweeps dangling weaks lazily so
  /// any future API that removes from `inner` does not cause this index to
  /// grow without bound.
  pub(crate) routes: MethodMap<Vec<Weak<Route>>>,
  /// Optional path prefix prepended to every `route()` call while it is set.
  /// Used by [`Router::mount_all_into`] and [`Router::scope`] (see v2 roadmap).
  /// Only consulted at registration time — zero cost on the dispatch hot path.
  pub(crate) pending_prefix: Option<String>,
  /// Global middleware chain applied to all routes.
  pub(crate) middlewares: ArcSwap<Vec<BoxMiddleware>>,
  /// Fast check: true when global middleware is registered (avoids `ArcSwap` load on hot path).
  pub(crate) has_global_middleware: AtomicBool,
  /// Optional fallback handler executed when no route matches.
  pub(crate) fallback: Option<BoxHandler>,
  /// Registered plugins for extending functionality.
  #[cfg(feature = "plugins")]
  pub(crate) plugins: Vec<Box<dyn TakoPlugin>>,
  /// Flag to ensure plugins are initialized only once.
  #[cfg(feature = "plugins")]
  pub(crate) plugins_initialized: AtomicBool,
  /// Signal arbiter for in-process event emission and handling.
  #[cfg(feature = "signals")]
  pub(crate) signals: SignalArbiter,
  /// Default timeout for all routes.
  pub(crate) timeout: Option<Duration>,
  /// Fallback handler executed when a request times out.
  pub(crate) timeout_fallback: Option<BoxHandler>,
  /// Global error handler for 5xx responses.
  pub(crate) error_handler: Option<ErrorHandler>,
  /// Global error handler for 4xx responses (opt-in; runs after dispatch).
  pub(crate) client_error_handler: Option<ErrorHandler>,
  /// Per-router typed state populated via [`Router::with_state`].
  /// `Arc` is shared with every dispatched request via the request extension
  /// so the `State<T>` extractor can read instance-local values.
  pub(crate) router_state: Arc<RouterState>,
  /// Fast-path flag: when `false`, dispatch skips the per-request Arc clone +
  /// extension insert that wires `router_state` into requests.
  pub(crate) has_router_state: AtomicBool,
}

impl Default for Router {
  #[inline]
  fn default() -> Self {
    Self::new()
  }
}

impl Router {
  /// Creates a new, empty router.
  #[must_use]
  pub fn new() -> Self {
    let router = Self {
      inner: MethodMap::new(),
      routes: MethodMap::new(),
      pending_prefix: None,
      middlewares: ArcSwap::new(Arc::default()),
      has_global_middleware: AtomicBool::new(false),
      fallback: None,
      #[cfg(feature = "plugins")]
      plugins: Vec::new(),
      #[cfg(feature = "plugins")]
      plugins_initialized: AtomicBool::new(false),
      #[cfg(feature = "signals")]
      signals: SignalArbiter::new(),
      timeout: None,
      timeout_fallback: None,
      error_handler: None,
      client_error_handler: None,
      router_state: Arc::new(RouterState::new()),
      has_router_state: AtomicBool::new(false),
    };

    #[cfg(feature = "signals")]
    {
      // Atomic first-write: under concurrent `Router::new` calls the
      // previous `get_state.is_none() → set_state` pair was TOCTOU and let
      // two threads each install their own arbiter. `get_or_init_state`
      // resolves both to the same `Arc<SignalArbiter>`.
      let arbiter_clone = router.signals.clone();
      let _ = crate::state::get_or_init_state::<SignalArbiter, _>(move || arbiter_clone);
    }

    router
  }
}
