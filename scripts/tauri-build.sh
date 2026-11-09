#!/usr/bin/env bash
# Tauri downloads external bundler tools (linuxdeploy/NSIS) without retrying
# HTTP failures. Retry only transient bundler gateway/server errors; compilation
# and all other packaging failures still fail immediately. Cargo reuses its build.
set -uo pipefail
log=$(mktemp)
trap 'rm -f "$log"' EXIT
for attempt in 1 2 3; do
  pnpm tauri build "$@" 2>&1 | tee "$log"
  status=${PIPESTATUS[0]}
  if [ "$status" -eq 0 ]; then exit 0; fi
  if [ "$attempt" -eq 3 ] || ! grep -Eq 'failed to bundle project:.*http status: (500|502|503|504)' "$log"; then
    exit "$status"
  fi
  echo "Transient bundler download failure; retrying ($attempt/3)" >&2
  sleep "$((attempt * 10))"
done
