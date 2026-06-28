#!/usr/bin/env bash
set -euo pipefail

cd web
npm ci
npm run test:coverage
npm run build
