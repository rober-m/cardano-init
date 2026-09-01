#!/bin/sh
# Publish or clear Dingo's standard connection keys in the project .env.
set -eu

project_root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
env_path="$project_root/.env"
mode=${1:-set}
port=${DINGO_DEVNET_BLOCKFROST_PORT:-3000}

case "$port" in
  ''|*[!0-9]*)
    echo "DINGO_DEVNET_BLOCKFROST_PORT must be a numeric TCP port." >&2
    exit 1
    ;;
esac
if [ "$port" -lt 1 ] || [ "$port" -gt 65535 ]; then
  echo "DINGO_DEVNET_BLOCKFROST_PORT must be between 1 and 65535." >&2
  exit 1
fi

case "$mode" in
  set)
    indexer_url="http://localhost:$port/api/v0/"
    indexer_port=$port
    ;;
  clear)
    indexer_url=
    indexer_port=
    ;;
  *)
    echo "usage: sh scripts/set-env.sh [set|clear]" >&2
    exit 2
    ;;
esac

umask 022
tmp_dir=$(mktemp -d "$project_root/.cardano-init-env.XXXXXX")
tmp_path="$tmp_dir/env"
cleanup() {
  rm -f "$tmp_path"
  rmdir "$tmp_dir" 2>/dev/null || true
}
trap cleanup 0 1 2 15

input=/dev/null
if [ -f "$env_path" ]; then
  input=$env_path
fi

awk -v indexer_url="$indexer_url" -v indexer_port="$indexer_port" '
  BEGIN { seen_url = 0; seen_port = 0 }
  /^INDEXER_URL=/ {
    if (!seen_url) print "INDEXER_URL=" indexer_url
    seen_url = 1
    next
  }
  /^INDEXER_PORT=/ {
    if (!seen_port) print "INDEXER_PORT=" indexer_port
    seen_port = 1
    next
  }
  { print }
  END {
    if (!seen_url) print "INDEXER_URL=" indexer_url
    if (!seen_port) print "INDEXER_PORT=" indexer_port
  }
' "$input" > "$tmp_path"

mv "$tmp_path" "$env_path"
if [ "$mode" = set ]; then
  echo "Dingo Blockfrost API: $indexer_url"
else
  echo "Cleared Dingo connection details from $env_path"
fi
