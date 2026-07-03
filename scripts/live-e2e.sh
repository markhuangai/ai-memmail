#!/usr/bin/env bash
set -euo pipefail

: "${CONTROL_PANEL_KEY:=live-e2e-local}"
: "${AI_MEMMAIL_CONFIG:=config/config.yaml}"
: "${AI_MEMMAIL_HTTP_PORT:=18080}"
: "${PLAYWRIGHT_BASE_URL:=http://127.0.0.1:${AI_MEMMAIL_HTTP_PORT}}"
: "${AI_MEMMAIL_LIVE_E2E_RUN_ID:=live-$(date +%s)-$$}"
: "${AI_MEMMAIL_LIVE_E2E_DB_HOST:=127.0.0.1}"
: "${AI_MEMMAIL_LIVE_E2E_DB_PORT:=15432}"
export CONTROL_PANEL_KEY AI_MEMMAIL_CONFIG AI_MEMMAIL_HTTP_PORT PLAYWRIGHT_BASE_URL
export AI_MEMMAIL_LIVE_E2E_RUN_ID AI_MEMMAIL_LIVE_E2E_DB_HOST AI_MEMMAIL_LIVE_E2E_DB_PORT

if [ ! -f "$AI_MEMMAIL_CONFIG" ]; then
  echo "$AI_MEMMAIL_CONFIG does not exist. Create it from config/config.example.yaml." >&2
  exit 1
fi

cleanup() {
  docker compose logs --tail=120 app postgres || true
  docker compose stop app postgres || true
  docker compose rm -f app postgres || true
}
trap cleanup EXIT

docker compose up -d postgres

AI_MEMMAIL_LIVE_E2E=1 cargo test -p ai-memmail-server --test live_e2e -- --nocapture

docker compose up --build -d app

cd web
npm ci
E2E_LIVE=1 npm run e2e
