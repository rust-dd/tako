//! Request dispatch: route matching, the middleware/timeout pipeline, and the
//! TSR / 405 / 404 cold paths.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use http::Method;
use http::StatusCode;
use http_body::Body;
use smallvec::SmallVec;

use super::Dispatch;
use super::Router;
use super::fast_path::clear_route_entries;
use super::fast_path::set_matched_path;
use super::fast_path::set_params;
use crate::body::TakoBody;
use crate::handler::BoxHandler;
use crate::middleware::Next;
use crate::route::Route;
use crate::types::Request;
use crate::types::Response;

/// Builds a status response with an empty body.
#[inline]
pub(crate) fn empty_status_response(status: StatusCode) -> Response {
  let mut resp = crate::responder::new_response(TakoBody::empty());
  *resp.status_mut() = status;
  resp
}

impl Router {
  /// Executes the given endpoint through the global middleware chain.
  ///
  /// This helper is used for cases like TSR redirects and default 404 responses,
  /// ensuring that router-level middleware (e.g., CORS) always runs.
  pub(super) async fn run_with_global_middlewares_for_endpoint(
    &self,
    req: Request,
    endpoint: BoxHandler,
  ) -> Response {
    if self.has_global_middleware.load(Ordering::Acquire) {
      Next {
        global_middlewares: self.middlewares.load_full(),
        route_middlewares: Arc::default(),
        index: 0,
        endpoint,
      }
      .run(req)
      .await
    } else {
      endpoint.call(req).await
    }
  }

  /// Dispatches an incoming request to the appropriate route handler.
  #[inline]
  pub async fn dispatch(&self, req: Request) -> Response {
    match self.begin(req, None) {
      Dispatch::Handler(handler) => handler.await,
      Dispatch::Full(req) => self.dispatch_full(req).await,
    }
  }

  /// Runs a request through middleware, timeouts, fallbacks, and error hooks.
  pub(crate) async fn dispatch_full(&self, req: Request) -> Response {
    #[cfg(feature = "plugins")]
    if self.setup_plugins_once().is_err() {
      return empty_status_response(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let request_parts = self.error_handler_with_parts.as_ref().map(|_| {
      let mut saved = Request::default();
      *saved.method_mut() = req.method().clone();
      *saved.uri_mut() = req.uri().clone();
      *saved.version_mut() = req.version();
      *saved.headers_mut() = req.headers().clone();
      *saved.extensions_mut() = req.extensions().clone();
      saved.into_parts().0
    });
    let is_head = req.method() == Method::HEAD;

    let (mut parts, body) = req.into_parts();
    let route = {
      let matched = self
        .inner
        .get(&parts.method)
        .and_then(|router| router.at(parts.uri.path()).ok())
        .or_else(|| {
          if is_head {
            self
              .inner
              .get(&Method::GET)
              .and_then(|router| router.at(parts.uri.path()).ok())
          } else {
            None
          }
        });
      if let Some(matched) = matched {
        set_params(&mut parts.extensions, matched.value, &matched.params);
        set_matched_path(&mut parts.extensions, matched.value);
        Some(matched.value.as_ref())
      } else {
        clear_route_entries(&mut parts.extensions);
        None
      }
    };
    self.set_route_entries(&mut parts.extensions, route);
    let req = Request::from_parts(parts, body);

    #[cfg(feature = "signals")]
    let signals = super::request_signals::RequestSignals::new(&self.signals, route, &req);
    #[cfg(feature = "signals")]
    if let Some(signals) = &signals {
      signals.started().await;
    }

    let response = if let Some(route) = route {
      // Failures still pass through error formatting and completion signals.
      #[cfg(feature = "plugins")]
      let setup_error = route.setup_plugins_once().err();
      #[cfg(not(feature = "plugins"))]
      let setup_error = None::<String>;
      if let Some(res) = Self::enforce_protocol_guard(route, &req)
        .or_else(|| setup_error.map(|_| empty_status_response(StatusCode::INTERNAL_SERVER_ERROR)))
      {
        res
      } else {
        let timeout_router = route.scoped_timeout.as_deref().unwrap_or(self);
        let effective_timeout = route
          .get_timeout()
          .or(timeout_router.timeout)
          .or(self.timeout);

        // Fast atomic check: skip ArcSwap loads entirely when no middleware is registered.
        let needs_chain = self.has_global_middleware.load(Ordering::Acquire)
          || route.has_middleware.load(Ordering::Acquire);

        {
          if !needs_chain && effective_timeout.is_none() {
            route.handler.call(req).await
          } else {
            let next = Next {
              global_middlewares: self.middlewares.load_full(),
              route_middlewares: route.middlewares.load_full(),
              index: 0,
              endpoint: route.handler.clone(),
            };
            timeout_router
              .run_with_timeout(req, next, effective_timeout)
              .await
          }
        }
      }
    } else {
      // Cold path: no direct match — try TSR redirect / 405 / fallback.
      // String allocation is acceptable here.
      let tsr_path = {
        let p = req.uri().path();
        if p.ends_with('/') {
          p.trim_end_matches('/').to_string()
        } else {
          format!("{p}/")
        }
      };

      let tsr_match = self
        .inner
        .get(req.method())
        .and_then(|router| router.at(&tsr_path).ok())
        .or_else(|| {
          if is_head {
            self
              .inner
              .get(&Method::GET)
              .and_then(|router| router.at(&tsr_path).ok())
          } else {
            None
          }
        });
      if let Some(matched) = tsr_match
        && matched.value.tsr
      {
        let location = match req.uri().query() {
          Some(query) => format!("{tsr_path}?{query}"),
          None => tsr_path,
        };
        let handler = move |_req: Request| {
          let location = location.clone();
          async move {
            match http::HeaderValue::from_str(&location) {
              Ok(loc) => {
                let mut resp = empty_status_response(StatusCode::TEMPORARY_REDIRECT);
                resp.headers_mut().insert(http::header::LOCATION, loc);
                resp
              }
              Err(_) => empty_status_response(StatusCode::BAD_REQUEST),
            }
          }
        };

        self
          .run_with_global_middlewares_for_endpoint(req, BoxHandler::new::<_, (Request,)>(handler))
          .await
      } else {
        // Method-mismatch detection: if the same path is registered for any
        // *other* method, RFC 9110 mandates 405 with an `Allow` header rather
        // than 404. This is the cold path; iterating the 9 standard methods
        // is cheap.
        let allowed = self.collect_allowed_methods(req.uri().path());
        if !allowed.is_empty() {
          let allow_value = join_methods(&allowed);
          let handler = move |_req: Request| {
            let allow_value = allow_value.clone();
            async move {
              // `allow_value` is built from `Method::as_str()` for the
              // registered methods, so it only contains ASCII method tokens
              // — `HeaderValue::from_str` is statically infallible. Use the
              // fallible API and ignore the impossible error rather than
              // panicking.
              let mut resp = empty_status_response(StatusCode::METHOD_NOT_ALLOWED);
              if let Ok(v) = http::HeaderValue::from_str(&allow_value) {
                resp.headers_mut().insert(http::header::ALLOW, v);
              }
              resp
            }
          };
          self
            .run_with_global_middlewares_for_endpoint(
              req,
              BoxHandler::new::<_, (Request,)>(handler),
            )
            .await
        } else if let Some(handler) = &self.fallback {
          self
            .run_with_global_middlewares_for_endpoint(req, handler.clone())
            .await
        } else {
          let handler = |_req: Request| async { empty_status_response(StatusCode::NOT_FOUND) };

          self
            .run_with_global_middlewares_for_endpoint(
              req,
              BoxHandler::new::<_, (Request,)>(handler),
            )
            .await
        }
      }
    };

    let mut response = self.maybe_apply_error_handler(response);
    if (response.status().is_client_error() || response.status().is_server_error())
      && let (Some(handler), Some(parts)) = (&self.error_handler_with_parts, &request_parts)
    {
      response = handler(parts, response);
    }
    if is_head {
      let status = response.status();
      if !status.is_informational()
        && status != StatusCode::NO_CONTENT
        && status != StatusCode::NOT_MODIFIED
        && !response
          .headers()
          .contains_key(http::header::TRANSFER_ENCODING)
        && let Some(length) = response.body().size_hint().exact()
      {
        response
          .headers_mut()
          .entry(http::header::CONTENT_LENGTH)
          .or_insert(http::HeaderValue::from(length));
      }
      *response.body_mut() = TakoBody::empty();
    }

    #[cfg(feature = "signals")]
    if let Some(signals) = signals {
      signals.complete(response.status()).await;
    }

    response
  }

  /// Applies the appropriate error handler if one is set:
  /// - 5xx → [`Router::error_handler`]
  /// - 4xx → [`Router::client_error_handler`]
  fn maybe_apply_error_handler(&self, response: Response) -> Response {
    let status = response.status();
    if status.is_server_error() {
      if let Some(handler) = &self.error_handler {
        return handler(response);
      }
    } else if status.is_client_error()
      && let Some(handler) = &self.client_error_handler
    {
      return handler(response);
    }
    response
  }

  /// Returns every method that has a route matching the given path.
  ///
  /// Used by the 405 / `Allow` cold-path branch in [`Router::dispatch`]; not on
  /// the fast path. Iterates all standard methods (O(9)) plus any custom ones.
  fn collect_allowed_methods(&self, path: &str) -> SmallVec<[Method; 4]> {
    let mut allowed = SmallVec::<[Method; 4]>::new();
    for (method, m) in self.inner.iter() {
      if m.at(path).is_ok() {
        allowed.push(method);
      }
    }
    if allowed.contains(&Method::GET) && !allowed.contains(&Method::HEAD) {
      allowed.push(Method::HEAD);
    }
    allowed
  }

  /// Ensures the request HTTP version satisfies the route's configured protocol guard.
  /// Returns `Some(Response)` with 505 HTTP Version Not Supported when the request
  /// doesn't match the guard, otherwise returns `None` to continue dispatch.
  fn enforce_protocol_guard(route: &Route, req: &Request) -> Option<Response> {
    if let Some(guard) = route.protocol_guard()
      && guard != req.version()
    {
      return Some(empty_status_response(
        StatusCode::HTTP_VERSION_NOT_SUPPORTED,
      ));
    }
    None
  }
}

/// Joins a slice of HTTP methods into a comma-separated `Allow`-header value.
fn join_methods(methods: &[Method]) -> String {
  let mut out = String::with_capacity(methods.len() * 8);
  for (i, m) in methods.iter().enumerate() {
    if i > 0 {
      out.push_str(", ");
    }
    out.push_str(m.as_str());
  }
  out
}
