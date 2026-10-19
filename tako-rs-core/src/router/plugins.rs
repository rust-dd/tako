//! Plugin registration/initialization, `OpenAPI` collection, and route-index GC.

#[cfg(any(feature = "utoipa", feature = "vespera"))]
use http::Method;

use super::Router;
#[cfg(feature = "plugins")]
use crate::plugins::TakoPlugin;

impl Router {
  /// Adds a router-wide plugin; its `setup` runs once, at server start or on first dispatch.
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
  /// #[derive(Clone)]
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

  /// Sets up nested routers' plugins, then this router's, once; later calls return that result.
  #[cfg(feature = "plugins")]
  #[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
  #[doc(hidden)]
  pub fn setup_plugins_once(&self) -> Result<(), String> {
    self
      .plugins_initialized
      .get_or_init(|| {
        for child in &self.nested_routers {
          child.setup_plugins_once()?;
          #[cfg(feature = "signals")]
          self.signals.merge_from(&child.signals);
        }
        for plugin in &self.plugins {
          plugin.setup(self).map_err(|error| {
            let message = format!("plugin {}: {error:#}", plugin.name());
            tracing::error!(error = %message, "router plugin initialization failed");
            message
          })?;
        }
        Ok(())
      })
      .clone()
  }

  /// `(method, path, metadata)` for every route that has `OpenAPI` metadata attached.
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
          result.push((method.clone(), route.path.to_string(), openapi));
        }
      }
    }

    result
  }
}
