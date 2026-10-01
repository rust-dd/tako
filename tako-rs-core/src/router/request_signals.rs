use std::sync::Arc;
use std::time::Instant;

use http::StatusCode;

use crate::route::Route;
use crate::signals::Signal;
use crate::signals::SignalArbiter;
use crate::signals::app_signals;
use crate::signals::ids;
use crate::types::Request;

pub(super) struct RequestSignals {
  router: SignalArbiter,
  route: Option<(Arc<str>, SignalArbiter)>,
  method: String,
  path: String,
  started: Instant,
}

/// Whether any app, router, or route listener wants request signals.
pub(super) fn listening(router: &SignalArbiter, route: Option<&Route>) -> bool {
  app_signals().has_listeners()
    || router.has_listeners()
    || route.is_some_and(|route| route.signals.has_listeners())
}

impl RequestSignals {
  pub fn new(router: &SignalArbiter, route: Option<&Route>, req: &Request) -> Option<Self> {
    if !listening(router, route) {
      return None;
    }
    let trace = Self {
      router: router.clone(),
      route: route.map(|route| (route.path.clone(), route.signal_arbiter())),
      method: req.method().to_string(),
      path: req.uri().path().to_string(),
      started: Instant::now(),
    };
    Some(trace)
  }

  pub async fn started(&self) {
    self.emit(ids::REQUEST_STARTED, None, false).await;
    if self.route.is_some() {
      self.emit(ids::ROUTE_REQUEST_STARTED, None, true).await;
    }
  }

  pub async fn complete(self, status: StatusCode) {
    if self.route.is_some() {
      self
        .emit(ids::ROUTE_REQUEST_COMPLETED, Some(status), true)
        .await;
    }
    self.emit(ids::REQUEST_COMPLETED, Some(status), false).await;
  }

  async fn emit(&self, id: &'static str, status: Option<StatusCode>, route_event: bool) {
    let mut signal = Signal::with_capacity(id, 5)
      .meta("method", self.method.clone())
      .meta("path", self.path.clone())
      .meta(
        "route",
        self
          .route
          .as_ref()
          .map_or("unmatched", |(path, _)| path.as_ref()),
      );
    if let Some(status) = status {
      signal = signal.meta("status", status.as_u16().to_string()).meta(
        "duration_us",
        self.started.elapsed().as_micros().to_string(),
      );
    }
    if route_event && let Some((_, arbiter)) = &self.route {
      arbiter.emit(signal.clone()).await;
    }
    self.router.emit(signal.clone()).await;
    app_signals().emit(signal).await;
  }
}
