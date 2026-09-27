//! HTTP-date conversion shared by file responders.

use std::time::Duration;
use std::time::UNIX_EPOCH;

pub(crate) fn format_http_date(unix_secs: u64) -> String {
  httpdate::fmt_http_date(UNIX_EPOCH + Duration::from_secs(unix_secs))
}

pub(crate) fn parse_http_date(header: &str) -> Option<u64> {
  httpdate::parse_http_date(header.trim())
    .ok()?
    .duration_since(UNIX_EPOCH)
    .ok()
    .map(|d| d.as_secs())
}
