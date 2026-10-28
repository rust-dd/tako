use std::collections::HashMap;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use tokio::sync::mpsc;

use super::Message;
use super::QUEUED_RECORDS;
use super::Trace;

pub(super) fn create_file(path: &Path) -> io::Result<File> {
  let mut options = OpenOptions::new();
  options.write(true).create_new(true);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
  }
  options.open(path)
}

pub(super) fn run(directory: &Path, mut receiver: mpsc::Receiver<Message>, stopped: &AtomicBool) {
  let mut files = HashMap::<String, Output>::new();
  let mut processed = 0;
  while let Some(message) = receiver.blocking_recv() {
    match message {
      Message::Record(trace, bytes) => {
        files
          .entry(trace.name.clone())
          .or_insert_with(|| Output::new(directory, trace.clone()))
          .write(&bytes);
        trace.pending.fetch_sub(1, Ordering::AcqRel);
        if trace.closed.load(Ordering::Acquire) && trace.pending.load(Ordering::Acquire) == 0 {
          files.remove(&trace.name);
        }
      }
      Message::Close(trace) => {
        if trace.pending.load(Ordering::Acquire) == 0 {
          files.remove(&trace.name);
        }
      }
      Message::Stop => {}
    }
    processed += 1;
    if processed == QUEUED_RECORDS || receiver.is_empty() {
      // A full queue can discard a close notification; sweep completed traces as well.
      files.retain(|_, output| {
        !output.trace.closed.load(Ordering::Acquire)
          || output.trace.pending.load(Ordering::Acquire) != 0
      });
      processed = 0;
    }
    if stopped.load(Ordering::Acquire) && receiver.is_empty() {
      break;
    }
  }
}

struct Output {
  trace: Arc<Trace>,
  file: Option<File>,
  bytes: u64,
}

impl Output {
  fn new(directory: &Path, trace: Arc<Trace>) -> Self {
    let file = match create_file(&directory.join(&trace.name)) {
      Ok(file) => Some(file),
      Err(error) => {
        trace.fail(error);
        None
      }
    };
    Self {
      trace,
      file,
      bytes: 0,
    }
  }

  fn write(&mut self, bytes: &[u8]) {
    let Some(file) = &mut self.file else { return };
    if let Err(error) = file.write_all(bytes) {
      // Restore the last complete record if the device accepted only part of this one.
      let _ = file.set_len(self.bytes);
      self.trace.fail(error);
      self.file = None;
    } else {
      self.bytes += bytes.len() as u64;
    }
  }
}
