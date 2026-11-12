use super::encoding::Encoding;

/// RFC 9110 q-values: `q=0` rejects a coding and unlisted codings follow `*`. The server
/// prefers br, gzip, deflate, then zstd.
pub(crate) fn choose_encoding(header: &str, enabled: &[Encoding]) -> Option<Encoding> {
  const SERVER_ORDER: &[Encoding] = &[
    Encoding::Brotli,
    Encoding::Gzip,
    Encoding::Deflate,
    #[cfg(feature = "zstd")]
    Encoding::Zstd,
  ];
  let mut accepted = [None; SERVER_ORDER.len()];
  let mut wildcard = None;
  for (coding, allowed) in header.split(',').filter_map(parse_coding) {
    if coding == "*" {
      wildcard.get_or_insert(allowed);
    } else if let Some(index) = SERVER_ORDER
      .iter()
      .position(|encoding| coding.eq_ignore_ascii_case(encoding.as_str()))
    {
      accepted[index].get_or_insert(allowed);
    }
  }

  SERVER_ORDER.iter().enumerate().find_map(|(index, &enc)| {
    (enabled.contains(&enc) && accepted[index].or(wildcard).unwrap_or(false)).then_some(enc)
  })
}

/// Borrows each coding. A missing q is 1.0; per RFC 9110 an entry with an
/// unparsable q is dropped rather than defaulted to full strength.
fn parse_coding(piece: &str) -> Option<(&str, bool)> {
  let mut parts = piece.trim().split(';');
  let coding = parts.next()?.trim();
  if coding.is_empty() {
    return None;
  }
  let mut q = 1.0_f32;
  for param in parts {
    let param = param.trim();
    let qv = param
      .strip_prefix("q=")
      .or_else(|| param.strip_prefix("Q="));
    if let Some(qv) = qv {
      q = qv.parse().ok()?;
    }
  }
  Some((coding, q > 0.0))
}
