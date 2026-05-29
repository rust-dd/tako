//! Circuit-breaker middleware.
//!
//! Implements a classic three-state circuit (closed → open → half-open) keyed
//! by route template (or by a caller-defined key function). When the failure
//! ratio over the configured rolling window exceeds the trip threshold, the
//! breaker opens and short-circuits subsequent requests with `503 Service
//! Unavailable` until `cool_down` elapses, after which a single probe is
//! permitted (half-open). One success closes the breaker; one failure opens
//! it again.
//!
//! "Failure" defaults to a 5xx response, but callers can supply a custom
//! classifier (e.g. include 408 / 429 / network errors thrown as 502).
//!
//! The rolling window is approximated with a single moving counter pair
//! (success / failure) reset on cool-down. This keeps the hot path lock-free
//! and is sufficient for breaker semantics — full sliding-window precision
//! would require a per-bucket histogram and is deliberately out of scope.

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
use tako_core::body::TakoBody;
use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::router_state::MatchedPath;
use tako_core::types::Request;
use tako_core::types::Response;

const STATE_CLOSED: u8 = 0;
const STATE_OPEN: u8 = 1;
const STATE_HALF_OPEN: u8 = 2;

/// RAII releaser for the half-open `probe_in_flight` slot.
///
/// The slot must be returned to `0` on **every** exit path, including
/// panic unwind and future-cancellation (e.g. when this breaker is wrapped
/// by `Timeout` and the deadline drops the inner future before it
/// resolves). The previous implementation released the slot with a plain
/// `store(0)` after `next.run(req).await`, so a cancel mid-await left the
/// slot stuck at `1` forever — the breaker permanently saw an in-flight
/// probe and rejected every subsequent request on that key with 503.
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
  /// Instant (as Duration since `Self::start`) when the breaker opened.
  opened_at: Mutex<Option<Instant>>,
  /// Whether a half-open probe is currently in flight (one at a time).
  probe_in_flight: AtomicU8,
  /// Start of the current accounting window. Counts reset to zero each
  /// time the window rolls over so a long-running closed breaker doesn't
  /// dilute its failure ratio with months-old data.
  window_start: Mutex<Option<Instant>>,
}

impl State {
  fn reset_window(&self) {
    self.successes.store(0, Ordering::Relaxed);
    self.failures.store(0, Ordering::Relaxed);
    *self.window_start.lock() = Some(Instant::now());
  }

  /// Tumbling-window roll-over. Called before each accounting update in the
  /// closed/half-open paths so the failure ratio observes only the most
  /// recent `window_duration` of traffic.
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

/// Circuit-breaker middleware.
pub struct CircuitBreaker {
  /// Minimum number of requests in the window before the breaker can trip.
  min_requests: u64,
  /// Failure ratio (0.0–1.0) at or above which the breaker opens.
  failure_ratio: f32,
  /// How long to stay open before allowing a half-open probe.
  cool_down: Duration,
  /// Status returned while the breaker is open.
  open_status: StatusCode,
  /// `Retry-After` header value (seconds) appended on rejection.
  retry_after_secs: u32,
  /// Length of the tumbling failure-ratio window. Counters reset on each
  /// roll-over so a closed breaker doesn't dilute its ratio with old data.
  window: Duration,
  /// Keys requests for separate breakers.
  key_fn: KeyFn,
  /// Decides whether a response counts as a failure.
  classifier: Classifier,
  /// Per-key state.
  states: Arc<SccHashMap<String, Arc<State>>>,
}

impl Default for CircuitBreaker {
  fn default() -> Self {
    Self::new()
  }
}

impl CircuitBreaker {
  /// Creates a breaker with sensible defaults: trip at 50% failures over the
  /// last 20 requests (within a 60s window), cool down for 30s.
  pub fn new() -> Self {
    Self {
      min_requests: 20,
      failure_ratio: 0.5,
      cool_down: Duration::from_secs(30),
      open_status: StatusCode::SERVICE_UNAVAILABLE,
      retry_after_secs: 30,
      window: Duration::from_secs(60),
      // Default keying is by route **template** (e.g. `/users/{id}`), not
      // raw path (`/users/42`). Templated keys bound the breaker map's
      // cardinality at the number of registered routes — keying by raw
      // path lets an unauthenticated client insert one `Arc<State>` per
      // distinct `/a1`, `/a2`, ... and exhaust memory.
      // Falls back to a single constant for requests dispatched outside
      // the router (extremely unusual — only when this middleware is
      // wired up without a matching route).
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

  /// Sets the tumbling window length over which failures are counted.
  pub fn window(mut self, d: Duration) -> Self {
    self.window = d.max(Duration::from_secs(1));
    self
  }

  /// Sets the minimum sample size before the breaker can open.
  pub fn min_requests(mut self, n: u64) -> Self {
    self.min_requests = n.max(1);
    self
  }

  /// Sets the failure ratio (0.0–1.0) that trips the breaker.
  pub fn failure_ratio(mut self, ratio: f32) -> Self {
    self.failure_ratio = ratio.clamp(0.0, 1.0);
    self
  }

  /// Sets the cool-down duration the breaker stays open.
  pub fn cool_down(mut self, d: Duration) -> Self {
    self.cool_down = d;
    self
  }

  /// Sets the response status returned when the breaker is open.
  pub fn open_status(mut self, status: StatusCode) -> Self {
    self.open_status = status;
    self
  }

  /// Sets the `Retry-After` header value emitted on rejection.
  pub fn retry_after_secs(mut self, secs: u32) -> Self {
    self.retry_after_secs = secs;
    self
  }

  /// Plug a custom key function (e.g. per-tenant or per-IP breakers).
  pub fn key_fn<F>(mut self, f: F) -> Self
  where
    F: Fn(&Request) -> String + Send + Sync + 'static,
  {
    self.key_fn = Arc::new(f);
    self
  }

  /// Plug a custom failure classifier.
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

        // Half-open: allow exactly one probe at a time. The slot is held
        // by `ProbeSlotGuard`, whose `Drop` impl releases it on every exit
        // — normal return, panic, or future cancellation by a wrapping
        // Timeout. The previous `store(0)` after the await leaked the
        // slot on cancel and stranded the breaker in half-open forever.
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

        // Tumbling window: zero the counters when the current window has
        // elapsed so a long-running closed breaker compares the failure
        // ratio against recent traffic, not against accumulated history.
        // The roll-over only fires in the closed/half-open accounting
        // branches — open-state never updates these counters.
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
