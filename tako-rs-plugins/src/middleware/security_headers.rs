//! `nosniff`, `X-Frame-Options: DENY` and a strict referrer policy by default; HSTS, CSP and
//! cross-origin policies are opt-in. `X-XSS-Protection` is omitted: browsers ignore it.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::HeaderValue;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

/// Per-request nonce inserted into request extensions by [`SecurityHeaders::csp_with_nonce`]
/// and [`SecurityHeaders::csp_report_only`].
///
/// ```rust,ignore
/// let nonce = req.extensions().get::<CspNonce>().expect("CSP middleware mounted").0.clone();
/// let html = format!(r#"<script nonce="{nonce}">…</script>"#);
/// ```
#[derive(Debug, Clone)]
pub struct CspNonce(pub String);

#[derive(Clone)]
enum CspMode {
  Static(HeaderValue),
  WithNonce { template: String, report_only: bool },
}

/// Methods taking `&'static str` panic when the value is not a valid header value.
pub struct SecurityHeaders {
  frame_options: HeaderValue,
  hsts: bool,
  hsts_max_age: u64,
  hsts_include_subdomains: bool,
  hsts_preload: bool,
  referrer_policy: HeaderValue,
  csp: Option<CspMode>,
  coop: Option<HeaderValue>,
  coep: Option<HeaderValue>,
  corp: Option<HeaderValue>,
  permissions_policy: Option<HeaderValue>,
}

impl Default for SecurityHeaders {
  fn default() -> Self {
    Self::new()
  }
}

impl SecurityHeaders {
  pub fn new() -> Self {
    Self {
      frame_options: HeaderValue::from_static("DENY"),
      hsts: false,
      hsts_max_age: 31_536_000,
      hsts_include_subdomains: true,
      hsts_preload: false,
      referrer_policy: HeaderValue::from_static("strict-origin-when-cross-origin"),
      csp: None,
      coop: None,
      coep: None,
      corp: None,
      permissions_policy: None,
    }
  }

  /// Defaults to `DENY`.
  pub fn frame_options(mut self, value: &'static str) -> Self {
    self.frame_options = HeaderValue::from_static(value);
    self
  }

  /// Off by default; once on, `includeSubDomains` is sent unless disabled.
  pub fn hsts(mut self, enable: bool) -> Self {
    self.hsts = enable;
    self
  }

  /// Seconds; defaults to one year.
  pub fn hsts_max_age(mut self, seconds: u64) -> Self {
    self.hsts_max_age = seconds;
    self
  }

  /// Defaults to true.
  pub fn hsts_include_subdomains(mut self, on: bool) -> Self {
    self.hsts_include_subdomains = on;
    self
  }

  /// Defaults to false. The preload list requires `max-age >= 31536000` and
  /// `includeSubDomains`.
  pub fn hsts_preload(mut self, on: bool) -> Self {
    self.hsts_preload = on;
    self
  }

  /// Defaults to `strict-origin-when-cross-origin`.
  pub fn referrer_policy(mut self, value: &'static str) -> Self {
    self.referrer_policy = HeaderValue::from_static(value);
    self
  }

  /// Emits a static `Content-Security-Policy` header.
  pub fn csp(mut self, value: &'static str) -> Self {
    self.csp = Some(CspMode::Static(HeaderValue::from_static(value)));
    self
  }

  /// Replaces `{nonce}` in `template` with a fresh nonce per request, also exposed as
  /// [`CspNonce`]; a result that is not a valid header value is not sent.
  pub fn csp_with_nonce(mut self, template: impl Into<String>) -> Self {
    self.csp = Some(CspMode::WithNonce {
      template: template.into(),
      report_only: false,
    });
    self
  }

  /// Same as [`Self::csp_with_nonce`], but emit `Content-Security-Policy-Report-Only`.
  pub fn csp_report_only(mut self, template: impl Into<String>) -> Self {
    self.csp = Some(CspMode::WithNonce {
      template: template.into(),
      report_only: true,
    });
    self
  }

  pub fn coop(mut self, value: &'static str) -> Self {
    self.coop = Some(HeaderValue::from_static(value));
    self
  }

  pub fn coep(mut self, value: &'static str) -> Self {
    self.coep = Some(HeaderValue::from_static(value));
    self
  }

  pub fn corp(mut self, value: &'static str) -> Self {
    self.corp = Some(HeaderValue::from_static(value));
    self
  }

  pub fn permissions_policy(mut self, value: &'static str) -> Self {
    self.permissions_policy = Some(HeaderValue::from_static(value));
    self
  }
}

fn rand_nonce() -> String {
  // 18 bytes encode to 24 base64 chars; UUID v4 fixes 6 bits, leaving 138 random ones.
  use base64::Engine;

  let u1 = uuid::Uuid::new_v4().into_bytes();
  let u2 = uuid::Uuid::new_v4().into_bytes();
  let mut buf = [0u8; 18];
  buf[..16].copy_from_slice(&u1);
  buf[16..].copy_from_slice(&u2[..2]);
  base64::engine::general_purpose::STANDARD_NO_PAD.encode(buf)
}

impl IntoMiddleware for SecurityHeaders {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let frame_options = self.frame_options;
    let hsts_value = if self.hsts {
      let mut buf = format!("max-age={}", self.hsts_max_age);
      if self.hsts_include_subdomains {
        buf.push_str("; includeSubDomains");
      }
      if self.hsts_preload {
        buf.push_str("; preload");
      }
      Some(HeaderValue::from_str(&buf).expect("valid HSTS header"))
    } else {
      None
    };
    let referrer_policy = self.referrer_policy;
    let csp = Arc::new(self.csp);
    let coop = self.coop;
    let coep = self.coep;
    let corp = self.corp;
    let permissions_policy = self.permissions_policy;

    move |mut req: Request, next: Next| {
      let frame_options = frame_options.clone();
      let hsts_value = hsts_value.clone();
      let referrer_policy = referrer_policy.clone();
      let csp = csp.clone();
      let coop = coop.clone();
      let coep = coep.clone();
      let corp = corp.clone();
      let permissions_policy = permissions_policy.clone();

      Box::pin(async move {
        // Before `next`, so the handler can read the nonce from request extensions.
        let prepared_csp: Option<(HeaderValue, bool)> = match csp.as_ref() {
          None => None,
          Some(CspMode::Static(v)) => Some((v.clone(), false)),
          Some(CspMode::WithNonce {
            template,
            report_only,
          }) => {
            let nonce = rand_nonce();
            let value = template.replace("{nonce}", &nonce);
            req.extensions_mut().insert(CspNonce(nonce));
            HeaderValue::from_str(&value)
              .ok()
              .map(|hv| (hv, *report_only))
          }
        };

        let mut resp = next.run(req).await;
        let headers = resp.headers_mut();

        headers.insert(
          "x-content-type-options",
          HeaderValue::from_static("nosniff"),
        );
        headers.insert("x-frame-options", frame_options);
        headers.insert("referrer-policy", referrer_policy);

        if let Some(hsts) = hsts_value {
          headers.insert("strict-transport-security", hsts);
        }

        if let Some((v, report_only)) = prepared_csp {
          let key = if report_only {
            "content-security-policy-report-only"
          } else {
            "content-security-policy"
          };
          headers.insert(key, v);
        }

        if let Some(v) = coop {
          headers.insert("cross-origin-opener-policy", v);
        }
        if let Some(v) = coep {
          headers.insert("cross-origin-embedder-policy", v);
        }
        if let Some(v) = corp {
          headers.insert("cross-origin-resource-policy", v);
        }
        if let Some(v) = permissions_policy {
          headers.insert("permissions-policy", v);
        }

        resp
      })
    }
  }
}
