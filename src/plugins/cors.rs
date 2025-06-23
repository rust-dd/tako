use anyhow::Result;
use http::{
    HeaderName, HeaderValue, Method, StatusCode,
    header::{
        ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS,
        ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_MAX_AGE, ORIGIN,
    },
};

use crate::{
    body::TakoBody,
    plugins::TakoPlugin,
    router::Router,
    types::{Request, Response},
};

#[derive(Clone)]
pub struct Config {
    pub origins: Vec<String>,
    pub methods: Vec<Method>,
    pub headers: Vec<HeaderName>,
    pub allow_credentials: bool,
    pub max_age_secs: Option<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            origins: Vec::new(),
            methods: vec![
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
                Method::OPTIONS,
            ],
            headers: Vec::new(),
            allow_credentials: false,
            max_age_secs: Some(3600),
        }
    }
}

pub struct CorsBuilder(Config);

impl CorsBuilder {
    pub fn new() -> Self {
        Self(Config::default())
    }

    pub fn allow_origin(mut self, o: impl Into<String>) -> Self {
        self.0.origins.push(o.into());
        self
    }

    pub fn allow_methods(mut self, m: &[Method]) -> Self {
        self.0.methods = m.to_vec();
        self
    }

    pub fn allow_headers(mut self, h: &[HeaderName]) -> Self {
        self.0.headers = h.to_vec();
        self
    }

    pub fn allow_credentials(mut self, allow: bool) -> Self {
        self.0.allow_credentials = allow;
        self
    }

    pub fn max_age_secs(mut self, secs: u32) -> Self {
        self.0.max_age_secs = Some(secs);
        self
    }

    pub fn build(self) -> CorsPlugin {
        CorsPlugin { cfg: self.0 }
    }
}

#[derive(Clone)]
pub struct CorsPlugin {
    cfg: Config,
}

impl Default for CorsPlugin {
    fn default() -> Self {
        Self {
            cfg: Config::default(),
        }
    }
}

impl TakoPlugin for CorsPlugin {
    fn name(&self) -> &'static str {
        "CorsPlugin"
    }

    fn setup(&self, router: &mut Router) -> Result<()> {
        let cfg = self.cfg.clone();

        Ok(())
    }

    // async fn start(&self) -> anyhow::Result<()> {
    //     Ok(())
    // }

    // async fn stop(&self) -> anyhow::Result<()> {
    //     Ok(())
    // }
}

async fn handle_cors(mut req: Request, next: Next<'_>, cfg: Config) -> Result<Response> {
    let origin = req.headers().get(ORIGIN).cloned();

    // Pre-flight?
    if req.method() == Method::OPTIONS {
        let mut resp = hyper::Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(TakoBody::empty())
            .unwrap();
        add_cors_headers(&cfg, origin, &mut resp);
        return Ok(resp);
    }

    // Normál kérés
    let mut resp = next.run(req).await?;
    add_cors_headers(&cfg, origin, &mut resp);
    Ok(resp)
}

fn add_cors_headers(cfg: &Config, origin: Option<HeaderValue>, resp: &mut Response) {
    // Origin-matching
    let allow_origin = if cfg.origins.is_empty() {
        "*".to_string()
    } else if let Some(o) = &origin {
        let s = o.to_str().unwrap_or_default();
        if cfg.origins.iter().any(|p| p == s) {
            s.to_string()
        } else {
            return;
        }
    } else {
        return;
    };
    resp.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_str(&allow_origin).unwrap(),
    );

    // Methods
    let methods = if cfg.methods.is_empty() {
        None
    } else {
        Some(
            cfg.methods
                .iter()
                .map(|m| m.as_str())
                .collect::<Vec<_>>()
                .join(","),
        )
    };
    if let Some(v) = methods {
        resp.headers_mut().insert(
            ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_str(&v).unwrap(),
        );
    }

    // Headers
    if !cfg.headers.is_empty() {
        let h = cfg
            .headers
            .iter()
            .map(|h| h.as_str())
            .collect::<Vec<_>>()
            .join(",");
        resp.headers_mut().insert(
            ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_str(&h).unwrap(),
        );
    }

    // Credentials
    if cfg.allow_credentials {
        resp.headers_mut().insert(
            ACCESS_CONTROL_ALLOW_CREDENTIALS,
            HeaderValue::from_static("true"),
        );
    }

    // Max-Age
    if let Some(secs) = cfg.max_age_secs {
        resp.headers_mut().insert(
            ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_str(&secs.to_string()).unwrap(),
        );
    }
}
