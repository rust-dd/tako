//! Counts request body bytes as they stream, without buffering, through a [`ProgressTracker`]
//! extension and an optional callback.
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
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;

use bytes::Bytes;
use http_body::Body;
use http_body::Frame;
use http_body::SizeHint;
use pin_project_lite::pin_project;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::types::BoxError;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

#[derive(Debug, Clone)]
pub struct ProgressState {
  pub bytes_read: u64,
  /// From `Content-Length`, when present.
  pub total_bytes: Option<u64>,
}

impl ProgressState {
  /// Capped at 100; `None` without `Content-Length`.
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

/// Inserted into request extensions; reads the live byte count.
#[derive(Clone)]
pub struct ProgressTracker {
  bytes_read: Arc<AtomicU64>,
  total_bytes: Option<u64>,
}

impl ProgressTracker {
  pub fn state(&self) -> ProgressState {
    ProgressState {
      bytes_read: self.bytes_read.load(Ordering::Relaxed),
      total_bytes: self.total_bytes,
    }
  }

  pub fn bytes_read(&self) -> u64 {
    self.bytes_read.load(Ordering::Relaxed)
  }

  pub fn total_bytes(&self) -> Option<u64> {
    self.total_bytes
  }

  pub fn percent(&self) -> Option<u8> {
    self.state().percent()
  }
}

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
  pub fn new() -> Self {
    Self {
      callback: None,
      min_notify_interval: 0,
    }
  }

  /// Runs inline while the body is polled, so it must not block; a final call fires at its end.
  pub fn on_progress<F>(mut self, f: F) -> Self
  where
    F: Fn(ProgressState) + Send + Sync + 'static,
  {
    self.callback = Some(Arc::new(f));
    self
  }

  /// Minimum bytes between callbacks; 0, the default, notifies on every chunk.
  pub fn min_notify_interval_bytes(mut self, bytes: u64) -> Self {
    self.min_notify_interval = bytes;
    self
  }
}

pin_project! {
  struct ProgressBody<B> {
    #[pin]
    inner: B,
    bytes_read: Arc<AtomicU64>,
    total_bytes: Option<u64>,
    last_notified_at: u64,
    min_interval: u64,
    callback: Option<Arc<dyn Fn(ProgressState) + Send + Sync + 'static>>,
    final_notified: bool,
  }
}

impl<B> Body for ProgressBody<B>
where
  B: Body<Data = Bytes>,
  B::Error: Into<BoxError>,
{
  type Data = Bytes;
  type Error = BoxError;

  fn poll_frame(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
    let this = self.project();
    match this.inner.poll_frame(cx) {
      Poll::Ready(Some(Ok(frame))) => {
        if let Some(data) = frame.data_ref() {
          let added = data.len() as u64;
          let total = this.bytes_read.fetch_add(added, Ordering::Relaxed) + added;
          if let Some(cb) = this.callback.as_ref()
            && (*this.min_interval == 0 || total - *this.last_notified_at >= *this.min_interval)
          {
            *this.last_notified_at = total;
            cb(ProgressState {
              bytes_read: total,
              total_bytes: *this.total_bytes,
            });
          }
        }
        Poll::Ready(Some(Ok(frame)))
      }
      Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e.into()))),
      Poll::Ready(None) => {
        // Once, even for empty uploads, so callers always see the final total.
        if let Some(cb) = this.callback.as_ref()
          && !*this.final_notified
        {
          *this.final_notified = true;
          cb(ProgressState {
            bytes_read: this.bytes_read.load(Ordering::Relaxed),
            total_bytes: *this.total_bytes,
          });
        }
        Poll::Ready(None)
      }
      Poll::Pending => Poll::Pending,
    }
  }

  fn is_end_stream(&self) -> bool {
    self.inner.is_end_stream()
  }

  fn size_hint(&self) -> SizeHint {
    self.inner.size_hint()
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
        let total_bytes = req
          .headers()
          .get(http::header::CONTENT_LENGTH)
          .and_then(|v| v.to_str().ok())
          .and_then(|s| s.parse::<u64>().ok());

        let bytes_read = Arc::new(AtomicU64::new(0));

        let tracker = ProgressTracker {
          bytes_read: Arc::clone(&bytes_read),
          total_bytes,
        };
        req.extensions_mut().insert(tracker);

        let (parts, body) = req.into_parts();
        let progress_body = ProgressBody {
          inner: body,
          bytes_read,
          total_bytes,
          last_notified_at: 0,
          min_interval,
          callback,
          final_notified: false,
        };
        let req = http::Request::from_parts(parts, TakoBody::new(progress_body));

        next.run(req).await
      })
    }
  }
}
