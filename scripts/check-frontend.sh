#!/usr/bin/env bash
set -euo pipefail

scripts/check-line-size.sh
cd web
npm ci
npm run test:coverage
npm run build
