//! Moves accepted connections to the worker with the fewest live connections.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use tokio::net::TcpStream;
use tokio::sync::mpsc;

/// An accepted connection on its way to another worker.
pub(crate) type Handoff = (std::net::TcpStream, SocketAddr);

/// Live connection counts and inboxes shared by every worker.
pub(crate) struct Balancer {
  live: Box<[AtomicUsize]>,
  inboxes: Box<[mpsc::UnboundedSender<Handoff>]>,
}

/// One worker's handle on the shared balancer.
pub(crate) struct WorkerBalance {
  pub(crate) balancer: Arc<Balancer>,
  pub(crate) inbox: mpsc::UnboundedReceiver<Handoff>,
}

impl Balancer {
  pub(crate) fn for_workers(workers: usize) -> Vec<WorkerBalance> {
    let (senders, receivers): (Vec<_>, Vec<_>) =
      (0..workers).map(|_| mpsc::unbounded_channel()).unzip();
    let balancer = Arc::new(Self {
      live: (0..workers).map(|_| AtomicUsize::new(0)).collect(),
      inboxes: senders.into_boxed_slice(),
    });
    receivers
      .into_iter()
      .map(|inbox| WorkerBalance {
        balancer: Arc::clone(&balancer),
        inbox,
      })
      .collect()
  }

  /// Picks the worker that serves a connection `worker` accepted and counts it there.
  pub(crate) fn assign(&self, worker: usize) -> usize {
    let (least, least_live) = self
      .live
      .iter()
      .enumerate()
      .map(|(index, live)| (index, live.load(Ordering::Relaxed)))
      .min_by_key(|&(_, live)| live)
      .unwrap_or((worker, 0));
    let target = if least_live < self.live[worker].load(Ordering::Relaxed) {
      least
    } else {
      worker
    };
    self.live[target].fetch_add(1, Ordering::Relaxed);
    target
  }

  /// Sends a connection to `target`, returning it if that worker has stopped.
  pub(crate) fn hand_off(&self, target: usize, handoff: Handoff) -> Result<(), Handoff> {
    self.inboxes[target].send(handoff).map_err(|error| error.0)
  }

  /// Moves one counted connection from `from` to `to`.
  pub(crate) fn reassign(&self, from: usize, to: usize) {
    self.release(from);
    self.live[to].fetch_add(1, Ordering::Relaxed);
  }

  /// Uncounts a connection that `worker` will not serve.
  pub(crate) fn release(&self, worker: usize) {
    self.live[worker].fetch_sub(1, Ordering::Relaxed);
  }

  /// Keeps a connection `worker` accepted, or hands it to a less busy worker
  /// and returns `None`.
  pub(crate) fn keep_or_hand_off(
    self: &Arc<Self>,
    worker: usize,
    stream: TcpStream,
    peer: SocketAddr,
  ) -> Option<(TcpStream, LiveConnection)> {
    let target = self.assign(worker);
    if target == worker {
      return Some((stream, LiveConnection::new(Arc::clone(self), worker)));
    }
    let stream = match stream.into_std() {
      Ok(stream) => stream,
      Err(error) => {
        self.release(target);
        tracing::debug!("worker {worker}: could not detach {peer} for hand-off: {error}");
        return None;
      }
    };
    let Err((stream, _)) = self.hand_off(target, (stream, peer)) else {
      return None;
    };
    self.reassign(target, worker);
    match TcpStream::from_std(stream) {
      Ok(stream) => Some((stream, LiveConnection::new(Arc::clone(self), worker))),
      Err(error) => {
        self.release(worker);
        tracing::debug!("worker {worker}: could not reattach {peer}: {error}");
        None
      }
    }
  }
}

/// Keeps a worker's live count accurate for the lifetime of one connection.
pub(crate) struct LiveConnection {
  balancer: Arc<Balancer>,
  worker: usize,
}

impl LiveConnection {
  /// Takes over a count that [`Balancer::assign`] already recorded for `worker`.
  pub(crate) fn new(balancer: Arc<Balancer>, worker: usize) -> Self {
    Self { balancer, worker }
  }
}

impl Drop for LiveConnection {
  fn drop(&mut self) {
    self.balancer.release(self.worker);
  }
}

#[cfg(test)]
mod tests {
  use super::Balancer;

  #[test]
  fn assign_hands_off_only_when_the_accepting_worker_is_ahead() {
    let workers = Balancer::for_workers(3);
    let balancer = &workers[0].balancer;
    let picks: Vec<usize> = [0, 0, 0, 2, 2]
      .into_iter()
      .map(|worker| balancer.assign(worker))
      .collect();
    assert_eq!(picks, [0, 1, 2, 2, 0]);
  }

  #[test]
  fn released_connections_free_up_their_worker() {
    let workers = Balancer::for_workers(2);
    let balancer = &workers[0].balancer;
    assert_eq!(balancer.assign(0), 0);
    assert_eq!(balancer.assign(0), 1);
    balancer.release(1);
    assert_eq!(balancer.assign(0), 1);
  }
}
