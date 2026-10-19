use super::encoding::Encoding;

/// RFC 9110 q-values: `q=0` rejects a coding and unlisted codings follow `*`. The server
/// prefers br, gzip, deflate, then zstd.
pub(crate) fn choose_encoding(header: &str, enabled: &[Encoding]) -> Option<Encoding> {
  let parsed = parse_accept_encoding(header);
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

/// Lowercased `(coding, q)` pairs. A missing q is 1.0; per RFC 9110 an entry with an
/// unparsable q is dropped rather than defaulted to full strength.
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
          q = qv.parse().ok()?;
        }
      }
      Some((coding, q))
    })
    .collect()
}
