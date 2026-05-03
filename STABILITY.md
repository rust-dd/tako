# API stability policy

> Effective from the **2.0** release. The 1.x line continues to receive
> security backports per the schedule below; new feature work happens on
> 2.x and forward.

This document is contractual. If something is documented as stable here,
breaking it is a major-version event. If something is documented as
unstable, expect minor-version churn.

## Versioning

Tako follows [Semantic Versioning 2.0.0](https://semver.org/) with the
clarifications in this document. Public API on the `tako-rs` umbrella
crate is the primary stability surface.

| Bump | Examples |
|---|---|
| **Patch** (`x.y.z` → `x.y.z+1`) | Bug fixes that don't change observable behavior, internal refactors, doc fixes, performance work that doesn't shift API. |
| **Minor** (`x.y.z` → `x.y+1.0`) | Additive APIs, new optional cargo features, soft deprecations, new transports, new middleware, additional fields on non-`#[non_exhaustive]` structs *only* when guarded by a non-exhaustive marker, MSRV bump. |
| **Major** (`x.y.z` → `x+1.0.0`) | Removing a public item, changing a public function signature, narrowing trait bounds, renaming a re-export, raising the cargo feature graph in a way that breaks an existing config, dropping a transport, dropping support for a runtime. |

## Stable surface

The following are part of the stable API. Anything not listed here is
not part of the stability contract.

- Every public item under `tako::*` re-exported by `tako-rs/src/lib.rs`.
  This is the canonical entry point. Direct dependence on
  `tako-core` / `tako-extractors` / `tako-plugins` / `tako-server` /
  `tako-streams` / `tako-server-pt` is **not** covered by this policy
  outside of the umbrella re-exports.
- Every cargo feature on `tako-rs/Cargo.toml` (the umbrella) and the
  feature-graph it builds into the sub-crates.
- The `Server::builder()` API surface.
- The `Router::*` public methods.
- The `Responder` / `IntoResponse` traits and their blanket impls
  shipped today.
- The `FromRequest<'a>` and `FromRequestParts<'a>` traits, including
  the `+ Send + 'a` bound on the returned future.
- The error types that hit the wire (`Problem`, `JsonError`,
  `ContentLengthLimitError`, `ParamsError`, `ProxyHeader` fields when
  PROXY protocol is enabled, etc.).
- The `ConnInfo` struct and its enums (`PeerAddr`, `Transport`,
  `TlsInfo`).

## Signal IDs

Signal IDs emitted by `tako_core::signals::ids` are part of the stable
contract — operators write dashboards and alerts against them. Examples:

- `request.started`, `request.completed`
- `connection.opened`, `connection.closed`
- `server.started`
- `route.request.*`
- `queue.job.queued`, `.started`, `.completed`, `.failed`,
  `.retrying`, `.dead_letter`

Stability rules:

- **Renaming a signal ID is a major-version change.**
- **Adding a new signal ID is a minor-version change.**
- **Removing a signal ID is a major-version change.**
- Field shape on a signal payload is governed by the per-signal docs.
  Unless documented otherwise, payloads are `#[non_exhaustive]` and may
  grow new fields in minor releases.

## MSRV (Minimum Supported Rust Version)

- Tako tracks the **latest stable** Rust release.
- The MSRV is recorded in workspace `Cargo.toml` under
  `[workspace.package].rust-version`.
- Bumping the MSRV is allowed in any minor release as long as it is
  noted in the changelog. We do not need a major bump to follow stable.
- The CI matrix proves the published MSRV builds; `nightly` is run for
  Miri / fuzz / fmt only.

If you depend on a specific MSRV floor lower than what we ship, pin the
last release whose `rust-version` you can support and open an issue.

## Cargo features

- New cargo features may appear in minor releases.
- Removing a cargo feature is a major-version change.
- Renaming a feature is a major-version change. Aliases may be added in
  minor releases and the original kept around for one major cycle.
- A feature may flip from "off by default" to "on by default" in a
  minor release if and only if the implied dependencies don't add a new
  breaking transitive dependency.

## Deprecation policy

Soft-deprecation is the default. The cycle:

1. **Minor release N**: the item is annotated with `#[deprecated]` and
   the replacement is documented inline. Existing code keeps compiling.
2. **At least one minor release** elapses with the deprecation in
   place.
3. **Next major release**: the deprecated item may be removed.

Hard-removing a public item without a deprecation cycle is reserved for
security fixes and items shipped strictly inside the same major-version
cycle.

## `#[non_exhaustive]` discipline

Public configuration structs (`ServerConfig`, `Limits`,
`SessionMiddleware`, `RateLimiterBuilder`, `CompressionBuilder`,
`CorsBuilder`, etc.) and public enums that may grow variants are
annotated `#[non_exhaustive]`. This means:

- Adding a new field is a minor-version change.
- Adding a new variant is a minor-version change.
- Construction must go through builders or struct-update with `..` from
  a `Default::default()`.

If a `pub struct` or `pub enum` is **not** `#[non_exhaustive]`, that is
itself part of the contract — adding a field or variant requires a
major bump.

## Re-exports

The `tako-rs` umbrella re-exports symbols from sub-crates under
`tako::*` paths. The umbrella path is what is contractually stable, not
the underlying sub-crate path. Examples:

- `tako::Server` is stable. `tako_server::Server` may move.
- `tako::extractors::Json` is stable. The fact that it lives in
  `tako_core::extractors::json` is an implementation detail.

If you find yourself importing from `tako_core` / `tako_extractors`
directly, either open an issue requesting a re-export or accept that
your import may break in a minor release.

## Public traits

Adding a method to a public trait is a major-version change unless the
method has a default body that is sound to inherit.

The following traits are sealed (may not be implemented downstream):

- *(none yet — when sealing happens, it is documented here.)*

## Cargo SemVer warning

Until 2.0 ships, the `1.x` line on crates.io is the supported release.
Code on `main` is pre-2.0 and may break between commits. Lock to
specific git revisions if you must build against `main`.

## Backports

- The latest 1.x minor receives security backports through 2.0 + 6 months.
- Older 1.x lines are unsupported once a newer 1.x minor exists.

## Reporting a stability bug

If you observe an *accidental* break in a minor or patch release, open
an issue tagged `stability-regression`. We will:

1. Confirm the symptom.
2. Issue a patch release that restores compatibility.
3. Move the change behind a deprecation cycle and re-land it in the
   next major release.
