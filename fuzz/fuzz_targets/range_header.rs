#![no_main]

use libfuzzer_sys::fuzz_target;
use tako_rs_core::extractors::range::Range;

fuzz_target!(|data: &[u8]| {
  let Ok(value) = http::HeaderValue::from_bytes(data) else {
    return;
  };
  let mut headers = http::HeaderMap::new();
  headers.insert(http::header::RANGE, value);
  if let Ok(Some(range)) = Range::from_headers(&headers) {
    for spec in range.specs {
      for total in [0, 1, 1_000, u64::MAX] {
        if let Some((start, end)) = spec.resolve(total) {
          assert!(start <= end && end < total);
        }
      }
    }
  }
});
