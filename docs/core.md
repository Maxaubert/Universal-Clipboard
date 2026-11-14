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
- Echo layer 4 follows the plan word for word: content seen in the last 64 clips sent or applied
  is never sent again. **Open point for Max:** a user who copies an old item again (for example
  from a clipboard history) within those 64 clips will not see it sync. A time window (for
  example 10 s) would stop manager loops just as well. Not changed in M1.
- Settle time (150 ms) and read retries belong to the agent and the backends, not to the core.
- Membership and address messages are passed through as opaque bytes for M2 and M3. The agent
  must only `connect` peers that are valid members.

## Tests (plan 6.1)
- Codec: round trip of every message, random and mutated bytes never panic.
- Clock: causality property, future-stamp refusal, behind clock, tie-break.
- Convergence: three devices, random copies, deliveries and time steps (300 cases per run; set
  `PROPTEST_CASES` for more), with random clipboard managers and Windows line endings, plus all
  720 delivery orders of three copies at the same moment.
- Clipboard-manager ping-pong: text and image managers on every device, exactly one send per copy.
