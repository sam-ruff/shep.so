#!/usr/bin/env bash
set -euo pipefail

shep_phase=${1:-}
shep_source=${2:-}
if [[ $# != 2 || ! "$shep_source" =~ ^[0-9a-f]{40}$ || ! "$shep_phase" =~ ^(backend|action)$ ]]; then
  echo 'Usage: bash scripts/ci-performance.sh backend|action SOURCE_SHA' >&2
  exit 2
fi
shep_root=$(cd "$(dirname "$0")/.." && pwd)
cd "$shep_root"

export SHEP_PERFORMANCE_SOURCE="$shep_source"
export SHEP_PERFORMANCE_MODE=required
if [[ "$shep_phase" == backend ]]; then
  if SHEP_BENCH_REPORT_DIR=artifacts/performance cargo bench --locked --bench responsiveness; then
    exit 0
  else
    shep_required_status=$?
  fi
  if mkdir -p artifacts/diagnostics/backend && \
    SHEP_PERFORMANCE_MODE=diagnostic SHEP_BENCH_REPORT_DIR=artifacts/diagnostics/backend \
    RUST_LOG=shep::query_timing=debug cargo bench --locked --features test-support --bench responsiveness \
    > artifacts/diagnostics/backend/run.log 2>&1; then
    shep_diagnostic_status=0
  else
    shep_diagnostic_status=$?
  fi
else
  if python3 scripts/action_latency.py --samples 20 --output artifacts/performance/actions.json; then
    exit 0
  else
    shep_required_status=$?
  fi
  if mkdir -p artifacts/diagnostics/action && \
    SHEP_PERFORMANCE_MODE=diagnostic SHEP_E2E_ARTIFACTS="$shep_root/artifacts/diagnostics/action/e2e" \
    RUST_LOG=shep::review_timing=debug,iced_tiny_skia::timing=debug \
    python3 scripts/action_latency.py --samples 20 --output artifacts/diagnostics/action/actions.json \
    > artifacts/diagnostics/action/run.log 2>&1; then
    shep_diagnostic_status=0
  else
    shep_diagnostic_status=$?
  fi
fi
echo "Required $shep_phase timing failed with status $shep_required_status; diagnostic replay exited $shep_diagnostic_status." >&2
if [[ "$shep_phase" == action ]]; then
  echo 'Diagnostic action reports are ineligible for required-gate validation, regardless of their timings.' >&2
fi
exit "$shep_required_status"
