use std::future::Future;

use tokio::task::JoinSet;

pub(super) async fn next_connection<T>(
  accept: impl Future<Output = Option<T>>,
  shutdown: impl Future<Output = ()>,
  tasks: &mut JoinSet<()>,
) -> Option<T> {
  let mut accept = std::pin::pin!(accept);
  let mut shutdown = std::pin::pin!(shutdown);
  loop {
    tokio::select! {
      biased;
      () = &mut shutdown => return None,
      completed = tasks.join_next(), if !tasks.is_empty() => {
        if let Some(Err(error)) = completed {
          tracing::warn!("QUIC connection task failed: {error}");
        }
      }
      connection = &mut accept => return connection,
    }
  }
}

#[cfg(test)]
mod tests {
  use std::time::Duration;

  use tokio::task::JoinSet;

  use super::next_connection;

  #[tokio::test]
  async fn completed_connections_are_reaped_while_accept_is_idle() {
    let mut tasks = JoinSet::new();
    for _ in 0..4 {
      let completed: Vec<_> = (0..64).map(|_| tasks.spawn(async {})).collect();
      while completed.iter().any(|task| !task.is_finished()) {
        tokio::task::yield_now().await;
      }
    }
    let next = next_connection(
      std::future::pending::<Option<()>>(),
      tokio::time::sleep(Duration::from_millis(20)),
      &mut tasks,
    )
    .await;
    assert!(next.is_none());
    assert!(
      tasks.is_empty(),
      "completed connections remain retained: {}",
      tasks.len()
    );
  }

  #[tokio::test]
  async fn live_connections_remain_tracked_for_shutdown_drain() {
    let mut tasks = JoinSet::new();
    let (finish, finished) = tokio::sync::oneshot::channel();
    tasks.spawn(async {
      finished.await.unwrap();
    });
    let accepted = next_connection(
      std::future::ready(Some(42)),
      std::future::pending(),
      &mut tasks,
    )
    .await;
    assert_eq!(accepted, Some(42));
    assert_eq!(tasks.len(), 1);
    finish.send(()).unwrap();
    tasks.join_next().await.unwrap().unwrap();
    assert!(tasks.is_empty());
  }
}
