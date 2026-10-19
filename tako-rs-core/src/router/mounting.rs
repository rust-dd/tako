//! Router composition: macro mounting, prefix scoping, nesting, and merging.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::Router;

impl Router {
  /// Registers every route declared with `#[tako::route]`, `#[tako::get]` and the like, in
  /// link order (unspecified across crates); panics on a duplicate route like [`Router::route`].
  ///
  /// # Examples
  ///
  /// ```ignore
  /// use tako::{get, router::Router};
  ///
  /// #[get("/health")]
  /// async fn health() -> impl tako::responder::Responder { "ok" }
  ///
  /// let mut router = Router::new();
  /// router.mount_all();
  /// ```
  pub fn mount_all(&mut self) -> &mut Self {
    for register in TAKO_ROUTES {
      register(self);
    }
    self
  }

  /// [`Router::mount_all`] under `prefix`, minus its trailing `/`.
  ///
  /// # Examples
  ///
  /// ```ignore
  /// let mut router = Router::new();
  /// router.mount_all_into("/api"); // /users → /api/users, /health → /api/health
  /// ```
  pub fn mount_all_into(&mut self, prefix: &str) -> &mut Self {
    let saved = self.pending_prefix.take();
    self.pending_prefix = Some(prefix.to_string());
    // Restore the prefix even when a duplicate route panics.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
      for register in TAKO_ROUTES {
        register(self);
      }
    }));
    self.pending_prefix = saved;
    if let Err(payload) = result {
      std::panic::resume_unwind(payload);
    }
    self
  }

  /// Registers the routes that `build` adds under `prefix`; nested scopes join their prefixes.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::router::Router;
  /// use tako::responder::Responder;
  ///
  /// async fn list_users() -> impl Responder { "users" }
  /// async fn create_user() -> impl Responder { "created" }
  ///
  /// let mut router = Router::new();
  /// router.scope("/api/v1", |r| {
  ///     r.get("/users", list_users);
  ///     r.post("/users", create_user);
  /// });
  /// ```
  pub fn scope<F>(&mut self, prefix: &str, build: F) -> &mut Self
  where
    F: FnOnce(&mut Router),
  {
    let saved = self.pending_prefix.take();
    let new_prefix = match &saved {
      Some(parent) => {
        let parent = parent.trim_end_matches('/');
        if prefix.starts_with('/') {
          format!("{parent}{prefix}")
        } else {
          format!("{parent}/{prefix}")
        }
      }
      None => prefix.to_string(),
    };
    self.pending_prefix = Some(new_prefix);
    // Restore the prefix even when `build` panics, or later routes would inherit it.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(self)));
    self.pending_prefix = saved;
    if let Err(payload) = result {
      std::panic::resume_unwind(payload);
    }
    self
  }

  /// Mounts `child`'s routes under `prefix`, keeping its middleware, plugins, state, body limit
  /// and timeout but not its fallback or error handlers; panics on a conflicting route.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::router::Router;
  /// use tako::responder::Responder;
  ///
  /// async fn list_users() -> impl Responder { "users" }
  ///
  /// let mut api = Router::new();
  /// api.get("/users", list_users);
  ///
  /// let mut root = Router::new();
  /// root.nest("/api/v1", api); // /users → /api/v1/users
  /// ```
  pub fn nest(&mut self, prefix: &str, child: Router) -> &mut Self {
    let child = Arc::new(child);

    for (method, weak_vec) in child.routes.iter() {
      for weak in weak_vec {
        let Some(child_route) = weak.upgrade() else {
          continue;
        };

        let combined = combine_prefix_path(prefix, &child_route.path);
        let new_path = self.apply_pending_prefix(&combined);

        let mut new_route = child_route.cloned_with_path(new_path.clone());
        let configured = Arc::get_mut(&mut new_route).expect("new route is uniquely owned");
        configured.scoped_state = Some(Arc::new(crate::router_state::RouterState::layered(
          child_route
            .scoped_state
            .clone()
            .unwrap_or_else(|| child.router_state.clone()),
          self.router_state.clone(),
        )));
        if configured.scoped_timeout.is_none() && child.timeout.is_some() {
          configured.scoped_timeout = Some(child.clone());
        }
        configured.body_limit = configured.body_limit.or(child.body_limit);

        let scope = child.clone();
        let middleware = Arc::new(move |req, next: crate::middleware::Next| {
          let scope = scope.clone();
          Box::pin(async move {
            scope
              .run_with_global_middlewares_for_endpoint(
                req,
                crate::handler::BoxHandler::new::<_, (crate::types::Request,)>(move |req| {
                  let next = next.clone();
                  async move { next.run(req).await }
                }),
              )
              .await
          })
            as std::pin::Pin<Box<dyn std::future::Future<Output = crate::types::Response> + Send>>
        });
        let existing = new_route.middlewares.load_full();
        let mut merged = Vec::<crate::types::BoxMiddleware>::with_capacity(existing.len() + 1);
        merged.push(middleware);
        merged.extend(existing.iter().cloned());
        new_route.middlewares.store(Arc::new(merged));
        new_route.has_middleware.store(true, Ordering::Release);

        if let Err(err) = self
          .inner
          .get_or_default_mut(&method)
          .insert(new_path, new_route.clone())
        {
          panic!("Failed to nest route: {err}");
        }
        self
          .routes
          .get_or_default_mut(&method)
          .push(Arc::downgrade(&new_route));
      }
    }

    #[cfg(all(feature = "signals", not(feature = "plugins")))]
    self.signals.merge_from(&child.signals);

    self.nested_routers.push(child);
    self
  }

  /// [`Router::nest`] without a prefix.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::{router::Router, Method, responder::Responder, types::Request};
  ///
  /// async fn api_handler(_req: Request) -> impl Responder {
  ///     "API response"
  /// }
  ///
  /// async fn web_handler(_req: Request) -> impl Responder {
  ///     "Web response"
  /// }
  ///
  /// // Create API router
  /// let mut api_router = Router::new();
  /// api_router.route(Method::GET, "/users", api_handler);
  /// api_router.middleware(|req, next| async move {
  ///     println!("API middleware");
  ///     next.run(req).await
  /// });
  ///
  /// // Create main router and merge API router
  /// let mut main_router = Router::new();
  /// main_router.route(Method::GET, "/", web_handler);
  /// main_router.merge(api_router);
  /// ```
  pub fn merge(&mut self, other: Router) {
    self.nest("", other);
  }
}

/// Joins a path prefix and a child path, normalising the boundary slash.
fn combine_prefix_path(prefix: &str, path: &str) -> String {
  if prefix.is_empty() || prefix == "/" {
    return path.to_string();
  }
  let prefix = prefix.trim_end_matches('/');
  if path.is_empty() || path == "/" {
    return prefix.to_string();
  }
  if path.starts_with('/') {
    let mut out = String::with_capacity(prefix.len() + path.len());
    out.push_str(prefix);
    out.push_str(path);
    out
  } else {
    let mut out = String::with_capacity(prefix.len() + 1 + path.len());
    out.push_str(prefix);
    out.push('/');
    out.push_str(path);
    out
  }
}

/// Registrations emitted by the route attribute macros; [`Router::mount_all`] replays them.
#[linkme::distributed_slice]
pub static TAKO_ROUTES: [fn(&mut Router)] = [..];
