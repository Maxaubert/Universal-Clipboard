#!/bin/sh
# Spike S5, Simulator part: a test-profile agent on this Mac (fake clipboard, loopback) pairs
# with the spike app in a headless Simulator, and a copy moves both ways.
# Nothing opens on screen: Simulator.app is never started, and loopback needs no LAN prompt.
set -eu
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:$PATH"
ROOT=$(cd ../.. && pwd); T="${CARGO_TARGET_DIR:-$ROOT/target}"
DEV="${SIM:-iPhone 18 Pro}"; OUT="${OUT:-results}"
rm -rf run && mkdir -p run/m run/s "$OUT"
RUN=$(pwd)/run
(cd "$ROOT" && cargo build -q -p uc-agent -p uc-cli)
export UCLIP_PROFILE=s5mac UCLIP_STATE_DIR="$RUN/m" UCLIP_TEST_KEYSTORE=file \
  UCLIP_TEST_LOOPBACK=127.0.0.1 UCLIP_TEST_FAST=1 UCLIP_NO_START=1 UCLIP_LOG=debug
"$T/debug/uc-agent" </dev/null >/dev/null 2>&1 &
AG=$!
trap 'kill $AG 2>/dev/null || true; xcrun simctl shutdown "$UDID" 2>/dev/null || true' EXIT
sleep 2
PORT=$("$T/debug/uc" status --json | python3 -c 'import json,sys;print(json.load(sys.stdin)["port"])')
"$T/debug/uc" pair show --json > run/pair.json 2>&1 &
sleep 2
CODE=$(head -1 run/pair.json | python3 -c 'import json,sys;print(json.load(sys.stdin)["code"].replace(" ",""))')
echo "mac agent port $PORT, code $CODE"
UDID=$(xcrun simctl list devices available -j | python3 -c "
import json,sys
d=json.load(sys.stdin)['devices']
print(next(x['udid'] for v in d.values() for x in v if x['name']=='$DEV'))")
xcrun simctl boot "$UDID" 2>/dev/null || true
xcrun simctl bootstatus "$UDID" -b >/dev/null
APP=ios/build/dd/Build/Products/Debug-iphonesimulator/S5App.app
xcrun simctl install "$UDID" "$APP"
xcrun simctl launch --terminate-running-process "$UDID" com.universalclipboard.s5 \
  -dir "$RUN/s" -code "$CODE" -pairAddrs "127.0.0.1:$PORT" -loopback 127.0.0.1 -fileKeys "${FILEKEYS:-0}" >/dev/null
SOCK="$RUN/m/agent.sock"
clip() { python3 ipc.py "$SOCK" debug.clipboard | python3 -c 'import json,sys;print(json.load(sys.stdin).get("text") or "")'; }
# Wait for the Simulator's copy on the Mac, then copy on the Mac.
GOT=""
for i in $(seq 1 600); do
  GOT=$(clip); case "$GOT" in from-sim*) break;; esac; sleep 0.1
done
echo "mac received: $GOT"
python3 ipc.py "$SOCK" debug.copy "{\"text\":\"from-mac-$(date +%s)\"}" >/dev/null
python3 ipc.py "$SOCK" devices.list > "$OUT/mac-devices.json"
C=$(xcrun simctl get_app_container "$UDID" com.universalclipboard.s5 data)
for i in $(seq 1 600); do
  [ -f "$C/Documents/s5.json" ] && break; sleep 0.1
done
cp "$C/Documents/s5.json" "$OUT/sim.json"
echo "$GOT" > "$OUT/mac-received.txt"
cp run/m/agent.log "$OUT/mac-agent.log"; cp run/s/agent.log "$OUT/sim-agent.log" 2>/dev/null || true
cat "$OUT/sim.json"
