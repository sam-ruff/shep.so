#!/usr/bin/env bash
# Build an optimised production binary, then install for the current Linux user.
set -euo pipefail
shep_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "$(uname -s)" != Linux ]]; then
  echo 'This installer supports Linux only.' >&2
  exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo 'This installer needs Python 3. Install python3, then run it again.' >&2
  exit 1
fi
exec python3 "$shep_root/scripts/install_linux.py" --build "$@"
