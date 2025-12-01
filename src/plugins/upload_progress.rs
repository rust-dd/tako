use http::Method;

use crate::plugins::TakoPlugin;

#[derive(Clone)]
pub struct Config<'a> {
  pub path: &'a str,
}

pub struct UploadProgressPlugin<'a>(Config<'a>);

impl<'a> UploadProgressPlugin<'a> {
  pub fn new(config: Config<'a>) -> Self {
    Self(config)
  }
}

impl TakoPlugin for UploadProgressPlugin<'static> {
  fn name(&self) -> &'static str {
    "UploadProgressPlugin"
  }

  fn setup(&self, router: &crate::router::Router) -> anyhow::Result<()> {
    Ok(())
  }
}
