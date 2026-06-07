//! `Redoc` responder for serving the `Redoc` API documentation interface.

use crate::body::TakoBody;
use crate::openapi::ui::escape::html_escape;
use crate::responder::Responder;
use crate::types::Response;

/// `Redoc` responder that serves the `Redoc` API documentation interface.
///
/// # Examples
///
/// ```rust,ignore
/// use tako::openapi::ui::Redoc;
///
/// async fn redoc_handler(_: tako::types::Request) -> Redoc {
///     Redoc::new("/openapi.json")
///         .title("My API")
/// }
/// ```
pub struct Redoc {
  spec_url: String,
  title: String,
}

impl Redoc {
  /// Creates a new `Redoc` UI pointing to the given `OpenAPI` spec URL.
  pub fn new(spec_url: impl Into<String>) -> Self {
    Self {
      spec_url: spec_url.into(),
      title: "API Documentation".to_string(),
    }
  }

  /// Sets the page title.
  pub fn title(mut self, title: impl Into<String>) -> Self {
    self.title = title.into();
    self
  }
}

impl Responder for Redoc {
  fn into_response(self) -> Response {
    // `title` (HTML text) and `spec_url` (HTML attribute) both need escaping.
    let html = format!(
      r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    <link href="https://fonts.googleapis.com/css?family=Montserrat:300,400,700|Roboto:300,400,700" rel="stylesheet">
    <style>
        body {{ margin: 0; padding: 0; }}
    </style>
</head>
<body>
    <redoc spec-url="{spec_url}"></redoc>
    <script src="https://cdn.redoc.ly/redoc/latest/bundles/redoc.standalone.js"></script>
</body>
</html>"#,
      title = html_escape(&self.title),
      spec_url = html_escape(&self.spec_url)
    );

    let mut res = Response::new(TakoBody::from(html));
    res.headers_mut().insert(
      http::header::CONTENT_TYPE,
      http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    res
  }
}
