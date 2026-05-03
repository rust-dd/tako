# Routing

> **Status:** scaffold.

`Router` matches `(method, path)` pairs to handlers. Path syntax is
`matchit`-compatible: `{name}` for a free segment, `{*rest}` for a
catch-all, and `{name: T}` for a typed slot when using the
`#[tako::route]` macro family.

- Method shorthands: `router.get(path, h)`, `.post`, `.put`, `.patch`,
  `.delete`, `.head`, `.options`.
- Sub-routing: `router.nest(prefix, child)`, `router.scope(prefix,
  |r| { ... })`.
- Macros: `#[tako::get("/users/{id: u64}")]` defines a handler and
  registers it via the `linkme` distributed slice; pull all
  macro-defined routes in with `router.mount_all()` or
  `router.mount_all_into("/api")`.
- Wrong-method response: `405 Method Not Allowed` with the `Allow`
  header populated.
- Wrong-path response: configurable fallback via `router.fallback(...)`.
