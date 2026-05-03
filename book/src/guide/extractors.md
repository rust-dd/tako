# Extractors

> **Status:** scaffold.

Extractors read shape out of a request and surface it as typed handler
arguments. Bundled extractors include:

- Body: `Json<T>`, `Form<T>`, `Bytes`, `Multipart`, `TakoTypedMultipart`,
  `Protobuf<T>`, `SimdJson<T>`, `SonicJson<T>`.
- URL: `Path<T>`, `Query<T>`, `QueryMulti<T>`, `RawPath`, `RawQuery`,
  `MatchedPath`, `OriginalUri`, `Host`, `Scheme`.
- Headers: `HeaderMap`, `TypedHeader<H>`, `Accept`, `AcceptLanguage`,
  `Authorization`, `Bearer`, `ApiKey`.
- State / extensions: `State<T>`, `Extension<T>`, `ConnectInfo<T>`.
- Cookies: `CookieJar`, `PrivateCookieJar`, `SignedCookieJar` with
  `KeyRing` rotation.
- JWT: `JwtClaimsUnverified<T>` (parse-only), `JwtClaimsVerified<C>`
  (consumes `JwtAuth` middleware output).
- Validation: `Validated<T>` behind `validator` / `garde` features.
- Limits: `ContentLengthLimit<T, N>`.

`Json<T>` automatically dispatches to `sonic_rs` for large payloads
when the `simd` feature is on; the threshold is configurable per
route via `Route::simd_json(SimdJsonMode)`.
