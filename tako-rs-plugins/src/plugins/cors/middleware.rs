use http::HeaderValue;
use http::Method;
use http::StatusCode;
use http::header::ACCESS_CONTROL_ALLOW_CREDENTIALS;
use http::header::ACCESS_CONTROL_ALLOW_HEADERS;
use http::header::ACCESS_CONTROL_ALLOW_METHODS;
use http::header::ACCESS_CONTROL_ALLOW_ORIGIN;
use http::header::ACCESS_CONTROL_MAX_AGE;
use http::header::ACCESS_CONTROL_REQUEST_HEADERS;
use http::header::ACCESS_CONTROL_REQUEST_METHOD;
use http::header::ORIGIN;
use http::header::VARY;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::Next;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

use super::config::Config;

pub(crate) async fn handle_cors(req: Request, next: Next, cfg: &Config) -> Response {
  let origin = req.headers().get(ORIGIN).cloned();
  let request_headers = req.headers().get(ACCESS_CONTROL_REQUEST_HEADERS).cloned();
  let pna_request = req
    .headers()
    .get("access-control-request-private-network")
    .and_then(|v| v.to_str().ok())
    .is_some_and(|v| v.eq_ignore_ascii_case("true"));

  // Only a real preflight (`Origin` plus `Access-Control-Request-Method`) is answered here;
  // other OPTIONS requests reach their handlers.
  let is_preflight = req.method() == Method::OPTIONS
    && origin.is_some()
    && req.headers().contains_key(ACCESS_CONTROL_REQUEST_METHOD);
  if is_preflight {
    let mut resp = http::Response::builder()
      .status(StatusCode::NO_CONTENT)
      .body(TakoBody::empty())
      .expect("valid CORS preflight response");
    add_cors_headers(
      cfg,
      origin,
      request_headers.as_ref(),
      pna_request,
      &mut resp,
    );
    return resp;
  }

  let mut resp = next.run(req).await;
  add_cors_headers(cfg, origin, request_headers.as_ref(), false, &mut resp);
  resp
}

fn add_cors_headers(
  cfg: &Config,
  origin: Option<HeaderValue>,
  request_headers: Option<&HeaderValue>,
  pna_request: bool,
  resp: &mut Response,
) {
  // `Config::validate` keeps `*` away from credentials. A non-ASCII Origin, or a reflected
  // value that is not a valid header, adds no CORS headers instead of panicking.
  let allow_anything = cfg.origins.is_empty() && cfg.origin_matchers.is_empty();
  let (allow_origin, mirrored_origin) = if allow_anything {
    ("*".to_string(), false)
  } else if let Some(o) = &origin {
    let Ok(s) = o.to_str() else {
      return;
    };
    if cfg.origin_allowed(s) {
      (s.to_string(), true)
    } else {
      return;
    }
  } else {
    return;
  };

  let Ok(value) = HeaderValue::from_str(&allow_origin) else {
    return;
  };
  resp
    .headers_mut()
    .insert(ACCESS_CONTROL_ALLOW_ORIGIN, value);

  // A mirrored origin varies the response, so shared caches must key on it.
  if mirrored_origin {
    resp
      .headers_mut()
      .append(VARY, HeaderValue::from_static("Origin"));
  }

  let methods = if cfg.methods.is_empty() {
    None
  } else {
    Some(
      cfg
        .methods
        .iter()
        .map(http::Method::as_str)
        .collect::<Vec<_>>()
        .join(","),
    )
  };
  if let Some(v) = methods
    && let Ok(hv) = HeaderValue::from_str(&v)
  {
    resp.headers_mut().insert(ACCESS_CONTROL_ALLOW_METHODS, hv);
  }

  // `*` is invalid alongside credentials, so credentialed responses without an explicit list
  // reflect `Access-Control-Request-Headers` instead.
  if cfg.headers.is_empty() {
    if cfg.allow_credentials {
      // Reflection lets a compromised origin probe any header on a credentialed endpoint.
      static WARNED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
      let () = WARNED.get_or_init(|| {
        tracing::warn!(
          "CORS reflects `Access-Control-Request-Headers` while `allow_credentials=true` and no explicit `headers(...)` list is configured — set an explicit allow-list to harden the preflight policy",
        );
      });
      if let Some(req_h) = request_headers {
        resp
          .headers_mut()
          .insert(ACCESS_CONTROL_ALLOW_HEADERS, req_h.clone());
        resp.headers_mut().append(
          VARY,
          HeaderValue::from_static("Access-Control-Request-Headers"),
        );
      }
    } else {
      resp
        .headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("*"));
    }
  } else {
    let h = cfg
      .headers
      .iter()
      .map(http::HeaderName::as_str)
      .collect::<Vec<_>>()
      .join(",");
    if let Ok(hv) = HeaderValue::from_str(&h) {
      resp.headers_mut().insert(ACCESS_CONTROL_ALLOW_HEADERS, hv);
    }
  }

  if cfg.allow_credentials {
    resp.headers_mut().insert(
      ACCESS_CONTROL_ALLOW_CREDENTIALS,
      HeaderValue::from_static("true"),
    );
  }

  if let Some(secs) = cfg.max_age_secs
    && let Ok(hv) = HeaderValue::from_str(&secs.to_string())
  {
    resp.headers_mut().insert(ACCESS_CONTROL_MAX_AGE, hv);
  }

  // Private Network Access answers belong only on preflights that asked for them.
  if cfg.allow_private_network && pna_request {
    resp.headers_mut().insert(
      "access-control-allow-private-network",
      HeaderValue::from_static("true"),
    );
  }
}
