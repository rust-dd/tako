use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::SystemTime;

use tokio::sync::mpsc;
use tokio::sync::oneshot;

mod worker;
mod writer;

const QUEUED_RECORDS: usize = 256;
const MAX_RECORD_BYTES: usize = 64 * 1024;

pub(super) struct Qlog {
  sender: mpsc::Sender<Message>,
  stopped: Arc<AtomicBool>,
  done: Option<oneshot::Receiver<()>>,
}

impl Qlog {
  pub(super) fn new(directory: &Path) -> io::Result<Self> {
    let directory = directory.canonicalize()?;
    if !directory.is_dir() {
      return Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "qlog destination must be a directory",
      ));
    }
    let probe = directory.join(unique_name());
    drop(worker::create_file(&probe)?);
    std::fs::remove_file(probe)?;

    let (sender, receiver) = mpsc::channel(QUEUED_RECORDS);
    let (finished, done) = oneshot::channel();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = stopped.clone();
    std::thread::Builder::new()
      .name("tako-qlog".into())
      .spawn(move || {
        worker::run(&directory, receiver, &stop);
        let _ = finished.send(());
      })?;
    Ok(Self {
      sender,
      stopped,
      done: Some(done),
    })
  }

  pub(super) fn stream(&self) -> Option<quinn::QlogStream> {
    let mut config = quinn::QlogConfig::default();
    config.title(Some("Tako HTTP/3 server".into()));
    config.writer(Box::new(writer::Writer::new(
      self.sender.clone(),
      self.stopped.clone(),
    )));
    config.into_stream()
  }

  pub(super) async fn finish(mut self) {
    self.stop();
    if let Some(done) = self.done.take() {
      let _ = done.await;
    }
  }

  fn stop(&self) {
    self.stopped.store(true, Ordering::Release);
    let _ = self.sender.try_send(Message::Stop);
  }
}

impl Drop for Qlog {
  fn drop(&mut self) {
    self.stop();
  }
}

struct Trace {
  name: String,
  failed: AtomicBool,
  closed: AtomicBool,
  pending: AtomicUsize,
}

impl Trace {
  fn fail(&self, reason: impl std::fmt::Display) {
    if !self.failed.swap(true, Ordering::AcqRel) {
      tracing::warn!(file = %self.name, "HTTP/3 qlog trace stopped: {reason}");
    }
  }
}

enum Message {
  Record(Arc<Trace>, Vec<u8>),
  Close(Arc<Trace>),
  Stop,
}

fn unique_name() -> String {
  static NEXT: AtomicU64 = AtomicU64::new(0);
  let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
  let time = SystemTime::now()
    .duration_since(SystemTime::UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  format!("tako-{}-{time:x}-{sequence:x}.sqlog", std::process::id())
}
