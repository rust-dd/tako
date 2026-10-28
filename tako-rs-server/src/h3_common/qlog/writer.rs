use std::io;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use tokio::sync::mpsc;

use super::MAX_RECORD_BYTES;
use super::Message;
use super::Trace;

/// Buffers qlog's fragmented serializer writes until a complete JSON-sequence record is ready.
pub(super) struct Writer {
  sender: mpsc::Sender<Message>,
  stopped: Arc<AtomicBool>,
  trace: Arc<Trace>,
  record: Vec<u8>,
}

impl Writer {
  pub(super) fn new(sender: mpsc::Sender<Message>, stopped: Arc<AtomicBool>) -> Self {
    Self {
      sender,
      stopped,
      trace: Arc::new(Trace {
        name: super::unique_name(),
        failed: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        pending: AtomicUsize::new(0),
      }),
      record: Vec::new(),
    }
  }

  fn queue_record(&mut self) {
    self.trace.pending.fetch_add(1, Ordering::AcqRel);
    let message = Message::Record(self.trace.clone(), std::mem::take(&mut self.record));
    if let Err(error) = self.sender.try_send(message) {
      self.trace.pending.fetch_sub(1, Ordering::AcqRel);
      self.trace.fail(error);
    }
  }
}

impl Write for Writer {
  fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
    for chunk in bytes.split_inclusive(|byte| *byte == b'\n') {
      if self.stopped.load(Ordering::Acquire) || self.trace.failed.load(Ordering::Acquire) {
        self.record.clear();
        break;
      }
      if self.record.len() + chunk.len() > MAX_RECORD_BYTES {
        self.trace.fail("record exceeds the 64 KiB buffer limit");
        self.record.clear();
        break;
      }
      self.record.extend_from_slice(chunk);
      if chunk.last() == Some(&b'\n') {
        self.queue_record();
      }
    }
    Ok(bytes.len())
  }

  fn flush(&mut self) -> io::Result<()> {
    Ok(())
  }
}

impl Drop for Writer {
  fn drop(&mut self) {
    self.trace.closed.store(true, Ordering::Release);
    let _ = self.sender.try_send(Message::Close(self.trace.clone()));
  }
}

#[cfg(test)]
mod tests;
