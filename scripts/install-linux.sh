#!/usr/bin/env bash
# Build an optimized production binary, then install for the current Linux user.
set -euo pipefail
shep_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "$(uname -s)" != Linux ]]; then
  echo 'This installer supports Linux only.' >&2
  exit 1
fi
shep_build=1
for shep_arg in "$@"; do
  case "$shep_arg" in --binary|--uninstall|--help|-h) shep_build=0;; esac
done
if [[ "$shep_build" == 1 ]]; then
  if [[ -f "$shep_root/Cargo.toml" ]]; then
    cargo build --manifest-path "$shep_root/Cargo.toml" --release --locked --no-default-features
    set -- --binary "$shep_root/target/release/shep" "$@"
  elif [[ -x "$shep_root/shep" ]]; then
    set -- --binary "$shep_root/shep" "$@"
  else
    echo 'Run this script from the checkout or an extracted release archive.' >&2
    exit 1
  fi
fi
exec python3 "$shep_root/scripts/install_linux.py" "$@"
