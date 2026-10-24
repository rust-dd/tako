use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tako_rs_core::router::Router;
use tako_rs_core::types::BoxError;

use crate::h3_common::sessions::Sessions;

#[cfg(test)]
mod tests;

/// Serves one connection until the peer closes it or shutdown sends GOAWAY, then gives requests
/// `goaway_grace` to finish; an accepted WebTransport session takes the connection over.
pub(crate) async fn handle_connection(
  conn: quinn::Connection,
  router: Arc<Router>,
  remote_addr: SocketAddr,
  shutdown: tokio_util::sync::CancellationToken,
  goaway_grace: Duration,
) -> Result<(), BoxError> {
  let mut h3_conn = crate::h3_common::connection::accept(h3_quinn::Connection::new(conn)).await?;
  // Per-stream clones share this connection's refcount, avoiding cross-connection contention.
  let router = Arc::new(router);
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
                  if let Err(e) = sessions.serve(req, stream, &router, remote_addr).await {
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
        // GOAWAY(0) stops new requests while in-flight streams drain below; the token is sticky,
        // so connections accepted after shutdown began land here too.
        if let Err(e) = h3_conn.shutdown(0).await {
          tracing::debug!("HTTP/3 GOAWAY error: {e}");
        }
        break None;
      }
    }
  };

  if let Some(session) = session {
    session
      .run(
        h3_conn,
        router.as_ref().clone(),
        remote_addr,
        &shutdown,
        goaway_grace,
      )
      .await;
  }

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
