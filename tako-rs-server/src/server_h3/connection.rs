use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tako_rs_core::router::Router;
use tako_rs_core::types::BoxError;

use crate::h3_common::request::handle_request;

/// Handles a single HTTP/3 connection.
///
/// Races `accept()` against the per-connection shutdown notify; on shutdown,
/// emits a GOAWAY frame via `h3_conn.shutdown(0)` and waits up to `goaway_grace`
/// for any already-spawned request handlers to finish before returning.
pub(crate) async fn handle_connection(
  conn: quinn::Connection,
  router: Arc<Router>,
  remote_addr: SocketAddr,
  shutdown: tokio_util::sync::CancellationToken,
  goaway_grace: Duration,
) -> Result<(), BoxError> {
  let mut h3_conn = h3::server::Connection::new(h3_quinn::Connection::new(conn)).await?;
  let mut request_tasks = tokio::task::JoinSet::new();

  loop {
    tokio::select! {
      accepted = h3_conn.accept() => {
        match accepted {
          Ok(Some(resolver)) => {
            let router = router.clone();
            request_tasks.spawn(async move {
              match resolver.resolve_request().await {
                Ok((req, stream)) => {
                  if let Err(e) = handle_request(req, stream, router, remote_addr).await {
                    tracing::error!("HTTP/3 request error: {e}");
                  }
                }
                Err(e) => {
                  tracing::error!("HTTP/3 request resolve error: {e}");
                }
              }
            });
          }
          Ok(None) => break,
          Err(e) => {
            tracing::error!("HTTP/3 accept error: {e}");
            break;
          }
        }
      }
      _ = request_tasks.join_next(), if !request_tasks.is_empty() => {}
      () = shutdown.cancelled() => {
        // Send GOAWAY(0): the peer must not start any new request, but we
        // continue draining streams already in flight on this connection.
        // `CancellationToken::cancelled()` is sticky — connections that
        // handshake AFTER the server-level signal also observe the trigger.
        if let Err(e) = h3_conn.shutdown(0).await {
          tracing::debug!("HTTP/3 GOAWAY error: {e}");
        }
        break;
      }
    }
  }

  // Drain in-flight request handlers within the per-connection grace.
  let drain_deadline = tokio::time::Instant::now() + goaway_grace;
  let drain = tokio::time::timeout_at(drain_deadline, async {
    while request_tasks.join_next().await.is_some() {}
  });
  if drain.await.is_err() {
    tracing::debug!(
      "HTTP/3 connection grace ({:?}) elapsed; aborting {} request task(s)",
      goaway_grace,
      request_tasks.len()
    );
    request_tasks.shutdown().await;
  }

  Ok(())
}
