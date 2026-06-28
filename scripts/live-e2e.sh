#!/usr/bin/env bash
set -euo pipefail

if [ -f .ai-cred ]; then
  set -a
  # shellcheck disable=SC1091
  . ./.ai-cred
  set +a
fi

: "${CONTROL_PANEL_KEY:?CONTROL_PANEL_KEY is required in the environment or .ai-cred}"
: "${AI_MEMMAIL_CONFIG:=config/live.local.yaml}"
: "${POSTGRES_PORT:=15432}"
: "${CONTROL_PANEL_PORT:=18080}"
: "${PLAYWRIGHT_BASE_URL:=http://127.0.0.1:${CONTROL_PANEL_PORT}}"
export CONTROL_PANEL_KEY AI_MEMMAIL_CONFIG POSTGRES_PORT CONTROL_PANEL_PORT PLAYWRIGHT_BASE_URL

if [ ! -f "$AI_MEMMAIL_CONFIG" ]; then
  echo "$AI_MEMMAIL_CONFIG does not exist. Create it from config/live.local.example.yaml." >&2
  exit 1
fi

docker compose up --build -d postgres web worker
trap 'docker compose logs --tail=120 web worker postgres; docker compose down' EXIT

cd web
npm ci
E2E_LIVE=1 npm run e2e
