#!/usr/bin/env bash
# Publish all tako sub-crates to crates.io in topological order, then publish
# the umbrella `tako-rs` crate.
#
# Usage:
#   ./publish.sh                 # publish for real (runs gate: fmt + clippy + tests)
#   ./publish.sh --dry-run       # validate without uploading (still runs gate)
#   ./publish.sh --allow-dirty   # publish with uncommitted changes
#   ./publish.sh --skip-gate     # bypass fmt/clippy/test gate (NOT recommended)
#
# Notes:
# - Every sub-crate is currently marked `publish = false` in its Cargo.toml
#   ("Internal ... (not published)"). For an actual crates.io release of the
#   `tako-rs` family, that flag must be removed from each sub-crate first —
#   cargo refuses to upload a crate whose path-deps are unpublishable. This
#   script does not edit the manifests; it asssumes the user has prepared
#   them. `--dry-run` is still useful to validate the gate even without
#   flipping the flag.
# - Crates already published at the current local version are skipped, so the
#   script is safe to re-run if a publish fails partway through.
# - Modern cargo (>=1.66) waits for the registry index to sync after each
#   publish, so no manual sleep is needed.

set -euo pipefail

DRY_RUN=()
ALLOW_DIRTY=()
SKIP_GATE=0
for arg in "$@"; do
  case "$arg" in
    --dry-run)     DRY_RUN=(--dry-run) ;;
    --allow-dirty) ALLOW_DIRTY=(--allow-dirty) ;;
    --skip-gate)   SKIP_GATE=1 ;;
    -h|--help)
      sed -n '2,22p' "$0"; exit 0 ;;
    *) echo "unknown flag: $arg" >&2; exit 1 ;;
  esac
done

# Pre-publish gate: fmt + clippy + workspace test. Catches the kinds of
# regressions the v2 pre-release audit (`AUDIT.md`) was set up to prevent.
# Skip with `--skip-gate` if you really must (e.g., re-running after a
# transient crates.io upload failure on a known-good commit).
if [[ $SKIP_GATE -eq 0 ]]; then
  echo "==> pre-publish gate: cargo +nightly fmt --all --check"
  # rustfmt.toml uses `imports_granularity` + `group_imports`, both
  # nightly-only options — fmt has to run on nightly.
  cargo +nightly fmt --all -- --check

  echo "==> pre-publish gate: cargo clippy --workspace --all-features -- -D warnings"
  # `--all-features` is the strictest config: it activates both runtimes
  # (tokio + compio), every transport (TLS / HTTP/2 / HTTP/3 / WebTransport),
  # and every extractor / plugin. Workspace.lints sets `pedantic = warn`, so
  # `-D warnings` catches pedantic regressions too.
  cargo clippy --workspace --all-features --no-deps -- -D warnings

  echo "==> pre-publish gate: cargo test --workspace --all-features"
  cargo test --workspace --all-features
else
  echo "==> WARNING: --skip-gate is set; skipping fmt + clippy + test"
fi

# Topological order — each crate must come AFTER its internal deps.
# Verified against `tako-*/Cargo.toml` `tako-*.workspace = true` entries.
PUBLISH_ORDER=(
  "tako-macros"        # no internal deps
  "tako-core"          # no internal deps
  "tako-extractors"    # tako-core
  "tako-server"        # tako-core
  "tako-server-pt"     # tako-core
  "tako-streams"       # tako-core, tako-server
  "tako-plugins"       # tako-core, tako-extractors
  "tako-rs"            # umbrella — last
)

local_version() {
  # extract the [package] version from a crate's Cargo.toml. All sub-crates
  # set `version.workspace = true`, so they inherit from `[workspace.package]`
  # in the root manifest — read that once and reuse.
  awk '/^\[workspace\.package\]/{p=1; next} /^\[/{p=0} p && /^version[[:space:]]*=/{gsub(/"/,"",$3); print $3; exit}' Cargo.toml
}

registry_version() {
  # latest version on crates.io, "-" if not published
  local crate="$1"
  curl -fsS "https://crates.io/api/v1/crates/$crate" 2>/dev/null \
    | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('crate',{}).get('newest_version','-'))" \
    2>/dev/null || echo "-"
}

publish_one() {
  local spec="$1"
  # spec may be "crate-name" or "crate-name --no-verify"
  local crate="${spec%% *}"
  local extra=()
  if [[ "$spec" == *" "* ]]; then
    read -r -a extra <<< "${spec#* }"
  fi

  local lv rv
  lv=$(local_version)
  rv=$(registry_version "$crate")

  echo
  echo "==> $crate (local=$lv, registry=$rv)"

  if [[ ${#DRY_RUN[@]} -eq 0 && "$lv" == "$rv" ]]; then
    echo "    already published at $lv — skipping"
    return 0
  fi

  set -x
  cargo publish -p "$crate" \
    ${extra[@]+"${extra[@]}"} \
    ${DRY_RUN[@]+"${DRY_RUN[@]}"} \
    ${ALLOW_DIRTY[@]+"${ALLOW_DIRTY[@]}"}
  set +x
}

for spec in "${PUBLISH_ORDER[@]}"; do
  publish_one "$spec"
done

echo
echo "All crates processed."
