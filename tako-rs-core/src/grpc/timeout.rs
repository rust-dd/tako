//! `grpc-timeout` deadline propagation: parsing the header's unit-suffixed
//! duration and stashing the resulting [`GrpcDeadline`] in request extensions.

use std::time::Duration;
use std::time::Instant;

use crate::types::Request;

/// gRPC deadline propagated from the `grpc-timeout` request header.
#[derive(Debug, Clone, Copy)]
pub struct GrpcDeadline(pub Instant);

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
  let raw = req
    .headers()
    .get("grpc-timeout")
    .and_then(|v| v.to_str().ok())?;
  let dur = parse_grpc_timeout(raw)?;
  let deadline = GrpcDeadline(Instant::now().checked_add(dur)?);
  req.extensions_mut().insert(deadline);
  Some(deadline)
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
