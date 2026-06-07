//! Router composition: macro mounting, prefix scoping, nesting, and merging.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::Router;

impl Router {
  /// Registers every route declared via the `#[tako::route]` / `#[tako::get]`
  /// (and friends) attribute macros into this router.
  ///
  /// Each macro contributes a thunk into the global [`TAKO_ROUTES`] slice at
  /// link time; this method walks the slice and invokes each thunk against
  /// `self`, which calls [`Router::route`] under the hood. Routes are
  /// registered in the order the linker emits them — typically the order they
  /// appear within a translation unit, but unspecified across crates. If two
  /// thunks register the same `(method, path)` pair, the second call will
  /// panic, matching the behavior of [`Router::route`].
  ///
  /// # Why `linkme` and not explicit registration
  ///
  /// We keep the `linkme` distributed-slice strategy on purpose. The
  /// alternative — an explicit `register_routes!(my_crate::routes)` invocation
  /// per crate — was considered and rejected because:
  ///
  /// * Adding a handler would require touching three places (the handler
  ///   itself, the per-crate registration list, and the call site that
  ///   imports it) instead of one. The macro authoring story is the main
  ///   reason teams pick attribute routing in the first place.
  /// * Cross-crate path collisions panic at startup either way; explicit
  ///   registration does not buy any extra safety.
  /// * Link-order non-determinism only matters when two routes share a
  ///   `(method, path)` pair — that is already a hard failure and a CI test
  ///   catches it deterministically.
  /// * Prefix grouping is already covered by [`Router::mount_all_into`], so
  ///   "I want all my routes under `/api`" does not require explicit
  ///   registration.
  ///
  /// Callers that need stable, deterministic ordering should call
  /// [`Router::route`] directly.
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

  /// Like [`Router::mount_all`] but registers every macro-declared route under
  /// the given path prefix. The prefix is normalized (trailing `/` stripped),
  /// then prepended to each registered path. Useful when you want, e.g., all
  /// `#[get("/users")]` declarations to live under `/api`.
  ///
  /// Ordering across crates remains the linker's choice (see
  /// [`Router::mount_all`] for details).
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
    // Same panic-restore guard as `scope`: a route conflict from any
    // registered `#[tako_route]` macro now resets `pending_prefix`.
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

  /// Registers a group of routes under a shared path prefix.
  ///
  /// The closure receives `self` with the prefix active, so any `route()` /
  /// `get()` / `post()` etc. calls inside register the routes with the prefix
  /// prepended. Prefixes nest: a `scope("/v1", |r| r.scope("/users", …))`
  /// produces routes under `/v1/users`. Cold path; no dispatch impact.
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
    // Panic-safe restore of `pending_prefix`. A route-conflict panic in the
    // user-supplied `build` closure used to leave the temporary nested
    // prefix in place, permanently poisoning subsequent route registrations
    // on the same builder.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(self)));
    self.pending_prefix = saved;
    if let Err(payload) = result {
      std::panic::resume_unwind(payload);
    }
    self
  }

  /// Mounts every route from a child router under the given path prefix.
  ///
  /// Unlike [`Router::merge`], `nest` builds **new** `Arc<Route>` instances for
  /// each child route via `Route::cloned_with_path` — so re-nesting the same
  /// child cannot double-stack its global middleware onto the same shared
  /// `Arc<Route>`. The child router's global middleware chain is prepended to
  /// each newly-registered route's middleware chain (so child globals run
  /// before child-route middleware at dispatch time).
  ///
  /// Caveats:
  /// - Route-level plugins on the child are **not** carried over.
  /// - The child's fallback / error handlers are **not** inherited.
  ///
  /// # Panics
  ///
  /// Panics at registration time if mounting the child would conflict with a
  /// route already present on `self` (same method + same prefixed path).
  /// Mirrors the behavior of [`Router::route`] — route registration is a
  /// startup-time operation and conflicts are configuration bugs, not
  /// runtime conditions.
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
    let upstream_globals = child.middlewares.load_full();

    for (method, weak_vec) in child.routes.iter() {
      for weak in weak_vec {
        let Some(child_route) = weak.upgrade() else {
          continue;
        };

        let combined = combine_prefix_path(prefix, &child_route.path);
        let new_path = self.apply_pending_prefix(&combined);

        let new_route = child_route.cloned_with_path(new_path.clone());

        if !upstream_globals.is_empty() {
          let existing = new_route.middlewares.load_full();
          let mut merged = Vec::with_capacity(upstream_globals.len() + existing.len());
          merged.extend(upstream_globals.iter().cloned());
          merged.extend(existing.iter().cloned());
          new_route.has_middleware.store(true, Ordering::Release);
          new_route.middlewares.store(Arc::new(merged));
        }

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

    #[cfg(feature = "signals")]
    self.signals.merge_from(&child.signals);

    self
  }

  /// Merges another router into this router.
  ///
  /// This method combines routes and middleware from another router into the
  /// current one. Routes are copied over, and the other router's global middleware
  /// is prepended to each merged route's middleware chain.
  ///
  /// # Panics
  ///
  /// Panics at registration time if a merged route conflicts with one already
  /// present on `self` (same method + same path). Mirrors the behavior of
  /// [`Router::route`] and [`Router::nest`] — merge is a startup-time
  /// operation and route conflicts are configuration bugs.
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
    let upstream_globals = other.middlewares.load_full();

    for (method, weak_vec) in other.routes.iter() {
      for weak in weak_vec {
        if let Some(child_route) = weak.upgrade() {
          // Re-issue the route as a fresh `Arc<Route>` (same path) so we do
          // not mutate the child's middleware chain in-place — other router
          // instances may still hold the original `Arc` and would observe
          // unrelated middleware insertions otherwise.
          let new_route = child_route.cloned_with_path(child_route.path.clone());

          if !upstream_globals.is_empty() {
            let existing = new_route.middlewares.load_full();
            let mut merged = Vec::with_capacity(upstream_globals.len() + existing.len());
            merged.extend(upstream_globals.iter().cloned());
            merged.extend(existing.iter().cloned());
            new_route.has_middleware.store(true, Ordering::Release);
            new_route.middlewares.store(Arc::new(merged));
          }

          // Match `nest` semantics: a path conflict is a builder bug, not a
          // silent overwrite. Returning early via `let _ = … insert` would
          // throw away the existing route under a stable URL.
          if let Err(err) = self
            .inner
            .get_or_default_mut(&method)
            .insert(new_route.path.clone(), new_route.clone())
          {
            panic!(
              "Failed to merge route '{}' (method {:?}): {err}",
              new_route.path, method
            );
          }

          self
            .routes
            .get_or_default_mut(&method)
            .push(Arc::downgrade(&new_route));
        }
      }
    }

    #[cfg(feature = "signals")]
    self.signals.merge_from(&other.signals);
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

/// Distributed slice of route registration thunks.
///
/// Each `#[tako::route]` / `#[tako::get]` / etc. attribute contributes a
/// `fn(&mut Router)` closure that calls [`Router::route`] with the
/// generated `Params::METHOD` / `Params::PATH` and the handler. Iterating
/// the slice — what [`Router::mount_all`] does — replays every contribution
/// against the supplied router.
#[linkme::distributed_slice]
pub static TAKO_ROUTES: [fn(&mut Router)] = [..];
