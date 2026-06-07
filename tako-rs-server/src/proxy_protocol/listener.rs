//! HTTP listener/acceptor that parses PROXY protocol headers per connection.

use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::router::Router;
use tako_rs_core::types::BoxError;
use tokio::task::JoinSet;

use super::read_proxy_protocol;
use crate::ServerConfig;

/// Build an RFC 7239 `Forwarded` header value from the PROXY-protocol-supplied
/// peer address. IPv6 addresses get bracketed per the RFC's `node` ABNF.
fn format_forwarded(addr: SocketAddr) -> String {
  match addr {
    SocketAddr::V4(v4) => format!("for=\"{}:{}\"", v4.ip(), v4.port()),
    SocketAddr::V6(v6) => format!("for=\"[{}]:{}\"", v6.ip(), v6.port()),
  }
}

/// Starts an HTTP server that parses PROXY protocol headers on each connection.
///
/// The real client address from the PROXY header is inserted into request
/// extensions as `SocketAddr` (overriding the TCP peer address). The raw
/// `ProxyHeader` is also available via `req.extensions().get::<ProxyHeader>()`.
pub async fn serve_http_with_proxy_protocol(listener: tokio::net::TcpListener, router: Router) {
  if let Err(e) = run_proxy_http(
    listener,
    router,
    None::<std::future::Pending<()>>,
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("PROXY protocol HTTP server error: {e}");
  }
}

/// Starts an HTTP server with PROXY protocol support and graceful shutdown.
pub async fn serve_http_with_proxy_protocol_and_shutdown(
  listener: tokio::net::TcpListener,
  router: Router,
  signal: impl Future<Output = ()> + Send + 'static,
) {
  if let Err(e) = run_proxy_http(listener, router, Some(signal), ServerConfig::default()).await {
    tracing::error!("PROXY protocol HTTP server error: {e}");
  }
}

/// Like [`serve_http_with_proxy_protocol`] with caller-supplied [`ServerConfig`].
pub async fn serve_http_with_proxy_protocol_and_config(
  listener: tokio::net::TcpListener,
  router: Router,
  config: ServerConfig,
) {
  if let Err(e) = run_proxy_http(listener, router, None::<std::future::Pending<()>>, config).await {
    tracing::error!("PROXY protocol HTTP server error: {e}");
  }
}

/// Like [`serve_http_with_proxy_protocol_and_shutdown`] with caller-supplied [`ServerConfig`].
pub async fn serve_http_with_proxy_protocol_shutdown_and_config(
  listener: tokio::net::TcpListener,
  router: Router,
  signal: impl Future<Output = ()> + Send + 'static,
  config: ServerConfig,
) {
  if let Err(e) = run_proxy_http(listener, router, Some(signal), config).await {
    tracing::error!("PROXY protocol HTTP server error: {e}");
  }
}

async fn run_proxy_http(
  listener: tokio::net::TcpListener,
  router: Router,
  signal: Option<impl Future<Output = ()> + Send + 'static>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  let router = Arc::new(router);

  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  tracing::debug!(
    "Tako PROXY protocol HTTP listening on {}",
    listener.local_addr()?
  );

  let mut join_set = JoinSet::new();
  let mut accept_backoff = config.accept_backoff;
  let max_conn_semaphore = config
    .max_connections
    .map(|n| Arc::new(tokio::sync::Semaphore::new(n)));
  let drain_timeout = config.drain_timeout;
  let header_read_timeout = config.header_read_timeout;
  let keep_alive = config.keep_alive;
  let proxy_read_timeout = config.proxy_read_timeout;
  let cancel = tokio_util::sync::CancellationToken::new();
  if let Some(s) = signal {
    let cancel_for_signal = cancel.clone();
    tokio::spawn(async move {
      s.await;
      cancel_for_signal.cancel();
    });
  }

  loop {
    tokio::select! {
      result = listener.accept() => {
        let (mut stream, _tcp_addr) = match result {
          Ok(v) => { accept_backoff.reset(); v }
          Err(err) => {
            tracing::warn!("PROXY accept failed: {err}; backing off");
            accept_backoff.sleep_and_grow().await;
            continue;
          }
        };
        let permit = if let Some(sem) = &max_conn_semaphore {
          tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            permit = sem.clone().acquire_owned() => match permit {
              Ok(p) => Some(p),
              Err(_) => continue,
            },
          }
        } else {
          None
        };
        let _ = stream.set_nodelay(true);
        let router = router.clone();

        join_set.spawn(async move {
          // Parse PROXY protocol header under a read deadline so a stalled
          // client cannot pin a worker task forever.
          let proxy_header =
            match tokio::time::timeout(proxy_read_timeout, read_proxy_protocol(&mut stream)).await {
              Ok(Ok(h)) => h,
              Ok(Err(e)) => {
                tracing::warn!("Failed to parse PROXY protocol: {e}");
                return;
              }
              Err(_) => {
                tracing::warn!(
                  "PROXY protocol read deadline ({:?}) elapsed; dropping connection",
                  proxy_read_timeout,
                );
                return;
              }
            };

          let real_addr = proxy_header.source;
          let io = hyper_util::rt::TokioIo::new(stream);

          let svc = service_fn(move |mut req| {
            let router = router.clone();
            let proxy_header = proxy_header.clone();
            let real_addr = real_addr;
            async move {
              // Strip any inbound X-Forwarded-* / Forwarded: clients behind a
              // PROXY-protocol hop must not be able to spoof their address
              // through the header. The PROXY-protocol-supplied source becomes
              // the authoritative one; we re-emit a single `Forwarded` header
              // built from it so downstream middleware that follows RFC 7239
              // sees a consistent view instead of having to read the
              // `ConnInfo`/`SocketAddr` extension out of band.
              req.headers_mut().remove(http::header::FORWARDED);
              req.headers_mut().remove("x-forwarded-for");
              req.headers_mut().remove("x-forwarded-host");
              req.headers_mut().remove("x-forwarded-proto");

              if let Some(addr) = real_addr {
                let forwarded_value = format_forwarded(addr);
                if let Ok(v) = http::HeaderValue::from_str(&forwarded_value) {
                  req.headers_mut().insert(http::header::FORWARDED, v);
                }
                req.extensions_mut().insert(addr);
                req.extensions_mut().insert(ConnInfo::tcp(addr));
              }
              req.extensions_mut().insert(proxy_header);
              let response = router.dispatch(req.map(TakoBody::incoming)).await;
              Ok::<_, Infallible>(response)
            }
          });

          let mut http = http1::Builder::new();
          http.keep_alive(keep_alive);
          http.timer(hyper_util::rt::TokioTimer::new());
          if let Some(t) = header_read_timeout {
            http.header_read_timeout(t);
          }
          let conn = http.serve_connection(io, svc).with_upgrades();

          if let Err(err) = conn.await {
            if err.is_incomplete_message() {
              tracing::debug!("client disconnected mid-message on PROXY protocol connection: {err}");
            } else {
              tracing::error!("Error serving PROXY protocol connection: {err}");
            }
          }

          drop(permit);
        });
      }
      () = cancel.cancelled() => {
        tracing::info!("PROXY protocol HTTP server shutting down...");
        break;
      }
    }
  }

  let drain = tokio::time::timeout(drain_timeout, async {
    while join_set.join_next().await.is_some() {}
  });

  if drain.await.is_err() {
    tracing::warn!(
      "Drain timeout exceeded, aborting {} remaining connections",
      join_set.len()
    );
    join_set.abort_all();
  }

  tracing::info!("PROXY protocol HTTP server shut down gracefully");
  Ok(())
}
