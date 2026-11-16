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
- **Private** (plan 3.10): `ExcludeClipboardContentFromMonitorProcessing` or the legacy
  `Clipboard Viewer Ignore` present (KeePass 1.x and older tools; added 2026-11-15),
  `CanIncludeInClipboardHistory` = 0, or `CanUploadToCloudClipboard` = 0. A marker that is listed
  but cannot be read, or is shorter than 4 bytes, counts as private.
- **Exception:** when our origin marker is present, `CanUploadToCloudClipboard` = 0 is ignored,
  because our own writes set it. The engine skips such a clip anyway (marker first).
- For private copies the content is never read. The snapshot has flags only. File copies: see
  "File copies" below.

## Write
- One open and close: our marker `UniversalClipboard.Origin` first, then
  `CanUploadToCloudClipboard` = 0, then the data. The marker goes first so even a write that fails
  half way is never sent again.
- Text as `CF_UNICODETEXT`. An image as `PNG` plus `CF_DIBV5` (32-bit, straight alpha, sRGB).
  Windows makes `CF_DIB` and `CF_BITMAP` from the `CF_DIBV5` for older programs.
- We do not set `CanIncludeInClipboardHistory`, so received clips stay in Win+V (plan 3.8).
- No delayed rendering.

## File copies (file sync, 2026-11-16)
Spec: `docs/plan/2026-11-16-file-sync.md` sections 2.4 and 6. Pure code in `src/files.rs`
(tested on every OS, with proptests), OS calls in `src/win.rs`.
- **Read:** when `CF_HDROP` is on the clipboard, only it is read. `parse_hdrop` takes the
  `DROPFILES` header and the double-NUL list. Paths become `PathBuf` through
  `OsString::from_wide`, so names with lone surrogates stay exact. Relative entries are left out.
  The text and image next to a file copy (a file name, an icon) are not read.
- **Refused lists:** an ANSI list (`fWide = 0`), a cut header, or `pFiles` outside the block give
  no paths. A path that runs to the end of the block without its NUL is left out. The snapshot
  then has `has_files` and no paths, and the engine skips it like a virtual copy.
- **Virtual:** `FileGroupDescriptorW` without `CF_HDROP` (Outlook attachment, an item inside a
  zip opened in Explorer) sets `files_virtual`. With `CF_HDROP` next to it, the paths win.
- **Cloud placeholders:** `uc_clip_win::cloud_only(path)` reads the attributes only
  (`GetFileAttributesW`, which never downloads). `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`
  (OneDrive online-only), `FILE_ATTRIBUTE_RECALL_ON_OPEN` and `FILE_ATTRIBUTE_OFFLINE` count.
  The snapshot has no field for it: the agent's walk calls it for every item (or passes the
  directory listing's `file_attributes()` to `files::is_cloud_placeholder`) and skips the whole
  copy with `FilesCloudOnly`.
- **Write** (`write_files`), one open and close: marker, `CanUploadToCloudClipboard = 0`,
  `CF_HDROP` (wide, from `build_hdrop`), `Preferred DropEffect = 1` (copy, never cut), and for
  the single-image companion (spec 2.2.5) `PNG` plus `CF_DIBV5`. Paths are checked first
  (`uc_clip::check_file_paths`; a NUL in a path is refused too), so a bad call leaves the
  clipboard as it was. `\\?\C:\...` and `\\?\UNC\...` are written without the prefix (spec 6), because
  many apps do not expect it in a pasted list. Long paths (over 259 characters) are written as
  they are.
- **Tests:** `tests/files.rs` (winhost): spaces, non-BMP and a lone surrogate, 5,000 paths,
  garbage `DROPFILES`, write then every format present and the marker seen, the image companion,
  a 300-character path, bad paths refused. `cloud_only` is tested on a temp file with
  `FILE_ATTRIBUTE_OFFLINE` (no OneDrive needed). `tests/real.rs`: an Explorer-like copy and a
  virtual copy.
- **Not tested here:** a real OneDrive online-only file, and pasting our list into Explorer
  (that needs the visible desktop; the install test on the PC covers it).

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

## Reads that hang (2026-11-15)
- `WinClipboard::read` runs on its own reader thread and gives up after `READ_TIMEOUT` (2 s).
  A program that offers a format but does not render it (delayed rendering) can block
  `GetClipboardData`; the agent no longer waits with it. While that read is stuck, the next read
  returns `Busy` at once, and the reader keeps the clipboard open until the owner answers (the OS
  works that way; other programs see a busy clipboard for that time too).
- The agent tries a failed read 4 more times (0.5, 1, 2, 4 s), then notes "Another app kept the
  clipboard busy." in Last activity. Tests: `crates/uc-clip-win/tests/hostile.rs`.
