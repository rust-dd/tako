//! Context-aware escaping helpers shared by the `OpenAPI` UI responders.

/// HTML-escape user-controlled text for safe interpolation into HTML text
/// nodes *or* double-quoted attribute values. Protects against the obvious
/// XSS sinks if `title` / `spec_url` ever originate from request parameters
/// (e.g. `Router::get("/docs/:title", ...)`).
///
/// Escapes `&`, `<`, `>`, `"`, and `'` to the corresponding HTML entities —
/// the minimal set that closes both element-context and attribute-context
/// injection.
pub(crate) fn html_escape(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for c in s.chars() {
    match c {
      '&' => out.push_str("&amp;"),
      '<' => out.push_str("&lt;"),
      '>' => out.push_str("&gt;"),
      '"' => out.push_str("&quot;"),
      '\'' => out.push_str("&#39;"),
      _ => out.push(c),
    }
  }
  out
}

/// Escape `s` for use as a JavaScript string literal — including the
/// surrounding double quotes. Used for `url: {js}` / `theme: {js}` sinks in
/// `<script>` bodies, where HTML-entity encoding is NOT decoded.
///
/// Delegates to `serde_json::to_string`, which produces a valid JS string
/// literal (every JSON string is a valid JS string in browsers' modern
/// strict-superset parsers used here).
pub(crate) fn js_string(s: &str) -> String {
  serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}
