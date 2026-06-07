//! HTTP-date formatting and parsing for `Last-Modified` / `Date` headers.

/// IMF-fixdate (RFC 7231) formatter, sufficient for `Last-Modified` and `Date`.
pub(crate) fn format_http_date(unix_secs: u64) -> String {
  let days = unix_secs / 86400;
  let secs_of_day = unix_secs % 86400;
  let h = secs_of_day / 3600;
  let m = (secs_of_day % 3600) / 60;
  let s = secs_of_day % 60;

  let dow_idx = (days + 4) % 7;
  let dow_name = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][dow_idx as usize];

  let (year, month, day) = epoch_days_to_ymd(days as i64);
  let mon_name = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
  ][(month - 1) as usize];

  format!("{dow_name}, {day:02} {mon_name} {year:04} {h:02}:{m:02}:{s:02} GMT")
}

/// Parse an HTTP-date header value into Unix epoch seconds.
///
/// Delegates to the `httpdate` crate which accepts every format RFC 9110
/// §5.6.7 lists: IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`), RFC 850
/// (`Sunday, 06-Nov-94 08:49:37 GMT`), and asctime (`Sun Nov 6 08:49:37 1994`).
/// The previous hand-rolled IMF-fixdate-only parser rejected legitimate
/// clients (Java/.NET defaults still emit RFC 850 in places) and forced the
/// server to ship full bodies on `If-Modified-Since` despite a fresh cache.
pub(crate) fn parse_http_date(header: &str) -> Option<u64> {
  let st = httpdate::parse_http_date(header.trim()).ok()?;
  st.duration_since(std::time::UNIX_EPOCH)
    .ok()
    .map(|d| d.as_secs())
}

fn epoch_days_to_ymd(days: i64) -> (i64, i64, i64) {
  // Civil from days since 1970-01-01 — Howard Hinnant algorithm.
  let z = days + 719_468;
  let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
  let doe = z - era * 146_097;
  let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
  let y = yoe + era * 400;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let d = doy - (153 * mp + 2) / 5 + 1;
  let m = if mp < 10 { mp + 3 } else { mp - 9 };
  let y = if m <= 2 { y + 1 } else { y };
  (y, m, d)
}
