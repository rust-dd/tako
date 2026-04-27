---
name: dev-rules
description: General coding-style rules to apply to every project. Use when writing or editing source code, comments, or doc comments — especially when adding new files or refactoring.
---

# General Dev Rules

These are user-wide style rules. Apply them everywhere unless a project's own `CLAUDE.md` explicitly overrides one of them.

## Comments

### Banner / divider comments are forbidden

Do **not** write decorative section banners. Examples of what NOT to do:

```rust
// ----- top-level convenience re-exports -----------------------------------------
// =============== HELPERS ===============
// /////////////// Server entry points ///////////////
// --- Database setup ---
// ===== INTERNAL HELPERS =====
```

These add visual noise, make diffs ugly, and rot when code moves. If a section is large enough that a header feels useful, that is a signal to **split the file or move items into a submodule** instead of papering over it with a banner.

If a logical grouping inside one file truly helps the reader, use a **single-line plain comment** with a short noun phrase (no dashes, equals signs, slashes, or trailing fillers):

```rust
// re-exports from tako-core
pub use tako_core::body;
pub use tako_core::types;
```

### Don't restate what code already says

Skip comments that just paraphrase the next line. Code with good names doesn't need narration.

```rust
// BAD — restates the code
// Increment counter
counter += 1;

// BAD — narrates structure that's obvious from the AST
// Loop through users
for user in users { ... }
```

A comment earns its place only when it explains the **why** that isn't visible: a hidden constraint, a workaround, an invariant, a surprising performance choice.

### No "added for X / used by Y / changed in PR Z" notes

Don't reference tasks, PRs, callers, or migration history in comments. That belongs in commit messages and PR descriptions, where it stays attached to the diff and doesn't rot.

```rust
// BAD
// added for the new auth flow (PR #482)
// used by the worker pool refactor
// removed once compio 0.18 ships

// OK if the rationale itself is durable
// SO_REUSEPORT distributes accepts at the kernel level — required for
// thread-per-core to avoid contending on a single accept queue.
```

### No multi-paragraph docstrings unless the API is genuinely subtle

A one-line summary plus an example is enough for 95% of public items. Walls of doc text get skimmed and outdated. If something needs paragraphs of warnings, the API design is probably wrong.

### Stale `// TODO` / `// FIXME` markers

Only leave a TODO if there is a concrete next action. Vague TODOs ("// TODO: improve this") get ignored forever. If there is no clear next step, delete the comment and just write the code.

## Source-file structure

- Don't separate file regions with banner comments — let module boundaries, blank lines, and good ordering do the work.
- Keep imports in groups (std / external / local) without decorating the groups.
- Don't add a trailing comment to mark "end of impl block" or similar — the closing brace is sufficient.

## When in doubt

Prefer no comment over a marginal one. Reviewers can ask if something is unclear; deleting noise improves signal.
