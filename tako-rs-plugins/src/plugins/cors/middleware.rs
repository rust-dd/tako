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
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

use super::config::Config;

/// Handles CORS processing for incoming requests including preflight and actual requests.
pub(crate) async fn handle_cors(req: Request, next: Next, cfg: Config) -> impl Responder {
  let origin = req.headers().get(ORIGIN).cloned();
  let request_headers = req.headers().get(ACCESS_CONTROL_REQUEST_HEADERS).cloned();
  let pna_request = req
    .headers()
    .get("access-control-request-private-network")
    .and_then(|v| v.to_str().ok())
    .is_some_and(|v| v.eq_ignore_ascii_case("true"));

  // PPL-13: only short-circuit OPTIONS when it is an *actual* CORS preflight
  // (per Fetch spec: `Origin` + `Access-Control-Request-Method` headers).
  // The previous unconditional 204 intercepted legitimate non-CORS OPTIONS
  // handlers — capability discovery, `OPTIONS *` server-wide queries — and
  // returned an empty 204 with no `Allow` header in place of the handler's
  // response.
  let is_preflight = req.method() == Method::OPTIONS
    && origin.is_some()
    && req.headers().contains_key(ACCESS_CONTROL_REQUEST_METHOD);
  if is_preflight {
    let mut resp = http::Response::builder()
      .status(StatusCode::NO_CONTENT)
      .body(TakoBody::empty())
      .expect("valid CORS preflight response");
    add_cors_headers(
      &cfg,
      origin,
      request_headers.as_ref(),
      pna_request,
      &mut resp,
    );
    return resp.into_response();
  }

  let mut resp = next.run(req).await;
  add_cors_headers(&cfg, origin, request_headers.as_ref(), false, &mut resp);
  resp.into_response()
}

/// Adds CORS headers to HTTP responses based on configuration and request origin.
fn add_cors_headers(
  cfg: &Config,
  origin: Option<HeaderValue>,
  request_headers: Option<&HeaderValue>,
  pna_request: bool,
  resp: &mut Response,
) {
  // Origin validation and Access-Control-Allow-Origin header.
  //
  // Invariant guarded by `Config::validate`: when `allow_credentials = true`,
  // at least one origin or matcher is configured — so `*` is never emitted
  // alongside credentials.
  //
  // PPL-14:
  //  (a) `o.to_str().unwrap_or_default()` previously silenced invalid-byte
  //      Origin headers to empty-string, which then `origin_allowed("")`
  //      false'd, which silently emitted no header — attacker-malformed
  //      Origin hid the rejection from logs/metrics. Detect and bail
  //      cleanly instead.
  //  (b) `HeaderValue::from_str(&allow_origin).expect(...)` panicked if a
  //      mirrored origin contained CRLF/NUL (Origin reflection injection
  //      surface). Map the error to a silent bail so a malformed origin
  //      cannot crash the request task.
  let allow_anything = cfg.origins.is_empty() && cfg.origin_matchers.is_empty();
  let (allow_origin, mirrored_origin) = if allow_anything {
    ("*".to_string(), false)
  } else if let Some(o) = &origin {
    let Ok(s) = o.to_str() else {
      // Non-ASCII / control-byte Origin — bail cleanly.
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

  // Use the fallible API and bail on construction failure. The reflected
  // origin string is largely caller-controlled; even after the allow-list
  // check it may contain unexpected bytes if a custom matcher passes them.
  let Ok(value) = HeaderValue::from_str(&allow_origin) else {
    return;
  };
  resp
    .headers_mut()
    .insert(ACCESS_CONTROL_ALLOW_ORIGIN, value);

  // When the response varies on the request Origin (i.e. we mirrored it back),
  // shared caches must key on Origin to avoid cross-origin response leakage.
  if mirrored_origin {
    resp
      .headers_mut()
      .append(VARY, HeaderValue::from_static("Origin"));
  }

  // Access-Control-Allow-Methods header
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

  // Access-Control-Allow-Headers header.
  //
  // `*` is invalid in any "Allow-*" header when `Access-Control-Allow-Credentials: true`
  // (Fetch spec). Two strategies when no explicit list is configured:
  //   - credentials disallowed: emit `*` (browsers accept it).
  //   - credentials allowed: reflect the request's `Access-Control-Request-Headers`
  //     so the preflight succeeds without a footgun.
  if cfg.headers.is_empty() {
    if cfg.allow_credentials {
      // Security best-practice: with `Access-Control-Allow-Credentials: true`
      // the allow-list should be an explicit, server-controlled set. The
      // pre-flight `Access-Control-Request-Headers` value is attacker-
      // influenced; reflecting it blindly lets a compromised origin probe
      // any header against the credentialed endpoint. Emit a one-time
      // warning and continue with the legacy reflection for BC — apps
      // should set explicit `headers(...)` to silence this.
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
      // No `Access-Control-Request-Headers` to reflect → emit nothing.
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

  // Access-Control-Allow-Credentials header
  if cfg.allow_credentials {
    resp.headers_mut().insert(
      ACCESS_CONTROL_ALLOW_CREDENTIALS,
      HeaderValue::from_static("true"),
    );
  }

  // Access-Control-Max-Age header
  if let Some(secs) = cfg.max_age_secs
    && let Ok(hv) = HeaderValue::from_str(&secs.to_string())
  {
    resp.headers_mut().insert(ACCESS_CONTROL_MAX_AGE, hv);
  }

  // Private Network Access (PNA) — emit only on preflight responses where
  // the client signaled the request bit. Doing so on regular responses is a
  // spec violation.
  if cfg.allow_private_network && pna_request {
    resp.headers_mut().insert(
      "access-control-allow-private-network",
      HeaderValue::from_static("true"),
    );
  }
}
