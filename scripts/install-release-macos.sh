#!/usr/bin/env bash
# macOS's built-in tools only; no Python, Homebrew, or developer runtime needed.
set -euo pipefail
[[ "$(uname -s)" == Darwin ]] || { echo 'This installer is for macOS.' >&2; exit 1; }
for shep_tool in curl tar shasum osascript plutil sips iconutil; do
  command -v "$shep_tool" >/dev/null || { echo "macOS tool $shep_tool is missing. Restore the standard macOS command-line tools, then retry." >&2; exit 1; }
done
shep_scope=user
shep_explicit_scope=''
shep_prompt=1
shep_version=''
shep_prefix=''
while [[ $# -gt 0 ]]; do
  case "$1" in
    --user) [[ "$shep_explicit_scope" != system ]] || { echo 'Choose either --user or --system.' >&2; exit 1; }; shep_scope=user; shep_explicit_scope=user; shep_prompt=0; shift;;
    --yes) shep_prompt=0; shift;;
    --system) [[ "$shep_explicit_scope" != user ]] || { echo 'Choose either --user or --system.' >&2; exit 1; }; shep_scope=system; shep_explicit_scope=system; shep_prompt=0; shift;;
    --version|--prefix)
      [[ $# -ge 2 ]] || { echo "$1 requires a value." >&2; exit 1; }
      if [[ "$1" == --version ]]; then shep_version="$2"; else shep_prefix="$2"; fi
      shift 2;;
    --help|-h) echo 'Install Shep: [--version VERSION] [--user|--system] [--yes] [--prefix APPLICATIONS_FOLDER]'; exit 0;;
    *) echo "Unknown option: $1" >&2; exit 1;;
  esac
done
if [[ "$shep_prompt" == 1 && -t 1 ]]; then
  printf 'Install for [u]ser (default), [a]ll users, or [c]ancel? [u/a/c]: ' > /dev/tty
  IFS= read -r shep_choice < /dev/tty
  case "$shep_choice" in ''|u|user) ;; a|all) shep_scope=system;; *) echo 'Installation cancelled; nothing was changed.'; exit 1;; esac
fi
if [[ "$shep_scope" == system && -n "$shep_prefix" ]]; then
  echo 'All-user installation cannot combine --prefix.' >&2; exit 1
fi
shep_endpoint=https://api.github.com/repos/sam-ruff/shep.so/releases/latest
if [[ -n "$shep_version" ]]; then
  shep_version="${shep_version#v}"
  [[ "$shep_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || { echo 'Use a version such as 1.2.3.' >&2; exit 1; }
  shep_endpoint="https://api.github.com/repos/sam-ruff/shep.so/releases/tags/v$shep_version"
fi
shep_stage="$(mktemp -d "${TMPDIR:-/tmp}/shep-macos.XXXXXXXX")"
trap 'rm -rf -- "$shep_stage"' EXIT
shep_download() { curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 --connect-timeout 15 --max-time 300 "$1" --output "$2"; }
if ! shep_download "$shep_endpoint" "$shep_stage/release.json"; then
  echo 'No release could be downloaded. Check https://github.com/sam-ruff/shep.so/releases and try again.' >&2; exit 1
fi
# JSON parsing uses the system JavaScript bridge. Output is validated single-line
# fields, never shell code. Do not eval release metadata.
osascript -l JavaScript - "$shep_stage/release.json" "$(uname -m)" > "$shep_stage/selection" <<'JXA'
ObjC.import('Foundation');
function run(args) {
    const raw = $.NSString.alloc.initWithContentsOfFileEncodingError(args[0], $.NSUTF8StringEncoding, null);
    if (!raw) throw new Error('Cannot read release metadata');
    const release = JSON.parse(ObjC.unwrap(raw));
    const match = /^v?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)$/.exec(release.tag_name || '');
    if (!match || release.draft || !Array.isArray(release.assets)) throw new Error('Invalid published release');
    const arch = args[1];
    const architectures = arch === 'arm64' ? ['arm64', 'aarch64'] : arch === 'x86_64' ? ['x86_64', 'amd64'] : [];
    const names = architectures.map(a => 'shep-' + match[1] + '-darwin-' + a + '.tar.gz');
    const assets = release.assets.filter(a => names.indexOf(a.name) >= 0);
    const sums = release.assets.filter(a => a.name === 'SHA256SUMS');
    if (assets.length !== 1 || sums.length !== 1) throw new Error('No unique macOS/' + arch + ' archive and checksum are published; nothing was installed');
    const urls = [assets[0].browser_download_url, sums[0].browser_download_url];
    urls.forEach(url => { if (typeof url !== 'string' || !/^https:\/\/github\.com\/sam-ruff\/shep\.so\/releases\/download\/[^\s]+$/.test(url)) throw new Error('Unexpected release URL'); });
    return [match[1], assets[0].name].concat(urls).join('\n');
}
JXA
{ IFS= read -r shep_version; IFS= read -r shep_name; IFS= read -r shep_archive_url; IFS= read -r shep_sum_url; } < "$shep_stage/selection"
printf 'Downloading Shep %s for macOS…\n' "$shep_version"
shep_download "$shep_archive_url" "$shep_stage/release.tar.gz"
shep_download "$shep_sum_url" "$shep_stage/SHA256SUMS"
shep_expected="$(awk -v file="$shep_name" '{sub(/^\*/, "", $2); if ($2 == file) print tolower($1)}' "$shep_stage/SHA256SUMS")"
[[ "$shep_expected" =~ ^[0-9a-f]{64}$ ]] || { echo 'Missing or duplicate archive checksum; nothing was installed.' >&2; exit 1; }
shep_actual="$(shasum -a 256 "$shep_stage/release.tar.gz")"
[[ "${shep_actual%% *}" == "$shep_expected" ]] || { echo 'Release checksum mismatch; nothing was installed.' >&2; exit 1; }
# Never extract arbitrary archive paths. Only these two exact, unique regular
# files may stream into our own chosen destinations; links are rejected first.
tar -tzf "$shep_stage/release.tar.gz" > "$shep_stage/members"
for shep_member in shep assets/launcher.png; do
  [[ "$(awk -v file="$shep_member" '$0 == file {n++} END {print n+0}' "$shep_stage/members")" == 1 ]] || { echo "Archive is missing a unique $shep_member." >&2; exit 1; }
  shep_listing="$(tar -tvzf "$shep_stage/release.tar.gz" -- "$shep_member")"
  [[ "$shep_listing" == -* && "$shep_listing" != *$'\n'* ]] || { echo 'Archive contains a non-regular install file.' >&2; exit 1; }
done
shep_app="$shep_stage/Shep.app"
mkdir -p "$shep_app/Contents/MacOS" "$shep_app/Contents/Resources" "$shep_stage/shep.iconset"
tar -xOzf "$shep_stage/release.tar.gz" -- shep > "$shep_app/Contents/MacOS/shep"
tar -xOzf "$shep_stage/release.tar.gz" -- assets/launcher.png > "$shep_stage/launcher.png"
[[ -s "$shep_app/Contents/MacOS/shep" && -s "$shep_stage/launcher.png" ]] || { echo 'Archive install files are empty.' >&2; exit 1; }
chmod 755 "$shep_app/Contents/MacOS/shep"
for shep_size in 16 32 128 256 512; do
  sips -z "$shep_size" "$shep_size" "$shep_stage/launcher.png" --out "$shep_stage/shep.iconset/icon_${shep_size}x${shep_size}.png" >/dev/null
  shep_retina=$((shep_size * 2))
  sips -z "$shep_retina" "$shep_retina" "$shep_stage/launcher.png" --out "$shep_stage/shep.iconset/icon_${shep_size}x${shep_size}@2x.png" >/dev/null
done
iconutil -c icns "$shep_stage/shep.iconset" -o "$shep_app/Contents/Resources/shep.icns"
shep_plist="$shep_app/Contents/Info.plist"
plutil -create xml1 "$shep_plist"
plutil -insert CFBundleIdentifier -string so.shep.Shep "$shep_plist"
plutil -insert CFBundleExecutable -string shep "$shep_plist"
plutil -insert CFBundleName -string Shep "$shep_plist"
plutil -insert CFBundlePackageType -string APPL "$shep_plist"
plutil -insert CFBundleIconFile -string shep.icns "$shep_plist"
plutil -insert CFBundleShortVersionString -string "${shep_version%%-*}" "$shep_plist"
plutil -insert CFBundleVersion -string "${shep_version%%-*}" "$shep_plist"
plutil -insert NSHighResolutionCapable -bool YES "$shep_plist"
if [[ "$shep_scope" == system ]]; then shep_destination=/Applications/Shep.app
else shep_destination="${shep_prefix:-$HOME/Applications}/Shep.app"; fi
# The final, staged copy is the only elevated action. Keep rollback beside the
# destination so a failed replacement cannot discard the old application.
cat > "$shep_stage/apply.sh" <<'APPLY'
#!/usr/bin/env bash
set -euo pipefail
shep_source="$1"; shep_destination="$2"
shep_parent="$(dirname -- "$shep_destination")"
mkdir -p "$shep_parent"
if [[ -e "$shep_destination" || -L "$shep_destination" ]]; then
  [[ ! -L "$shep_destination" && -f "$shep_destination/Contents/Info.plist" ]] || { echo 'The destination is not an installed Shep application.' >&2; exit 1; }
  [[ "$(plutil -extract CFBundleIdentifier raw -o - "$shep_destination/Contents/Info.plist")" == so.shep.Shep ]] || { echo 'Refusing to replace a different application.' >&2; exit 1; }
fi
shep_slot="$(mktemp -d "$shep_parent/.shep-install.XXXXXXXX")"
shep_old=0
shep_cleanup() {
  shep_status=$?
  if [[ "$shep_status" != 0 && "$shep_old" == 1 && ! -e "$shep_destination" ]]; then
    if ! mv "$shep_slot/Previous.app" "$shep_destination"; then
      echo "The previous app is preserved at $shep_slot/Previous.app; restore it before retrying." >&2
      return
    fi
  fi
  rm -rf -- "$shep_slot"
}
trap shep_cleanup EXIT
cp -R "$shep_source" "$shep_slot/New.app"
if [[ -e "$shep_destination" ]]; then mv "$shep_destination" "$shep_slot/Previous.app"; shep_old=1; fi
mv "$shep_slot/New.app" "$shep_destination"
shep_old=0
APPLY
if [[ "$shep_scope" == system && "$(id -u)" != 0 ]]; then
  command -v sudo >/dev/null || { echo 'Administrator access is unavailable. Retry with --user.' >&2; exit 1; }
  sudo -- bash "$shep_stage/apply.sh" "$shep_app" "$shep_destination"
else
  bash "$shep_stage/apply.sh" "$shep_app" "$shep_destination"
fi
printf 'Installed %s. Open Shep from Applications; reopen an existing window after updating.\n' "$shep_destination"
