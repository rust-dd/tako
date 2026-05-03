# tako-fuzz

`cargo-fuzz` harness for Tako parsers and deserializers.

## Targets

- `proxy_protocol_parser` — exercises `tako_server::proxy_protocol::read_proxy_protocol` against arbitrary byte input.
- `path_params_deserializer` — exercises `tako_core::extractors::params::Params<T>::from_request` with arbitrary `(name, value)` slot pairs.
- `grpc_timeout_parser` — exercises `tako_core::grpc::parse_grpc_timeout` against arbitrary UTF-8.

## Running locally

```bash
# install once
cargo install cargo-fuzz

# pick a target and run
cargo +nightly fuzz run proxy_protocol_parser
cargo +nightly fuzz run path_params_deserializer
cargo +nightly fuzz run grpc_timeout_parser

# bound the run to e.g. 5 minutes
cargo +nightly fuzz run proxy_protocol_parser -- -max_total_time=300
```

`cargo-fuzz` requires the **nightly** toolchain because the harness depends on
`libfuzzer_sys`.

## In CI

Two jobs:

- `fuzz-build` — compiles every target on every PR. Catches bit rot.
- `fuzz-smoke` — runs each target for 60 s on PRs labeled `run-fuzz`. Failures
  become CI failures and the corpus is uploaded as an artifact.

## Adding a target

1. Add a binary entry to `Cargo.toml`:
   ```toml
   [[bin]]
   name = "my_target"
   path = "fuzz_targets/my_target.rs"
   test = false
   doc = false
   bench = false
   ```
2. Create `fuzz_targets/my_target.rs` using the `libfuzzer_sys::fuzz_target!`
   macro. Keep the inner closure side-effect-free apart from the parser
   call you are exercising.
3. Add the target to the `fuzz-smoke` matrix in `.github/workflows/ci.yml`.

## Triaging crashes

`cargo-fuzz` writes a reproducer file to `fuzz/artifacts/<target>/`. Check it
into a private branch (do NOT push corpus files to a public branch — they
may contain malicious input). Reproduce locally with:

```bash
cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<file>
```

Then write a regression unit test against the same input and fix the parser.

## Roadmap follow-ups

The following targets are on the roadmap but not yet wired:

- `multipart` parsing (`multer`-backed extractor)
- `JSON` extractor (covers both `serde_json` and the `simd-json` / `sonic-rs`
  fast paths)
- `urlencoded` form parsing
- JWT verification
- Cookie parsing

When adding any of these, follow the same pattern: feed the raw bytes into
the public extractor entry point and ignore the result — we are gating on
"does not panic", not "produces a specific value".
