#!/usr/bin/env bash
# Standalone entry point: curl this file from GitHub raw and pipe it to bash.
set -euo pipefail
if [[ "$(uname -s)" != Linux ]]; then
  echo 'Use the installer for your operating system; this entry point is for Linux.' >&2
  exit 1
fi
for shep_tool in curl python3; do
  command -v "$shep_tool" >/dev/null || { echo "Install $shep_tool, then run this installer again." >&2; exit 1; }
done
shep_stage="$(mktemp -d "${TMPDIR:-/tmp}/shep-download.XXXXXXXX")"
trap 'rm -rf -- "$shep_stage"' EXIT
curl --fail --show-error --silent --location --proto '=https' --tlsv1.2 --max-time 90 \
  https://raw.githubusercontent.com/sam-ruff/shep.so/main/scripts/install_release.py \
  --output "$shep_stage/install_release.py"
python3 "$shep_stage/install_release.py" --platform linux "$@"
