use anyhow::Result;

use crate::router::Router;

pub mod cors;
pub mod rate_limiter;

pub trait TakoPlugin: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    fn setup(&self, router: &Router) -> Result<()>;
}
