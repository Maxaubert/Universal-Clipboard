# Linux clipboard backends (M6)

Date: 2026-11-14. Crate: `uc-clip-linux`. Plan: sections 3.2, 3.8 to 3.10, 6.4, 7 (M6).

This page says how the Linux backends work, what is proven, and what is still open.

## Which backend runs

`session::detect` reads the environment. `LinuxClipboard::open` tries the backends in order
and keeps the first one that opens.

| Session | Order | Status |
|---|---|---|
| Wayland: KDE Plasma 6, Sway, Hyprland, other wlroots, COSMIC | `ext-data-control-v1`, then `wlr-data-control-unstable-v1`, then X11 through XWayland | data-control: tested in headless Sway |
| Wayland: GNOME | The same order. GNOME offers no data-control today, so the X11 path through XWayland is used. | **Experimental.** Spike S4 on a real GNOME machine is still open. We do not claim GNOME support. |
| X11 (XFCE, MATE, Cinnamon, KDE on X11, ...) | X11 | Tested in Xvfb |
| No display | none | The app shows "This desktop blocks background clipboard access." |

`LinuxClipboard::kind()` says which backend runs. `BackendKind::XWayland` reports
`is_experimental() == true`. The agent puts the kind in the diagnostic report, and the app must
not call the XWayland path "supported" until S4 says go.

## What is synced

- Only the `CLIPBOARD` selection. The primary selection (middle-click paste) is never synced.
- Text: read from `text/plain;charset=utf-8`, else `UTF8_STRING`, else `text/plain`, else
  `STRING` (Latin-1, turned into UTF-8). Written as all four UTF-8 types plus `TEXT`.
- `text/html` and `image/png`, as they are. Other image types are not converted.
- Each type is read with a 2 second idle timeout. A part over 64 MiB is refused (the core caps
  are lower, so this only stops a broken source).
- File copies (0.2.0, spec `docs/plan/2026-11-16-file-sync.md` section 6), see below.

## File copies

Date: 2026-11-16. Code: `mime.rs` (pure, tested on every OS), `LinuxClipboard::write_files`.

**Read.** A copy is a file copy when `x-special/gnome-copied-files` is offered (GTK file
managers), or when `text/uri-list` holds a `file:` URI (Dolphin, Thunar, PCManFM, most others).
Only the file list is read; the name text and icons next to it are ignored.
- `x-special/gnome-copied-files` is read first. Its first line is `copy` or `cut`; a cut is read
  as a copy (the source is never deleted). If it cannot be read, `text/uri-list` is tried.
- `parse_file_uris` follows RFC 2483 and RFC 8089: lines end in CRLF or LF, empty and `#`
  lines are skipped, the scheme is matched without case. `file:///p`, `file://localhost/p`
  and the short `file:/p` give a path. The path is percent-decoded as raw bytes (Linux names
  need not be UTF-8); a query or fragment is cut off; `%` without two hex digits stays as it is.
- Not a local path: `file://otherhost/p`, a relative path, a path with NUL. Other schemes
  (`https:`, `sftp:`, `trash:`) never give a path.
- `Snapshot.files` gets the local paths in list order. A list with other URIs as well keeps the
  local ones.
- `files_virtual` is set only when the copy is a file copy but no URI is a local path: GNOME
  Files in an SFTP folder or the trash (`x-special/gnome-copied-files` with `sftp:` or `trash:`
  URIs), or a uri-list with `file:` URIs of another host only. The core skips it as virtual.
- A `text/uri-list` of web links only is not a file copy: the text is synced as usual.
- A file list that cannot be read still counts as a file copy (so a file name is never synced
  as text), with no paths; the core skips it.

**Received files** go to `$XDG_CACHE_HOME/Universal Clipboard[-profile]/` (else `~/.cache`;
never `/tmp`, which is often RAM). Files get mode `0o755` when the sender's exec bit was set,
else `0o644` (tested end to end in `crates/uc-cli/tests/agents.rs`).

**Write** (`write_files`, received files): in one owner change, with our marker:

| Type | Value |
|---|---|
| `x-special/gnome-copied-files` | `copy`, then one `file://` URI per line, LF, no final newline (as Nautilus writes it) |
| `text/uri-list` | one `file://` URI per line, each ending in CRLF |
| `application/x-kde-cutselection` | `0`, so Dolphin pastes a copy, never a move |
| the four UTF-8 text types and `TEXT` | the paths, one per line (a paste into a terminal gives the paths) |
| `image/png` | only with the single-image companion (spec 2.2.5) |
| `application/x-uclip-origin` | our marker (echo layer 1) |

URIs percent-encode every byte except `A-Z a-z 0-9 - . _ ~ /`, so spaces, `#`, `%`, line
breaks and non-UTF-8 names round-trip. Relative paths or an empty list are refused before the
clipboard changes.

**Proven** (Docker, 2026-11-16): read and write on X11 (Xvfb) and Wayland (headless Sway,
`ext` and `wlr`), and interop with `xclip -t text/uri-list` and
`wl-copy`/`wl-paste --type text/uri-list`. **Not proven:** paste into a real file manager
(Nautilus, Dolphin). GNOME on Wayland stays experimental (spike S4): whether a file paste from
our XWayland write works in Nautilus is untested.

## Skipped clips

The backend reports these flags; the core then skips the clip. When a flag is set, no clip
content is read at all.

| Flag | When |
|---|---|
| `has_marker` | Our own marker type `application/x-uclip-origin` is offered (echo layer 1). Every write adds it. |
| `private` | `x-kde-passwordManagerHint` is `secret` (KeePassXC, KDE apps), or one of the nspasteboard.org types (`org.nspasteboard.ConcealedType`, `TransientType`, `AutoGeneratedType`) is offered. A hint that cannot be read counts as private. |
| `has_files` | A file copy (see above). `files` holds its paths; `files_virtual` says it has none on disk. Only the file list is read. |

## Change events and the counter

- `LinuxClipboard::subscribe()` gives a channel that gets the change counter on every new
  owner, our own writes included. The agent waits the settle time (150 ms) and then reads.
- The counter (`Snapshot::seq`) counts owner changes since the backend opened. `write` returns
  the counter of its own change, so echo layer 2 can skip it.

## How each backend works

**X11** (`x11rb`, pure Rust connection):
- An owner connection with a hidden 1x1 window. Its thread gets XFIXES `SetSelectionOwner`
  events for `CLIPBOARD` and answers `SelectionRequest`s for our writes: `TARGETS`,
  `TIMESTAMP` and each served type. Data over 256 KiB goes as `INCR` chunks. `MULTIPLE` is
  refused (no current app needs it).
- A reader connection with its own hidden window, used only by the caller's thread. So reading
  our own selection never waits on the owner thread.

**Wayland** (`wayland-client`, data-control):
- One macro writes the backend for both protocol versions (`ext` and `wlr`), so the logic exists
  once.
- An event thread dispatches the queue: it keeps the current offer and its types, and serves our
  sources. Each served type is written on its own short thread, so a slow reader never blocks
  events.
- Data-control version 1 is bound, so the `wlr` protocol sends no primary selection. The `ext`
  protocol may send it; those offers are destroyed at once.

## Autostart

`autostart` writes `~/.config/autostart/io.github.maxaubert.uclip.desktop` (or under
`$XDG_CONFIG_HOME`), with `Exec=<agent> --supervise` quoted per the Desktop Entry spec. The write
is atomic. `is_enabled` is false when a desktop settings app wrote `Hidden=true` or
`X-GNOME-Autostart-enabled=false`. The supervisor loop itself belongs to the agent (M7).

## Tests

- Pure rules (`mime`, `session`, `autostart`): unit tests, on every OS in CI.
- Live tests (`tests/live.rs`): against Xvfb and a headless Sway. They run only when
  `UCLIP_LIVE_X11` or `UCLIP_LIVE_WAYLAND` is set, and never inside WSLg (it bridges to the
  Windows clipboard). Cases: text, HTML and PNG from another app; our write with the marker; a
  KDE secret hint; a hint with another value; a file copy read (uri-list and GNOME list) and a
  web link; our file write read back by another client (every type, marker, image); a 3 MiB image both ways
  (X11 `INCR`); losing the selection to another app; the counter. Plus interop with `xclip` and
  `wl-copy`/`wl-paste`.
- Run them: `sh crates/uc-clip-linux/live/docker.sh` (Arch image: Sway with both data-control
  versions). CI runs `live/run.sh` on Ubuntu 24.04 (Sway 1.9, `wlr` only; the `ext` case skips
  there).

## Open points

- **GNOME (spike S4):** does XWayland get an XFIXES event for every Wayland copy, also with no
  X11 window focused, and can our X11 write reach Wayland apps? Until S4 answers on `lin-gnome`,
  the XWayland path stays experimental.
- **KWin:** the plan names `kwin_wayland --virtual` for the fast loop. Not in the Docker image
  yet (KDE in Docker is large); `lin-kde` covers KDE at the release gate.
- **Real sessions:** the plan's accept rule for M6 also needs `lin-kde` and `lin-gnome`. Those VMs
  are not built yet.
- **Supervisor loop** (`uc-agent --supervise`): done in M7 (`crates/uc-agent/src/supervise.rs`,
  test `crates/uc-cli/tests/supervise.rs`). The agent on X11 and Wayland is tested end to end
  by `crates/uc-cli/tests/linux_live.rs`, which `live/run.sh` runs.
