#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/development/mmtk-target-linux}"
log_dir="${MMTK_TEST_LOG_DIR:-$HOME/development/mmtk-validation-logs}"
mkdir -p "$log_dir"
cargo fmt --all -- --check 2>&1 | tee "$log_dir/fmt.log"
cargo check --lib --features malloc_mimalloc 2>&1 | tee "$log_dir/check.log"
cargo clippy --lib --features malloc_mimalloc -- -D warnings 2>&1 | tee "$log_dir/clippy.log"
cargo test --features malloc_mimalloc -- --test-threads=1 2>&1 | tee "$log_dir/serial.log"
cargo test --features malloc_mimalloc 2>&1 | tee "$log_dir/parallel.log"
