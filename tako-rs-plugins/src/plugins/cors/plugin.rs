use std::sync::Arc;

use anyhow::Result;
use tako_rs_core::plugins::TakoPlugin;
use tako_rs_core::router::Router;

use super::config::Config;
use super::middleware::handle_cors;

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
#[derive(Clone, Default)]
#[doc(alias = "cors")]
pub struct CorsPlugin {
  pub(crate) cfg: Config,
}

impl TakoPlugin for CorsPlugin {
  fn name(&self) -> &'static str {
    "CorsPlugin"
  }

  fn setup(&self, router: &Router) -> Result<()> {
    let cfg = Arc::new(self.cfg.clone());
    router.middleware(move |req, next| {
      let cfg = cfg.clone();
      async move { handle_cors(req, next, &cfg).await }
    });
    Ok(())
  }
}
