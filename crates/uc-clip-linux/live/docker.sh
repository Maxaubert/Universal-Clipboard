#!/bin/sh
# From the repo root on any machine with Docker: build the image, then run clippy and the
# live tests for uc-clip-linux in it. Nothing reaches the host's clipboard or screen.
set -eu
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
docker build -t uclip-linux-live "$ROOT/crates/uc-clip-linux/live"
MSYS_NO_PATHCONV=1 docker run --rm \
  -v "$ROOT":/src -w /src \
  -v uclip-linux-target:/target -v uclip-linux-cargo:/root/.cargo/registry \
  -e CARGO_TARGET_DIR=/target -e UCLIP_LIVE_REQUIRE_EXT=1 \
  uclip-linux-live sh -c 'cargo clippy --locked -p uc-clip-linux --all-targets -- -D warnings && sh crates/uc-clip-linux/live/run.sh'
