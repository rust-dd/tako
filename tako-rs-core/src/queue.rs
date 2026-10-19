//! In-memory background job queue with retries, delayed and deduplicated jobs, a dead letter
//! queue and graceful shutdown.
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

pub mod backend;
mod builder;
#[cfg(feature = "queue-cron")]
#[cfg_attr(docsrs, doc(cfg(feature = "queue-cron")))]
pub mod cron;
mod error;
mod job;
mod pending;
mod retry;
mod runtime;
#[cfg(feature = "signals")]
mod signals;
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
