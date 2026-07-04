#!/usr/bin/env bash
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
if command -v llvm-cov-17 >/dev/null 2>&1 && command -v llvm-profdata-17 >/dev/null 2>&1; then
  export LLVM_COV="${LLVM_COV:-$(command -v llvm-cov-17)}"
  export LLVM_PROFDATA="${LLVM_PROFDATA:-$(command -v llvm-profdata-17)}"
fi

scripts/check-line-size.sh
cargo fmt --all --check
cargo test --workspace -- --skip storage_pg::tests::

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "cargo-llvm-cov is required for the 90% backend coverage gate." >&2
  echo "Install it, then rerun: scripts/check-backend.sh" >&2
  exit 1
fi

cargo llvm-cov \
  --workspace \
  --lib \
  --fail-under-lines 90 \
  --ignore-filename-regex 'rustc-.*library|src/main.rs|src/.*/tests.rs|src/.*/tests/.*|src/[^/]+/tests.rs|src/ai.rs|src/ai_external.rs|src/mail_external.rs|src/storage_pg.rs|src/storage_pg/.*|src/web.rs|src/web/.*|src/worker.rs|src/worker/.*' \
  -- --skip storage_pg::tests::
