//! The CSRF middleware itself: the [`IntoMiddleware`] implementation.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::StatusCode;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

use super::config::Csrf;
use super::cookie::ensure_csrf_cookie;
use super::cookie::req_session_token;
use super::cookie::strip_csrf_seed_cookie;
use super::token::extract_cookie;
use super::token::is_unsafe_method;
use super::token::origin_allowed;
use crate::middleware::session::Session;

impl IntoMiddleware for Csrf {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let cookie_name = Arc::new(self.cookie_name);
    let header_name = Arc::new(self.header_name);
    let exempt_paths = Arc::new(self.exempt_paths);
    let secure = self.secure;
    let same_site = self.same_site;
    let trusted_origins = Arc::new(self.trusted_origins);
    let bind_to_session = self.bind_to_session;
    let session_key = Arc::new(self.session_key);

    move |req: Request, next: Next| {
      let cookie_name = cookie_name.clone();
      let header_name = header_name.clone();
      let exempt_paths = exempt_paths.clone();
      let trusted_origins = trusted_origins.clone();
      let session_key = session_key.clone();

      Box::pin(async move {
        let path = req.uri().path().to_string();
        // Snapshot the Session handle BEFORE we hand the request to the
        // downstream handler. Session lives in *request* extensions and gets
        // dropped together with `req`; reading it off `resp.extensions` (the
        // previous implementation) saw nothing, which silently disabled
        // CSRF session-binding entirely.
        let session = req.extensions().get::<Session>().cloned();

        // Issue path: safe methods or exempt paths short-circuit verification.
        let safe_method = !is_unsafe_method(req.method());
        let exempt = exempt_paths.iter().any(|p| path.starts_with(p.as_str()));
        if safe_method || exempt {
          let mut resp = next.run(req).await;
          // If the handler called `Session::rotate()` we must mint a fresh
          // CSRF token to invalidate any stolen pair from the pre-rotation
          // identity. Otherwise a privilege transition (login, role change)
          // would leave the CSRF cookie usable against the new session id.
          let rotated = session
            .as_ref()
            .is_some_and(crate::middleware::session::Session::rotation_requested);
          let seed = if rotated {
            None
          } else {
            req_session_token(&resp)
          };
          // PMW-12(a): the `__csrf_seed` cookie is an internal handler
          // hook; strip it before the response leaves the server so the
          // marker name never reaches the client.
          if seed.is_some() {
            strip_csrf_seed_cookie(&mut resp);
          }
          ensure_csrf_cookie(
            &mut resp,
            &cookie_name,
            secure,
            same_site,
            &session_key,
            bind_to_session,
            seed.as_ref(),
            session.as_ref(),
          );
          return resp;
        }

        // When `bind_to_session=true` is configured the only thing standing
        // between the attacker and a successful unsafe request would be the
        // double-submit cookie pattern (cookie == header). An XSS-stolen
        // cookie can be echoed into the matching header trivially, so without
        // a Session the binding is non-existent. Fail closed.
        if bind_to_session && session.is_none() {
          return (
            StatusCode::FORBIDDEN,
            "CSRF: session required for token binding",
          )
            .into_response();
        }

        // Extract candidate tokens.
        let cookie_token = extract_cookie(&req, &cookie_name).map(str::to_string);
        let header_token = req
          .headers()
          .get(header_name.as_str())
          .and_then(|v| v.to_str().ok())
          .map(str::to_string);
        let session_token = session.as_ref().and_then(|s| s.get::<String>(&session_key));

        let cookie_header_match = matches!(
          (cookie_token.as_deref(), header_token.as_deref()),
          (Some(c), Some(h)) if c == h && !c.is_empty()
        );
        // PMW-11: with `bind_to_session=true` (default), a session that has
        // not yet been seeded with the CSRF token leaves a bootstrap gap —
        // a stolen cookie (post-rotation, stale window) replayed with a
        // matching header would otherwise pass `session_match` via the
        // `(None, _) => true` arm. Fail-closed in that mode so the only
        // accepted path is "session present AND session token matches
        // cookie". The unbound mode (`bind_to_session=false`) keeps the
        // double-submit-only fallback.
        let session_match = match (&session_token, &cookie_token) {
          (Some(s), Some(c)) => s == c,
          // No session or empty session bucket — only safe under unbound
          // mode where the cookie/header double-submit is the sole guard.
          (None, _) => !bind_to_session,
          _ => false,
        };

        if cookie_header_match && session_match {
          let mut resp = next.run(req).await;
          let rotated = session
            .as_ref()
            .is_some_and(crate::middleware::session::Session::rotation_requested);
          let preferred = if rotated {
            None
          } else {
            session_token.or(cookie_token)
          };
          ensure_csrf_cookie(
            &mut resp,
            &cookie_name,
            secure,
            same_site,
            &session_key,
            bind_to_session,
            preferred.as_ref(),
            session.as_ref(),
          );
          return resp;
        }

        // Fallback: trusted Origin / Referer header.
        let trust_hit = if trusted_origins.is_empty() {
          false
        } else {
          let origin = req
            .headers()
            .get(http::header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
          let referer = req
            .headers()
            .get(http::header::REFERER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
          origin
            .as_deref()
            .is_some_and(|o| origin_allowed(o, &trusted_origins))
            || referer
              .as_deref()
              .is_some_and(|r| origin_allowed(r, &trusted_origins))
        };

        if trust_hit {
          let mut resp = next.run(req).await;
          let rotated = session
            .as_ref()
            .is_some_and(crate::middleware::session::Session::rotation_requested);
          let preferred = if rotated {
            None
          } else {
            session_token.or(cookie_token)
          };
          ensure_csrf_cookie(
            &mut resp,
            &cookie_name,
            secure,
            same_site,
            &session_key,
            bind_to_session,
            preferred.as_ref(),
            session.as_ref(),
          );
          return resp;
        }

        (StatusCode::FORBIDDEN, "CSRF token mismatch").into_response()
      })
    }
  }
}
