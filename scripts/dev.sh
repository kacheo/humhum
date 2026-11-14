#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Match the release workflow's ignored local configuration. Never print secrets.
if [[ -z "${CI:-}" && -f .env.local ]]; then
  set -a
  . ./.env.local
  set +a
fi
exec python3 scripts/dev.py "$@"
