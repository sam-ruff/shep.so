#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
chmod +x .githooks/pre-commit .githooks/commit-msg
git config core.hooksPath .githooks
echo "Installed format, Clippy, unit-test and Conventional Commit hooks."
