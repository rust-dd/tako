#!/usr/bin/env bash
# Publish workspace crates to crates.io in dependency order.

set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: ./publish.sh [--gate-only|--dry-run] [--allow-dirty] [--skip-gate]

  --gate-only   Run formatting, Clippy, and tests without accessing the registry.
  --dry-run     Validate Cargo packages without uploading, after the gate passes.
  --allow-dirty Allow publishing with uncommitted changes.
  --skip-gate   Skip formatting, Clippy, and tests; incompatible with --gate-only.
USAGE
}

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "$repo_root"

DRY_RUN=()
ALLOW_DIRTY=()
SKIP_GATE=0
GATE_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --gate-only)   GATE_ONLY=1 ;;
    --dry-run)     DRY_RUN=(--dry-run) ;;
    --allow-dirty) ALLOW_DIRTY=(--allow-dirty) ;;
    --skip-gate)   SKIP_GATE=1 ;;
    -h|--help)
      usage; exit 0 ;;
    *) echo "unknown flag: $arg" >&2; exit 1 ;;
  esac
done

if [[ $GATE_ONLY -eq 1 && $SKIP_GATE -eq 1 ]]; then
  echo "--gate-only cannot be combined with --skip-gate" >&2
  exit 2
fi

if [[ $SKIP_GATE -eq 0 ]]; then
  echo "==> pre-publish gate: formatting"
  # The import-formatting options in rustfmt.toml require nightly.
  cargo +nightly fmt --all -- --check

  for profile in default tokio-rich all-features; do
    echo "==> pre-publish gate: tests ($profile)"
    "$repo_root/scripts/check-workspace.sh" test "$profile"
    echo "==> pre-publish gate: Clippy ($profile)"
    "$repo_root/scripts/check-workspace.sh" clippy "$profile"
  done
else
  echo "==> WARNING: --skip-gate is set; skipping fmt + clippy + test"
fi

if [[ $GATE_ONLY -eq 1 ]]; then
  echo "Pre-publish gate passed."
  exit 0
fi

# Topological order — each crate must come AFTER its internal deps.
# Verified against `tako-*/Cargo.toml` `tako-*.workspace = true` entries.
PUBLISH_ORDER=(
  "tako-rs-macros"     # no internal deps
  "tako-rs-core"       # no internal deps
  "tako-rs-extractors" # tako-rs-core
  "tako-rs-server"     # tako-rs-core
  "tako-rs-server-pt"  # tako-rs-core
  "tako-rs-streams"    # tako-rs-core
  "tako-rs-plugins"    # tako-rs-core, tako-rs-extractors
  "tako-rs"            # umbrella — last
)

local_version() {
  # extract the [package] version from a crate's Cargo.toml. All sub-crates
  # set `version.workspace = true`, so they inherit from `[workspace.package]`
  # in the root manifest — read that once and reuse.
  awk '/^\[workspace\.package\]/{p=1; next} /^\[/{p=0} p && /^version[[:space:]]*=/{gsub(/"/,"",$3); print $3; exit}' Cargo.toml
}

registry_version() {
  # latest version on crates.io, "-" if not published. crates.io rejects
  # requests without a User-Agent (HTTP 403), so send one — otherwise this
  # always returns "-" and the skip check below never fires.
  local crate="$1"
  curl -fsS -H "User-Agent: tako-publish (dancixx@gmail.com)" \
    "https://crates.io/api/v1/crates/$crate" 2>/dev/null \
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

  local logf status
  logf=$(mktemp)
  set -x
  set +e
  cargo publish -p "$crate" \
    ${extra[@]+"${extra[@]}"} \
    ${DRY_RUN[@]+"${DRY_RUN[@]}"} \
    ${ALLOW_DIRTY[@]+"${ALLOW_DIRTY[@]}"} 2>&1 | tee "$logf"
  status=${PIPESTATUS[0]}
  set -e
  set +x

  if [[ $status -ne 0 ]]; then
    # Fallback skip: if the registry pre-check missed an existing version (API
    # lag, transient fetch failure), cargo still refuses with "already exists"
    # / "already uploaded". Treat that as a skip so the script stays re-runnable
    # instead of aborting under `set -e`.
    if grep -qiE "already (exists|uploaded)" "$logf"; then
      echo "    already on crates.io — skipping"
      rm -f "$logf"
      return 0
    fi
    rm -f "$logf"
    return "$status"
  fi
  rm -f "$logf"
}

for spec in "${PUBLISH_ORDER[@]}"; do
  publish_one "$spec"
done

echo
echo "All crates processed."
