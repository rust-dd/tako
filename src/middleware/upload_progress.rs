//! Upload progress tracking middleware.
//!
//! Wraps the request body to track upload progress and report it via a callback
//! or through request extensions. Handlers can access the progress tracker to
//! monitor bytes received.
//!
//! # Examples
//!
//! ```rust
//! use tako::middleware::upload_progress::UploadProgress;
//! use tako::middleware::IntoMiddleware;
//!
//! // With callback
//! let progress = UploadProgress::new()
//!     .on_progress(|state| {
//!         println!("{}% ({}/{})",
//!             state.percent().unwrap_or(0),
//!             state.bytes_read,
//!             state.total_bytes.unwrap_or(0),
//!         );
//!     });
//! let mw = progress.into_middleware();
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::middleware::IntoMiddleware;
use crate::middleware::Next;
use crate::types::Request;
use crate::types::Response;

/// Upload progress state accessible during and after upload.
#[derive(Debug, Clone)]
pub struct ProgressState {
  /// Number of bytes read so far.
  pub bytes_read: u64,
  /// Total expected bytes (from Content-Length), if known.
  pub total_bytes: Option<u64>,
}

impl ProgressState {
  /// Returns the upload percentage (0-100), if total is known.
  pub fn percent(&self) -> Option<u8> {
    self.total_bytes.map(|total| {
      if total == 0 {
        100
      } else {
        ((self.bytes_read as f64 / total as f64) * 100.0).min(100.0) as u8
      }
    })
  }
}

/// Shared progress tracker inserted into request extensions.
///
/// Handlers can access this to check current upload progress.
#[derive(Clone)]
pub struct ProgressTracker {
  bytes_read: Arc<AtomicU64>,
  total_bytes: Option<u64>,
}

impl ProgressTracker {
  /// Returns the current progress state.
  pub fn state(&self) -> ProgressState {
    ProgressState {
      bytes_read: self.bytes_read.load(Ordering::Relaxed),
      total_bytes: self.total_bytes,
    }
  }

  /// Returns the number of bytes read so far.
  pub fn bytes_read(&self) -> u64 {
    self.bytes_read.load(Ordering::Relaxed)
  }

  /// Returns the total expected bytes, if known.
  pub fn total_bytes(&self) -> Option<u64> {
    self.total_bytes
  }

  /// Returns the upload percentage (0-100), if total is known.
  pub fn percent(&self) -> Option<u8> {
    self.state().percent()
  }
}

/// Upload progress middleware configuration.
///
/// # Examples
///
/// ```rust
/// use tako::middleware::upload_progress::UploadProgress;
/// use tako::middleware::IntoMiddleware;
///
/// // Simple progress tracking (access via ProgressTracker in extensions)
/// let progress = UploadProgress::new();
///
/// // With progress callback
/// let progress = UploadProgress::new()
///     .on_progress(|state| {
///         if let Some(pct) = state.percent() {
///             println!("Upload: {pct}%");
///         }
///     })
///     .min_notify_interval_bytes(8192); // notify at most every 8KB
/// ```
pub struct UploadProgress {
  callback: Option<Arc<dyn Fn(ProgressState) + Send + Sync + 'static>>,
  min_notify_interval: u64,
}

impl Default for UploadProgress {
  fn default() -> Self {
    Self::new()
  }
}

impl UploadProgress {
  /// Creates a new upload progress middleware.
  pub fn new() -> Self {
    Self {
      callback: None,
      min_notify_interval: 0,
    }
  }

  /// Sets a callback that is called as bytes are received.
  pub fn on_progress<F>(mut self, f: F) -> Self
  where
    F: Fn(ProgressState) + Send + Sync + 'static,
  {
    self.callback = Some(Arc::new(f));
    self
  }

  /// Sets the minimum byte interval between progress notifications.
  ///
  /// This prevents the callback from being called too frequently for
  /// large uploads. Default is 0 (notify on every chunk).
  pub fn min_notify_interval_bytes(mut self, bytes: u64) -> Self {
    self.min_notify_interval = bytes;
    self
  }
}

impl IntoMiddleware for UploadProgress {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let callback = self.callback;
    let min_interval = self.min_notify_interval;

    move |mut req: Request, next: Next| {
      let callback = callback.clone();

      Box::pin(async move {
        // Extract total from Content-Length header
        let total_bytes = req
          .headers()
          .get(http::header::CONTENT_LENGTH)
          .and_then(|v| v.to_str().ok())
          .and_then(|s| s.parse::<u64>().ok());

        let bytes_read = Arc::new(AtomicU64::new(0));

        // Insert tracker into extensions for handler access
        let tracker = ProgressTracker {
          bytes_read: Arc::clone(&bytes_read),
          total_bytes,
        };
        req.extensions_mut().insert(tracker);

        // Collect body while tracking progress
        use http_body_util::BodyExt;
        let body = req.body_mut();
        let mut collected = Vec::new();
        let mut last_notified_at: u64 = 0;

        // Use frame-by-frame collection for progress reporting
        loop {
          match body.frame().await {
            Some(Ok(frame)) => {
              if let Some(data) = frame.data_ref() {
                collected.extend_from_slice(data);
                let total = bytes_read.fetch_add(data.len() as u64, Ordering::Relaxed)
                  + data.len() as u64;

                // Fire callback if interval threshold met
                if let Some(cb) = &callback {
                  if min_interval == 0 || total - last_notified_at >= min_interval {
                    last_notified_at = total;
                    cb(ProgressState {
                      bytes_read: total,
                      total_bytes,
                    });
                  }
                }
              }
            }
            Some(Err(_)) => break,
            None => break,
          }
        }

        // Final notification
        if let Some(cb) = &callback {
          let final_read = bytes_read.load(Ordering::Relaxed);
          if final_read != last_notified_at {
            cb(ProgressState {
              bytes_read: final_read,
              total_bytes,
            });
          }
        }

        // Reconstruct request with collected body
        let (parts, _) = req.into_parts();
        let req = http::Request::from_parts(
          parts,
          crate::body::TakoBody::from(bytes::Bytes::from(collected)),
        );

        next.run(req).await
      })
    }
  }
}
