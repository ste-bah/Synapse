#!/usr/bin/env bash
set -euo pipefail

if ! command -v jq >/dev/null 2>&1; then
  echo "check_dep_graph: jq is required" >&2
  exit 2
fi

metadata="$(mktemp)"
trap 'rm -f "$metadata"' EXIT

cargo metadata --format-version 1 >"$metadata"

core_id="$(jq -r '.packages[] | select(.name == "synapse-core") | .id' "$metadata")"
if [[ -z "$core_id" || "$core_id" == "null" ]]; then
  echo "check_dep_graph: synapse-core package missing from metadata" >&2
  exit 1
fi

bad_core_edges="$(
  jq -r --arg core "$core_id" '
    .resolve.nodes[]
    | select(.id == $core)
    | .deps[]?
    | select(.pkg | test("/crates/synapse-"))
    | .name
  ' "$metadata"
)"

if [[ -n "$bad_core_edges" ]]; then
  echo "check_dep_graph: synapse-core must not depend on internal crates:" >&2
  echo "$bad_core_edges" >&2
  exit 1
fi

cargo tree -p synapse-core --depth 1 --no-default-features |
  awk 'NR > 1 && /synapse-/ { print; bad = 1 } END { exit bad ? 1 : 0 }'

echo "check_dep_graph: ok"
