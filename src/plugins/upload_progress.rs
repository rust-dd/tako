use std::sync::Arc;

use dashmap::DashMap;
use http::Method;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::plugins::TakoPlugin;

#[derive(Clone)]
pub struct Config<'a> {
  pub path: &'a str,
}

#[derive(Clone)]
pub struct UploadProgress {
  pub id: Uuid,
  pub bytes: u64,
  pub total: Option<u64>,
}

#[derive(Clone)]
pub struct UploadProgressHub {
  inner: Arc<DashMap<Uuid, broadcast::Sender<UploadProgress>>>,
}

impl UploadProgressHub {
  pub fn new() -> Self {
    Self {
      inner: Arc::new(DashMap::new()),
    }
  }

  pub fn register(&self, id: Uuid) -> broadcast::Receiver<UploadProgress> {
    let (tx, rx) = broadcast::channel(100);
    self.inner.insert(id, tx);
    rx
  }

  pub fn subscribe(&self, id: &Uuid) -> Option<broadcast::Receiver<UploadProgress>> {
    self.inner.get(&id).map(|e| e.value().subscribe())
  }

  pub fn notify(&self, msg: UploadProgress) {
    if let Some(tx) = self.inner.get(&msg.id) {
      let _ = tx.send(msg);
    }
  }
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
