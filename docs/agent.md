# The agent and the uc CLI (M7): how the plan was read

Date: 2026-11-14. Crates: `uc-agent` (binary `uc-agent`), `uc-cli` (binary `uc`).
Plan: sections 2.2 to 2.7, 3.2, 3.8, 3.12, 4.1, 7 (M7). Protocol: 4, 5.7 to 5.9.

This page lists where the plan left room, and the choice M7 made.

## Shape

- One core task owns all state: the sync engine, the network (`uc-net`), the membership
  graph and pairing host (`uc-pair`), the key store (`uc-keys`) and the clipboard backend.
  It runs on the main thread of a 2-thread tokio runtime, so the macOS pasteboard (not
  `Send`) stays on one thread.
- Clipboard change events: Windows and Linux backends send them on a channel; macOS is polled
  with the M5 `Poller`. The core waits 150 ms (settle time) and then reads.
- Pairing stream I/O runs in its own tasks (`pairing.rs`). The inviter's state machine stays
  in the core, because it signs the `add` change on the graph.

## Files and where things live

| What | Where |
|---|---|
| State (`state.json`) | Windows `%LOCALAPPDATA%\UniversalClipboard[-profile]`, macOS `~/Library/Application Support/uclip[-profile]`, Linux `~/.local/share/uclip[-profile]` |
| Device key | key store entry `device-key` |
| Group secret | key store entries `group-secret` and `group-secret-prev` (4-byte epoch + 32 bytes) |
| Log | `agent.log` in the state folder, rotates at 256 KB to `agent.log.1` |
| IPC | Windows `\\.\pipe\uclip-<SID>[-profile]`; macOS `<state>/agent.sock`; Linux `$XDG_RUNTIME_DIR/uclip[-profile]/agent.sock` |
| Single agent | lock file `agent.lock` (held for the whole run) plus the first-instance pipe or the socket check. A second agent exits with code 3. |

**Plan 3.12 says the group secret is in `state.json`, wrapped with a key from the key
store.** M7 keeps it in the key store itself. The result is the same or stronger (the secret
never touches `state.json`), and it needs no extra cipher.

## IPC (plan 3.2)

JSON lines: request `{"id","call","params"}`, reply `{"id","ok","result"|"error"}`, events
`{"event","data"}` after `subscribe`. Calls: the plan's list plus `setup.status`,
`setup.step` and `version`. Test profiles also accept `debug.copy`, `debug.clipboard`,
`debug.wake` and `debug.shutdown`; the real app refuses them.

Hardening:
- Windows: `FILE_FLAG_FIRST_PIPE_INSTANCE`, DACL `D:P(A;;GA;;;<user SID>)`, remote clients
  refused. The agent checks each client process's SID; the client checks the server's SID
  before it sends anything. **Open (M9):** the check that the server is *our signed* agent
  needs code signing.
- Unix: socket `0600` in a `0700` folder; both sides check the peer UID (`SO_PEERCRED` /
  `getpeereid` through tokio).

## Sync connections (protocol 5.9)

- On every link the agent sends its own `Hello` first. `Hello` now carries the group ID (a new
  last field, `group`). After the peer's `Hello`: group check, removed check, then the agent
  sends `Membership` with the changes the peer lacks (`Graph::missing_for(peer heads)`). After
  the peer's `Membership`: insert, evaluate, and open the gate only if the peer is a member.
- Only then the engine gets the link (`Engine::connect` sends the engine's `Hello` and
  `Latest`), the network gets `ready` (addresses may flow), and `WantSecret` goes out if this
  device lacks the current secret.
- Before the gate opens, any other frame closes the link.
- **Probation (the admission hook M2 left open):** `uc-net` gives an unknown key one probation
  link at a time, for at most 10 s, with no addresses. The agent counts its frames and
  changes (64 KiB, 256 changes). After membership sync it either becomes a member (the link
  stays and becomes a normal link) or the agent ends the probation.
- **A removed key that connects** gets the whole graph and a `Removed` hint, then the link
  closes after 1 s. It reads nothing from that device. Without this, a device removed while
  it was off would never learn it (it would show "Asleep or away" for ever).
- `WantSecret` and `Secret` are new message types 7 and 8 on `uclip/1`. The agent routes
  them (and `Membership`, `AddrUpdate`) before the engine.

## Membership actions

- Remove: `sign_remove` (with the `rotate` and the new secret), spread to every open member
  **including the removed one**, then its link closes after 1 s so the frames get out.
- 5.7 "sync membership with every member (3 s) before signing a remove": **not done.** The
  open links already ran membership sync when they opened, so the past of the remove is
  current with every connected member. A member that is offline is the honest limit of 3.7.
- 5.8 rotation wait: 30 to 60 s (1 to 2 s with `UCLIP_TEST_FAST`).
- Self wipe (5.7): only from the agent's own evaluation. It wipes the group, makes a new
  device key and restarts the network with it. The status shows "Another device removed
  this one".
- `uc leave` (the window's "Remove this device and forget everything"): signs a remove of
  itself, spreads it, waits 1 s, wipes the group and the key, and turns start at login off.
  The Windows firewall rule offer is M9.

## Pairing

- `uc pair show`: makes the group if there is none, opens the pairing (`Host::open`),
  advertises `_uclip-pair._udp.local` (new in `uc-net`), prints the code as `418 207` and draws
  the QR code (`UCLIP1:...`) in the terminal, then waits for `pair.added`.
- `uc pair enter`: code mode only. Computers type the code (plan 2.3); the QR payload is taken
  only from an in-app scanner (protocol 4.3), so the CLI does not take it. The joiner dials
  every inviter it finds with the new `Net::dial_pair_any` (any server key: the PAKE proves who
  answered), at most 8 at a time, 32 in total, 20 s. A wrong code is reported 2 s after the
  last inviter answered.
- "Leave your other devices and join these?": the CLI asks; `--yes` answers yes.
- Test profiles on loopback: `uc-pair`'s LAN rule has no loopback switch, so the agent shows a
  loopback pairing connection to it as one private on-link address. `uc-net` already applied
  its own rule with the loopback test policy. Only test profiles with `UCLIP_TEST_LOOPBACK`.

## Settings

Keys (`uc settings set <key> <value>`): `start-at-login`, `sync-images`, `image-limit-mb`
(1 to 25, the plan's "image size cap"), `keep-formatting`, `forward-apple-clips` (macOS
only), `device-name` (a `rename` change when in a group).

- `keep-formatting` is stored; HTML writing is M11, so it changes nothing yet.
- **Decision 8 (forward Apple Handoff clips) is still open.** Default **off**, the safer
  choice: the macOS backend does not even read the content of such a clip
  (`set_read_remote_content(false)`), and the engine skips it (`SkipReason::Remote`). The
  setting turns both on.

## OS hooks

| | Windows | macOS | Linux |
|---|---|---|---|
| Start at login | `HKCU\...\Run` value `UniversalClipboard`. Store build: startup task (M9, spike S8). | LaunchAgent plist `io.github.maxaubert.uclip.agent` in `~/Library/LaunchAgents` (`KeepAlive` on crash). `SMAppService` needs the app bundle: M9. | XDG autostart with `uc-agent --supervise` |
| Test profiles | sandbox key `HKCU\Software\UniversalClipboard-test\<profile>\Run` | `<state>/autostart/LaunchAgents` | `<state>/autostart` |
| Sleep and wake | `PowerRegisterSuspendResumeNotification`, plus the time-jump check | time-jump check (a 1 s tick that comes more than 5 s late, or a wall clock jump) | same as macOS |
| Crash restart | none yet (M9 decides with the installer) | `KeepAlive` | `--supervise`: 1 s doubling to 60 s, reset after 60 s up; stops on exit 0 or 3 |

- **Windows firewall order rule:** the real app does not start the network before `uc setup`
  (or the window) answered the firewall row. "No" gives outbound-only mode
  (`NetConfig::accept_incoming = false`). M7 only records the answer; the rule itself (one UAC
  prompt) comes from the M9 helper. Whether Windows shows its own alert for our UDP sockets in
  outbound-only mode is checked on `win-b` in M9.
- **macOS paste access (plan 3.9):** the agent reads `accessBehavior`; "Ask" or "Deny" gives
  "Needs attention". The "detect first" step uses the pasteboard **type list**, which reads no
  content and shows no prompt (M5 checked this); the `detect*` pattern methods add nothing for
  us, because we sync every text, not patterns. `uc setup` makes the one read on purpose
  (row "Read what you copy"). Proving that this is silent on the general pasteboard is spike
  S2 with Max at the Mac.
- **Port in use:** `uc-net` moves to a new port and the agent stores it; peers learn it by
  gossip.

## Test switches

Only test profiles (any `UCLIP_PROFILE` that is not empty) read these; see `profile.rs`.
`UCLIP_STATE_DIR`, `UCLIP_TEST_KEYSTORE=file`, `UCLIP_TEST_CLIPBOARD=os`,
`UCLIP_TEST_PASTEBOARD`, `UCLIP_TEST_LOOPBACK`, `UCLIP_TEST_PAIR_ADDRS` (read by `uc`),
`UCLIP_TEST_FAST`. **A test profile never opens the user's clipboard:** the OS backend only
inside `tools/winhost` (Windows), on a named pasteboard (macOS), or on a display the test
started (Linux, Docker). Elsewhere it refuses, and two tests check that.

## Tests

| Test | Where it ran (2026-11-14) |
|---|---|
| `uc-cli` unit: every settings-window action maps to a `uc` command; every IPC call is reachable; `--json` everywhere; no send/get | PC, Mac, WSL |
| `agents.rs`: 3 agents paired through the CLI (third from the second device), copy matrix, rename, pause and resume, settings, remove (the removed device learns it), leave, no content in logs or report; restart and reconnect, newest clip after reconnect, single agent; wrong code; `uc setup` and start at login in the sandbox | PC, Mac, WSL |
| `winhost.rs`: one agent on the real Windows clipboard in a private window station, paired with a fake-clipboard agent, copy matrix both ways | PC |
| `macpb.rs`: one agent on the real macOS backend with a named pasteboard, copy matrix | Mac over ssh, no prompt, nothing on screen |
| `linux_live.rs`: an X11 agent (Xvfb) and a Wayland agent (headless Sway), copy matrix | Docker (`live/docker.sh`), CI `linux-clipboard` |
| `supervise.rs`: kill -9 the agent, the supervisor starts it again; a clean exit ends it | Mac, WSL |
| guard tests: a test profile refuses the user clipboard outside winhost / without a named pasteboard | PC, Mac |
| `uc-net` loopback: probation link, removed keys, promotion to member | PC, Mac, WSL |

## Idle use (plan M7 target: under 0.5% CPU, under 40 MB)

Two release agents, paired and connected on loopback, measured for 120 s after 10 s of rest.

| OS | CPU (of one core) | Memory |
|---|---|---|
| Windows 11 (PC, fake clipboard) | 0.026% and 0.039% | working set 15.2 MB and 12.4 MB |
| macOS 27.2 (Mac mini; agent A on the real backend, named pasteboard, polling) | 0.067% (A) and 0.033% | RSS 12.4 MB and 10.9 MB |
| Linux (WSL Ubuntu, fake clipboard) | 0.05% and 0.06% | RSS 8.8 MB and 8.9 MB |

## Not done here (open)

- **Cross-OS runner (plan 6.5):** `tools/e2e/cross-os.ps1` is written down to the host check
  and stops with "PENDING" until the lab VMs exist (decision 9, M0a). The reboot checks in
  `win-b`, `mac-b` and the Linux VMs wait for the same.
- **Two users on one computer:** the design gives each user an own pipe (SID in the name),
  socket, state folder and port, but it was not run with a second user account.
- Signed-server check on the Windows pipe, `SMAppService`, the Store startup task and the
  firewall rule: M9.
- The Opus security review with a fresh context covers M3 and M7 together (plan M10).
