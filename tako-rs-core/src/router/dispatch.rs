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

#[inline]
pub(crate) fn empty_status_response(status: StatusCode) -> Response {
  let mut resp = crate::responder::new_response(TakoBody::empty());
  *resp.status_mut() = status;
  resp
}

impl Router {
  /// Runs `endpoint` behind the global middleware, so it also covers redirects, 405s and 404s.
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

  /// Runs routing, middleware and request signal hooks under one request timeout.
  /// Cancellation drops pending hooks without replaying completion events.
  #[inline]
  pub async fn dispatch(&self, req: Request) -> Response {
    match self.begin(req, None) {
      Dispatch::Handler(handler) => handler.await,
      Dispatch::Full(req) => self.dispatch_full(req).await,
    }
  }

  /// Runs a request through middleware, timeouts, fallbacks, and error hooks.
  pub(crate) async fn dispatch_full(&self, mut req: Request) -> Response {
    #[cfg(feature = "plugins")]
    if self.setup_plugins_once().is_err() {
      return empty_status_response(StatusCode::INTERNAL_SERVER_ERROR);
    }
    if req
      .extensions()
      .get::<crate::router_state::OriginalUriMarker>()
      .is_none()
    {
      let original = crate::router_state::OriginalUriMarker(req.uri().clone());
      req.extensions_mut().insert(original);
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
    let timeout_router = route
      .and_then(|route| route.scoped_timeout.as_deref())
      .unwrap_or(self);
    let effective_timeout = route
      .and_then(Route::get_timeout)
      .or(timeout_router.timeout)
      .or(self.timeout);
    let fallback_request =
      effective_timeout.and_then(|_| timeout_router.snapshot_for_timeout(&req));
    let lifecycle = async {
      #[cfg(feature = "signals")]
      if let Some(signals) = &signals {
        signals.started().await;
      }
      let response = self.dispatch_endpoint(req, route, is_head).await;
      let response = self.finish_response(response, is_head, request_parts.as_ref());
      #[cfg(feature = "signals")]
      if let Some(signals) = signals {
        signals.complete(response.status()).await;
      }
      response
    };
    match super::timeout::run_with_timeout(lifecycle, effective_timeout).await {
      Ok(response) => response,
      Err(()) => {
        let response = timeout_router.handle_timeout(fallback_request).await;
        self.finish_response(response, is_head, request_parts.as_ref())
      }
    }
  }

  async fn dispatch_endpoint(
    &self,
    req: Request,
    route: Option<&Route>,
    is_head: bool,
  ) -> Response {
    if let Some(route) = route {
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
        // Skip the `ArcSwap` loads when no middleware is registered.
        let needs_chain = self.has_global_middleware.load(Ordering::Acquire)
          || route.has_middleware.load(Ordering::Acquire);

        if !needs_chain {
          route.handler.call(req).await
        } else {
          let next = Next {
            global_middlewares: self.middlewares.load_full(),
            route_middlewares: route.middlewares.load_full(),
            index: 0,
            endpoint: route.handler.clone(),
          };
          next.run(req).await
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
        // RFC 9110: a path routed only for other methods gets 405 with `Allow`, not 404.
        let allowed = self.collect_allowed_methods(req.uri().path());
        if !allowed.is_empty() {
          let allow_value = join_methods(&allowed);
          let handler = move |_req: Request| {
            let allow_value = allow_value.clone();
            async move {
              // Method tokens are always valid header bytes; this only avoids a panic path.
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
    }
  }

  fn finish_response(
    &self,
    response: Response,
    is_head: bool,
    request_parts: Option<&http::request::Parts>,
  ) -> Response {
    let mut response = self.maybe_apply_error_handler(response);
    if (response.status().is_client_error() || response.status().is_server_error())
      && let (Some(handler), Some(parts)) = (&self.error_handler_with_parts, request_parts)
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

    response
  }

  /// Hands 5xx responses to [`Router::error_handler`], 4xx to [`Router::client_error_handler`].
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

  /// Methods with a route for `path`, plus HEAD wherever GET is routed.
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

  /// Answers 505 when the request's HTTP version differs from the route's guard.
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
