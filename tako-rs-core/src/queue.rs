//! In-memory background job queue with named queues, retry policies, and dead letter support.
//!
//! Provides a lightweight task queue for deferring work to background workers —
//! useful for sending emails, webhooks, async processing, etc.
//!
//! # Features
//!
//! - **Named queues** — separate logical channels (e.g. `"email"`, `"webhook"`)
//! - **Configurable workers** — per-queue concurrency limit
//! - **Retry policy** — fixed or exponential backoff with max attempts
//! - **Delayed jobs** — schedule execution after a duration
//! - **Dead letter queue** — failed jobs stored for inspection
//! - **Graceful shutdown** — drain in-flight jobs before exit
//!
//! # Examples
//!
//! ```rust,no_run
//! use tako::queue::{Queue, RetryPolicy, Job};
//! use std::time::Duration;
//!
//! # async fn example() {
//! let queue = Queue::builder()
//!     .workers(4)
//!     .retry(RetryPolicy::exponential(3, Duration::from_secs(1)))
//!     .build();
//!
//! queue.register("send_email", |job: Job| async move {
//!     let to: String = job.deserialize()?;
//!     println!("Sending email to {to}");
//!     Ok(())
//! });
//!
//! queue.push("send_email", &"user@example.com").await.unwrap();
//! # }
//! ```

/// Pluggable queue backend abstraction (v2). The bundled `Queue` keeps its
/// in-process semantics; opt into a remote broker via [`backend::QueueBackend`].
pub mod backend;

/// Builder for configuring a [`Queue`].
mod builder;

/// Cron scheduling on top of `QueueBackend` (opt-in via `queue-cron` feature).
#[cfg(feature = "queue-cron")]
#[cfg_attr(docsrs, doc(cfg(feature = "queue-cron")))]
pub mod cron;

/// Error type for queue operations.
mod error;

/// Job and dead-letter task types.
mod job;

/// Retry/backoff configuration.
mod retry;

/// Queue runtime: builder and lifecycle handles.
mod runtime;

/// Queue signal ids and emission helper.
#[cfg(feature = "signals")]
mod signals;

/// Background worker loop draining pending jobs.
mod worker;

pub use builder::QueueBuilder;
pub use error::QueueError;
pub use job::DeadJob;
pub use job::Job;
pub use retry::RetryPolicy;
pub use runtime::Queue;
#[cfg(feature = "signals")]
pub(crate) use signals::emit_queue_signal;
#[cfg(feature = "signals")]
pub use signals::signal_ids;
