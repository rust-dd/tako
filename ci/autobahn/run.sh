#!/usr/bin/env bash
#
# Runs the Autobahn fuzzingclient suite against the bundled echo server.
#
# Required tools on PATH:
#   - cargo (with the workspace already cloned)
#   - docker
#
# Exit codes:
#   0    every case is OK or NON-STRICT (interpreted as pass per Autobahn).
#   1    the runner crashed, the server didn't come up, or any case is
#        FAILED / INFORMATIONAL_FAILED / WRONG_CODE / UNCLEAN.
#
# Usage:
#   ci/autobahn/run.sh                      # full case set per fuzzingclient.json
#   AUTOBAHN_CASES='1.*,2.*' ci/autobahn/run.sh   # restrict cases (override)

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
CONF_DIR="${ROOT_DIR}/ci/autobahn"
REPORT_DIR="${ROOT_DIR}/ci/autobahn/reports"

cd "${ROOT_DIR}"

echo "==> Building autobahn-echo example (release)"
cargo build --release --manifest-path examples/autobahn-echo/Cargo.toml

echo "==> Starting autobahn-echo on 127.0.0.1:9001"
"${ROOT_DIR}/examples/autobahn-echo/target/release/autobahn-echo" &
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null || true' EXIT

# Give the server a moment to bind. Avoid a hard sleep loop — poll for the
# port to actually accept a TCP connection.
for _ in $(seq 1 50); do
  if (echo > /dev/tcp/127.0.0.1/9001) 2>/dev/null; then
    break
  fi
  sleep 0.1
done
if ! (echo > /dev/tcp/127.0.0.1/9001) 2>/dev/null; then
  echo "autobahn-echo failed to bind 127.0.0.1:9001" >&2
  exit 1
fi
echo "==> server up, running fuzzingclient"

mkdir -p "${REPORT_DIR}"

CASES_OVERRIDE="${AUTOBAHN_CASES:-}"
if [[ -n "${CASES_OVERRIDE}" ]]; then
  IFS=',' read -ra CASE_ARR <<<"${CASES_OVERRIDE}"
  CASES_JSON=$(printf '"%s",' "${CASE_ARR[@]}")
  CASES_JSON="[${CASES_JSON%,}]"
  CONF_FILE="${REPORT_DIR}/fuzzingclient.local.json"
  python3 - "${CONF_DIR}/fuzzingclient.json" "${CONF_FILE}" "${CASES_JSON}" <<'PY'
import json, sys
src, dst, cases = sys.argv[1], sys.argv[2], sys.argv[3]
conf = json.load(open(src))
conf["cases"] = json.loads(cases)
open(dst, "w").write(json.dumps(conf, indent=2))
PY
else
  CONF_FILE="${CONF_DIR}/fuzzingclient.json"
fi

# `--network host` lets the container talk to the server bound on the
# host loopback. Linux-only — that's fine for CI (ubuntu-latest).
docker run --rm --network host \
  -v "${CONF_DIR}:/config:ro" \
  -v "${REPORT_DIR}:/reports" \
  -v "${CONF_FILE}:/config/fuzzingclient.json:ro" \
  crossbario/autobahn-testsuite \
  wstest -m fuzzingclient -s /config/fuzzingclient.json

# Stop the server before grading the report so any lingering output flushes.
kill "${SERVER_PID}" 2>/dev/null || true
wait "${SERVER_PID}" 2>/dev/null || true
trap - EXIT

INDEX="${REPORT_DIR}/clients/index.json"
if [[ ! -f "${INDEX}" ]]; then
  echo "report index missing: ${INDEX}" >&2
  exit 1
fi

python3 - "${INDEX}" <<'PY'
import json, sys
report = json.load(open(sys.argv[1]))
fail = 0
total = 0
for agent, cases in report.items():
    for case_id, info in cases.items():
        total += 1
        behavior = info.get("behavior", "UNKNOWN")
        # OK = passed the spec; NON-STRICT = passed with caveats; INFORMATIONAL = informational only.
        if behavior not in ("OK", "NON-STRICT", "INFORMATIONAL"):
            fail += 1
            print(f"FAIL  {agent}  {case_id}  {behavior}", file=sys.stderr)
print(f"==> Autobahn: {total - fail}/{total} passed")
sys.exit(1 if fail else 0)
PY
