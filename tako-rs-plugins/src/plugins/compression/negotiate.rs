//! `Accept-Encoding` parsing and server-side encoding negotiation.

use super::encoding::Encoding;

/// Selects the best compression encoding based on client preferences and server capabilities.
///
/// Honors RFC 9110 quality values: a token with `q=0` is rejected, an unlisted
/// token defers to the wildcard `*` if present, otherwise it is unacceptable.
/// Server preference order is `br > gzip > deflate > zstd`.
pub(crate) fn choose_encoding(header: &str, enabled: &[Encoding]) -> Option<Encoding> {
  let parsed = parse_accept_encoding(header);
  // Pull `*` once — it determines acceptance of any encoding not listed explicitly.
  let wildcard_q = parsed.iter().find(|(c, _)| c == "*").map(|(_, q)| *q);

  let acceptable = |enc: Encoding| -> bool {
    let name = enc.as_str();
    match parsed.iter().find(|(c, _)| c == name) {
      Some((_, q)) => *q > 0.0,
      None => wildcard_q.is_some_and(|q| q > 0.0),
    }
  };

  // Server preference order — Brotli first for ratio, Gzip second for compatibility.
  let server_order: [Encoding; 3] = [Encoding::Brotli, Encoding::Gzip, Encoding::Deflate];
  if let Some(enc) = server_order
    .into_iter()
    .find(|&enc| enabled.contains(&enc) && acceptable(enc))
  {
    return Some(enc);
  }

  #[cfg(feature = "zstd")]
  {
    if enabled.contains(&Encoding::Zstd) && acceptable(Encoding::Zstd) {
      return Some(Encoding::Zstd);
    }
  }

  None
}

/// Parses an `Accept-Encoding` header into `(token, q)` pairs.
///
/// Tokens are lowercased. `q=` is honored when valid and absent → `1.0`.
///
/// PPL-15: per RFC 9110 / RFC 7231 §5.3, a malformed q-value (e.g.
/// `gzip;q=`, `gzip;q=banana`) means the entire entry MUST be ignored, not
/// silently defaulted to full strength. Previously a malformed q parsed to
/// `1.0`, so a client sending `gzip;q=` would erroneously get gzip as the
/// most preferred encoding even though they intended to disable it or
/// signal something else. Drop entries with a present-but-unparseable `q=`.
fn parse_accept_encoding(header: &str) -> Vec<(String, f32)> {
  header
    .split(',')
    .filter_map(|piece| {
      let piece = piece.trim();
      if piece.is_empty() {
        return None;
      }
      let mut parts = piece.split(';');
      let coding = parts.next()?.trim().to_ascii_lowercase();
      if coding.is_empty() {
        return None;
      }
      let mut q: f32 = 1.0;
      for param in parts {
        let param = param.trim();
        let qv = param
          .strip_prefix("q=")
          .or_else(|| param.strip_prefix("Q="));
        if let Some(qv) = qv {
          // Malformed q-value → drop the whole entry (RFC 9110).
          q = qv.parse().ok()?;
        }
      }
      Some((coding, q))
    })
    .collect()
}
