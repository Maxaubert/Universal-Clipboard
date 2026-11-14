#!/bin/sh
# Start Xvfb and a headless Sway, then run the live clipboard tests against them.
# Run this only in Docker or in CI (never on a desktop session): see live/README.md.
set -eu

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
chmod 700 "$XDG_RUNTIME_DIR"
# Never reuse a desktop's sockets.
unset WAYLAND_DISPLAY DISPLAY

Xvfb :99 -screen 0 320x240x24 -nolisten tcp >/tmp/uclip-xvfb.log 2>&1 &
XVFB=$!
printf 'xwayland disable\n' >/tmp/uclip-sway.conf
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
  sway -c /tmp/uclip-sway.conf >/tmp/uclip-sway.log 2>&1 &
SWAY=$!
trap 'kill $SWAY $XVFB 2>/dev/null || true' EXIT

WL=""
for _ in $(seq 100); do
  WL=$(cd "$XDG_RUNTIME_DIR" && ls wayland-* 2>/dev/null | grep -v '\.lock$' | head -n 1 || true)
  if [ -S /tmp/.X11-unix/X99 ] && [ -n "$WL" ]; then break; fi
  sleep 0.1
done
if [ -z "$WL" ]; then
  echo "sway did not start:"; cat /tmp/uclip-sway.log; exit 1
fi
echo "Xvfb on :99, sway on $WL ($(sway --version))"

export DISPLAY=:99 WAYLAND_DISPLAY="$WL" UCLIP_LIVE_X11=:99 UCLIP_LIVE_WAYLAND="$WL"
cargo test --locked -p uc-clip-linux --test live -- --nocapture "$@"
