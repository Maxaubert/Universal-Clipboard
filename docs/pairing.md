# Pairing, membership and keys (M3): how the protocol was built

Date: 2026-11-14. Crates: `uc-pair`, `uc-keys`. Spec: `docs/protocol.md` (version 1), plan 3.12.

This page lists the places where the code had to make a choice, and where to find each part.
The protocol page wins over this page. Clarifications that M3 needed are in protocol PR #13.

## Where things are

| Protocol section | Code |
|---|---|
| 4.4 SPAKE2 wrapper, low-order check | `uc-pair/src/pake.rs`: `Pake`, `check_peer_msg` |
| 4.5 transcript and confirmation | `pake.rs`: `confirmation`, `verify_mac` |
| 4.4, 4.7 frames | `frame.rs` |
| 4.2 to 4.7 flow, limits, lockout | `session.rs`: `Host`, `InviterRun`, `JoinerRun` |
| 4.3 QR payload | `qr.rs`: `QrPayload::parse` (strict), `to_text` |
| 4.8 LAN rule | `lan.rs`: `is_lan_peer` |
| 5 membership | `chain.rs`: `Change`, `Graph`, `Evaluation`, `self_action` |
| 5.9 sync order | `gate.rs`: `Gate` |
| 6 test vectors | `uc-pair/tests/vectors/*.json`, `tests/vectors.rs` |
| 3.12 key store | `uc-keys/src/lib.rs` |

## Transport

- The pairing code does no I/O. `PairTransport` gives three things from the TLS layer: the peer's
  device key, the exporter value (`EXPORTER-uclip-pair-1`, empty context, 32 bytes) and the
  remote address. M2 implements it on the real QUIC connection.
- The caller moves frames: `frame::Decoder` splits the stream, the state machines return the bytes
  to send. Time is passed in as milliseconds, as in `uc-core`.
- The driver owns the 10-second step timer. On a timeout or a dropped connection it calls
  `InviterRun::on_close` (or just drops the run). A counted try stays counted.

## Inviter

- `Host` is one per device and lives as long as the agent. It holds the one open pairing and the
  lockout count, in memory only. Clones share one state behind a mutex.
- `Host::accept` checks, in this order: a pairing is open (else `Refuse::NotOpen`: do not accept
  the ALPN), the LAN rule (else close before `PairHello`), at most 4 connections (else send
  `PairAbort(3)`, not a try).
- The check "tries left" and the count "+1" are one step under the mutex, before `PairReply` is
  built. A test runs 8 threads against one code and gets exactly 5 replies.
- A bad `msg_b` (low order, wrong side byte, not a point) is a counted try and gets `PairAbort(4)`.
- A run that was counted and ends without a correct `conf_b` adds 1 to the failed tries. At 5
  failed tries the code dies; the QR code stays valid. Opening a new pairing while one is open
  replaces it.
- After a correct `conf_b` the pairing is over for both modes. Other runs get `PairAbort(2)` at
  their next step.

## Joiner

- `JoinerRun::start` refuses in QR mode when the TLS key is not the key in the QR code. Nothing
  is sent.
- A bad `PairReply` or a wrong `conf_a` returns `JoinError::CloseSilently`. The joiner sends no
  `PairAbort` and no `PairConfirm`.
- `PairState` is checked in full before anything is returned: every change parses and its
  signature passes, the first is the only genesis, every change is stored directly (no pool),
  the inviter's TLS key and the joiner's key are members, and the secret matches the current
  secret change.

## Membership

- The graph stores changes in arrival order and keeps, for each change, its depth and its past as
  a bit set. The canonical order is kept as a sorted index list.
- "Effective" is computed once for each remove, when it is stored, by evaluating the view "past of
  r". It never changes after that.
- A graph is created with its group ID (`Graph::with_group_id`) or from its genesis. Any other
  genesis is refused.
- Signing helpers use the current heads as parents. `sign_remove` also signs the `rotate` and
  returns the new secret, except when a device removes itself.
- `needs_rotate` tells the agent that an effective remove has no valid rotate after it (5.8 wait
  of 30 to 60 seconds is the agent's job).
- `self_action` is the only way to decide a self-wipe. A bare "removed" hint has no input here.

## Key store (`uc-keys`)

| OS | Backend | Notes |
|---|---|---|
| Windows | Credential Manager through `keyring` 3.6 (`windows-native`) | Per user, DPAPI. Checked once on the PC with the ignored test `os_store`. |
| macOS | Login Keychain through `keyring` 3.6 (`apple-native`) | Not yet limited to the signed app; see open points. |
| Linux | Secret Service through `keyring` 3.6 (`sync-secret-service`, `crypto-rust`) | Needs `libdbus-1` (build: `libdbus-1-dev`). If the service does not answer, `open_store` uses a `0600` file store and reports `StoreKind::FileFallback`. |
| Tests | `MemoryKeyStore`, `FileKeyStore` | `UCLIP_TEST_KEYSTORE=file` with a profile picks the file store. |

Why `sync-secret-service` and not `async-secret-service`: the async one pulls `zbus` 4, whose
`zvariant` and `zbus_names` crates are marked unmaintained (RUSTSEC-2026-0370 to 0372), and
`cargo deny` fails on that.

## Crypto crate versions

`spake2 =0.4.0` brings `curve25519-dalek` 4, `sha2` 0.10, `hkdf` 0.12 and `rand_core` 0.6. M3 uses
the same majors for `hmac` (0.12) and `ed25519-dalek` (2), so one copy of each is built.

## Known limits

- The `spake2` crate copies the password into its own `Vec` and does not wipe it. Our own copies
  are wiped (`zeroize`).
- The vectors check the SPAKE2 output of this crate version against itself. The transcript, HKDF
  and HMAC layers were checked once with an independent Python script (2026-11-14).

## Open points

1. macOS: the Keychain item is not yet limited to the signed app (plan 3.12). That needs the app
   bundle and an access group; do it with the app build (M7).
2. The Opus security review with a fresh context (plan M3 accept rule) has not run yet.
3. Two protocol readings wait for a yes in PR #13: strict `device_type` (0 to 7), and "no add or
   rename accepted from 1,024 changes on".
