# Windows clipboard backend (M4): how the plan was read

Date: 2026-11-14. Crates: `uc-clip-win`, `tools/winhost` (crate `uc-winhost`). Plan: 3.8, 3.9,
3.10, 6.4, 7 (M4). Spike: `docs/spikes/s3-winstation.md`.

## Watch
- One thread per `WinClipboard` owns a message-only window (`HWND_MESSAGE`). It calls
  `AddClipboardFormatListener` and runs a message loop. No polling.
- Each `WM_CLIPBOARDUPDATE` puts the clipboard sequence number on `WinClipboard::changes()`.
  Our own writes show up there too. The engine skips them by the marker and the sequence number.
- The same window owns our writes. Its thread pumps messages, so other programs that empty the
  clipboard never wait on us.

## Read
- One short `OpenClipboard`, copy the bytes out, close. PNG and DIB conversion happen after the
  close, so we never hold the clipboard during slow work.
- Busy clipboard: 5 tries, 100 ms apart (about 500 ms, plan 3.8), then `ClipboardError::Busy`.
- Text: `CF_UNICODETEXT` (Windows converts `CF_TEXT` for us). Stops at the first NUL. Line
  endings stay CRLF; the engine turns them into LF.
- HTML: `HTML Format` is read, and only the fragment (`StartFragment` to `EndFragment`, else
  `StartHTML` to `EndHTML`) goes into the `text/html` part. Bad offsets or non-UTF-8 drop the
  HTML part, not the clip. Writing HTML is M11 (`TODO(M11)` in `win.rs`).
- Image: registered `PNG` first (cut after `IEND`, because clipboard memory is rounded up), else
  `CF_DIBV5`, else `CF_DIB`, converted to PNG. Windows lists made-up formats too, so a program
  that sets only `CF_BITMAP` still gives us a `CF_DIB`.
- DIB parser: 1, 4, 8 bit with palette; 16 and 32 bit with `BI_RGB` or bit fields; 24 bit;
  top-down and bottom-up. RLE, JPEG and PNG inside a DIB are skipped. Size limits (16,384 px per
  side, 100 MP) are checked before any allocation. A 32-bit image whose alpha bytes are all zero
  is taken as opaque, because many programs leave that byte at zero.

## Skip rules
- **Private** (plan 3.10): `ExcludeClipboardContentFromMonitorProcessing` present,
  `CanIncludeInClipboardHistory` = 0, or `CanUploadToCloudClipboard` = 0. A marker that is listed
  but cannot be read, or is shorter than 4 bytes, counts as private.
- **Exception:** when our origin marker is present, `CanUploadToCloudClipboard` = 0 is ignored,
  because our own writes set it. The engine skips such a clip anyway (marker first).
- **Files:** `CF_HDROP`, and also `FileGroupDescriptorW` (virtual files, for example an Outlook
  attachment). The plan names only `CF_HDROP`; virtual files are the same kind of copy.
- For private copies and file copies the content is never read. The snapshot has flags only.

## Write
- One open and close: our marker `UniversalClipboard.Origin` first, then
  `CanUploadToCloudClipboard` = 0, then the data. The marker goes first so even a write that fails
  half way is never sent again.
- Text as `CF_UNICODETEXT`. An image as `PNG` plus `CF_DIBV5` (32-bit, straight alpha, sRGB).
  Windows makes `CF_DIB` and `CF_BITMAP` from the `CF_DIBV5` for older programs.
- We do not set `CanIncludeInClipboardHistory`, so received clips stay in Win+V (plan 3.8).
- No delayed rendering.

## winhost (tests on the real clipboard)
- `uc_winhost::isolated_test(name, body)`: the test binary starts itself again with
  `--exact name` inside a private window station (NULL name) and desktop `uc-winhost`,
  with `CREATE_NO_WINDOW` and the output sent to a temp file. The child checks that it really runs
  in that station (`require_isolated`) before it touches the clipboard.
- **Found in M4:** a NULL-name station is not new per call. Windows names it after the logon
  session (`Service-0x0-<LUID>$`), so every run in the session shares it and its clipboard. S3 did
  not see this because it ran once. So `run` holds a session-wide named mutex
  (`Local\UniversalClipboard.winhost`) while a child runs: one child at a time, also across
  parallel nextest processes. Tests must not expect an empty clipboard at the start.
- The host also compares its own clipboard sequence number before and after. In CI a change fails
  the test. Locally it only warns, because Max may copy something while tests run.
- The CLI: `winhost [--timeout SECS] <exe> [args...]` runs any program (for example the agent)
  the same way.

## Open points
- **Cloud clipboard arrivals** (plan 3.8, M4 task): not checked. It needs a signed-in Microsoft
  account with cloud clipboard on and a second PC. `WinClipboard::list_formats()` gives the
  format list of such a copy, so the check is one recorded format list away. Until then, the
  content hash (echo layer 4) is the protection.
- Write of `CF_HTML`: M11.
