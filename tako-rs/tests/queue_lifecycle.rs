use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tako::queue::Queue;
use tako::queue::QueueRunError;
use tokio::sync::Notify;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

async fn bounded<T>(future: impl Future<Output = T>) -> T {
  let future = std::pin::pin!(future);
  let deadline = std::pin::pin!(async {
    #[cfg(not(feature = "compio"))]
    tokio::time::sleep(Duration::from_secs(1)).await;
    #[cfg(feature = "compio")]
    compio::time::sleep(Duration::from_secs(1)).await;
  });
  match futures_util::future::select(future, deadline).await {
    futures_util::future::Either::Left((result, _)) => result,
    futures_util::future::Either::Right(_) => panic!("queue lifecycle operation did not finish"),
  }
}

struct Released(Option<oneshot::Sender<()>>);

impl Drop for Released {
  fn drop(&mut self) {
    if let Some(sender) = self.0.take() {
      let _ = sender.send(());
    }
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn shutdown_before_start_preserves_pending_jobs() {
  let queue = Queue::new();
  let ready = queue.push("ready", &1).await.unwrap();
  let delayed = queue
    .push_delayed("delayed", &2, Duration::from_secs(3600))
    .await
    .unwrap();

  queue.shutdown(Duration::from_secs(1)).await;

  assert_eq!(queue.pending_count(), 0);
  let mut ids = queue
    .dead_letters()
    .iter()
    .map(|job| job.id)
    .collect::<Vec<_>>();
  ids.sort_unstable();
  assert_eq!(ids, [ready, delayed]);
  queue.start();
  assert!(queue.push("late", &()).await.is_err());
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn shutdown_waits_for_workers_that_have_not_been_polled() {
  let queue = Queue::new();
  queue.push("pending", &()).await.unwrap();
  queue.start();

  queue.shutdown(Duration::from_secs(1)).await;

  assert_eq!(queue.pending_count(), 0);
  assert_eq!(queue.dead_letter_count(), 1);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn repeated_start_calls_do_not_multiply_workers() {
  let queue = Queue::builder().workers(2).build();
  let (started, mut received) = mpsc::unbounded_channel();
  let release = Arc::new(Notify::new());
  let gate = release.clone();
  queue.register("blocked", move |_| {
    let started = started.clone();
    let gate = gate.clone();
    async move {
      started.send(()).unwrap();
      gate.notified().await;
      Ok(())
    }
  });
  for _ in 0..8 {
    queue.push("blocked", &()).await.unwrap();
  }
  queue.start();
  queue.clone().start();
  queue.start();
  bounded(received.recv()).await.unwrap();
  bounded(received.recv()).await.unwrap();

  queue.shutdown(Duration::from_millis(10)).await;

  assert_eq!(received.len(), 0, "extra workers started queued jobs");
  release.notify_waiters();
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn dropping_the_last_queue_releases_idle_handlers() {
  let queue = Queue::new();
  let (released, finished) = oneshot::channel();
  let resource = Released(Some(released));
  queue.register("idle", move |_| {
    std::hint::black_box(&resource);
    async { Ok(()) }
  });
  queue.start();
  drop(queue);

  bounded(finished).await.unwrap();
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn timeout_cancels_running_jobs_and_preserves_all_unfinished_jobs() {
  let queue = Queue::builder().workers(1).build();
  let (started, running) = oneshot::channel();
  let started = std::sync::Mutex::new(Some(started));
  let (released, finished) = oneshot::channel();
  let resource = Arc::new(Released(Some(released)));
  queue.register("blocked", move |_| {
    let resource = resource.clone();
    let started = started.lock().unwrap().take().unwrap();
    async move {
      started.send(()).unwrap();
      std::future::pending::<()>().await;
      drop(resource);
      Ok(())
    }
  });
  let active = queue.push("blocked", &1).await.unwrap();
  let pending = queue.push("blocked", &2).await.unwrap();
  let handle = queue.spawn();
  bounded(running).await.unwrap();

  assert_eq!(
    handle.shutdown(Duration::ZERO).await,
    Err(QueueRunError::Timeout)
  );
  bounded(handle.join()).await;

  finished.await.unwrap();
  assert_eq!(handle.result().await, Err(QueueRunError::Timeout));
  assert_eq!(queue.pending_count(), 0);
  assert_eq!(queue.inflight_count(), 0);
  let mut ids = queue
    .dead_letters()
    .iter()
    .map(|job| job.id)
    .collect::<Vec<_>>();
  ids.sort_unstable();
  assert_eq!(ids, [active, pending]);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn all_shutdown_waiters_observe_worker_completion() {
  let queue = Queue::builder().workers(2).build();
  let handle = queue.spawn();
  let other = handle.clone();

  let (first, second, result) = bounded(async {
    futures_util::join!(
      handle.shutdown(Duration::from_secs(1)),
      other.shutdown(Duration::from_secs(1)),
      handle.result(),
    )
  })
  .await;

  assert_eq!(first, Ok(()));
  assert_eq!(second, Ok(()));
  assert_eq!(result, Ok(()));
  assert!(queue.push("closed", &()).await.is_err());
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn dropping_the_last_handle_releases_a_handler_that_captures_the_queue() {
  let queue = Queue::builder().workers(1).build();
  let captured = queue.clone();
  let (released, mut finished) = oneshot::channel();
  let resource = Released(Some(released));
  queue.register("recursive", move |_| {
    std::hint::black_box((&resource, &captured));
    async { Ok(()) }
  });
  let handle = queue.spawn();
  let remaining = queue.spawn();
  drop(queue);
  drop(handle);
  assert!(matches!(
    finished.try_recv(),
    Err(oneshot::error::TryRecvError::Empty)
  ));

  drop(remaining);

  bounded(finished).await.unwrap();
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn explicit_handle_keeps_workers_alive_after_queue_drop() {
  let queue = Queue::builder().workers(1).build();
  let (started, running) = oneshot::channel();
  let started = std::sync::Mutex::new(Some(started));
  let release = Arc::new(Notify::new());
  let gate = release.clone();
  queue.register("blocked", move |_| {
    let started = started.lock().unwrap().take().unwrap();
    let gate = gate.clone();
    async move {
      started.send(()).unwrap();
      gate.notified().await;
      Ok(())
    }
  });
  queue.push("blocked", &()).await.unwrap();
  let handle = queue.spawn();
  bounded(running).await.unwrap();
  drop(queue);
  release.notify_one();

  assert_eq!(
    bounded(handle.shutdown(Duration::from_secs(1))).await,
    Ok(())
  );
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn panicking_handler_cleanup_reports_worker_failure_to_all_waiters() {
  struct PanicsOnDrop;
  impl Drop for PanicsOnDrop {
    fn drop(&mut self) {
      panic!("handler capture cleanup failed");
    }
  }
  let queue = Queue::new();
  let resource = PanicsOnDrop;
  queue.register("cleanup", move |_| {
    std::hint::black_box(&resource);
    async { Ok(()) }
  });
  let handle = queue.spawn();

  let (first, second) = bounded(async {
    futures_util::join!(handle.shutdown(Duration::from_secs(1)), handle.result())
  })
  .await;

  assert_eq!(first, Err(QueueRunError::WorkerPanicked));
  assert_eq!(second, Err(QueueRunError::WorkerPanicked));
}

fn run_future<F: Future>(future: F) -> F::Output {
  #[cfg(not(feature = "compio"))]
  {
    tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .unwrap()
      .block_on(future)
  }
  #[cfg(feature = "compio")]
  compio::runtime::Runtime::new().unwrap().block_on(future)
}

fn racing_starters(
  queue: &Queue,
  barrier: Arc<std::sync::Barrier>,
  ready: mpsc::UnboundedSender<()>,
) -> (
  tokio::sync::watch::Sender<()>,
  Vec<std::thread::JoinHandle<()>>,
) {
  let (stop, stopped) = tokio::sync::watch::channel(());
  let threads = (0..8)
    .map(|_| {
      let queue = queue.clone();
      let barrier = barrier.clone();
      let ready = ready.clone();
      let mut stopped = stopped.clone();
      std::thread::spawn(move || {
        run_future(async move {
          barrier.wait();
          queue.start();
          ready.send(()).unwrap();
          let _ = stopped.changed().await;
        });
      })
    })
    .collect();
  (stop, threads)
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn concurrent_start_calls_have_the_configured_parallelism() {
  let queue = Queue::builder().workers(3).build();
  let (started, mut received) = mpsc::unbounded_channel();
  queue.register("blocked", move |_| {
    let started = started.clone();
    async move {
      started.send(()).unwrap();
      std::future::pending::<()>().await;
      Ok(())
    }
  });
  for _ in 0..20 {
    queue.push("blocked", &()).await.unwrap();
  }
  let (ready, mut waiting) = mpsc::unbounded_channel();
  let (stop, threads) = racing_starters(&queue, Arc::new(std::sync::Barrier::new(8)), ready);
  for _ in 0..8 {
    bounded(waiting.recv()).await.unwrap();
  }
  for _ in 0..3 {
    bounded(received.recv()).await.unwrap();
  }
  let handle = queue.spawn();
  assert_eq!(
    handle.shutdown(Duration::from_millis(10)).await,
    Err(QueueRunError::Timeout)
  );
  bounded(handle.join()).await;
  drop(stop);
  for thread in threads {
    thread.join().unwrap();
  }

  assert_eq!(received.len(), 0);
  assert_eq!(queue.dead_letter_count(), 20);
  assert_eq!(queue.inflight_count(), 0);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn concurrent_start_and_shutdown_preserve_every_job() {
  use std::sync::atomic::AtomicUsize;
  use std::sync::atomic::Ordering;

  let queue = Queue::builder().workers(3).build();
  let completed = Arc::new(AtomicUsize::new(0));
  let count = completed.clone();
  queue.register("count", move |_| {
    let count = count.clone();
    async move {
      count.fetch_add(1, Ordering::SeqCst);
      Ok(())
    }
  });
  for _ in 0..64 {
    queue.push("count", &()).await.unwrap();
  }
  let (ready, mut waiting) = mpsc::unbounded_channel();
  let barrier = Arc::new(std::sync::Barrier::new(9));
  let (stop, threads) = racing_starters(&queue, barrier.clone(), ready);
  barrier.wait();
  queue.shutdown(Duration::from_secs(1)).await;
  for _ in 0..8 {
    bounded(waiting.recv()).await.unwrap();
  }
  let handle = queue.spawn();
  assert_eq!(bounded(handle.result()).await, Ok(()));
  drop(stop);
  for thread in threads {
    thread.join().unwrap();
  }

  assert_eq!(
    completed.load(Ordering::SeqCst) + queue.dead_letter_count(),
    64
  );
  assert_eq!(queue.inflight_count(), 0);
  assert_eq!(queue.pending_count(), 0);
  assert!(queue.push("late", &()).await.is_err());
}

#[test]
fn dropping_the_worker_runtime_reports_abort_without_leaking_join_waiters() {
  let queue = Queue::new();
  let handle = run_future(async { queue.spawn() });

  assert_eq!(
    run_future(bounded(handle.result())),
    Err(QueueRunError::Aborted)
  );
}
