# Core library (M1): how the plan was read

Date: 2026-11-14. Crates: `uc-core`, `uc-clip`, `uc-testkit`. Plan: section 3.8, 3.11, 6.1, 7 (M1).

This page lists the places where the plan left room, and the choice M1 made. Later milestones
can change a choice, but then they change this page too.

## Crate split
- `uc-clip` owns `Format`, `Snapshot`, the `Clipboard` trait and `FakeClipboard`. It does not
  know the clip model, so the OS backends (M4 to M6) depend only on `uc-clip`.
- `uc-core` depends on `uc-clip`. The engine (`sync::Engine`) does no I/O. The caller passes the
  clipboard and the time (`now_ms`) into each call. Peers are a `Peer` trait that sends frames.
- `uc-testkit` has `FakeNet`: several engines, fake clipboards, fake clipboard managers, and
  in-memory links that keep order per link (like one QUIC stream).

## Wire format
- Every frame starts with the protocol version byte, then a 1-byte message type, then the
  `postcard` body. The plan says "every stream"; per frame is simpler and costs one byte.
- Unknown message types decode to "skip". Bytes after a known body are ignored, so a newer
  version can add fields at the end.
- Accepted versions: the current and the previous one.
- `Hello` has one field the plan does not list: `wall_ms`, the sender's clock. A device uses it
  to learn that its own clock is more than 2 minutes ahead ("The date and time on this device
  are wrong"), because the peer refuses its clips silently.

## Clip model
- Wire ID: BLAKE3 over (format, size, BLAKE3 of the bytes) of each part, in a fixed order. This
  is a hash of the parts as sent, and a `Latest` header can be checked without the bytes.
- Content hash: text with all line endings folded to LF and trailing NULs removed; image as
  width, height and RGBA8 pixels. HTML is not in it.
- Line endings: on read, CRLF **and a lone CR** become LF, so the wire never holds a CR. On
  write to a CRLF clipboard, LF becomes CRLF. The engine does this, using
  `Clipboard::line_ending()`.
- HTML is only valid next to a plain text part.

## Size caps
- Caps are binary: text 1 MiB, HTML 2 MiB, image 25 MiB, push limit 256 KiB.
- In a mixed clip each part is checked on its own. Text over its cap takes its HTML with it.
  If no part is left, the clip is not synced.
- Images: header check first (16,384 px per side, 100 MP). Non-interlaced PNGs are hashed row
  by row. Interlaced PNGs are decoded whole (up to 400 MB at 100 MP); M10 may cap this lower.

## Sync rules
- A full member pushes clips up to 256 KiB to full members and sends `Latest` for larger ones.
  Pull-only peers always get `Latest` and ask with `Want`.
- **A full member passes on a clip that came from a pull-only client** to its other full
  members. Layer 3 ("never send a wire ID you received") is read as a rule for clipboard change
  events; without this pass-on, a phone's copy would reach only the one member it talked to.
- A newer clip older than 1 hour is **held** (it can be served to phones and applied by an
  explicit fetch) but not put on the clipboard. While paused, received clips are held too.
- Explicit fetch (`Engine::fetch`) uses the newest header the peers sent. Call it after the
  `Hello`/`Latest` exchange.
- Echo layers 3 and 4 compare a local copy with the **current clip only** (the one held: last
  sent or received), by wire ID and by content hash. On receive, only a newer stamp counts.
  So an older item copied again (same link twice, clipboard history) is a new copy and syncs.
  Manager loops still stop: a rewritten clip always equals the current one. (Changed
  2026-11-15 after a bug: a link copied a second time in Flare never reached the Mac. Before,
  the last 64 IDs and content hashes were blocked.)
- Settle time (150 ms) and read retries belong to the agent and the backends, not to the core.
- Membership and address messages are passed through as opaque bytes for M2 and M3. The agent
  must only `connect` peers that are valid members.

## File copies (0.2.0, 2026-11-16)

Spec: `docs/plan/2026-11-16-file-sync.md` sections 2 to 4. Code: `uc-core/src/files.rs`
(manifest), `sync.rs` (engine), `msg.rs` (codec). Tests: `uc-testkit/tests/files.rs`.

- **Wire.** `Format` and `ClipHeader` did not change: files never appear in `Latest` or `Clip`
  (a 0.1.x peer closes the link on an unknown format). While the current copy is files,
  `Latest { header: None }` goes out. New types 9 to 11: `FilesOffer`, `FilesWant`,
  `FilesCancel`. Both `Hello`s carry `HelloExt { features }` after the old body (bit 0 =
  files); a peer without it reads as features 0. Every later `Hello` field goes into `HelloExt`.
- **Current copy** is `Clip` or `Files`; both carry a clock stamp, highest stamp wins.
- **Walk generation.** A local file copy gives `Event::WalkFiles { gen }`; any newer copy bumps
  the generation, and `on_local_files` drops a stale one. The stamp is taken only after the
  walk and the hashes, so text copied during a long walk is newer.
- **Only the origin offers and serves**, and only after the peer's engine `Hello` shows bit 0.
  A peer without the bit gets nothing and stays connected (`SkipReason::PeerNoFiles`).
- **Receiving.** One pending offer; fetched only if it is the newest copy (and not older than
  1 hour, not paused, enough free space; "Copy smaller files automatically" does not matter here). `files_applied` makes it current and
  sets the own-write sequence, so received files are never offered again.
- **Sends.** At most 2 run at once, more wait in a FIFO queue. A new `FilesWant` for the same
  copy from the same peer resets the old stream. A newer current copy cancels every send.
- **Cancel and retry.** Any newer copy cancels the pending fetch (`Event::CancelTransfer`). A
  fetch cut by a lost link is asked again from the start when the origin is back, at most 3
  times. Stop (`stop_transfer`) is never retried.
- **Echo layer 4** for files: BLAKE3 over `uclip/files/1` and the manifest, so the same folder
  copied twice is not sent twice.
- **Manifest rules** (both sides, before anything touches disk): relative `/` paths in NFC,
  no `..`, no absolute or drive paths, no empty parts, no NUL, 255 bytes per name, 4,096 per
  path, depth 64, 100,000 entries, at most 16 MiB encoded. Windows receivers make names safe
  (`< > : " | ? *` and control characters become `_`, reserved names get `_`) and number
  duplicates ` (2)`.

## Tests (plan 6.1)
- Codec: round trip of every message, random and mutated bytes never panic.
- Clock: causality property, future-stamp refusal, behind clock, tie-break.
- Convergence: three devices, random copies, deliveries and time steps (300 cases per run; set
  `PROPTEST_CASES` for more), with random clipboard managers and Windows line endings, plus all
  720 delivery orders of three copies at the same moment.
- Clipboard-manager ping-pong: text and image managers on every device, exactly one send per copy.
- File copies (`uc-testkit/tests/files.rs`, fake net): small copy syncs, over the limit
  skipped, picked copy over the limit syncs, text during a walk wins, offers only after the
  engine `Hello`, old peer, cancel both ways, no re-offer, no double send, FIFO with 3
  receivers, retry and give-up, hostile manifests, stop, pause, `auto-file-copy` off (only automatic copies skipped, picked walks and transfers go on), no space.
