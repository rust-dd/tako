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
use super::token::is_unsafe_method;
use super::token::origin_trusted;
use super::token::path_is_exempt;
use crate::middleware::session::Session;
use crate::middleware::session::extract_cookie_value;

impl IntoMiddleware for Csrf {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let stored = super::stored::StoredCsrf::from_config(&self).map(Arc::new);
    let cookie_name = Arc::new(self.cookie_name);
    let header_name = Arc::new(self.header_name);
    let exempt_paths = Arc::new(self.exempt_paths);
    let secure = self.secure;
    let same_site = self.same_site;
    let trusted_origins = Arc::new(self.trusted_origins);
    let bind_to_session = self.bind_to_session;
    let session_key = Arc::new(self.session_key);

    move |req: Request, next: Next| {
      let stored = stored.clone();
      let cookie_name = cookie_name.clone();
      let header_name = header_name.clone();
      let exempt_paths = exempt_paths.clone();
      let trusted_origins = trusted_origins.clone();
      let session_key = session_key.clone();

      Box::pin(async move {
        if let Some(stored) = stored {
          return stored
            .handle(req, next)
            .await
            .unwrap_or_else(crate::stores::runtime::unavailable);
        }
        // Read before `next` consumes the request: the session lives in request extensions.
        let session = req.extensions().get::<Session>().cloned();

        let safe_method = !is_unsafe_method(req.method());
        let exempt = path_is_exempt(req.uri().path(), &exempt_paths);
        if safe_method || exempt {
          let mut resp = next.run(req).await;
          // After `Session::rotate()` mint a fresh token so a pair stolen before a privilege
          // change cannot be used against the new session id.
          let rotated = session.as_ref().is_some_and(Session::rotation_requested);
          let seed = if rotated {
            None
          } else {
            req_session_token(&resp)
          };
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

        // Without a session, a stolen cookie echoed into the header would pass. Fail closed.
        if bind_to_session && session.is_none() {
          return (
            StatusCode::FORBIDDEN,
            "CSRF: session required for token binding",
          )
            .into_response();
        }

        let cookie_token = extract_cookie_value(&req, &cookie_name).map(str::to_string);
        let header_token = req
          .headers()
          .get(header_name.as_str())
          .and_then(|v| v.to_str().ok());
        let session_token = session.as_ref().and_then(|s| s.get::<String>(&session_key));

        let cookie_header_match = matches!(
          (cookie_token.as_deref(), header_token),
          (Some(c), Some(h)) if c == h && !c.is_empty()
        );
        // Bound mode accepts only a session token equal to the cookie; an unseeded session
        // fails, otherwise a replayed stolen pair would pass.
        let session_match = match (&session_token, &cookie_token) {
          (Some(s), Some(c)) => s == c,
          (None, _) => !bind_to_session,
          _ => false,
        };

        let verified =
          (cookie_header_match && session_match) || origin_trusted(&req, &trusted_origins);
        if !verified {
          return (StatusCode::FORBIDDEN, "CSRF token mismatch").into_response();
        }

        let mut resp = next.run(req).await;
        let rotated = session.as_ref().is_some_and(Session::rotation_requested);
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
        resp
      })
    }
  }
}
