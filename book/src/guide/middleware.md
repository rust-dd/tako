# Middleware

Middleware in Tako wraps a handler with cross-cutting logic — auth,
logging, rate limiting, compression — without changing the handler
itself. Anything implementing
[`IntoMiddleware`](https://docs.rs/tako-rs/latest/tako/middleware/trait.IntoMiddleware.html)
can be attached either to a single route or to the whole router.

```rust,ignore
use tako::Method;
use tako::middleware::IntoMiddleware;
use tako::middleware::basic_auth::BasicAuth;
use tako::middleware::bearer_auth::BearerAuth;
use tako::middleware::request_id::RequestId;
use tako::responder::Responder;
use tako::router::Router;
use tako::types::Request;

async fn admin_only(_: Request) -> impl Responder { "secret" }
async fn webhook(_: Request) -> impl Responder { "ack" }

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;

  let basic = BasicAuth::single("admin", "pw")
    .realm("Admin")
    .into_middleware();
  let bearer = BearerAuth::static_token("hunter2").into_middleware();
  let request_id = RequestId::new().into_middleware();

  let mut router = Router::new();
  router.middleware(request_id); // global: every request gets X-Request-ID

  router
    .route(Method::GET, "/admin", admin_only)
    .middleware(basic);

  router
    .route(Method::POST, "/webhook", webhook)
    .middleware(bearer);

  tako::serve(listener, router).await;
  Ok(())
}
```

`tako-plugins` ships the bundled middleware set. The most common
modules:

- **Authentication** — `basic_auth::BasicAuth`,
  `bearer_auth::BearerAuth`, `api_key_auth::ApiKeyAuth`,
  `jwt_auth::JwtAuth<V>` (constant-time comparison, runtime key
  rotation).
- **Sessions and CSRF** — `session::SessionMiddleware`,
  `csrf::CsrfMiddleware`.
- **Cross-cutting** — `request_id::RequestId`, `access_log::AccessLog`,
  `traceparent::Traceparent`, `body_limit::BodyLimit`,
  `timeout::Timeout`, `etag::Etag`, `tenant::Tenant`,
  `circuit_breaker::CircuitBreaker`, `ip_filter::IpFilter`,
  `hmac_signature::HmacSignature`, `json_schema::JsonSchema`,
  `problem_json::ProblemJson`, `upload_progress::UploadProgress`,
  `security_headers::SecurityHeaders`.
- **Plugin-style middleware** (router-level installers) —
  `plugins::cors::Cors`, `plugins::compression::Compression`,
  `plugins::rate_limiter::RateLimiter`,
  `plugins::idempotency::Idempotency`,
  `plugins::metrics::{PrometheusMetricsConfig, OtelMetricsConfig}`.

Stateful middleware (sessions, rate limit, idempotency, JWKS, CSRF)
plug their persistence in through the `tako_plugins::stores::*`
traits. The in-process default is an `Arc<Mutex<HashMap<...>>>`;
companion crates (Redis, Postgres) implement the same traits without
changing handler code.

See also:

- [`examples/auth`](https://github.com/rust-dd/tako/tree/main/examples/auth)
  for the basic/bearer/JWT setup,
- [`examples/health`](https://github.com/rust-dd/tako/tree/main/examples/health)
  for healthcheck wiring,
- [`examples/multipart`](https://github.com/rust-dd/tako/tree/main/examples/multipart)
  and [`examples/upload-progress`](https://github.com/rust-dd/tako/tree/main/examples/upload-progress)
  for upload flows.

> The `Timeout` middleware currently ships a tokio-runtime path. A
> compio variant is on the v2 follow-up list — until then, prefer
> per-route timeouts via `Router::timeout` on compio.
