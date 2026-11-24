#!/bin/sh
# End to end in a private Xvfb with saved answers instead of the network:
# the page in three window sizes (three, two and one column), the keys,
# Listen (a fake NPLAY socket and a fake NIRUAMP), Enter opening a story,
# and a notice sent once for an orange SMHI warning but not for a VMA test.
# Needs Xvfb, xdotool, ffmpeg (screenshots, optional) and python3.
# Usage: tests/xvfb.sh PATH/TO/nirutoday [SHOTS-DIR]
set -eu
BIN=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
HERE=$(cd "$(dirname "$0")" && pwd)
SHOTS=${2:-}
SB=$(mktemp -d)
PIDS=""
cleanup() {
  for p in $PIDS; do kill "$p" 2>/dev/null || true; done
  if [ -n "${KEEP:-}" ]; then echo "sandbox $SB"; else rm -rf "$SB"; fi
}
trap cleanup EXIT INT TERM
export HOME=$SB/home XDG_CONFIG_HOME=$SB/home/.config XDG_CACHE_HOME=$SB/home/.cache XDG_RUNTIME_DIR=$SB/run
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$SB/bin" "$SB/fx"
chmod 700 "$XDG_RUNTIME_DIR"
# Saved answers under the names the program asks for today.
F=$HERE/fixtures
for f in smhi-forecast.json smhi-warnings.json vma-14.json krisinformation.json sr-125.xml sr-83.xml sr-latest-5380.json traffic.json; do
  cp "$F/$f" "$SB/fx/$f"
done
cp "$F/day.json" "$SB/fx/day-$(date +%Y-%m-%d).json"
cp "$F/prices.json" "$SB/fx/prices-$(date +%Y-%m-%d)-SE3.json"
cp "$F/prices.json" "$SB/fx/prices-$(date -d tomorrow +%Y-%m-%d)-SE3.json"
export NIRUTODAY_FIXTURES=$SB/fx
# Fakes: the browser, notices, NIRUAMP (a process named niruamp) and NPLAY.
printf '#!/bin/sh\necho "$1" >> %s/opened\n' "$SB" > "$SB/bin/xdg-open"
printf '#!/bin/sh\necho "$*" >> %s/notices\n' "$SB" > "$SB/bin/notify-send"
printf '#!/bin/sh\nsleep 60\n' > "$SB/bin/niruamp"
chmod +x "$SB/bin/"*
export PATH="$SB/bin:$PATH"
export NPLAY_SERVICE_SOCKET=$SB/run/control.sock
python3 - "$NPLAY_SERVICE_SOCKET" "$SB/nplay.log" <<'EOF' &
import json, socket, sys, os
path, log = sys.argv[1], sys.argv[2]
s = socket.socket(socket.AF_UNIX); s.bind(path); s.listen(4)
while True:
    c, _ = s.accept(); f = c.makefile('rwb')
    for line in f:
        req = json.loads(line)
        open(log, 'a').write(json.dumps(req) + '\n')
        f.write((json.dumps({"jsonrpc": "2.0", "id": req["id"], "result": {"ok": True}}) + '\n').encode()); f.flush()
EOF
PIDS="$PIDS $!"

D_NUM=$((40 + $$ % 20))
Xvfb ":$D_NUM" -screen 0 1920x1080x24 -nolisten tcp >/dev/null 2>&1 &
PIDS="$PIDS $!"
export DISPLAY=:$D_NUM
sleep 1
fail() { echo "FAIL: $*" >&2; [ -f "$SB/out.log" ] && tail -20 "$SB/out.log" >&2; exit 1; }
shot() { [ -n "$SHOTS" ] && ffmpeg -loglevel error -y -f x11grab -video_size 1920x1080 -i "$DISPLAY" -frames:v 1 "$SHOTS/$1.png" || true; }
alive() { kill -0 "$APP" 2>/dev/null || fail "nirutoday died ($*)"; }
key() { xdotool key --window "$W" "$@"; sleep 0.3; }
wait_for() { n=$(( $1 * 10 )); shift; while [ $n -gt 0 ]; do "$@" && return 0; sleep 0.1; n=$((n - 1)); done; return 1; }

start() {
  "$BIN" >"$SB/out.log" 2>&1 &
  APP=$!
  PIDS="$PIDS $APP"
  W=$(xdotool search --sync --onlyvisible --class nirutoday | head -1)
  xdotool windowfocus "$W" 2>/dev/null || true
}

start
[ -f "$XDG_CONFIG_HOME/nirutoday/config.toml" ] || fail "the first start wrote no config.toml"
for size in 1920x1080:wide 1280x800:laptop 640x900:narrow; do
  geo=${size%%:*}; name=${size#*:}
  xdotool windowmove "$W" 0 0 windowsize "$W" "${geo%x*}" "${geo#*x}"
  sleep 1.2
  alive "$name"
  shot "$name"
done
rss=$(awk '/VmRSS/ {print $2}' "/proc/$APP/status")
[ "$rss" -lt 80000 ] || fail "memory $rss kB"
echo "page drawn in three sizes, ${rss} kB"
xdotool windowsize "$W" 1600 950; sleep 0.8

# Keys: tabs, stories, help, tomorrow's prices, zoom, scrolling.
for k in Tab Tab shift+Tab Down Down Up question Escape t t plus plus 0 minus Next Prior Home; do key "$k"; done
alive keys
echo "keys ok"

# A notice for the orange warning, none for the VMA test.
wait_for 5 test -s "$SB/notices" || fail "no notice for the orange warning"
grep -q "Snö" "$SB/notices" || fail "the notice is not about the warning: $(cat "$SB/notices")"
grep -q "VMA" "$SB/notices" && fail "a VMA test raised a notice"
echo "notices ok"

# Listen: P4 Väst live goes to NPLAY (the LOCAL tab plays the first source).
key l
wait_for 8 grep -q play_external "$SB/nplay.log" || fail "Listen sent nothing to NPLAY"
grep play_external "$SB/nplay.log" | grep -q 'srapi/219.mp3' || fail "not the P4 Väst stream: $(cat "$SB/nplay.log")"
echo "listen ok"

# Ekot (its own tab) has no live channel: its newest broadcast plays.
key Tab; key Tab
key l
wait_for 8 sh -c "grep -c play_external '$SB/nplay.log' | grep -q 2" || fail "Ekot's newest broadcast was not played"
grep play_external "$SB/nplay.log" | tail -1 | grep -q 'ljudfil/test.mp3' || fail "not the newest broadcast"
echo "latest broadcast ok"

# Enter opens the selected story.
key Return
wait_for 5 test -s "$SB/opened" || fail "Enter opened nothing"
grep -q '^https://www.sverigesradio.se/artikel/' "$SB/opened" || fail "opened $(cat "$SB/opened")"
echo "open ok"
shot final

# Once only: a restart sends no second notice.
kill "$APP"; wait "$APP" 2>/dev/null || true
before=$(wc -l < "$SB/notices")
start; sleep 2; alive restart
[ "$(wc -l < "$SB/notices")" = "$before" ] || fail "the warning was notified again after a restart"
echo "notice once ok"

# --print reads the same data.
"$BIN" --print > "$SB/print.txt" 2>/dev/null || fail "--print failed"
grep -q "Name day Gudrun och Rune" "$SB/print.txt" || fail "--print: $(cat "$SB/print.txt")"
grep -q "Ljungskile" "$SB/print.txt" || fail "--print has no place"
echo "print ok"
echo PASS
