//! `grpc-timeout` deadline propagation: parsing the header's unit-suffixed
//! duration, stashing the resulting [`GrpcDeadline`] in request extensions,
//! extracting it in handlers, and the timer that ends a streaming reply.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;
use std::time::Instant;

use http::Extensions;
use http::HeaderMap;
use http::request::Parts;

use crate::extractors::FromRequest;
use crate::extractors::FromRequestParts;
use crate::types::Request;

/// gRPC deadline propagated from the `grpc-timeout` request header.
///
/// Extract it as `Option<GrpcDeadline>` in a handler, then pass it to
/// [`GrpcServerStream::with_deadline`](super::GrpcServerStream::with_deadline)
/// or bound unary work with [`remaining`](Self::remaining).
#[derive(Debug, Clone, Copy)]
pub struct GrpcDeadline(pub Instant);

impl GrpcDeadline {
  /// Time left until the deadline, zero once it has passed.
  pub fn remaining(&self) -> Duration {
    self.0.saturating_duration_since(Instant::now())
  }
}

/// Parse the `grpc-timeout` header value (e.g. `"100m"`, `"5S"`, `"1H"`).
///
/// Accepts one to eight ASCII digits followed by a supported unit.
pub fn parse_grpc_timeout(value: &str) -> Option<Duration> {
  let value = value.trim();
  if !(2..=9).contains(&value.len()) || !value.is_ascii() {
    return None;
  }
  let (num, unit) = value.split_at(value.len() - 1);
  if !num.bytes().all(|byte| byte.is_ascii_digit()) {
    return None;
  }
  let num = num.parse::<u64>().ok()?;
  let dur = match unit {
    "n" => Duration::from_nanos(num),
    "u" => Duration::from_micros(num),
    "m" => Duration::from_millis(num),
    "S" => Duration::from_secs(num),
    "M" => Duration::from_secs(num.checked_mul(60)?),
    "H" => Duration::from_secs(num.checked_mul(3600)?),
    _ => return None,
  };
  Some(dur)
}

/// Extract the deadline (if any) from a request's `grpc-timeout` header.
///
/// Inserts a [`GrpcDeadline`] into request extensions when present so handlers
/// and middleware can honor the cancellation contract.
///
/// Uses `Instant::checked_add` so an attacker-supplied near-`u64::MAX`-second
/// `grpc-timeout` (e.g. `"18446744073709551615S"`) cannot panic the server on
/// overflow — instead the header is treated as if absent, matching the
/// no-deadline default.
pub fn read_grpc_deadline(req: &mut Request) -> Option<GrpcDeadline> {
  let deadline = deadline_from(req.headers(), req.extensions())?;
  req.extensions_mut().insert(deadline);
  Some(deadline)
}

/// Reuses a deadline stored by [`read_grpc_deadline`] so every reader of one
/// request sees the same instant.
fn deadline_from(headers: &HeaderMap, extensions: &Extensions) -> Option<GrpcDeadline> {
  if let Some(deadline) = extensions.get::<GrpcDeadline>() {
    return Some(*deadline);
  }
  let raw = headers.get("grpc-timeout")?.to_str().ok()?;
  let dur = parse_grpc_timeout(raw)?;
  Some(GrpcDeadline(Instant::now().checked_add(dur)?))
}

impl<'a> FromRequestParts<'a> for Option<GrpcDeadline> {
  type Error = Infallible;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Ok(deadline_from(&parts.headers, &parts.extensions)))
  }
}

impl<'a> FromRequest<'a> for Option<GrpcDeadline> {
  type Error = Infallible;

  fn from_request(
    req: &'a mut Request,
  ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Ok(deadline_from(req.headers(), req.extensions())))
  }
}

#[cfg(not(feature = "compio"))]
type Sleep = Pin<Box<tokio::time::Sleep>>;

/// Compio timers are `!Send`, while response bodies must be `Send`. The timer
/// is created and polled by the connection task that drives the body, which
/// never leaves its runtime thread.
#[cfg(feature = "compio")]
type Sleep = send_wrapper::SendWrapper<Pin<Box<dyn Future<Output = ()>>>>;

/// Runtime timer for a [`GrpcDeadline`], armed on first poll so it registers
/// with the runtime that drives the response.
pub(crate) struct DeadlineTimer {
  deadline: Instant,
  sleep: Option<Sleep>,
}

impl DeadlineTimer {
  pub(crate) fn new(deadline: GrpcDeadline) -> Self {
    Self {
      deadline: deadline.0,
      sleep: None,
    }
  }

  pub(crate) fn poll_elapsed(&mut self, cx: &mut Context<'_>) -> Poll<()> {
    let deadline = self.deadline;
    #[cfg(not(feature = "compio"))]
    let sleep = self
      .sleep
      .get_or_insert_with(|| Box::pin(tokio::time::sleep_until(deadline.into())));
    #[cfg(feature = "compio")]
    let sleep = self.sleep.get_or_insert_with(|| {
      let sleep: Pin<Box<dyn Future<Output = ()>>> = Box::pin(compio::time::sleep_until(deadline));
      send_wrapper::SendWrapper::new(sleep)
    });
    sleep.as_mut().poll(cx)
  }
}

#[cfg(test)]
mod tests {
  use std::time::Duration;

  use super::parse_grpc_timeout;

  #[test]
  fn malformed_units_and_numeric_prefixes_are_rejected_without_panicking() {
    for value in ["99ƿ", "é", "1💥", "+1S", "123456789m", "S", ""] {
      assert_eq!(parse_grpc_timeout(value), None, "{value:?}");
    }
    assert_eq!(
      parse_grpc_timeout("99999999n"),
      Some(Duration::from_nanos(99_999_999))
    );
    assert_eq!(parse_grpc_timeout("1H"), Some(Duration::from_secs(3600)));
  }
}
