#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: $0 <test|clippy> <default|tokio-rich|all-features>" >&2
}

if [[ $# -ne 2 ]]; then
  usage
  exit 2
fi

action=$1
profile=$2
case "$action" in
  test|clippy) ;;
  *) usage; exit 2 ;;
esac

cargo_args=(--workspace)
case "$profile" in
  default) ;;
  tokio-rich)
    tokio_features="ws,sse,proxy-protocol,udp,per-thread,tls,http2,http3,webtransport,plugins,signals,multipart,simd,protobuf,ip-filter,hmac-signature,json-schema,zstd,validator,garde,typed-header,zero-copy-extractors,async-graphql,grpc,utoipa,vespera,metrics-prometheus,metrics-opentelemetry,tako-tracing,jwt-simple,file-stream,jemalloc,ahash,graphiql,queue-cron,socket-activation,vsock,utoipa-yaml"
    cargo_args+=(--features "$tokio_features")
    ;;
  all-features) cargo_args+=(--all-features) ;;
  *) usage; exit 2 ;;
esac

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"
case "$action" in
  test) exec cargo test "${cargo_args[@]}" ;;
  clippy) exec cargo clippy "${cargo_args[@]}" --no-deps --tests --examples -- -D warnings ;;
esac
