# Extractors

Extractors read shape out of a request and surface it as typed handler
arguments. Any type implementing
[`FromRequest`](https://docs.rs/tako-rs/latest/tako/extractors/trait.FromRequest.html)
(consumes the body) or
[`FromRequestParts`](https://docs.rs/tako-rs/latest/tako/extractors/trait.FromRequestParts.html)
(headers / URL only) can be used as a handler parameter. A handler
may take any number of extractors; Tako runs them in order before
calling the handler.

```rust
use serde::{Deserialize, Serialize};
use tako::Method;
use tako::extractors::json::Json;
use tako::extractors::path::Path;
use tako::extractors::query::Query;
use tako::extractors::state::State;
use tako::responder::Responder;
use tako::router::Router;

#[derive(Deserialize)]
struct ListQuery { page: u32, per_page: u32 }

#[derive(Deserialize)]
struct UserPath { id: u64 }

#[derive(Deserialize, Serialize)]
struct CreatePost { title: String, body: String }

#[derive(Clone)]
struct Db; // imagine a real pool here

async fn list_posts(
  Path(UserPath { id }): Path<UserPath>,
  Query(q): Query<ListQuery>,
  State(_db): State<Db>,
) -> impl Responder {
  format!("user={id}, page={}, per_page={}", q.page, q.per_page)
}

async fn create_post(
  Path(UserPath { id }): Path<UserPath>,
  Json(p): Json<CreatePost>,
) -> Json<CreatePost> {
  println!("creating post for user={id}: {:?}", p.title);
  Json(p)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;

  let mut router = Router::new();
  router.with_state(Db);
  router.route(Method::GET,  "/users/{id}/posts", list_posts);
  router.route(Method::POST, "/users/{id}/posts", create_post);

  tako::serve(listener, router).await;
  Ok(())
}
```

Bundled extractors fall into a few groups:

- **Body** — `Json<T>`, `Form<T>`, `Bytes`, `Multipart`,
  `TakoTypedMultipart`, `Protobuf<T>`, `SimdJson<T>`, `SonicJson<T>`.
- **URL** — `Path<T>`, `Query<T>`, `QueryMulti<T>`, `RawPath`,
  `RawQuery`, `MatchedPath`, `OriginalUri`, `Host`, `Scheme`.
- **Headers** — `HeaderMap`, `TypedHeader<H>`, `Accept`,
  `AcceptLanguage`, `Authorization`, `Bearer`, `ApiKey`, `Range`,
  `IpAddr`.
- **State / extensions** — `State<T>`, `Extension<T>`,
  `ConnectInfo<T>`.
- **Cookies** — `CookieJar`, `PrivateCookieJar`, `SignedCookieJar`
  with `KeyRing` rotation.
- **JWT** — `JwtClaimsUnverified<T>` (parse-only),
  `JwtClaimsVerified<C>` (consumes `JwtAuth` middleware output).
- **Validation** — `Validated<T>` behind the `validator` / `garde`
  feature flags.
- **Limits** — `ContentLengthLimit<T, N>` to bound body sizes per
  handler.

`Json<T>` automatically dispatches to `sonic_rs` for large payloads
when the `simd` feature is on; the threshold is configurable per
route via `Route::simd_json(SimdJsonMode)`. The `zero-copy-extractors`
feature enables borrowing variants for hot-path handlers.

See also:

- [`examples/extractors-multi`](https://github.com/rust-dd/tako/tree/main/examples/extractors-multi)
  for combining `Params + Query + Json`,
- [`examples/typed-routes`](https://github.com/rust-dd/tako/tree/main/examples/typed-routes)
  for the macro-generated typed-slot path,
- [`examples/multipart`](https://github.com/rust-dd/tako/tree/main/examples/multipart)
  for `Multipart`,
- [`examples/json-header-map`](https://github.com/rust-dd/tako/tree/main/examples/json-header-map)
  for `HeaderMap` plus `Json`.
