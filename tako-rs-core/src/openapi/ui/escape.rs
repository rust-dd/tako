/// Escapes `&`, `<`, `>`, `"` and `'`, so `s` is safe in HTML text and quoted attributes.
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

/// Quotes `s` for a `<script>` body, escaping `<` so HTML cannot close the script.
pub(crate) fn js_string(s: &str) -> String {
  serde_json::to_string(s)
    .unwrap_or_else(|_| "\"\"".to_string())
    .replace('<', "\\u003c")
}
