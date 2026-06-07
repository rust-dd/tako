use anyhow::Result;
use tako_rs_core::plugins::TakoPlugin;
use tako_rs_core::router::Router;

use super::config::Config;
use super::middleware::handle_cors;

/// CORS plugin for handling cross-origin resource sharing in Tako applications.
///
/// `CorsPlugin` implements the `TakoPlugin` trait to provide comprehensive CORS support
/// including preflight request handling, origin validation, and response header
/// management. It automatically handles OPTIONS preflight requests and adds appropriate
/// CORS headers to all responses based on the configured policy.
///
/// # Examples
///
/// ```rust
/// use tako::plugins::cors::{CorsPlugin, CorsBuilder};
/// use tako::plugins::TakoPlugin;
/// use tako::router::Router;
/// use http::Method;
///
/// // Basic setup with default permissive policy
/// let cors = CorsPlugin::default();
/// let mut router = Router::new();
/// router.plugin(cors);
///
/// // Custom restrictive policy for production
/// let prod_cors = CorsBuilder::new()
///     .allow_origin("https://myapp.com")
///     .allow_methods(&[Method::GET, Method::POST])
///     .allow_credentials(true)
///     .build();
/// router.plugin(prod_cors);
/// ```
#[derive(Clone)]
#[doc(alias = "cors")]
pub struct CorsPlugin {
  pub(crate) cfg: Config,
}

impl Default for CorsPlugin {
  /// Creates a CORS plugin with permissive default configuration.
  fn default() -> Self {
    Self {
      cfg: Config::default(),
    }
  }
}

impl TakoPlugin for CorsPlugin {
  /// Returns the plugin name for identification and debugging.
  fn name(&self) -> &'static str {
    "CorsPlugin"
  }

  /// Sets up the CORS plugin by registering middleware with the router.
  fn setup(&self, router: &Router) -> Result<()> {
    let cfg = self.cfg.clone();
    router.middleware(move |req, next| {
      let cfg = cfg.clone();
      async move { handle_cors(req, next, cfg).await }
    });
    Ok(())
  }
}
