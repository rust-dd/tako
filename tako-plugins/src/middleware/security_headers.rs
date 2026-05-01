//! Security headers middleware.
//!
//! Emits a curated set of HTTP response headers that close the most common
//! injection / framing / cross-origin gaps. Defaults match modern browser
//! advice (OWASP Secure Headers Project, web.dev / MDN guidance):
//!
//! - `X-Content-Type-Options: nosniff`
//! - `X-Frame-Options: DENY`
//! - `Referrer-Policy: strict-origin-when-cross-origin`
//! - `Strict-Transport-Security` (opt-in via [`SecurityHeaders::hsts`])
//! - `Content-Security-Policy` (opt-in via [`SecurityHeaders::csp`] /
//!   [`SecurityHeaders::csp_with_nonce`])
//! - `Cross-Origin-Opener-Policy: same-origin` (opt-in)
//! - `Cross-Origin-Embedder-Policy: require-corp` (opt-in)
//! - `Cross-Origin-Resource-Policy: same-origin` (opt-in)
//! - `Permissions-Policy` (opt-in)
//!
//! `X-XSS-Protection` is intentionally **not** emitted — modern browsers
//! ignore the header and OWASP recommends removing it. CSP is the
//! authoritative replacement.
//!
//! Per-request CSP nonces are exposed as a [`CspNonce`] extension so handlers
//! can interpolate them into inline `<script>` / `<style>` blocks. The header
//! emitted to the client substitutes the nonce into a template string.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::HeaderValue;
use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::types::Request;
use tako_core::types::Response;

/// Per-request CSP nonce inserted into request extensions when
/// [`SecurityHeaders::csp_with_nonce`] is configured.
///
/// Handlers can substitute the value into inline scripts:
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
  WithNonce { template: String, header: bool },
}

/// Security headers middleware configuration.
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
  /// Creates a SecurityHeaders middleware with sensible defaults.
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

  /// Sets the `X-Frame-Options` value (e.g. `"DENY"`, `"SAMEORIGIN"`).
  pub fn frame_options(mut self, value: &'static str) -> Self {
    self.frame_options = HeaderValue::from_static(value);
    self
  }

  /// Enables / disables `Strict-Transport-Security`.
  pub fn hsts(mut self, enable: bool) -> Self {
    self.hsts = enable;
    self
  }

  /// Sets the HSTS `max-age`. Default: 1 year.
  pub fn hsts_max_age(mut self, seconds: u64) -> Self {
    self.hsts_max_age = seconds;
    self
  }

  /// Toggles `includeSubDomains` on HSTS. Default: true.
  pub fn hsts_include_subdomains(mut self, on: bool) -> Self {
    self.hsts_include_subdomains = on;
    self
  }

  /// Toggles `preload` on HSTS. Default: false. Submission to the HSTS
  /// preload list requires `max-age >= 31536000` and `includeSubDomains`.
  pub fn hsts_preload(mut self, on: bool) -> Self {
    self.hsts_preload = on;
    self
  }

  /// Sets the `Referrer-Policy` value.
  pub fn referrer_policy(mut self, value: &'static str) -> Self {
    self.referrer_policy = HeaderValue::from_static(value);
    self
  }

  /// Emits a static `Content-Security-Policy` header.
  pub fn csp(mut self, value: &'static str) -> Self {
    self.csp = Some(CspMode::Static(HeaderValue::from_static(value)));
    self
  }

  /// Emits a per-request CSP with a fresh nonce. The template must contain
  /// the literal substring `{nonce}`, which is replaced before emission. The
  /// generated nonce is also inserted into request extensions as
  /// [`CspNonce`].
  pub fn csp_with_nonce(mut self, template: impl Into<String>) -> Self {
    self.csp = Some(CspMode::WithNonce {
      template: template.into(),
      header: false,
    });
    self
  }

  /// Same as [`Self::csp_with_nonce`], but emit `Content-Security-Policy-Report-Only`.
  pub fn csp_report_only(mut self, template: impl Into<String>) -> Self {
    self.csp = Some(CspMode::WithNonce {
      template: template.into(),
      header: true,
    });
    self
  }

  /// Sets `Cross-Origin-Opener-Policy`.
  pub fn coop(mut self, value: &'static str) -> Self {
    self.coop = Some(HeaderValue::from_static(value));
    self
  }

  /// Sets `Cross-Origin-Embedder-Policy`.
  pub fn coep(mut self, value: &'static str) -> Self {
    self.coep = Some(HeaderValue::from_static(value));
    self
  }

  /// Sets `Cross-Origin-Resource-Policy`.
  pub fn corp(mut self, value: &'static str) -> Self {
    self.corp = Some(HeaderValue::from_static(value));
    self
  }

  /// Sets `Permissions-Policy`.
  pub fn permissions_policy(mut self, value: &'static str) -> Self {
    self.permissions_policy = Some(HeaderValue::from_static(value));
    self
  }
}

fn rand_nonce() -> String {
  // 18 random bytes → 24-char base64. UUID v4 covers 16 random bytes; pad
  // with the second half of a fresh UUID for the remaining 2.
  let u1 = uuid::Uuid::new_v4().into_bytes();
  let u2 = uuid::Uuid::new_v4().into_bytes();
  let mut buf = [0u8; 18];
  buf[..16].copy_from_slice(&u1);
  buf[16..].copy_from_slice(&u2[..2]);
  use base64::Engine;
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
        // Generate the per-request nonce up front so the handler can read it
        // back from request extensions before the response is built.
        let prepared_csp: Option<(HeaderValue, bool)> = match csp.as_ref() {
          None => None,
          Some(CspMode::Static(v)) => Some((v.clone(), false)),
          Some(CspMode::WithNonce { template, header }) => {
            let nonce = rand_nonce();
            let value = template.replace("{nonce}", &nonce);
            req.extensions_mut().insert(CspNonce(nonce));
            HeaderValue::from_str(&value)
              .ok()
              .map(|hv| (hv, *header))
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
