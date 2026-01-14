#[cfg(feature = "signals")]
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use compio::net::TcpListener;
use cyper_core::HyperStream;
use hyper::server::conn::http1;
use hyper::service::service_fn;

use crate::body::TakoBody;
use crate::router::Router;
#[cfg(feature = "signals")]
use crate::signals::Signal;
#[cfg(feature = "signals")]
use crate::signals::SignalArbiter;
#[cfg(feature = "signals")]
use crate::signals::ids;
use crate::types::BoxError;
#[cfg(feature = "signals")]
use crate::types::BuildHasher;

pub async fn serve(listener: TcpListener, router: Router) {
  run(listener, router).await.unwrap()
}

async fn run(listener: TcpListener, router: Router) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  crate::tracing::init_tracing();

  let router = Arc::new(router);
  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  let addr_str = listener.local_addr()?.to_string();

  #[cfg(feature = "signals")]
  {
    // Emit server.started
    let mut server_meta: HashMap<String, String, BuildHasher> =
      HashMap::with_hasher(BuildHasher::default());
    server_meta.insert("addr".to_string(), addr_str.clone());
    server_meta.insert("transport".to_string(), "tcp".to_string());
    server_meta.insert("tls".to_string(), "false".to_string());
    SignalArbiter::emit_app(Signal::with_metadata(ids::SERVER_STARTED, server_meta)).await;
  }

  tracing::debug!("Tako listening on {}", addr_str);

  loop {
    let (stream, addr) = listener.accept().await?;
    let io = HyperStream::new(stream);
    let router = router.clone();

    compio::runtime::spawn(async move {
      #[cfg(feature = "signals")]
      {
        // Emit connection.opened
        let mut conn_open_meta: HashMap<String, String, BuildHasher> =
          HashMap::with_hasher(BuildHasher::default());
        conn_open_meta.insert("remote_addr".to_string(), addr.to_string());
        SignalArbiter::emit_app(Signal::with_metadata(
          ids::CONNECTION_OPENED,
          conn_open_meta,
        ))
        .await;
      }

      let svc = service_fn(move |mut req| {
        let router = router.clone();
        async move {
          #[cfg(feature = "signals")]
          let path = req.uri().path().to_string();
          #[cfg(feature = "signals")]
          let method = req.method().to_string();

          req.extensions_mut().insert(addr);

          #[cfg(feature = "signals")]
          {
            let mut req_meta: HashMap<String, String, BuildHasher> =
              HashMap::with_hasher(BuildHasher::default());
            req_meta.insert("method".to_string(), method.clone());
            req_meta.insert("path".to_string(), path.clone());
            SignalArbiter::emit_app(Signal::with_metadata(ids::REQUEST_STARTED, req_meta)).await;
          }

          // Map hyper body to TakoBody to keep request body independent
          let response = router.dispatch(req.map(TakoBody::new)).await;

          #[cfg(feature = "signals")]
          {
            let mut done_meta: HashMap<String, String, BuildHasher> =
              HashMap::with_hasher(BuildHasher::default());
            done_meta.insert("method".to_string(), method);
            done_meta.insert("path".to_string(), path);
            done_meta.insert("status".to_string(), response.status().as_u16().to_string());
            SignalArbiter::emit_app(Signal::with_metadata(ids::REQUEST_COMPLETED, done_meta)).await;
          }

          Ok::<_, Infallible>(response)
        }
      });

      let mut http = http1::Builder::new();
      http.keep_alive(true);
      let conn = http.serve_connection(io, svc).with_upgrades();

      if let Err(err) = conn.await {
        tracing::error!("Error serving connection: {err}");
      }

      #[cfg(feature = "signals")]
      {
        // Emit connection.closed
        let mut conn_close_meta: HashMap<String, String, BuildHasher> =
          HashMap::with_hasher(BuildHasher::default());
        conn_close_meta.insert("remote_addr".to_string(), addr.to_string());
        SignalArbiter::emit_app(Signal::with_metadata(
          ids::CONNECTION_CLOSED,
          conn_close_meta,
        ))
        .await;
      }
    })
    .detach();
  }
}
