//! Swagger UI responder for serving the classic `OpenAPI` documentation interface.

use crate::body::TakoBody;
use crate::openapi::ui::escape::html_escape;
use crate::openapi::ui::escape::js_string;
use crate::responder::Responder;
use crate::types::Response;

/// Swagger UI responder that serves the classic `OpenAPI` documentation interface.
///
/// # Examples
///
/// ```rust,ignore
/// use tako::openapi::ui::SwaggerUi;
///
/// async fn swagger_handler(_: tako::types::Request) -> SwaggerUi {
///     SwaggerUi::new("/openapi.json")
///         .title("My API Documentation")
/// }
/// ```
pub struct SwaggerUi {
  spec_url: String,
  title: String,
}

impl SwaggerUi {
  /// Creates a new Swagger UI pointing to the given `OpenAPI` spec URL.
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

impl Responder for SwaggerUi {
  fn into_response(self) -> Response {
    // Escape user-controlled strings per context: `title` is HTML text;
    // `spec_url` is interpolated inside a `<script>` JS string where HTML
    // entities are NOT decoded — use a JSON-shaped JS string literal.
    let html = format!(
      r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css">
</head>
<body>
    <div id="swagger-ui"></div>
    <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
    <script>
        window.onload = () => {{
            SwaggerUIBundle({{
                url: {spec_url},
                dom_id: '#swagger-ui',
                presets: [
                    SwaggerUIBundle.presets.apis,
                    SwaggerUIBundle.SwaggerUIStandalonePreset
                ],
                layout: "StandaloneLayout"
            }});
        }};
    </script>
</body>
</html>"#,
      title = html_escape(&self.title),
      spec_url = js_string(&self.spec_url)
    );

    let mut res = Response::new(TakoBody::from(html));
    res.headers_mut().insert(
      http::header::CONTENT_TYPE,
      http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    res
  }
}
