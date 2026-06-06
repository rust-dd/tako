---
name: docs-writing
description: Conventions for writing and maintaining tako documentation pages under website/content/docs/. Page templates (transport / extractor / middleware / plugin / concept / tutorial / guide / reference), the frontmatter schema, meta.json sidebar wiring, RustExample-backed examples, and the audit-script contract. Invoke whenever a new public type ships and needs a docs page, or when fixing rot in an existing page.
---

# Docs writing — tako

The docs site lives under `website/` (Fumadocs + Next.js + MDX, Bun,
deployed to `tako.rust-dd.com`). Content is `website/content/docs/`.
Sections with a single page live as a flat
`website/content/docs/<section>.mdx`; sections with multiple pages live as
`website/content/docs/<section>/<name>.mdx` plus a `meta.json` sidebar
manifest. Folder-form sections today: `getting-started/`, `concepts/`,
`transports/`, `extractors/`, `middleware/`, `tutorials/`, `reference/`.
All of them carry an `index.mdx` overview **except `reference/`**, whose
`meta.json` lists only `migration, stability, features, api`.

This SKILL is the **per-page authoring contract**. The audit script
(`website/scripts/docs-audit.ts`) and the linter
(`website/scripts/lint-mdx.ts`) enforce the rules under §2, §4, and §7.

When a section page outgrows a single MDX file (≈ 500 lines), promote it
to folder-form: create `<section>/index.mdx` (move the overview content
here) plus per-feature `<section>/<name>.mdx` pages, add a
`<section>/meta.json` sidebar manifest, and remove `<section>.mdx`.

## 1. Trigger map — which template to use

```
Adding a public …          →  Page goes …                       →  Template
──────────────────────────    ───────────────────────────────       ─────────
transport / protocol       →  transports/<name>.mdx              →  §3.1 Transport
extractor                  →  expand extractors/<group>.mdx      →  §3.2 Extractor
                              (body / request-meta / auth / cookies)
middleware                 →  expand middleware/<group>.mdx      →  §3.3 Middleware
                              (auth / security / traffic / metrics)
plugin (TakoPlugin)        →  plugins.mdx (or plugins/<name>)    →  §3.4 Plugin
cross-cutting trait/idea   →  concepts/<name>.mdx                →  §3.5 Concept
end-to-end use case        →  tutorials/<name>.mdx               →  §3.6 Tutorial
request-handling guide     →  flat <name>.mdx                    →  §3.7 Guide
normative reference        →  reference/<name>.mdx               →  §3.8 Reference
```

Crate ownership (drives the `crate:` key and "which page" calls):

| Crate                | Owns                                                        |
|----------------------|------------------------------------------------------------|
| `tako-rs`            | umbrella re-export (`tako::*`)                              |
| `tako-rs-core`       | routing, handlers, middleware traits, body/request/response, state, signals, queue, GraphQL/gRPC/OpenAPI helpers |
| `tako-rs-extractors` | concrete request extractors                                |
| `tako-rs-server`     | HTTP/1.1, HTTP/2, HTTP/3, TLS, raw TCP/UDP/Unix, PROXY, compio variants |
| `tako-rs-streams`    | WebSocket, SSE, file streaming, static files, WebTransport |
| `tako-rs-plugins`    | bundled middleware + plugins                               |
| `tako-rs-macros`     | `#[tako::route]` / `#[tako::get]` family                   |
| `tako-rs-server-pt`  | thread-per-core entry point                                |

If a new type does not fit an existing section, **stop and ask** — adding
a new top-level section is a sidebar decision, not a per-page one.

## 2. Frontmatter schema (mandatory)

Every page begins with frontmatter validated by zod in
`website/source.config.ts` and re-checked by `website/scripts/lint-mdx.ts`.

```yaml
---
title: <human-readable name>                 # required
description: <one sentence, 20-160 chars>     # required; comfort window 24-152
category: transport                           # concept | guide | transport | extractor | middleware | plugin | tutorial | reference
subcategory: diffusion                        # optional, free string
crate: tako-rs-server                         # optional; must match /^tako-rs(-[a-z]+)*$/
module_path: tako::server::serve_h3           # optional
since: 2.0.0                                   # optional; /^\d+\.\d+(\.\d+)?(-[a-z0-9.]+)?$/
status: stable                                # stable | experimental | deprecated
runtime: both                                 # tokio | compio | both — tako's dual-runtime axis
features: [http3, tls]                         # optional; cargo features needed (inline flow array)
replaced_by: <slug>                            # REQUIRED when status: deprecated
---
```

Current house convention: every page carries `title`, `description`,
`category`, `since: 2.0.0`, `status: stable`. Add `crate`, `runtime`, and
`features` on catalog pages (transports / extractors / middleware) where
they are meaningful. `description` is the OG meta + search snippet — keep
it one sentence, ≤ 152 chars to avoid the soft warning.

## 3. Page templates

Each skeleton is modelled on a real page — open the cited page for the
full house style before writing a new one.

### 3.1 Transport — base: `transports/http.mdx`

```mdx
---
title: HTTP/3
description: …
category: transport
crate: tako-rs-server
runtime: tokio
features: [http3]
since: 2.0.0
status: stable
---

# HTTP/3

<one-paragraph what + when. State runtime support up front.>

## Serving it

```rust
// minimal serve example, real API names from tako-rs-server
```

## Configuration            ## TLS / certificates (if relevant)
## Constraints              <Callout type="warn"> for tokio-only / feature gates </Callout>

See also: /docs/concepts/runtimes, /docs/transports (catalog).
```

Use a `<Tabs items={['Tokio','Compio']}>` split only when the transport
ships on both runtimes with different entry points.

### 3.2 Extractor — base: `extractors/body.mdx`

Group page (body / request-meta / auth / cookies). One `##` section per
extractor: the real type name, what it pulls from the request, a handler
snippet, and the feature gate. Put a `<Callout type="warn">` on any
security-sensitive extractor (e.g. unverified JWT claims). Cross-link the
matching middleware (`/docs/middleware/auth`).

### 3.3 Middleware — base: `middleware/auth.mdx`

Group page (auth / security / traffic / metrics). Lead with the
middleware model (`.into_middleware()` vs `router.plugin(...)`), then one
`##` per middleware: constructor, what it does, ordering notes. Verify the
real type names against `tako-rs-plugins` — they differ from the README's
friendly names (`Csrf` not `CsrfMiddleware`, `CookieSigned` not
`SignedCookieJar`).

### 3.4 Plugin — base: `plugins.mdx`

Explain the `TakoPlugin` trait, the `plugins` feature, and the
plugin-vs-middleware split (a plugin can `install()` its own routes;
middleware only wraps the chain). Link to the four middleware group pages.

### 3.5 Concept — base: `concepts/architecture.mdx`

Cross-cutting explanation (architecture, request lifecycle, runtimes,
design philosophy). Prose + tables over code. No per-API depth — link out
to the catalog pages for that.

### 3.6 Tutorial — base: `tutorials/rest-api.mdx`

End-to-end, runnable. Prefer `<RustExample path="examples/…/src/main.rs" />`
over hand-written code. Walk the reader from empty `main` to a working
service; link each building block to its reference page.

### 3.7 Guide — base: `routing.mdx`

Flat top-level page for a request-handling topic (routing, state, streams,
queue, signals, observability, deployment). Task-oriented prose with
focused snippets.

### 3.8 Reference — base: `reference/features.mdx`

Normative: the cargo feature graph, migration ledger, stability policy,
API index. Tables + exact, exhaustive content. The feature page must cover
every flag in the README "Feature Flags" table.

## 4. meta.json sidebar wiring

- **Top-level** `content/docs/meta.json` orders the whole sidebar and uses
  `"---Section title---"` string entries as group separators (e.g.
  `"---Transports---"`). Folder names (`transports`, `concepts`, …) and
  flat slugs (`routing`, `state`, …) are listed in display order.
- **Folder** `<section>/meta.json` is `{ "title": "...", "pages": [...] }`.
  List `index` first when the folder has an overview page.
- The audit **fails** if an `.mdx` file is not listed in its folder's
  `pages` array. Every new page must be added to the relevant `meta.json`.

## 5. MDX components

Registered globally in `website/mdx-components.tsx` — no import in MDX.

- **`Tabs` / `Tab`** — only for a genuine multi-variant split; the
  canonical tako case is Tokio vs Compio. A lone snippet is a plain
  fenced ```rust block, not a one-tab `Tabs`.
- **`Callout`** — `<Callout type="warn">…</Callout>` (`info | warn |
  error`). Use `warn`/`error` for security gotchas and tokio-only
  constraints.
- **`RustExample`** — `<RustExample path="examples/auth/src/main.rs" />`
  inlines a real workspace file verbatim. The path is relative to the
  **repo root** (`/Users/danixx/Desktop/tako`), not `website/`. The file
  must exist or the audit fails.

Internal links are **site paths**, never `.md`: top-level `foo.mdx` →
`/docs/foo`; `transports/http.mdx` → `/docs/transports/http`; folder index
`transports/index.mdx` → `/docs/transports`.

## 6. Examples must be real

Accuracy is the whole job. Read the crate source before writing — do not
invent signatures or type names. Prefer `<RustExample>` pointing at a real
file in `examples/` over a hand-written snippet. Keep tokio-only vs compio
support correct (the README transport matrix is the reference: HTTP/2 +
TLS on both runtimes; QUIC / gRPC / raw sockets are tokio-only). When a
type's friendly name in the README differs from its exported name, document
the exported name and mention the catalog name once.

## 7. The audit-script contract

Run before every commit that touches docs:

```bash
cd website
bun run typecheck    # tsc --noEmit
bun run lint:mdx     # frontmatter zod schema (scripts/lint-mdx.ts)
bun run audit        # lint:mdx + meta coverage + RustExample + link graph
bun run build        # next build — MDX + zod schema, the source of truth
```

- **`lint:mdx`** hard-fails on: missing `title`/`description`;
  `description` length outside `[20, 160]`; `category` / `status` /
  `runtime` outside their enums; `crate` not matching
  `/^tako-rs(-[a-z]+)*$/`; `status: deprecated` without `replaced_by`. It
  soft-warns when `description` is outside the `[24, 152]` comfort window.
- **`audit`** runs `lint:mdx`, then hard-fails on: an `.mdx` file missing
  from its directory's `meta.json` `pages`; a `<RustExample path>` that
  does not exist; a broken internal `/docs/...` link.

### New-page checklist

1. Write `content/docs/<…>.mdx` with full frontmatter (§2) and an `# H1`
   matching `title`.
2. Add its slug to the relevant `meta.json` `pages` array (§4).
3. Ground every example in real source; use `<RustExample>` for runnable
   files (§6).
4. `bun run audit && bun run build` — both green before committing.
