use anyhow::Result;

use crate::router::Router;

pub mod cors;
pub mod rate_limiter;

/// The `TakoPlugin` trait defines the interface for plugins in the Tako framework.
/// Plugins implementing this trait can extend the functionality of the framework by
/// providing custom middleware, handlers, or other features.
///
/// # Required Methods
/// - `name`: Returns the name of the plugin.
/// - `setup`: Configures the plugin by attaching it to the router.
///
/// # Example
/// ```rust
/// use tako::plugins::TakoPlugin;
/// use tako::router::Router;
/// use anyhow::Result;
///
/// struct MyPlugin;
///
/// impl TakoPlugin for MyPlugin {
///     fn name(&self) -> &'static str {
///         "MyPlugin"
///     }
///
///     fn setup(&self, router: &Router) -> Result<()> {
///         // Plugin setup logic here
///         Ok(())
///     }
/// }
/// ```
pub trait TakoPlugin: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    fn setup(&self, router: &Router) -> Result<()>;
}
