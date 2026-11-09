#![cfg(feature = "plugins")]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

#[cfg(feature = "compio")]
use compio::net::TcpListener;
use tako_rs_core::plugins::TakoPlugin;
use tako_rs_core::router::Router;
#[cfg(feature = "compio")]
use tako_rs_server::CompioServer as Server;
#[cfg(not(feature = "compio"))]
use tako_rs_server::Server;
use tako_rs_server::ServerConfig;
use tako_rs_server::ServerConfigError;
#[cfg(not(feature = "compio"))]
use tokio::net::TcpListener;

#[derive(Clone)]
struct StartupPlugin(Arc<AtomicBool>);

impl TakoPlugin for StartupPlugin {
  fn name(&self) -> &'static str {
    "startup-state"
  }

  fn setup(&self, router: &Router) -> anyhow::Result<()> {
    self.0.store(true, Ordering::SeqCst);
    router.middleware(|request, next| async move { next.run(request).await });
    Ok(())
  }
}

async fn check_startup() {
  let initialized = Arc::new(AtomicBool::new(false));
  let mut router = Router::new();
  router.plugin(StartupPlugin(initialized.clone()));
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let error = Server::builder()
    .config(ServerConfig {
      max_connections: Some(0),
      ..ServerConfig::default()
    })
    .build()
    .try_spawn_http(listener, router)
    .err()
    .unwrap();

  assert_eq!(
    error.downcast_ref::<ServerConfigError>().unwrap().field(),
    "max_connections"
  );
  assert!(!initialized.load(Ordering::SeqCst));
}

#[cfg(not(feature = "compio"))]
#[tokio::test]
async fn invalid_configuration_does_not_initialize_plugins() {
  check_startup().await;
}

#[cfg(feature = "compio")]
#[test]
fn invalid_configuration_does_not_initialize_plugins() {
  compio::runtime::Runtime::new()
    .unwrap()
    .block_on(check_startup());
}
