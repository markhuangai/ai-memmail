#!/usr/bin/env bash
set -euo pipefail

: "${CONTROL_PANEL_KEY:=live-e2e-local}"
: "${AI_MEMMAIL_CONFIG:=config/config.yaml}"
: "${PLAYWRIGHT_BASE_URL:=http://127.0.0.1:18080}"
export CONTROL_PANEL_KEY AI_MEMMAIL_CONFIG PLAYWRIGHT_BASE_URL

if [ ! -f "$AI_MEMMAIL_CONFIG" ]; then
  echo "$AI_MEMMAIL_CONFIG does not exist. Create it from config/config.example.yaml." >&2
  exit 1
fi

AI_MEMMAIL_ROLE=web docker compose up --build -d postgres app
trap 'docker compose logs --tail=120 app postgres; docker compose stop app postgres; docker compose rm -f app postgres' EXIT

AI_MEMMAIL_LIVE_E2E=1 cargo test -p ai-memmail-server --test live_e2e -- --nocapture

cd web
npm ci
E2E_LIVE=1 npm run e2e
