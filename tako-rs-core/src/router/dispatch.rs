//! Request dispatch: route matching, the middleware/timeout pipeline, and the
//! TSR / 405 / 404 cold paths.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use http::Method;
use http::StatusCode;
use smallvec::SmallVec;

use super::Router;
use crate::body::TakoBody;
use crate::extractors::params::PathParams;
use crate::handler::BoxHandler;
use crate::middleware::Next;
use crate::route::Route;
#[cfg(feature = "signals")]
use crate::signals::Signal;
#[cfg(feature = "signals")]
use crate::signals::SignalArbiter;
#[cfg(feature = "signals")]
use crate::signals::ids;
use crate::types::Request;
use crate::types::Response;

/// Builds an empty-body response with the given status code without going
/// through `http::response::Builder`. The builder API returns `Result` to
/// surface invalid header values, but for the router's hot-path 404 / 405 /
/// 408 / 505 responses we have no headers to fail on, so the result is
/// statically infallible. This helper avoids `.expect("valid …")` calls in
/// the dispatch path.
#[inline]
pub(crate) fn empty_status_response(status: StatusCode) -> Response {
  let mut resp = http::Response::new(TakoBody::empty());
  *resp.status_mut() = status;
  resp
}

impl Router {
  /// Executes the given endpoint through the global middleware chain.
  ///
  /// This helper is used for cases like TSR redirects and default 404 responses,
  /// ensuring that router-level middleware (e.g., CORS) always runs.
  async fn run_with_global_middlewares_for_endpoint(
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
  pub async fn dispatch(&self, mut req: Request) -> Response {
    // Per-router state: only inject when at least one `with_state` was called.
    // The atomic load is monomorphic and cheap; the Arc clone (atomic incref)
    // only happens for routers that actually use instance-local state.
    if self.has_router_state.load(Ordering::Acquire) {
      req.extensions_mut().insert(Arc::clone(&self.router_state));
    }

    // App-level request signal — emitted here so every transport gets it for
    // free without duplicating the boilerplate. The cost is a single string
    // formatting pair per request and is gated to the `signals` feature.
    #[cfg(feature = "signals")]
    let (req_method_str, req_path_str) = (req.method().to_string(), req.uri().path().to_string());
    #[cfg(feature = "signals")]
    {
      SignalArbiter::emit_app(
        Signal::with_capacity(ids::REQUEST_STARTED, 2)
          .meta("method", req_method_str.clone())
          .meta("path", req_path_str.clone()),
      )
      .await;
    }

    // Phase 1: Route lookup using a borrowed path — no String allocation on the
    // hot path. The block scope ensures all borrows on `req` are released before
    // we need to mutate it.
    let route_match = {
      if let Some(method_router) = self.inner.get(req.method())
        && let Ok(matched) = method_router.at(req.uri().path())
      {
        let route = Arc::clone(matched.value);
        let mut it = matched.params.iter();
        let first = it.next();
        let params = first.map(|(fk, fv)| {
          let mut p = SmallVec::<[(String, String); 4]>::new();
          p.push((fk.to_string(), fv.to_string()));
          for (k, v) in it {
            p.push((k.to_string(), v.to_string()));
          }
          PathParams(p)
        });
        Some((route, params))
      } else {
        None
      }
    };

    // Phase 2: Dispatch — `req` is no longer borrowed, safe to mutate.
    let response = if let Some((route, params)) = route_match {
      // Protocol guard: short-circuit dispatch *but fall through* to the shared
      // completion tail (error-handler + REQUEST_COMPLETED signal). Returning
      // here would leak the in-flight signal pair (REQUEST_STARTED already
      // emitted above without a matching REQUEST_COMPLETED).
      if let Some(res) = Self::enforce_protocol_guard(&route, &req) {
        res
      } else {
        #[cfg(feature = "signals")]
        let route_signals = route.signal_arbiter();

        // Initialize route-level plugins on first request
        #[cfg(feature = "plugins")]
        route.setup_plugins_once();

        // Inject route-level SIMD JSON config into request extensions
        if let Some(mode) = route.get_simd_json_mode() {
          req.extensions_mut().insert(mode);
        }

        if let Some(params) = params {
          req.extensions_mut().insert(params);
        }

        // Inject the matched route template (e.g. `/users/{id}`) so handlers
        // and middleware can label metrics/logs by the routing key, not the
        // concrete URI.
        req
          .extensions_mut()
          .insert(crate::router_state::MatchedPath(route.path.clone()));

        // Determine effective timeout: route-level overrides router-level
        let effective_timeout = route.get_timeout().or(self.timeout);

        // Fast atomic check: skip ArcSwap loads entirely when no middleware is registered.
        let needs_chain = self.has_global_middleware.load(Ordering::Acquire)
          || route.has_middleware.load(Ordering::Acquire);

        #[cfg(feature = "signals")]
        {
          // Reuse the strings already formatted for REQUEST_STARTED instead of
          // re-allocating per request on the hot path. Cheap `String::clone` is
          // a single Vec dup; route-level signals consume the clones for the
          // STARTED emission and the final move into ROUTE_REQUEST_COMPLETED.
          let method_str = req_method_str.clone();
          let path_str = req_path_str.clone();
          let route_template = route.path.clone();

          route_signals
            .emit(
              Signal::with_capacity(ids::ROUTE_REQUEST_STARTED, 3)
                .meta("method", method_str.clone())
                .meta("path", path_str.clone())
                .meta("route", route_template.clone()),
            )
            .await;

          let response = if !needs_chain && effective_timeout.is_none() {
            route.handler.call(req).await
          } else {
            let next = Next {
              global_middlewares: self.middlewares.load_full(),
              route_middlewares: route.middlewares.load_full(),
              index: 0,
              endpoint: route.handler.clone(),
            };
            self.run_with_timeout(req, next, effective_timeout).await
          };

          route_signals
            .emit(
              Signal::with_capacity(ids::ROUTE_REQUEST_COMPLETED, 4)
                .meta("method", method_str)
                .meta("path", path_str)
                .meta("route", route_template)
                .meta("status", response.status().as_u16().to_string()),
            )
            .await;

          response
        }

        #[cfg(not(feature = "signals"))]
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
            self.run_with_timeout(req, next, effective_timeout).await
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

      if let Some(method_router) = self.inner.get(req.method())
        && let Ok(matched) = method_router.at(&tsr_path)
        && matched.value.tsr
      {
        let handler = move |_req: Request| {
          let tsr_path = tsr_path.clone();
          async move {
            // `tsr_path` is reconstructed from registered route segments and
            // the incoming URI path. It can technically contain bytes that
            // are invalid in an HTTP header value (CR/LF/NUL) if the request
            // path is crafted maliciously — in that case fall back to a
            // bare 308 without a `Location` header rather than panicking.
            match http::HeaderValue::from_str(&tsr_path) {
              Ok(loc) => {
                let mut resp = empty_status_response(StatusCode::TEMPORARY_REDIRECT);
                resp.headers_mut().insert(http::header::LOCATION, loc);
                resp
              }
              Err(_) => empty_status_response(StatusCode::TEMPORARY_REDIRECT),
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

    let response = self.maybe_apply_error_handler(response);

    #[cfg(feature = "signals")]
    {
      SignalArbiter::emit_app(
        Signal::with_capacity(ids::REQUEST_COMPLETED, 3)
          .meta("method", req_method_str)
          .meta("path", req_path_str)
          .meta("status", response.status().as_u16().to_string()),
      )
      .await;
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
