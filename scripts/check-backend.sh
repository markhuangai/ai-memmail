#!/usr/bin/env bash
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
if [[ -z "${LLVM_COV:-}" || -z "${LLVM_PROFDATA:-}" ]]; then
  rust_host="$(rustc -vV | sed -n 's/^host: //p')"
  rust_llvm_major="$(rustc -vV | sed -n 's/^LLVM version: \([0-9][0-9]*\).*/\1/p')"
  rust_tool_dir="$(rustc --print sysroot)/lib/rustlib/${rust_host}/bin"
  if [[ -x "${rust_tool_dir}/llvm-cov" && -x "${rust_tool_dir}/llvm-profdata" ]]; then
    export LLVM_COV="${LLVM_COV:-${rust_tool_dir}/llvm-cov}"
    export LLVM_PROFDATA="${LLVM_PROFDATA:-${rust_tool_dir}/llvm-profdata}"
  elif [[ -n "${rust_llvm_major}" ]] \
    && command -v "llvm-cov-${rust_llvm_major}" >/dev/null 2>&1 \
    && command -v "llvm-profdata-${rust_llvm_major}" >/dev/null 2>&1; then
    export LLVM_COV="${LLVM_COV:-$(command -v "llvm-cov-${rust_llvm_major}")}"
    export LLVM_PROFDATA="${LLVM_PROFDATA:-$(command -v "llvm-profdata-${rust_llvm_major}")}"
  fi
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
