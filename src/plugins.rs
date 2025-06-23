use anyhow::Result;

use crate::router::Router;

pub mod cors;

#[async_trait::async_trait]
pub trait TakoPlugin: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    fn setup(&self, router: &mut Router) -> Result<()>;

    // fn start(&self) -> impl Future<Output = Result<()>>;

    // fn stop(&self) -> impl Future<Output = Result<()>>;
}
