#!/usr/bin/env bash
set -euo pipefail

shep_root=$(cd "$(dirname "$0")/.." && pwd)
cd "$shep_root"
shep_version=${1:-}
shep_source=${2:-}
if [[ $# -gt 2 || ! "$shep_source" =~ ^[0-9a-f]{40}$ ]]; then
  echo 'Usage: bash scripts/ci-desktop-linux.sh VERSION_OR_EMPTY SOURCE_SHA' >&2
  exit 2
fi
if [[ -n "$shep_version" && ! "$shep_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo 'Expected a semantic release version without a v prefix.' >&2
  exit 2
fi
if [[ $(id -u) == 0 ]]; then
  echo 'Desktop CI requires a non-root runner for the browser sandbox.' >&2
  exit 2
fi

if [[ ${SHEP_DESKTOP_CONTAINER:-0} != 1 ]]; then
  [[ $(uname -m) == x86_64 ]] || { echo 'Desktop Linux releases require an x86_64 host.' >&2; exit 2; }
  shep_uid=$(id -u)
  shep_gid=$(id -g)
  shep_image="shep-desktop-linux:$(sha256sum .github/desktop-linux.Dockerfile | cut -c1-16)-$shep_uid-$shep_gid"
  # The build has no checkout context, credentials, profile or Docker socket mount.
  docker build --build-arg "SHEP_UID=$shep_uid" --build-arg "SHEP_GID=$shep_gid" \
    --tag "$shep_image" - < .github/desktop-linux.Dockerfile
  shep_environment=()
  for shep_name in SHEP_GOOGLE_CLIENT_ID SHEP_GOOGLE_CLIENT_SECRET; do
    if [[ -n ${!shep_name:-} ]]; then shep_environment+=(--env "$shep_name"); fi
  done
  exec docker run --rm --init --user "$shep_uid:$shep_gid" --shm-size=2g \
    --security-opt "seccomp=$shep_root/.github/desktop-linux-seccomp.json" \
    --security-opt no-new-privileges \
    --mount "type=bind,source=$shep_root,target=/workspace" \
    --env SHEP_DESKTOP_CONTAINER=1 \
    "${shep_environment[@]}" "$shep_image" \
    bash scripts/ci-desktop-linux.sh "$shep_version" "$shep_source"
fi

test -f /etc/shep-desktop-ci
export CARGO_BUILD_JOBS=4
unset CARGO_TARGET_DIR SHEP_SKIP_E2E
mkdir -p artifacts/ci/cargo artifacts/logs artifacts/performance
test "$(git rev-parse HEAD)" = "$shep_source"
rustc --version
node --version
google-chrome --version
gnome-shell --version
ldd --version
# Fail before the long build if the runner disallows Chromium's namespace sandbox.
unshare --user --map-root-user true
node scripts/ci-browser-smoke.mjs

if [[ -n "$shep_version" ]]; then
  python3 scripts/release.py "$shep_version" --stamp-only --source "$shep_source"
fi
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo clippy --locked --no-default-features -- -D warnings
cargo test --locked --all-features
cargo test --locked -p shep-html-pixbuf
python3 scripts/test_profile_core.py
python3 -m unittest discover -s tests -p 'test_*.py'
cargo build --locked --release --no-default-features -p shep --bin shep
cargo build --locked --profile test-ui --features test-support
cargo bench --locked --bench responsiveness
python3 scripts/e2e.py
python3 scripts/html_latency.py --samples 20 --output artifacts/performance/html.json
python3 scripts/performance_gate.py
if [[ -n "$shep_version" ]]; then
  python3 scripts/release.py "$shep_version" --no-stamp \
    --target x86_64-unknown-linux-gnu --source "$shep_source"
fi
