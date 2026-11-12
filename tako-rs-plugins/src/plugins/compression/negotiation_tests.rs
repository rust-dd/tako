use super::Encoding;
use super::negotiate::choose_encoding;

#[test]
fn negotiation_preserves_preferences_exclusions_and_parameters() {
  let enabled = [Encoding::Deflate, Encoding::Gzip, Encoding::Brotli];
  for (header, expected) in [
    ("", None),
    ("identity", None),
    ("gzip", Some(Encoding::Gzip)),
    ("GZip", Some(Encoding::Gzip)),
    (" gzip ; Q=0.5 ", Some(Encoding::Gzip)),
    ("deflate, gzip, br", Some(Encoding::Brotli)),
    ("gzip;q=1, br;q=0.1", Some(Encoding::Brotli)),
    ("br;q=0, gzip;q=0.5", Some(Encoding::Gzip)),
    ("gzip;q=0, br;q=0, *;q=1", Some(Encoding::Deflate)),
    ("gzip, *;q=0", Some(Encoding::Gzip)),
    ("gzip;q=0, *;q=0", None),
    ("unknown;q=1, *;q=0.5", Some(Encoding::Brotli)),
    (", , ;q=1, gzip", Some(Encoding::Gzip)),
    ("gzip;extension=value;q=0.5", Some(Encoding::Gzip)),
    ("gzip;q=invalid", None),
    ("gzip;q=invalid;q=1", None),
    ("gzip;q=0;q=1", Some(Encoding::Gzip)),
    ("gzip;q=1;q=0", None),
    ("gzip;q=0, GZIP;q=1", None),
    ("gzip;q=invalid, GZIP;q=1", Some(Encoding::Gzip)),
    ("*;q=0, *;q=1", None),
    ("*;q=invalid, *;q=1", Some(Encoding::Brotli)),
  ] {
    assert_eq!(choose_encoding(header, &enabled), expected, "{header}");
  }
}

#[test]
fn negotiation_only_selects_enabled_codecs() {
  assert_eq!(choose_encoding("*", &[]), None);
  assert_eq!(choose_encoding("gzip, br", &[Encoding::Deflate]), None);
  assert_eq!(
    choose_encoding("gzip, *", &[Encoding::Deflate]),
    Some(Encoding::Deflate)
  );
}

#[cfg(feature = "zstd")]
#[test]
fn zstd_is_the_last_preference_and_honors_explicit_exclusion() {
  let enabled = [Encoding::Zstd, Encoding::Deflate];
  assert_eq!(
    choose_encoding("zstd, deflate", &enabled),
    Some(Encoding::Deflate)
  );
  assert_eq!(choose_encoding("ZSTD", &enabled), Some(Encoding::Zstd));
  assert_eq!(choose_encoding("zstd;q=0, *", &[Encoding::Zstd]), None);
}
