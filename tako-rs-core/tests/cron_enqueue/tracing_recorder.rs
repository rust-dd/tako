use std::collections::BTreeMap;

use tokio::sync::mpsc;
use tracing::Event;
use tracing::Level;
use tracing::Metadata;
use tracing::Subscriber;
use tracing::span::Attributes;
use tracing::span::Id;
use tracing::span::Record;

pub struct Log {
  pub level: Level,
  pub fields: BTreeMap<String, String>,
}

pub struct LogRecorder(mpsc::UnboundedSender<Log>);

impl LogRecorder {
  pub fn channel() -> (Self, mpsc::UnboundedReceiver<Log>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    (Self(sender), receiver)
  }
}

impl Subscriber for LogRecorder {
  fn enabled(&self, metadata: &Metadata<'_>) -> bool {
    metadata.target().ends_with("queue::cron")
  }

  fn new_span(&self, _: &Attributes<'_>) -> Id {
    Id::from_u64(1)
  }
  fn record(&self, _: &Id, _: &Record<'_>) {}
  fn record_follows_from(&self, _: &Id, _: &Id) {}
  fn enter(&self, _: &Id) {}
  fn exit(&self, _: &Id) {}

  fn event(&self, event: &Event<'_>) {
    let mut fields = Fields(BTreeMap::new());
    event.record(&mut fields);
    let _ = self.0.send(Log {
      level: *event.metadata().level(),
      fields: fields.0,
    });
  }
}

struct Fields(BTreeMap<String, String>);

impl tracing::field::Visit for Fields {
  fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
    self.0.insert(field.name().to_owned(), value.to_owned());
  }

  fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
    self.0.insert(field.name().to_owned(), format!("{value:?}"));
  }
}
