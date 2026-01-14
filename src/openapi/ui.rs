//! OpenAPI UI helpers for serving interactive API documentation.
//!
//! This module provides responders for serving popular OpenAPI UI interfaces:
//! - **Swagger UI**: The classic OpenAPI documentation interface
//! - **Scalar**: A modern, beautiful API documentation UI
//! - **RapiDoc**: A feature-rich API documentation viewer
//!
//! All UIs are served via CDN, requiring no additional dependencies.
//!
//! # Examples
//!
//! ```rust,ignore
//! use tako::openapi::ui::{SwaggerUi, Scalar, RapiDoc};
//! use tako::{router::Router, Method};
//!
//! let mut router = Router::new();
//!
//! // Serve Swagger UI at /docs
//! router.route(Method::GET, "/docs", |_| async {
//!     SwaggerUi::new("/openapi.json")
//! });
//!
//! // Serve Scalar at /scalar
//! router.route(Method::GET, "/scalar", |_| async {
//!     Scalar::new("/openapi.json")
//! });
//!
//! // Serve RapiDoc at /rapidoc
//! router.route(Method::GET, "/rapidoc", |_| async {
//!     RapiDoc::new("/openapi.json")
//! });
//! ```

use crate::body::TakoBody;
use crate::responder::Responder;
use crate::types::Response;

/// Swagger UI responder that serves the classic OpenAPI documentation interface.
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
  /// Creates a new Swagger UI pointing to the given OpenAPI spec URL.
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
                url: "{spec_url}",
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
      title = self.title,
      spec_url = self.spec_url
    );

    let mut res = Response::new(TakoBody::from(html));
    res.headers_mut().insert(
      http::header::CONTENT_TYPE,
      http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    res
  }
}

/// Scalar responder that serves a modern, beautiful API documentation UI.
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
  /// Creates a new Scalar UI pointing to the given OpenAPI spec URL.
  pub fn new(spec_url: impl Into<String>) -> Self {
    Self {
      spec_url: spec_url.into(),
      title: "API Documentation".to_string(),
      theme: ScalarTheme::default(),
    }
  }

  /// Sets the page title.
  pub fn title(mut self, title: impl Into<String>) -> Self {
    self.title = title.into();
    self
  }

  /// Sets the Scalar theme.
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
      title = self.title,
      spec_url = self.spec_url,
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

/// RapiDoc responder that serves a feature-rich API documentation viewer.
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

/// Theme options for RapiDoc UI.
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

/// Render style options for RapiDoc.
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
  /// Creates a new RapiDoc UI pointing to the given OpenAPI spec URL.
  pub fn new(spec_url: impl Into<String>) -> Self {
    Self {
      spec_url: spec_url.into(),
      title: "API Documentation".to_string(),
      theme: RapiDocTheme::default(),
      render_style: RapiDocRenderStyle::default(),
    }
  }

  /// Sets the page title.
  pub fn title(mut self, title: impl Into<String>) -> Self {
    self.title = title.into();
    self
  }

  /// Sets the RapiDoc theme.
  pub fn theme(mut self, theme: RapiDocTheme) -> Self {
    self.theme = theme;
    self
  }

  /// Sets the render style.
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
      title = self.title,
      spec_url = self.spec_url,
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

/// Redoc responder that serves the Redoc API documentation interface.
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
  /// Creates a new Redoc UI pointing to the given OpenAPI spec URL.
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
      title = self.title,
      spec_url = self.spec_url
    );

    let mut res = Response::new(TakoBody::from(html));
    res.headers_mut().insert(
      http::header::CONTENT_TYPE,
      http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    res
  }
}
