use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tokio::sync::mpsc;

use super::Writer;
use crate::h3_common::qlog::MAX_RECORD_BYTES;
use crate::h3_common::qlog::Message;

#[test]
fn fragmented_records_are_queued_only_when_complete() {
  let (sender, mut receiver) = mpsc::channel(4);
  let mut writer = Writer::new(sender, Arc::new(AtomicBool::new(false)));
  writer.write_all(b"\x1e{\"time\":").unwrap();
  writer.flush().unwrap();
  assert!(receiver.try_recv().is_err());
  writer.write_all(b"1}\n").unwrap();
  let Message::Record(_, bytes) = receiver.try_recv().unwrap() else {
    panic!("expected record")
  };
  assert_eq!(bytes, b"\x1e{\"time\":1}\n");
}

#[test]
fn a_full_queue_stops_the_trace_at_a_complete_record() {
  let (sender, mut receiver) = mpsc::channel(1);
  let mut writer = Writer::new(sender, Arc::new(AtomicBool::new(false)));
  writer.write_all(b"\x1e{}\n").unwrap();
  writer.write_all(b"\x1e{\"time\":").unwrap();
  writer.write_all(b"1}\n").unwrap();
  let Message::Record(_, bytes) = receiver.try_recv().unwrap() else {
    panic!("expected record")
  };
  assert_eq!(bytes, b"\x1e{}\n");
  writer.write_all(b"\x1e{\"time\":2}\n").unwrap();
  assert!(receiver.try_recv().is_err());
}

#[test]
fn an_oversized_record_is_discarded_without_a_partial_write() {
  let (sender, mut receiver) = mpsc::channel(1);
  let mut writer = Writer::new(sender, Arc::new(AtomicBool::new(false)));
  writer.write_all(&vec![b' '; MAX_RECORD_BYTES + 1]).unwrap();
  writer.write_all(b"\n").unwrap();
  assert!(receiver.try_recv().is_err());
}
