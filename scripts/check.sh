#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
python3 -m unittest discover -s tests -p 'test_*.py'
cargo bench --bench responsiveness
if [[ "${SHEP_SKIP_E2E:-0}" != "1" ]]; then
  cargo build --profile test-ui --features test-support
  python3 scripts/e2e.py
  python3 scripts/performance_gate.py
fi
