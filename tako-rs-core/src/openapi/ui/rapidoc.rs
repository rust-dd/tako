use crate::body::TakoBody;
use crate::openapi::ui::escape::html_escape;
use crate::responder::Responder;
use crate::types::Response;

/// Serves the `RapiDoc` viewer for the spec at `spec_url`.
///
/// # Examples
///
/// ```rust,ignore
/// use tako::openapi::ui::RapiDoc;
///
/// async fn rapidoc_handler(_: tako::types::Request) -> RapiDoc {
///     RapiDoc::new("/openapi.json")
///         .title("My API")
///         .theme(RapiDocTheme::Dark)
/// }
/// ```
pub struct RapiDoc {
  spec_url: String,
  title: String,
  theme: RapiDocTheme,
  render_style: RapiDocRenderStyle,
}

/// Theme options for `RapiDoc` UI.
#[derive(Clone, Copy, Default)]
pub enum RapiDocTheme {
  #[default]
  Light,
  Dark,
}

impl RapiDocTheme {
  fn as_str(&self) -> &'static str {
    match self {
      RapiDocTheme::Light => "light",
      RapiDocTheme::Dark => "dark",
    }
  }
}

/// Render style options for `RapiDoc`.
#[derive(Clone, Copy, Default)]
pub enum RapiDocRenderStyle {
  #[default]
  Read,
  View,
  Focused,
}

impl RapiDocRenderStyle {
  fn as_str(&self) -> &'static str {
    match self {
      RapiDocRenderStyle::Read => "read",
      RapiDocRenderStyle::View => "view",
      RapiDocRenderStyle::Focused => "focused",
    }
  }
}

impl RapiDoc {
  pub fn new(spec_url: impl Into<String>) -> Self {
    Self {
      spec_url: spec_url.into(),
      title: "API Documentation".to_string(),
      theme: RapiDocTheme::default(),
      render_style: RapiDocRenderStyle::default(),
    }
  }

  /// Page title, `API Documentation` by default.
  pub fn title(mut self, title: impl Into<String>) -> Self {
    self.title = title.into();
    self
  }

  pub fn theme(mut self, theme: RapiDocTheme) -> Self {
    self.theme = theme;
    self
  }

  pub fn render_style(mut self, style: RapiDocRenderStyle) -> Self {
    self.render_style = style;
    self
  }
}

impl Responder for RapiDoc {
  fn into_response(self) -> Response {
    let html = format!(
      r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    <script type="module" src="https://unpkg.com/rapidoc/dist/rapidoc-min.js"></script>
</head>
<body>
    <rapi-doc
        spec-url="{spec_url}"
        theme="{theme}"
        render-style="{render_style}"
        show-header="false"
        allow-try="true"
    ></rapi-doc>
</body>
</html>"#,
      title = html_escape(&self.title),
      spec_url = html_escape(&self.spec_url),
      theme = self.theme.as_str(),
      render_style = self.render_style.as_str()
    );

    let mut res = Response::new(TakoBody::from(html));
    res.headers_mut().insert(
      http::header::CONTENT_TYPE,
      http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    res
  }
}
