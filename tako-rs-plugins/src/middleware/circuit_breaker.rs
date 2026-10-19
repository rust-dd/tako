//! Per-route circuit breaker: opens when the failure ratio over a tumbling window trips the
//! threshold, then admits one half-open probe after the cool-down.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use http::HeaderValue;
use http::StatusCode;
use http::header::RETRY_AFTER;
use parking_lot::Mutex;
use scc::HashMap as SccHashMap;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::router_state::MatchedPath;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

const STATE_CLOSED: u8 = 0;
const STATE_OPEN: u8 = 1;
const STATE_HALF_OPEN: u8 = 2;

/// Frees the half-open probe slot on every exit, including panic and cancellation by an
/// outer `Timeout`; a leaked slot would reject the key forever.
struct ProbeSlotGuard<'a>(&'a AtomicU8);

impl Drop for ProbeSlotGuard<'_> {
  fn drop(&mut self) {
    self.0.store(0, Ordering::Release);
  }
}

#[derive(Default)]
struct State {
  state: AtomicU8,
  successes: AtomicU64,
  failures: AtomicU64,
  opened_at: Mutex<Option<Instant>>,
  probe_in_flight: AtomicU8,
  window_start: Mutex<Option<Instant>>,
}

impl State {
  fn reset_window(&self) {
    self.successes.store(0, Ordering::Relaxed);
    self.failures.store(0, Ordering::Relaxed);
    *self.window_start.lock() = Some(Instant::now());
  }

  /// Tumbling window, so the failure ratio reflects only recent traffic.
  fn maybe_roll_window(&self, window_duration: Duration) {
    let now = Instant::now();
    let mut start_guard = self.window_start.lock();
    let should_roll = match *start_guard {
      Some(start) => now.duration_since(start) >= window_duration,
      None => true,
    };
    if should_roll {
      *start_guard = Some(now);
      drop(start_guard);
      self.successes.store(0, Ordering::Relaxed);
      self.failures.store(0, Ordering::Relaxed);
    }
  }
}

type KeyFn = Arc<dyn Fn(&Request) -> String + Send + Sync + 'static>;
type Classifier = Arc<dyn Fn(&Response) -> bool + Send + Sync + 'static>;

pub struct CircuitBreaker {
  min_requests: u64,
  failure_ratio: f32,
  cool_down: Duration,
  open_status: StatusCode,
  retry_after_secs: u32,
  window: Duration,
  key_fn: KeyFn,
  classifier: Classifier,
  states: Arc<SccHashMap<String, Arc<State>>>,
}

impl Default for CircuitBreaker {
  fn default() -> Self {
    Self::new()
  }
}

impl CircuitBreaker {
  /// Trips at a 50% failure ratio once 20 requests land in a 60s window; stays open for 30s.
  pub fn new() -> Self {
    Self {
      min_requests: 20,
      failure_ratio: 0.5,
      cool_down: Duration::from_secs(30),
      open_status: StatusCode::SERVICE_UNAVAILABLE,
      retry_after_secs: 30,
      window: Duration::from_secs(60),
      // Route templates, not raw paths, so clients cannot grow the map with distinct paths.
      key_fn: Arc::new(|req: &Request| {
        req
          .extensions()
          .get::<MatchedPath>()
          .map_or_else(|| "<unmatched>".to_string(), |mp| mp.as_str().to_string())
      }),
      classifier: Arc::new(|resp: &Response| resp.status().is_server_error()),
      states: Arc::new(SccHashMap::new()),
    }
  }

  /// Clamped to at least one second.
  pub fn window(mut self, d: Duration) -> Self {
    self.window = d.max(Duration::from_secs(1));
    self
  }

  /// Requests needed in the window before the breaker can open; at least 1.
  pub fn min_requests(mut self, n: u64) -> Self {
    self.min_requests = n.max(1);
    self
  }

  /// Clamped to 0.0–1.0; a failure ratio at or above it trips the breaker.
  pub fn failure_ratio(mut self, ratio: f32) -> Self {
    self.failure_ratio = ratio.clamp(0.0, 1.0);
    self
  }

  /// How long the breaker stays open before a half-open probe.
  pub fn cool_down(mut self, d: Duration) -> Self {
    self.cool_down = d;
    self
  }

  /// Defaults to 503.
  pub fn open_status(mut self, status: StatusCode) -> Self {
    self.open_status = status;
    self
  }

  /// `Retry-After` sent with rejections; defaults to 30.
  pub fn retry_after_secs(mut self, secs: u32) -> Self {
    self.retry_after_secs = secs;
    self
  }

  /// One breaker per distinct key, never evicted, so keep the key space bounded.
  pub fn key_fn<F>(mut self, f: F) -> Self
  where
    F: Fn(&Request) -> String + Send + Sync + 'static,
  {
    self.key_fn = Arc::new(f);
    self
  }

  /// Decides which responses count as failures; defaults to 5xx.
  pub fn classifier<F>(mut self, f: F) -> Self
  where
    F: Fn(&Response) -> bool + Send + Sync + 'static,
  {
    self.classifier = Arc::new(f);
    self
  }
}

fn build_open_response(status: StatusCode, retry_after: u32) -> Response {
  let mut resp = http::Response::builder()
    .status(status)
    .body(TakoBody::from("circuit breaker open"))
    .expect("valid breaker response");
  if let Ok(v) = HeaderValue::from_str(&retry_after.to_string()) {
    resp.headers_mut().insert(RETRY_AFTER, v);
  }
  resp
}

impl IntoMiddleware for CircuitBreaker {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let min_requests = self.min_requests;
    let failure_ratio = self.failure_ratio;
    let cool_down = self.cool_down;
    let open_status = self.open_status;
    let retry_after_secs = self.retry_after_secs;
    let window = self.window;
    let key_fn = self.key_fn;
    let classifier = self.classifier;
    let states = self.states;

    move |req: Request, next: Next| {
      let key_fn = key_fn.clone();
      let classifier = classifier.clone();
      let states = states.clone();
      Box::pin(async move {
        let key = key_fn(&req);
        let state = states
          .entry_async(key.clone())
          .await
          .or_insert_with(|| Arc::new(State::default()))
          .clone();

        let cur = state.state.load(Ordering::Acquire);
        if cur == STATE_OPEN {
          let opened = *state.opened_at.lock();
          if let Some(at) = opened {
            if at.elapsed() < cool_down {
              return build_open_response(open_status, retry_after_secs);
            }
            // Cool-down elapsed: transition to half-open if we win the race.
            if state
              .state
              .compare_exchange(
                STATE_OPEN,
                STATE_HALF_OPEN,
                Ordering::AcqRel,
                Ordering::Acquire,
              )
              .is_ok()
            {
              state.reset_window();
            }
          }
        }

        // Half-open admits one probe at a time.
        let cur = state.state.load(Ordering::Acquire);
        let _probe_guard = if cur == STATE_HALF_OPEN {
          if state
            .probe_in_flight
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
          {
            return build_open_response(open_status, retry_after_secs);
          }
          Some(ProbeSlotGuard(&state.probe_in_flight))
        } else {
          None
        };

        let resp = next.run(req).await;
        let failed = (classifier)(&resp);

        state.maybe_roll_window(window);

        if failed {
          let f = state.failures.fetch_add(1, Ordering::Relaxed) + 1;
          let s = state.successes.load(Ordering::Relaxed);
          let total = f + s;
          let ratio = f as f32 / total.max(1) as f32;
          let should_open = match cur {
            STATE_HALF_OPEN => true,
            _ => total >= min_requests && ratio >= failure_ratio,
          };
          if should_open {
            state.state.store(STATE_OPEN, Ordering::Release);
            *state.opened_at.lock() = Some(Instant::now());
          }
        } else {
          state.successes.fetch_add(1, Ordering::Relaxed);
          if cur == STATE_HALF_OPEN {
            state.state.store(STATE_CLOSED, Ordering::Release);
            state.reset_window();
          }
        }

        resp
      })
    }
  }
}
