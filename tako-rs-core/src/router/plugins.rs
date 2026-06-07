//! Plugin registration/initialization, `OpenAPI` collection, and route-index GC.

#[cfg(any(feature = "utoipa", feature = "vespera"))]
use http::Method;

use super::Router;
#[cfg(feature = "plugins")]
use crate::plugins::TakoPlugin;

impl Router {
  /// Registers a plugin with the router.
  ///
  /// Plugins extend the router's functionality by providing additional features
  /// like compression, CORS handling, rate limiting, or custom behavior. Plugins
  /// are initialized once when the server starts.
  ///
  /// # Examples
  ///
  /// ```rust
  /// # #[cfg(feature = "plugins")]
  /// use tako::{router::Router, plugins::TakoPlugin};
  /// # #[cfg(feature = "plugins")]
  /// use anyhow::Result;
  ///
  /// # #[cfg(feature = "plugins")]
  /// struct LoggingPlugin;
  ///
  /// # #[cfg(feature = "plugins")]
  /// impl TakoPlugin for LoggingPlugin {
  ///     fn name(&self) -> &'static str {
  ///         "logging"
  ///     }
  ///
  ///     fn setup(&self, _router: &Router) -> Result<()> {
  ///         println!("Logging plugin initialized");
  ///         Ok(())
  ///     }
  /// }
  ///
  /// # #[cfg(feature = "plugins")]
  /// # fn example() {
  /// let mut router = Router::new();
  /// router.plugin(LoggingPlugin);
  /// # }
  /// ```
  #[cfg(feature = "plugins")]
  #[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
  pub fn plugin<P>(&mut self, plugin: P) -> &mut Self
  where
    P: TakoPlugin + Clone + Send + Sync + 'static,
  {
    self.plugins.push(Box::new(plugin));
    self
  }

  /// Returns references to all registered plugins.
  #[cfg(feature = "plugins")]
  #[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
  pub(crate) fn plugins(&self) -> Vec<&dyn TakoPlugin> {
    self.plugins.iter().map(AsRef::as_ref).collect()
  }

  /// Initializes all registered plugins exactly once.
  #[cfg(feature = "plugins")]
  #[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
  #[doc(hidden)]
  pub fn setup_plugins_once(&self) {
    use std::sync::atomic::Ordering;

    // Hot-path fast exit: see `Route::setup_plugins_once`. Acquire-load
    // pairs with the Release half of the swap so plugin-published state
    // is visible by the time we skip the RMW.
    if self.plugins_initialized.load(Ordering::Acquire) {
      return;
    }

    if !self.plugins_initialized.swap(true, Ordering::SeqCst) {
      for plugin in self.plugins() {
        // Surface plugin setup errors loudly — a silently-skipped CORS,
        // auth, rate-limit, or CSRF plugin would leave the server
        // running without the protection the operator expected
        // (security-relevant fail-open). Cold path — first dispatch only.
        if let Err(e) = plugin.setup(self) {
          tracing::error!(
            plugin = plugin.name(),
            error = %e,
            "router-level TakoPlugin::setup failed; plugin not active"
          );
        }
      }
    }
  }

  /// Collects `OpenAPI` metadata from all registered routes.
  ///
  /// Returns a vector of tuples containing the HTTP method, path, and `OpenAPI`
  /// metadata for each route that has `OpenAPI` information attached.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// use tako::{router::Router, Method};
  ///
  /// let mut router = Router::new();
  /// router.route(Method::GET, "/users", list_users)
  ///     .summary("List users")
  ///     .tag("users");
  ///
  /// for (method, path, openapi) in router.collect_openapi_routes() {
  ///     println!("{} {} - {:?}", method, path, openapi.summary);
  /// }
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn collect_openapi_routes(&self) -> Vec<(Method, String, crate::openapi::RouteOpenApi)> {
    let mut result = Vec::new();

    for (method, weak_vec) in self.routes.iter() {
      for weak in weak_vec {
        if let Some(route) = weak.upgrade()
          && let Some(openapi) = route.openapi_metadata()
        {
          result.push((method.clone(), route.path.clone(), openapi));
        }
      }
    }

    result
  }

  /// Drops dangling `Weak<Route>` entries from the per-method `routes` index.
  ///
  /// All current routes stay live for the router's lifetime, so this is a
  /// no-op in well-behaved code. It exists as a safety valve: if any future
  /// API ever removes from `inner` (hot reload, route deregistration), or if
  /// downstream code holds the `Arc<Route>` returned from [`Router::route`]
  /// past the router's lifetime, this method bounds the size of the index.
  ///
  /// Cold path; safe to call repeatedly. Linear in the total number of
  /// registered routes.
  pub fn compact_routes(&mut self) {
    for weak_vec in self.routes.iter_mut() {
      weak_vec.retain(|w| w.strong_count() > 0);
    }
  }
}
