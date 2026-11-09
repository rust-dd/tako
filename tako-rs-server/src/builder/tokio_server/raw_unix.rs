use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use super::Server;
use crate::ServerHandle;
use crate::builder::spawn::failed_handle;
use crate::builder::spawn::make_handle;
use crate::builder::spawn::spawn_done;

impl Server {
  /// Spawns raw Unix sockets; startup errors are returned by [`ServerHandle::result`].
  pub fn spawn_unix_raw<F>(&self, path: impl Into<PathBuf>, handler: F) -> ServerHandle
  where
    F: Fn(
        tokio::net::UnixStream,
        tokio::net::unix::SocketAddr,
      ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>
      + Send
      + Sync
      + 'static,
  {
    if let Err(error) = self.config.validate() {
      return failed_handle(error.into(), self.config.drain_timeout);
    }
    let path = path.into();
    let config = self.config.clone();
    let (handle, signal) = make_handle(config.drain_timeout, None);
    spawn_done(&handle, async move {
      crate::server_unix::serve_unix_with_config(path, handler, signal, config)
        .await
        .map_err(Into::into)
    });
    handle
  }

  /// Validates configuration and binds the raw Unix listener before starting tasks.
  pub async fn try_spawn_unix_raw<F>(
    &self,
    path: impl Into<PathBuf>,
    handler: F,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError>
  where
    F: Fn(
        tokio::net::UnixStream,
        tokio::net::unix::SocketAddr,
      ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>
      + Send
      + Sync
      + 'static,
  {
    self.config.validate()?;
    let path = path.into();
    let listener = crate::server_unix::listener::bind_unix_listener(&path).await?;
    let config = self.config.clone();
    let (handle, signal) = make_handle(config.drain_timeout, None);
    spawn_done(&handle, async move {
      crate::server_unix::run_listener(listener, &path, handler, signal, config)
        .await
        .map_err(Into::into)
    });
    Ok(handle)
  }
}
