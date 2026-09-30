use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tako_rs_core::router::Router;
use tako_rs_core::types::BoxError;

use crate::h3_common::sessions::Sessions;

/// Handles a single HTTP/3 connection.
///
/// Races `accept()` against the per-connection shutdown notify; on shutdown,
/// emits a GOAWAY frame via `h3_conn.shutdown(0)` and waits up to `goaway_grace`
/// for any already-spawned request handlers to finish before returning. An
/// accepted WebTransport session takes the connection over until it ends.
pub(crate) async fn handle_connection(
  conn: quinn::Connection,
  router: Arc<Router>,
  remote_addr: SocketAddr,
  shutdown: tokio_util::sync::CancellationToken,
  goaway_grace: Duration,
) -> Result<(), BoxError> {
  let mut h3_conn = crate::h3_common::connection::accept(h3_quinn::Connection::new(conn)).await?;
  let mut request_tasks = tokio::task::JoinSet::new();
  let mut sessions = Sessions::new();

  let session = loop {
    tokio::select! {
      accepted = h3_conn.accept() => {
        match accepted {
          Ok(Some(resolver)) => {
            let router = router.clone();
            let sessions = sessions.sender();
            request_tasks.spawn(async move {
              match resolver.resolve_request().await {
                Ok((req, stream)) => {
                  if let Err(e) = sessions.serve(req, stream, router, remote_addr).await {
                    tracing::error!("HTTP/3 request error: {e}");
                  }
                }
                Err(e) => {
                  tracing::error!("HTTP/3 request resolve error: {e}");
                }
              }
            });
          }
          Ok(None) => break None,
          Err(e) => {
            tracing::error!("HTTP/3 accept error: {e}");
            break None;
          }
        }
      }
      session = sessions.recv() => break Some(session),
      _ = request_tasks.join_next(), if !request_tasks.is_empty() => {}
      () = shutdown.cancelled() => {
        // Send GOAWAY(0): the peer must not start any new request, but we
        // continue draining streams already in flight on this connection.
        // `CancellationToken::cancelled()` is sticky — connections that
        // handshake AFTER the server-level signal also observe the trigger.
        if let Err(e) = h3_conn.shutdown(0).await {
          tracing::debug!("HTTP/3 GOAWAY error: {e}");
        }
        break None;
      }
    }
  };

  if let Some(session) = session {
    session
      .run(h3_conn, router, remote_addr, &shutdown, goaway_grace)
      .await;
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
