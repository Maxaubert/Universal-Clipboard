#!/usr/bin/env bash
# Run the uc-net network-namespace lab (plan 6.3) inside WSL Ubuntu (or any Linux box) as root.
#
#   wsl -d Ubuntu -u root -- bash tools/netsim/run-wsl.sh [filter] [log]
#
# Needs: rustup toolchain 1.99.0, iproute2, nftables, wireguard-tools, iputils (tc/netem).
# The Linux target dir stays inside Linux (/root/uc-target), never in the Windows tree.
# UC_NETNS_SLEEP_SECS sets the sleep-and-wake row's sleep (default 60; the plan says 300).
set -euo pipefail
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:/usr/sbin:/usr/bin:/sbin:/bin"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/root/uc-target}"
filter="${1:-row}"
log="${2:-/root/netns.log}"
cargo build -q -p uc-net --examples --tests
cargo test -p uc-net --test netns -- --ignored --test-threads=1 --nocapture "$filter" >"$log" 2>&1 || true
grep -E "^test |test result|after|took" "$log"
