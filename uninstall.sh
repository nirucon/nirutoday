#!/usr/bin/env bash
# Remove NIRUTODAY. With --purge its settings and cache go too.
set -euo pipefail
purge=0
for arg in "$@"; do
  case "$arg" in
    --purge) purge=1 ;;
    -h|--help) echo "usage: ./uninstall.sh [--purge]"; exit 0 ;;
    *) echo "unknown option: $arg (try --help)" >&2; exit 2 ;;
  esac
done
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}"
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}"
pkill -u "$(id -u)" -x nirutoday 2>/dev/null || :
rm -f "$HOME/.local/bin/nirutoday" "$DATA/applications/nirutoday.desktop" "$DATA/icons/hicolor/scalable/apps/nirutoday.svg"
if [[ $purge == 1 ]]; then
  rm -rf "$CONFIG/nirutoday" "$CACHE/nirutoday"
  echo "NIRUTODAY removed, with its settings and cache."
else
  rm -rf "$CACHE/nirutoday"
  echo "NIRUTODAY removed. Settings stay in $CONFIG/nirutoday (--purge removes them)."
fi
