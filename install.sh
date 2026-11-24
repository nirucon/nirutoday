#!/usr/bin/env bash
# NIRUTODAY installer: the program, a menu entry and its icon. No root, no
# system files. Settings are written by the program at its first start.
#
#   ./install.sh           the prebuilt release binary, checked with SHA-256
#   ./install.sh --build   build this checkout with cargo
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"
BIN="$HOME/.local/bin"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
APPS="$DATA/applications"
ICONS="$DATA/icons/hicolor/scalable/apps"

MODE=auto
for arg in "$@"; do
  case "$arg" in
    --build) MODE=build ;;
    --prebuilt) MODE=prebuilt ;;
    -y|--yes) ;;
    -h|--help) echo "usage: ./install.sh [--build|--prebuilt]"; exit 0 ;;
    *) echo "unknown option: $arg (try --help)" >&2; exit 2 ;;
  esac
done
[[ $(id -u) != 0 ]] || { echo "Run ./install.sh as your own user, not root." >&2; exit 1; }
[[ $(uname -m) == x86_64 ]] || { echo "The release binary is for x86_64 Linux; use --build." >&2; [[ $MODE == build ]] || exit 1; }

VERSION="${NIRUTODAY_VERSION:-}"
[[ -z $VERSION && -f Cargo.toml ]] && VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[[ -n $VERSION ]] || { echo "Cannot tell the version (set NIRUTODAY_VERSION)." >&2; exit 1; }
printf '\033[1mNIRUTODAY %s\033[0m: today at a glance for NIRUX\n' "$VERSION"
command -v curl >/dev/null || echo "Note: NIRUTODAY fetches with curl; install it."

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
fetch() { # URL FILE
  if command -v curl >/dev/null; then
    curl -fsSL --retry 3 --connect-timeout 20 -o "$2" "$1"
  elif command -v wget >/dev/null; then
    wget -qO "$2" "$1"
  else
    echo "curl or wget is needed" >&2; return 1
  fi
}
verify256() {
  local file=$1 sums=$2 name expected actual
  name=$(basename "$file")
  expected=$(awk -v n="$name" '$2 == n || $2 == "*"n {print $1}' "$sums" | head -1)
  [[ -n $expected ]] || { echo "SHA256SUMS has no line for $name" >&2; exit 1; }
  actual=$(sha256sum "$file" | awk '{print $1}')
  [[ $actual == "$expected" ]] || { echo "Checksum mismatch for $name: not installed." >&2; exit 1; }
}

if [[ $MODE == build ]]; then
  command -v cargo >/dev/null || { echo "Rust/cargo is needed for --build" >&2; exit 1; }
  cargo build --release --locked
  SRC=target/release/nirutoday
elif [[ $MODE == prebuilt || ( $MODE == auto && -x ./nirutoday ) ]]; then
  SRC=./nirutoday
  [[ -f SHA256SUMS ]] && verify256 "$SRC" SHA256SUMS
else
  URL="${NIRUTODAY_RELEASE_URL:-https://github.com/nirucon/nirutoday/releases/download/v${VERSION}}"
  echo "Downloading NIRUTODAY…"
  fetch "$URL/nirutoday" "$TMP/nirutoday"
  fetch "$URL/SHA256SUMS" "$TMP/SHA256SUMS"
  verify256 "$TMP/nirutoday" "$TMP/SHA256SUMS"
  SRC="$TMP/nirutoday"
fi

mkdir -p "$BIN" "$APPS" "$ICONS"
install -m 0755 "$SRC" "$BIN/.nirutoday.new.$$"
mv -f "$BIN/.nirutoday.new.$$" "$BIN/nirutoday"
install -m 0644 "$ROOT/assets/nirutoday.svg" "$ICONS/nirutoday.svg"
sed "s|^Exec=nirutoday|Exec=$BIN/nirutoday|" "$ROOT/assets/nirutoday.desktop" > "$APPS/nirutoday.desktop"
chmod 0644 "$APPS/nirutoday.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" >/dev/null 2>&1 || :
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$DATA/icons/hicolor" >/dev/null 2>&1 || :
"$BIN/nirutoday" --version
echo "Start NIRUTODAY from the launcher; settings: ~/.config/nirutoday/config.toml"
