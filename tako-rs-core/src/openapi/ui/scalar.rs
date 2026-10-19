use crate::body::TakoBody;
use crate::openapi::ui::escape::html_escape;
use crate::responder::Responder;
use crate::types::Response;

/// Serves the Scalar API reference for the spec at `spec_url`.
///
/// # Examples
///
/// ```rust,ignore
/// use tako::openapi::ui::Scalar;
///
/// async fn scalar_handler(_: tako::types::Request) -> Scalar {
///     Scalar::new("/openapi.json")
///         .title("My API")
///         .theme(ScalarTheme::Purple)
/// }
/// ```
pub struct Scalar {
  spec_url: String,
  title: String,
  theme: ScalarTheme,
}

/// Theme options for Scalar UI.
#[derive(Clone, Copy, Default)]
pub enum ScalarTheme {
  #[default]
  Default,
  Purple,
  Saturn,
  BluePlanet,
  Moon,
  DeepSpace,
}

impl ScalarTheme {
  fn as_str(&self) -> &'static str {
    match self {
      ScalarTheme::Default => "default",
      ScalarTheme::Purple => "purple",
      ScalarTheme::Saturn => "saturn",
      ScalarTheme::BluePlanet => "bluePlanet",
      ScalarTheme::Moon => "moon",
      ScalarTheme::DeepSpace => "deepSpace",
    }
  }
}

impl Scalar {
  pub fn new(spec_url: impl Into<String>) -> Self {
    Self {
      spec_url: spec_url.into(),
      title: "API Documentation".to_string(),
      theme: ScalarTheme::default(),
    }
  }

  /// Page title, `API Documentation` by default.
  pub fn title(mut self, title: impl Into<String>) -> Self {
    self.title = title.into();
    self
  }

  pub fn theme(mut self, theme: ScalarTheme) -> Self {
    self.theme = theme;
    self
  }
}

impl Responder for Scalar {
  fn into_response(self) -> Response {
    let html = format!(
      r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
</head>
<body>
    <script id="api-reference" data-url="{spec_url}"></script>
    <script>
        var configuration = {{
            theme: '{theme}'
        }};
        document.getElementById('api-reference').dataset.configuration = JSON.stringify(configuration);
    </script>
    <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
</body>
</html>"#,
      title = html_escape(&self.title),
      spec_url = html_escape(&self.spec_url),
      theme = self.theme.as_str()
    );

    let mut res = Response::new(TakoBody::from(html));
    res.headers_mut().insert(
      http::header::CONTENT_TYPE,
      http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    res
  }
}
