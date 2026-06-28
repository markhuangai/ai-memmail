#!/usr/bin/env bash
set -euo pipefail

: "${CONTROL_PANEL_KEY:=live-e2e-local}"
: "${AI_MEMMAIL_CONFIG:=config/local.yaml}"
: "${PLAYWRIGHT_BASE_URL:=http://127.0.0.1:18080}"
export CONTROL_PANEL_KEY AI_MEMMAIL_CONFIG PLAYWRIGHT_BASE_URL

if [ ! -f "$AI_MEMMAIL_CONFIG" ]; then
  echo "$AI_MEMMAIL_CONFIG does not exist. Create it from config/config.example.yaml." >&2
  exit 1
fi

docker compose up --build -d postgres web worker
trap 'docker compose logs --tail=120 web worker postgres; docker compose down' EXIT

cd web
npm ci
E2E_LIVE=1 npm run e2e
