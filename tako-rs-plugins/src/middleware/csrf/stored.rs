use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use http::StatusCode;
use http::header;
use tako_rs_core::middleware::Next;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

use super::config::Csrf;
use super::cookie::strip_csrf_seed_cookie;
use super::token::build_cookie;
use super::token::extract_cookie;
use super::token::is_unsafe_method;
use super::token::origin_allowed;
use crate::middleware::session::SameSite;
use crate::middleware::session::Session;
use crate::stores::CsrfTokenStore;
use crate::stores::StoreResult;
use crate::stores::runtime;

pub(crate) struct StoredCsrf {
  backend: Arc<dyn CsrfTokenStore>,
  cookie_name: String,
  header_name: String,
  session_key: String,
  identity_key: String,
  exempt_paths: Vec<String>,
  origins: Vec<String>,
  secure: bool,
  same_site: SameSite,
  ttl: Duration,
  single_use: bool,
  started: AtomicBool,
}

impl StoredCsrf {
  pub(crate) fn from_config(config: &Csrf) -> Option<Self> {
    Some(Self {
      backend: config.store.clone()?,
      cookie_name: config.cookie_name.clone(),
      header_name: config.header_name.clone(),
      session_key: config.session_key.clone(),
      identity_key: format!("{}_store_identity", config.session_key),
      exempt_paths: config.exempt_paths.clone(),
      origins: config.trusted_origins.clone(),
      secure: config.secure,
      same_site: config.same_site,
      ttl: config.token_ttl,
      single_use: config.single_use,
      started: AtomicBool::new(false),
    })
  }

  pub(crate) async fn handle(&self, request: Request, next: Next) -> StoreResult<Response> {
    let Some(session) = request.extensions().get::<Session>().cloned() else {
      return Ok(
        (
          StatusCode::FORBIDDEN,
          "CSRF: session required for token storage",
        )
          .into_response(),
      );
    };
    if !self.started.swap(true, Ordering::Relaxed) {
      runtime::sweep(&self.backend, Duration::from_secs(150), |backend| {
        Box::pin(async move { backend.sweep().await })
      });
    }
    let mut identity = session
      .get::<String>(&self.identity_key)
      .unwrap_or_else(|| {
        let identity = uuid::Uuid::new_v4().simple().to_string();
        session.set(&self.identity_key, &identity);
        identity
      });
    let exempt = self
      .exempt_paths
      .iter()
      .any(|path| request.uri().path().starts_with(path));
    if is_unsafe_method(request.method()) && !exempt {
      let candidate = request
        .headers()
        .get(self.header_name.as_str())
        .and_then(|value| value.to_str().ok());
      let cookie = extract_cookie(&request, &self.cookie_name);
      let verified = if let Some(candidate) =
        candidate.filter(|candidate| !candidate.is_empty() && Some(*candidate) == cookie)
      {
        self
          .backend
          .validate(&identity, candidate, self.single_use)
          .await?
      } else {
        false
      };
      let trusted = [header::ORIGIN, header::REFERER].iter().any(|name| {
        request
          .headers()
          .get(name)
          .and_then(|value| value.to_str().ok())
          .is_some_and(|origin| origin_allowed(origin, &self.origins))
      });
      if !verified && !trusted {
        return Ok((StatusCode::FORBIDDEN, "CSRF token invalid").into_response());
      }
    }
    let mut response = next.run(request).await;
    strip_csrf_seed_cookie(&mut response);
    if session.is_destroyed() {
      return Ok(response);
    }
    if session.rotation_requested() {
      identity = uuid::Uuid::new_v4().simple().to_string();
      session.set(&self.identity_key, &identity);
      session.remove(&self.session_key);
    }
    let token = if let Some(token) = session.get::<String>(&self.session_key)
      && self.backend.validate(&identity, &token, false).await?
    {
      token
    } else {
      self.backend.issue(&identity, self.ttl).await?
    };
    session.set(&self.session_key, &token);
    let retained = response
      .headers()
      .get_all(header::SET_COOKIE)
      .iter()
      .filter(|value| {
        value
          .to_str()
          .ok()
          .and_then(|value| value.split_once('='))
          .is_none_or(|(name, _)| name.trim() != self.cookie_name)
      })
      .cloned()
      .collect::<Vec<_>>();
    response.headers_mut().remove(header::SET_COOKIE);
    for value in retained {
      response.headers_mut().append(header::SET_COOKIE, value);
    }
    let cookie = build_cookie(&self.cookie_name, &token, self.secure, self.same_site);
    response
      .headers_mut()
      .append(header::SET_COOKIE, cookie.parse()?);
    Ok(response)
  }
}
