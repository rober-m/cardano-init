#!/bin/sh
# Run Compose with a stable project-scoped name from any working directory.
set -eu

devnet_dir=$(CDPATH='' cd "$(dirname "$0")/.." && pwd)
project_root=$(CDPATH='' cd "$devnet_dir/.." && pwd)

if [ -z "${COMPOSE_PROJECT_NAME:-}" ]; then
  project_name=$(basename "$project_root")
  project_name=$(printf '%s' "$project_name" |
    tr '[:upper:]' '[:lower:]' |
    sed -E -e 's/[^a-z0-9_-]/-/g' -e 's/^[^a-z0-9]+//')
  if [ -z "$project_name" ]; then
    project_name=cardano-init
  fi
  COMPOSE_PROJECT_NAME="$project_name-dingo-devnet"
  export COMPOSE_PROJECT_NAME
fi

cd "$devnet_dir"
exec docker compose "$@"
