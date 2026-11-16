# Networking (M2): how the plan was read

Date: 2026-11-14. Crate: `uc-net`. Plan: sections 3.3 to 3.5, 4.1, 6.3, 7 (M2). Protocol page 4.8.
Spike result: `docs/spikes/s1-interfaces.md`.

This page lists the places where the plan left room, and the choice M2 made.

## Transport: plain QUIC (`quinn`), not iroh

The plan says: iroh 1.x with relay, discovery, hole punching and port mapper off, **or plain QUIC
if iroh cannot turn something off cleanly**. iroh 1.4.1 cannot (source read on 2026-11-14,
`iroh-1.4.1/src/endpoint/quic.rs` and `socket/remote_map/remote_state.rs`):

| iroh part | Can it be turned off? |
|---|---|
| Relay | Yes: `RelayMode::Disabled` (the default of `Builder::empty()`). |
| n0 DNS / pkarr lookup | Yes: `clear_address_lookup()`. |
| Port mapper (UPnP, NAT-PMP, PCP) | Yes: `portmapper_config(Disabled)` or no `portmapper` feature. |
| **Hole punching (QUIC NAT traversal)** | **No.** The default transport config sets `max_remote_nat_traversal_addresses`, and the setter ignores every value below 8, so 0 is impossible. Each side sends all its local candidate addresses to the peer, and the client side probes them. |
| **New paths after the handshake** | **No.** QUIC multipath is always on. A peer can open paths from other addresses after the LAN check, so a check on the first packet is not enough. |
| Our own sockets (tie to an interface) | Only with `unstable-custom-transports`, which may change in any release. |

So `uc-net` uses `quinn` 0.11 with `rustls` 0.23 (ring) and **raw Ed25519 public keys** (RFC
7250), the same key scheme iroh uses. The device key and the wire protocol stay the same as the
plan says; only the dialing differs (by key and remembered address, no iroh address book).
`docs/protocol.md` already works with plain QUIC (TLS key and exporter, section M0c note).

What this gives:

- Nothing in the app can reach the internet: no relay, no lookup service, no STUN, no NAT
  traversal, no port mapping. `tests/lan_only.rs` checks that none of those crates is in
  `Cargo.lock` and that no internet endpoint is in the source.
- Server-side **connection migration is off**, so a connection never moves to a new address.
- We own every socket, so the interface tie (plan 3.5 options b and c) needs no unstable API.

TLS details: TLS 1.3 only, no SNI, no session tickets, no 0-RTT. The dialer checks that the
server key is the device it dialed. The server takes any well-formed key in the handshake and
the node closes non-members right after it (`CLOSE_NOT_ADMITTED`), before any frame.

## The LAN rule on sync connections

`addr::allowed_on` is the protocol 4.8 rule. It runs on the **first packet** of every incoming
connection (`Incoming::remote_address`), before the handshake starts, against the interface the
packet came in on. A failing source is dropped with `Incoming::ignore` (no answer at all, so a
scanner learns nothing). The same rule runs on every dial target, every gossiped address and
every mDNS answer. Public IPv4, public IPv6 (also on a "LAN" with public prefixes), CGNAT
(Tailscale) and loopback fail.

The endpoint binds **only LAN-class addresses** of physical interfaces (RFC 1918, 169.254/16,
fe80::/10, fc00::/7). It never binds a wildcard. On a network that gives only public
addresses, nothing listens.

## Sockets and interfaces

- One UDP socket per LAN address of every physical interface, all on the same fixed port.
  The `SocketBinder` trait makes them; `OsBinder` ties each socket to its interface
  (`SO_BINDTODEVICE`, `IP_BOUND_IF`, `IP_UNICAST_IF`). Android and iOS bring their own binder.
- Interfaces are scanned every 2 s and on `Net::wake()`. A change keeps the sockets of
  addresses that stay (their links stay up) and adds or closes the others. If a new address
  cannot take the fixed port, everything moves to a new port and `NetEvent::PortChanged` tells
  the agent (peers learn it through gossip).
- Classification (`ifaces::classify`) is a pure function, tested with recorded adapter lists
  from the PC, the Mac mini, WSL and a netns with WireGuard (`tests/fixtures/`).
- **The host's adapter on a virtual switch is the LAN** (2026-11-14, Max's PC). With the NIC
  bound to a Hyper-V *external* switch, the NIC has no address and the host's LAN address is
  on `vEthernet (<switch>)`. Before the fix every vEthernet was `Virtual`, so the agent bound
  no socket at all ("endpoint bound sockets=0", pairing said "Cannot find the other device").
  Rule: a Hyper-V host adapter (`vEthernet` / "Hyper-V Virtual Ethernet") with an **IPv4
  default gateway that is private and inside its own private IPv4 subnet** counts as
  `Physical`. Internal switches (Default Switch, WSL) have no default gateway and stay
  `Virtual`. VPN detection runs first, so a VPN never qualifies. Gateways come from netdev
  (`GetAdaptersAddresses` with `GAA_FLAG_INCLUDE_GATEWAYS` on Windows, the default route on
  Linux and macOS), recorded as `RawIface.gateways`.
  Same rule on Linux for a host **bridge or bond** (`br0`, Proxmox `vmbr0`, `bond0`) that
  holds the default route; docker, virbr, `br-*`, lxc and cni bridges are caught by name
  first and stay `Virtual`. **Not on macOS**: VM bridging there keeps the host address on
  `en*`, and `bridge0` is the Thunderbolt bridge or Internet Sharing. Other Windows virtual
  adapters (VMware, VirtualBox) stay `Virtual` even with a gateway.
- **No usable network.** When no socket can be bound, `uc-net` logs one WARN with each
  skipped interface and why, names only (`vEthernet (WSL): virtual, no LAN default gateway;
  Ethernet: no address`). The agent then shows "Needs attention" with the reason "This device
  found no network it can use." (`fix_code` `no-lan`), and `uc pair enter` fails at once with
  the same text (error code `no-lan`) instead of "Cannot find the other device".
- **"A VPN is up" needs a non-link-local address on a VPN interface.** macOS always has 4 to 8
  `utun` system tunnels with only `fe80::` (seen on the Mac mini); counting them would raise
  the VPN help on every Mac.

## Discovery

1. **Remembered addresses** (`LanBook`): up to 16 per device, the last that worked first. Every
   working connection, gossip, mDNS answer and pairing adds to it. Persisted by the agent with
   `LanBook::encode`.
2. **Signed gossip** (`gossip`): `AddrUpdate.signed` is defined here. Format in the module
   text: domain `uclip-addr-v1`, device key, `seq` (wall ms, strictly rising), port, a VPN flag,
   up to 8 LAN addresses, Ed25519 signature (`verify_strict`). Only member announcements are
   kept, only newer ones, and a new one is passed on to the other ready members.
   **`PairState.addr_book` (protocol 4.7)** is `u16 count || count × (u16 len || signed)`, the
   members' newest signed announcements (`LanBook::export_announcements`).
3. **mDNS** `_uclip._udp.local.` with `mdns-sd` 0.21, only on physical interfaces (every other
   interface is disabled in the daemon, loopback too). Instance name and host name are the tag
   `HMAC-SHA256(group secret, "uclip-mdns-v1" || slot u64 || device key)[..12]` in hex, 10-minute
   slots. The device key is in the HMAC so two members never clash, and a member knows which
   member it saw. Recognised: current and previous secret, slots -1, 0, +1. The name probe is
   skipped (the names are unique by construction).
   **Interfaces go to `mdns-sd` by OS index, never by name** (2026-11-14, Max's PC): on
   Windows `Interface.name` is the adapter GUID (netdev) but `mdns-sd` (via `if-addrs`) names
   interfaces by friendly name (`vEthernet (uc-lan)`), so no interface was ever enabled and
   the PC never found the Mac ("Cannot find the other device"). `discovery::mdns_indexes` and
   `mdns_if_kinds` give `IfKind::IndexV4(i)` and `IndexV6(i)`; loopback is disabled again
   after each enable. `tests/mdns_index.rs` checks on each CI OS that every usable address has
   the same index in `if-addrs`. Real check, headless, listen only:
   `cargo run -p uc-net --example mdns-check [-- --secs 5]` prints the interfaces, what
   `mdns-sd` enabled and the `_uclip-pair._udp` / `_uclip._udp` instances it found.
   Other name uses were checked: the socket tie uses the index on Windows and macOS and the
   kernel name on Linux (`SO_BINDTODEVICE`, same as netdev), IPv6 scope IDs use the index.

## Links and reconnect

- One QUIC connection per peer, one bidirectional stream, frames as `u32 length || frame`.
  `QuicPeer` implements `uc_core::Peer`. Plus one unidirectional data stream per file transfer
  (below).
- Both sides dial. The device with the larger key waits 300 ms first. If two connections still
  meet, both sides keep the one dialed by the smaller key; after 2 s the newest connection wins
  (the old one may be dead after a peer restart).
- Backoff 0.25 s doubling to 30 s per peer; reset on a new address, an interface change and on
  wake. Wake is `Net::wake()` from the OS, or found by the loop itself (a tick late by more than
  5 s, or the wall clock jumping): rescan, redial at once.
- Keep-alive 5 s, idle timeout 24 s: a dead path is found in under 30 s (plan 3.3). The worst
  case is keep-alive + idle timeout, because the idle clock starts at the last packet received.
  (Was 15 s + 29 s until 2026-11-15, which measured up to 44 s for a hard-killed peer.)

## Data streams for file transfers (file sync, 2026-11-16)

Plan: `docs/plan/2026-11-16-file-sync.md` sections 3 and 6. Code: `link.rs`, `node.rs`,
`endpoint::transport`. Tests: `tests/streams.rs`.

- Clipboard frames stay on the frame stream. File bytes never go through `Link::send` (an
  unbounded queue, and a 28 MiB frame limit).
- **Credit only on member links.** The transport starts every connection with
  `max_concurrent_uni_streams = 0`, so the peer cannot open a single data stream.
  `Link::allow_streams` raises it to 8 (a QUIC MAX_STREAMS frame). The node calls it when a
  link becomes the current link of a member, or when membership sync turns a probation link
  into a member link. Pairing connections and probation links never get credit:
  `open_stream` there fails with `StreamError::Refused` after the handshake timeout.
- **Delivery.** Every link runs an `accept_uni` loop (`LinkEvent::Stream`). The node passes a
  stream on as `NetEvent::Stream { peer, link, stream }` only when `link` is the peer's
  current link and the peer is a member; anything else is stopped with code 2 at once. The
  agent still checks that it asked this peer for that copy (its outstanding `FilesWant`).
- **Sending.** `Link::open_stream` gives a `SendStream` at priority -1 (the frame stream has
  0, so buffered clipboard frames go out first). `write` waits for flow control: a slow
  receiver slows the sender, nothing piles up in memory. `finish` waits until the peer has
  every byte. Dropping a `SendStream` without `finish` resets it with code 2 (a bare quinn
  stream would end cleanly, and the receiver would see a short file as complete).
- **Cancel codes** (stream codes, a separate space from the close codes): 1 = superseded
  (`STREAM_SUPERSEDED`), 2 = aborted (`STREAM_ABORTED`). The sender uses `reset(code)`, the
  receiver `stop(code)`; the other side then gets `StreamError::Reset(code)` or
  `Stopped(code)`. A dropped `RecvStream` stops with 2, so read until `Ok(None)` before
  dropping it. Never close the link to cancel: that ends the clipboard frames too.
- **Windows, set explicitly** (`endpoint.rs`): `stream_receive_window` 8 MiB (per stream, frame
  stream included), connection `receive_window` 9 x 8 MiB (room for the frame stream and all 8
  data streams, so stalled transfers can never block frames; quinn's default is unlimited),
  `send_window` 16 MiB (quinn's default is 10 MB).
- **Link replaced or lost.** The transfer fails on both sides with `StreamError::LinkClosed`,
  never as a short clean end. The receiver asks again on the new link (plan 2.2.7).
- Measured 2026-11-16 (`one_gigabyte_crosses_a_data_stream`, debug build, Windows 11
  loopback): 1 GiB in 8.9 s, 115 MiB/s; a clipboard frame sent halfway arrived in 21 ms.

## Path state and the VPN rule

`path::evaluate` (pure, table-tested): `Lan`, `Connecting` (first 20 s), then
`NoPath(VpnBlocksLan)` if the peer was seen on this LAN before and a VPN is up here or was up on
the peer (from its announcement), `NoPath(Unreachable)` if seen before without a VPN (asleep,
firewall, guest network), `NoPath(NotFound)` if never seen here. Only `VpnBlocksLan` raises a
"Needs attention" reason (`Attention::VpnBlocksLan`, screen L12).

## Left for M3 and M7

- Admission: `Net::set_members` decides who may open a sync link. **Done in M7:** the protocol
  5.9 probation path. An unknown key gets one probation link at a time for at most 10 s, with
  no addresses; the agent runs membership sync on it and calls `Net::end_probation` or
  `set_members`. `NetConfig::probation` turns it off; `Net::set_removed` refuses keys outright.
  M7 also added `NetConfig::accept_incoming` (outbound-only mode), `Net::dial_pair_any` and
  the `_uclip-pair._udp` mDNS service. See `docs/agent.md`.
- `Net::ready(peer)` after membership sync passed: only then does a device send addresses.
- Pairing: `uclip-pair/1` connections come out as `NetEvent::Pairing`, LAN rule applied;
  `Net::dial_pair` dials one. Both sides get the same TLS exporter (tested).
- The Windows firewall rule (`RemoteAddress=LocalSubnet`, Private profile) is M9.
