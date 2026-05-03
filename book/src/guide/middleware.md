# Middleware

> **Status:** scaffold.

Middleware wraps handlers via `IntoMiddleware`. Bundled middleware
lives in `tako-plugins` and includes:

- Authentication: `BasicAuth`, `BearerAuth`, `ApiKeyAuth`,
  `JwtAuth<V>`, with constant-time comparisons and runtime key
  rotation.
- Sessions, CSRF, rate limiting, idempotency.
- Cross-cutting: `Timeout`, `Traceparent`, `AccessLog`,
  `ProblemJson`, `CircuitBreaker`, `IpFilter`, `Healthcheck`,
  `Etag`, `Tenant`, `HmacSignature`, `JsonSchema`.
- Compression, CORS, security headers.

Backends for stateful middleware (sessions, rate limit, idempotency,
JWKS, CSRF tokens) plug in via the `tako_plugins::stores::*` traits.

> The `Timeout` middleware currently ships a tokio-runtime path. A
> compio-runtime variant is on the v2 follow-up list.
