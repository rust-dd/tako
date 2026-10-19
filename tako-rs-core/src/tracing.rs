//! A `tracing-subscriber` formatter with source locations, installed by [`init_tracing`].

use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;

pub use tracing::level_filters::LevelFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const LEVEL_OFF: u8 = 0;
const LEVEL_ERROR: u8 = 1;
const LEVEL_WARN: u8 = 2;
const LEVEL_INFO: u8 = 3;
const LEVEL_DEBUG: u8 = 4;
const LEVEL_TRACE: u8 = 5;

static TRACING_LEVEL: AtomicU8 = AtomicU8::new(LEVEL_DEBUG);

fn encode_level(filter: LevelFilter) -> u8 {
  if filter == LevelFilter::OFF {
    LEVEL_OFF
  } else if filter == LevelFilter::ERROR {
    LEVEL_ERROR
  } else if filter == LevelFilter::WARN {
    LEVEL_WARN
  } else if filter == LevelFilter::INFO {
    LEVEL_INFO
  } else if filter == LevelFilter::DEBUG {
    LEVEL_DEBUG
  } else {
    LEVEL_TRACE
  }
}

fn decode_level(value: u8) -> LevelFilter {
  match value {
    LEVEL_OFF => LevelFilter::OFF,
    LEVEL_ERROR => LevelFilter::ERROR,
    LEVEL_WARN => LevelFilter::WARN,
    LEVEL_INFO => LevelFilter::INFO,
    LEVEL_TRACE => LevelFilter::TRACE,
    _ => LevelFilter::DEBUG,
  }
}

/// Sets the level [`init_tracing`] installs (default: DEBUG); it has no effect afterwards.
pub fn set_tracing_level(level_filter: LevelFilter) {
  TRACING_LEVEL.store(encode_level(level_filter), Ordering::Relaxed);
}

/// Installs the formatting subscriber as the global default; does nothing on later calls or
/// when another subscriber is already installed.
pub fn init_tracing() {
  use std::sync::Once;
  static INIT: Once = Once::new();
  INIT.call_once(|| {
    let _ = tracing_subscriber::registry()
      .with(
        tracing_subscriber::fmt::layer()
          .with_span_events(FmtSpan::CLOSE)
          .with_file(true)
          .with_line_number(true)
          .with_level(true)
          .with_filter(decode_level(TRACING_LEVEL.load(Ordering::Relaxed))),
      )
      .try_init();
  });
}
