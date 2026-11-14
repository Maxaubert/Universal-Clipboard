# Spike S1: iroh and interfaces (partial result, PC and WSL only)

Date: 2026-11-14. Done during M2, because the lab VMs (`win-b`, `mac-b`, `lin-kde`) are not up
yet (PR #3 is open, and this session has no Hyper-V rights). Plan: sections 3.3, 3.5, 6.3, 7
(S1 row). Code: `crates/uc-net`. Choices: `docs/net.md`.

**Result so far: change.** The transport is plain QUIC (`quinn`), not iroh. Linux is proven in
network namespaces, including the overlapping route. Windows and macOS VPN behaviour, the
Windows firewall prompts and the consumer VPN defaults still need the VMs (list at the end).

## 1. iroh or plain QUIC: plain QUIC

Read in the iroh 1.4.1 source (crates.io, 2026-11-14):

- Relay, n0 DNS and pkarr lookup and the port mapper can be turned off.
- **QUIC NAT traversal (hole punching) cannot.** `QuicTransportConfigBuilder::new()` always sets
  `max_remote_nat_traversal_addresses`, and `max_remote_nat_traversal_addresses(n)` ignores any
  `n` below 8 with a warning. Every connection sends its local candidate addresses to the peer
  and the client probes the peer's candidates.
- **QUIC multipath is always on**: a peer can add paths from new addresses after the handshake,
  so the "close non-local peers before the TLS handshake finishes" rule (plan 4.1) cannot be
  held by a check on the first packet.
- Pre-bound or interface-tied sockets need `unstable-custom-transports`.

Plain `quinn` 0.11.12 + `rustls` 0.23.46 with raw Ed25519 keys (RFC 7250) has none of this:
no candidates, no migration (turned off on the server), and we make every socket. So plan 3.5
options (a), (b) and (c) collapse into one: **our own sockets, bound to the interface address
and tied to the interface**, with a stable API. iroh is not in `Cargo.lock` at all, and a test
keeps it out.

## 2. Physical interface detection: done on 4 real machines or setups

`cargo run -p uc-net --example ifaces` on each, recorded as test fixtures:

| Machine | Physical (up, with address) | Excluded | Note |
|---|---|---|---|
| PC, Windows 11 | Realtek 5GbE | 2 Hyper-V vEthernet (WSL, Default Switch), 2 OpenVPN DCO (Surfshark), Bluetooth PAN, loopback | 3 MediaTek Wi-Fi 7 adapters are physical but down |
| Mac mini, macOS 27.2 | en0 (Ethernet), en1 (Wi-Fi) | bridge0, awdl0, llw0, anpi0..3, ap1, nan0, gif0, stf0, 8 × utun | en2, en3, en7 (Thunderbolt) are physical but have no address |
| WSL Ubuntu 26.04 | eth0 (hv_netvsc) | lo | |
| WSL netns | (veth, forced in tests) | wg0 (WireGuard), bridge, dummy | |

**Finding (fixed):** macOS has 4 to 8 `utun` system tunnels up at all times, each with only an
`fe80::` address (iCloud Private Relay, Continuity and similar). Counting them as "a VPN is up"
would show the VPN help on every Mac. The rule now needs a non-link-local address on the VPN
interface. A down Windows VPN adapter also has a `169.254` address; it does not count either.

## 3. mDNS: `mdns-sd`

`mdns-sd` 0.21.7: one daemon, every interface disabled (loopback too), then each physical
interface enabled by name. iroh's mDNS crate is moot without iroh. Measured in WSL netns: three
fresh nodes find each other and connect about 1.3 s after start with the RFC 6762 name probe;
the probe is now skipped (the names are per-device HMAC tags, unique by construction).
Measured after that change: see section 4, row 1.

## 4. Network rows (plan 6.3), WSL network namespaces, kernel 6.18

All 10 rows pass (`tools/netsim/run-wsl.sh`, `crates/uc-net/tests/netns.rs`). Each test builds
a bridge and one namespace per device; the device runs `examples/lan-node.rs`: real `Net`,
`OsBinder` (`SO_BINDTODEVICE`), real `mdns-sd`, a sync engine with a fake clipboard.

| Row | Setup | Result |
|---|---|---|
| Same LAN | 3 nodes, mDNS only | Full mesh, direct on `lan0`. Times in the table below. |
| Multicast blocked | nft drops 224.0.0.0/4 and ff02::/16 udp 5353 | Found by remembered address. A never-seen node: `NoPath(NotFound)` on all sides. |
| VPN, LAN allowed | WireGuard `wg0` with the default route | `vpn=true` seen; direct path on `lan0` to `10.77.0.2`, not through the tunnel. |
| **VPN, overlapping route** | two /25 routes into `wg0` cover the /24 LAN; `ip route get` shows `dev wg0` | **Direct LAN path.** `SO_BINDTODEVICE(lan0)` wins over the route table. Socket shows `Tie::Device`. |
| VPN, kill switch | nft drops all but `lo` and `wg0` after the link was up | `NoPath(VpnBlocksLan)` on the VPN side **and** on the other side (from the peer's signed VPN flag). |
| Client isolation | bridge port isolation after the link was up | `NoPath(Unreachable)` both sides (guest-network help), no VPN false alarm. |
| Both inbound-blocked | nft drops udp 4500 inbound on both | No path, no crash. Heals when one side opens and the other only allows replies. |
| Address change | node 2 moves from .2 to .12 | Both peers reconnect to `.12`, same port, in about 0.5 s (plan: under 5 s). |
| Sleep and wake | `SIGSTOP` node 2, copy on node 1, `SIGCONT` | Reconnect and the newest clip arrives about 0.3 s after wake. Run with 60 s and 300 s (the plan's 5 minutes). |
| Lossy Wi-Fi | netem 5 % loss, 100 ms on every port | 20 copies in a row; every node ends with the last one. |

Detection rule checks (false alarms and misses), all in the table above:
- VPN up and LAN works: no alarm (row 3, 4).
- No VPN and no path: `Unreachable`, never `VpnBlocksLan` (row 6, and the loopback test).
- VPN blocks LAN: alarm on both sides within idle timeout + 20 s (row 5).
- A peer never seen on this LAN: `NotFound`, no alarm (row 2).
- macOS system tunnels: no alarm (fixture test).

## 5. Pending: needs the lab VMs or a person

| Item | Needs | Why it is not done |
|---|---|---|
| Windows: does `IP_UNICAST_IF` + address bind beat an overlapping WireGuard route? | `win-b` with WireGuard | Never on Max's PC: a kill switch or route change there would cut his network, and the PC is managed by Defender for Endpoint. |
| Windows: full tunnel with LAN allowed, kill switch | `win-b` | same |
| macOS: does `IP_BOUND_IF` beat an overlapping route? Full tunnel, kill switch | `mac-b` | tart is not installed (needs Max's yes for the tap, lab doc 2026-11-13) |
| Windows firewall: which alerts appear and when, Private vs Public profile, outbound-only mode, two PCs that both refused | `win-b` (+ PC as the second side) | A listening socket on a LAN address can show a firewall dialog; nothing may appear on Max's screen. The loopback tests bind 127.0.0.x only. |
| macOS Local Network prompt for the agent | Mac or `mac-b` (S2 overlap) | on-screen prompt |
| `mdns-sd` next to the Windows DNS client's own mDNS responder (both on 5353) on a real LAN | PC + `win-b` | same as the firewall row |
| Real Wi-Fi client isolation and a real guest network | a person with the router | |
| The LAN setting of 4 or 5 consumer VPNs (Mullvad, NordVPN, Proton VPN, Surfshark, ExpressVPN) | web research + `win-b` | not started; the help text (L12) does not depend on it |
| M2 accept: "on `win-b` and on `mac-b`: WireGuard full tunnel with LAN allowed, kill switch and overlapping route each give the result S1 wrote down" | both VMs | |

What to run when the VMs exist: build `lan-node` on each VM, start two nodes (VM and PC or
two VMs) with `--peer`, then apply the three VPN setups. Expected results to confirm or correct:
full tunnel with LAN allowed gives `UP ... <lan iface>`; kill switch gives
`PATH <n> NoPath(VpnBlocksLan)`; overlapping route gives the direct path if the OS tie wins
(Linux: proven), else `NoPath(VpnBlocksLan)` and the VPN help (plan 3.5 says that is acceptable).
