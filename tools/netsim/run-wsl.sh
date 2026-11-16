#!/usr/bin/env bash
# Run the network-namespace lab (plan 6.3) inside WSL Ubuntu (or any Linux box) as root:
# the uc-net rows (`crates/uc-net/tests/netns.rs`) and the file sync rows with two real agents
# (`crates/uc-cli/tests/netns_files.rs`: 200 MB over lossy Wi-Fi, sleep and wake mid-transfer).
#
#   wsl -d Ubuntu -u root -- bash tools/netsim/run-wsl.sh [filter] [log]
#
# Needs: rustup toolchain 1.99.0, iproute2, nftables, wireguard-tools, iputils (tc/netem), procps.
# The Linux target dir stays inside Linux (/root/uc-target), never in the Windows tree.
# UC_NETNS_SLEEP_SECS sets the sleep of the sleep-and-wake rows (row 9 default 60, the plan
# says 300; row 12 default 40, longer than the 24 s QUIC idle timeout).
set -euo pipefail
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:/usr/sbin:/usr/bin:/sbin:/bin"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/root/uc-target}"
filter="${1:-row}"
log="${2:-/root/netns.log}"
cargo build -q -p uc-net --examples --tests
cargo build -q -p uc-agent -p uc-cli --bins --tests
{
  cargo test -p uc-net --test netns -- --ignored --test-threads=1 --nocapture "$filter"
  cargo test -p uc-cli --test netns_files -- --ignored --test-threads=1 --nocapture "$filter"
} >"$log" 2>&1 || true
grep -E "^test |test result|after|took|MiB|sleeps|asked for|panicked" "$log"
