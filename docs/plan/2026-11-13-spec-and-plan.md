# Universal Clipboard: product spec and implementation plan

Date: 2026-11-13. Status: revision 3. Revision 2 came from an independent review. Revision 3 applies
Max's scope change of 2026-11-13: local network only (see "Decisions after approval").
Author: planning agent (Fable). Build agents: Opus 5.5 (core, security, platform-native) and
Sonnet 5.5 (UI, glue, packaging scripts, tests, docs).

Inputs:
- Feasibility report: `Documents\Claude\research\tools\2026-11-13-clipsync-consumer-app-feasibility.md`
- Private prototype: `~/.claude/skills/device-transfer` (`clipsync_pc.ps1`, `clipsync.swift`, `pc_clip.ps1`)
- Web checks done today, by the planner and by the reviewer (section 10).
- The review of revision 1. Section 11 lists what changed and what was not taken over.

How to read this: sections 1 to 6 are the spec. Section 7 is the build plan. Section 9 lists the
decisions that are yours. Every choice in the text is a decision unless it says "open" or "unproven".

---

## Decisions after approval (2026-11-13)

Max approved the plan and the build on 2026-11-13. These decisions came after the text below was
written. Where they conflict with the text, they win.

- The name stays "Universal Clipboard". Only an App Store display name may differ later. The
  Copybridge and Clipbridge name check is skipped.
- No "send" and no "get" actions anywhere in the UI or the apps. The latest copy is on every
  device automatically.
- The pairing code has 6 digits (for example `418 207`). The QR code comes first, then the code.
  The PAKE (SPAKE2, decided in M0c, see `docs/protocol.md`) allows 5 tries, so a guess works 1 time in about 200,000. There is
  no long code (removed on 2026-11-13, see the scope change below).
- The desktop UI design is locked: `docs/design/desktop` (12 `.dc.html` screens, Device tiles
  style, Figtree, integrated title bar, 5-step setup indicator). The mobile design is not done.
- Relay: none. This replaces the old relay decision (see the scope change below). Max uses
  Mullvad with "Local network sharing" on.
- Max has an Apple Developer account. There is no Google Play account and no Android phone yet
  (emulator only). There is no Windows signing yet.
- **Scope change (2026-11-13): LOCAL NETWORK ONLY.** This wins over every older line in this file.
  - No relay server, no internet path, no long code, no pairing over the internet, no n0
    discovery or DNS. Devices sync only when they are on the same local network. Nothing of ours
    runs on a server, so there is no server cost and no server to attack.
  - Transport is LAN only: iroh with the relay turned off and no address lookup service (or plain
    QUIC if spike S1 finds iroh not worth it for LAN only). Discovery stays: mDNS plus remembered
    addresses plus address gossip.
  - VPN case: while a VPN is on, the app still tries the physical LAN path (per-interface sockets,
    overlapping-route work stay). If the VPN blocks the LAN completely, no path exists. The app
    detects this and shows help: turn on the VPN's local network sharing (for example Mullvad
    "Local network sharing"). It does not fall back to the internet.
  - Pairing: QR code and the 6-digit code, both on the same local network only.
  - Phones: no "Send to my devices" or "Get from my devices" Shortcuts. The latest copy is on every
    device. iPhone and iPad get it when the app opens. A copy on a closed iOS app is sent through
    the Share sheet. See section 2.6 for the honest limits.
  - Milestone M9b (relay service) and spike S6 are dropped. Decisions 2 and 7 are closed.
- Approved setup work: Android SDK on the PC, Rust and Node on the Mac, the lab VMs (a Hyper-V
  external switch on the PC is fine: the network drops for a few seconds once).

---

## 1. Goal and non-goals

### Goal
Copy on one device, paste on another. For normal people. Setup takes a few minutes, one time:
install, accept a few permissions, add a device with a code. After that the app is invisible and
works without attention.

### What v1 does
- Syncs the clipboard between Windows, macOS and Linux computers, in any mix, automatically, both ways.
- Syncs plain text and one image. Formatted text (HTML) is planned for v1.0 but may move to v1.1 (M11).
- Groups of 2 to 8 devices. All devices in a group share one clipboard.
- iPhone, iPad and Android are in v1, with the limits that iOS and Android set (section 2.6).
- Works when a device runs a VPN and is on the same physical network, if the VPN allows local
  network traffic. If it does not, the app says so and shows how to turn it on (section 3.5).
- No account and no cloud storage. Content is end-to-end encrypted between your devices.
  **There is no server at all.** Devices talk directly to each other on the local network
  (sections 3.14 and 4.1).
- No system tray icon. A background agent starts at login. Opening the app shows the settings window.
- Skips copies that a password manager marks as private.
- Updates itself, with a rollback if an update is bad.

### Non-goals for v1
- No clipboard history. The app holds only the newest clip, in memory.
- No file sync (copied files or folders are ignored). LocalSend does that job.
- No accounts, no web app, no browser extension.
- No sharing between different people. A group is one person's devices.
- No automatic iPhone to computer sync without a tap. iOS does not allow it.
- No iOS keyboard extension and no Android accessibility service in v1 (section 9).
- No Flatpak, Snap or AppImage. No `.rpm` in the first release (section 8.1). No Mac App Store build.
- No Intel Mac build (it cannot be tested: there is no Intel Mac).
- No Android APK outside Google Play.
- **Local network only (2026-11-13).** No relay server, no internet path, no pairing over the
  internet, no long code. Devices on different networks do not sync.

### Minimum OS versions

| Platform | Minimum | Why |
|---|---|---|
| Windows | Windows 11 (tested). Windows 10 22H2 installs and should work, but is not tested and not promised. | WebView2, clipboard history formats. Windows 10 is out of support. |
| macOS | macOS 14, Apple silicon only | `SMAppService`, one test line |
| Linux | Ubuntu 22.04 or newer and other distributions with WebKitGTK 4.1. Supported desktops: section 3.9. | Tauri 2 |
| Android | Android 10 (API 29). Target SDK 37. | Clipboard rules, Play target rule |
| iOS, iPadOS | 18 | Current APIs for Local Network and pasteboard privacy |

---

## 2. User experience

Design rule: one window, few words, no technical terms. The UI never shows clipboard content.
It shows only facts like "Text, from Max's Mac, 2 minutes ago".

UI work starts with mockups that you approve (your global rule). See milestone M8a.

### 2.1 Download and install

| OS | What the user does | What they see |
|---|---|---|
| Windows | Installs from the Microsoft Store (first choice, if spike S8 passes), or downloads `Setup.exe` from GitHub | Store: normal install, no warning. GitHub exe: per-user install, no admin prompt, and a SmartScreen warning until a signing certificate is approved (section 8.1). The app opens when the install ends. |
| macOS | Downloads the `.dmg`, drags the app to Applications, opens it | Notarized, so only the normal "downloaded from the internet" question. |
| Linux | Installs the `.deb` | The package adds our apt repository, so the system updater keeps the app current. The app appears in the app menu. |
| iPhone, iPad | Installs from the App Store | Normal install. |
| Android | Installs from Google Play | Normal install. |

### 2.2 First run on a computer (3 screens)

**Screen 1: Welcome.** One sentence: "Copy on one device, paste on another." One button: Continue.
A small link opens the privacy policy.

**Screen 2: Permissions.** A short list. Each row has a one-line reason and a button. The OS prompt
appears only after the user presses the button, so it is never a surprise. A green check appears
when a row is done.

| OS | Row | OS prompt | Why | If refused |
|---|---|---|---|---|
| Windows | Let your devices find this PC | One UAC prompt. It adds one firewall allow rule for the agent, for Private AND Public network profiles (many home Wi-Fi networks are set to Public). With the Store build the rule comes from the package and there is no prompt (S8). | Other devices can connect to this PC. | The agent runs outbound-only. Sync still works when the other device accepts connections. |
| Windows | Start with Windows | None. | The app works after a restart. | Sync stops after a restart until the app is opened. |
| macOS | Find devices on your network | "Local Network" prompt. | Needed to reach devices on the LAN. | Sync does not work on the network until this is allowed. The row shows how to turn it on in System Settings. |
| macOS | Read what you copy | Onboarding makes one clipboard read on purpose, so the app appears in "Paste from Other Apps". The row then opens that settings page, where the user picks Allow. Shown only if macOS needs it (section 3.9). | The agent must read the clipboard without a prompt each time. | Sync from this Mac does not work. Sync to this Mac still works. |
| macOS | Start at login | A system notice "Login item added". No question. | Works after a restart. | Same as Windows. |
| macOS | Tell me when sync stops (optional) | Notification prompt. | The app has no tray icon, so this is the only way to warn. | No warnings. The settings window still shows the state. |
| Linux | Start at login | None. | Works after a restart. | Same as Windows. |

**Order rule on Windows:** the agent does not start its network endpoint before the user has
answered the firewall row. If it listened earlier, Windows could show its own "Windows Security
Alert" from an invisible process, and a dismissal there creates a block rule that beats our allow
rule (reviewer's knowledge, verified in S1).

Windows clipboard access and Linux clipboard access on supported desktops need no permission.
On a Linux desktop that we cannot support, the app says so on this screen and stops (section 3.9).

**Screen 3: Add a device.** Two choices on one screen:
- "This is my first device" shows a QR code and a 6-digit code (section 2.3).
- "I have a code" shows one input field.

When pairing ends, the window shows "Done. You can close this window. Copy something to try it."
Closing the window leaves the agent running. There is no tray icon and no Dock icon.

### 2.3 Adding a device with a code

Both devices must be on the same local network. The app says so on this screen.

On a device that is already set up (or the first device):
1. Open the app. Press "Add a device".
2. The screen shows a **QR code** first and a **6-digit code** under it, for example `418 207`.
   Both are valid for 10 minutes. The 6-digit code allows 5 tries.

On the new device:
3. Phones and tablets: scan the QR code, or type the 6 digits.
4. Computers: type the 6 digits.
5. Both screens show "Connected to <device name>". Done.

Rules:
- **Pairing works only on the same local network.** Nothing about a pairing is ever published
  outside the LAN (section 3.6 says why).
- Either device can show the code. A third device can be added from any device in the group.
- There is no second confirm step. The code check itself proves both sides (section 3.6).
- A new device takes a default name from the OS ("Max's PC"). The user can rename it later.
- If the new device already belongs to another group, it asks "Leave your other devices and join
  these?" before it continues. Groups never merge in v1.
- Wrong code: "That code is not right. Check it and try again." After 5 tries: "Make a new code on
  the other device." The device that shows the code says "Someone else tried this code" if a try
  failed that the user did not make.
- No device found within 20 seconds: "Cannot find the other device. Check that both devices are on
  the same Wi-Fi or network. If one of them uses a VPN, turn on Local network sharing in the VPN."

### 2.4 Daily use on computers
Nothing to do. Copy on one, paste on another, usually within one second on a LAN.
A computer that wakes from sleep, or joins the network again, gets the group's newest clip
automatically if that clip is less than 1 hour old. Older clips are not applied on their own, so a
laptop that was away does not lose its own clipboard to something old. (When a phone opens the app,
it takes the newest clip with no age limit, section 3.8.)

### 2.5 The settings window (opening the app again)

One window, about 420 x 560 px.

- **Top:** state line. One of: "Working", "Paused", "No other device is on right now",
  "Needs attention" (with a Fix button).
- **Devices list:** name, type icon, state ("Connected", "Asleep or away", "Blocked by a VPN or firewall"). Each row has a menu: Rename, Remove.
- **Add a device** button.
- **Pause** switch, with "Pause for 1 hour" and "Until I turn it on".
- **Settings** (collapsed):
  - Start at login (on).
  - Sync images (on).
  - Keep formatting (on, when M11 ships). Off means plain text only.
  - macOS only: "Also pass on what Apple's Universal Clipboard brings" (section 3.8, decision 8).
  - This device's name.
- **Last activity:** kind, direction, device, time. Never the content.
- **Help:** "Copy a diagnostic report" (no clipboard content, no keys), version, privacy policy,
  "Remove this device and forget everything".

**Remove a device:** confirm dialog. The device disappears on all devices in the group. The removed
device shows "This device was removed" at its next start and returns to the first-run screen.

**Remove this device and forget everything:** tells the group that this device leaves, wipes the
key, the state and the autostart entry, and (Windows exe) offers to remove the firewall rule.
This is the documented clean way to leave on every OS (section 8.5).

### 2.6 Phones and tablets, stated honestly

iOS and Android do not let an app read the clipboard in the background. Android lets an app write it
in the background (to be proven in spike S7). iOS does not let an app run in the background for
this at all. There are no "Send" or "Get" buttons or Shortcuts: the latest copy is on every device,
and the app does the moving. The limits are the ones that Apple and Google set:

| Direction | Android | iPhone and iPad |
|---|---|---|
| Computer to phone | **Automatic** while the phone is awake and on the same network, if S7 passes. A small permanent notification is required by Android. After deep sleep the phone catches up when it wakes. Fallback if S7 fails: a notification "Tap to copy". | **When the app opens.** The app takes the group's newest clip and puts it on the clipboard. Nothing happens while the app is closed. |
| Phone to computer | **When the app opens** (it reads the clipboard once, in the foreground, and shares it if it is new). From a closed app: the Share sheet entry "Universal Clipboard", which sends a copy of the shared text or image. | **When the app opens** (with the iOS paste setting on "Allow", no prompt; else one "Paste" prompt). From a closed app: the Share sheet entry, which sends a copy of the shared text or image. |
| Phone to phone | Same as the rows above, per side. | Same, and see the limit below. |

Honest limits, stated in the app: a phone that is closed does not send what you copied in another
app. On iPhone and iPad, a copy made in another app reaches the other devices only after you open
our app or use the Share sheet. This is Apple's rule, not ours.

**Limit that the app states plainly: a group needs at least one computer (or Android phone) that
is on, and on the same network as the phone.** An iPhone or iPad has no background agent. It can
only talk to a member that is running at that moment. A group of only iPhones and iPads can never
sync. If no always-on member is reachable, the phone shows "No computer is on right now on this
network. Phones can only sync while one of your computers is on." with a Retry button. It never
shows a silent empty result.

The app says the rule in plain words on the phone's first-run screen: "Open the app to sync your
phone. Apple and Google do not allow more."

**What we do not build (changed on 2026-11-13).** No App Intents, no Shortcuts, no Quick Settings
tile, no "Send to my devices" text-selection entry. The only share entry is the standard Share
sheet, with the app's own name, and it sends a copy. This is checked on a real iPhone in spike S5,
because the Simulator does not enforce pasteboard and Local Network privacy.

**iPhone and iPad first run:**
1. Welcome.
2. Permissions: "Local Network" (needed to find your computers; without it the phone cannot sync).
   Camera (asked only when the user presses "Scan QR code"; typing a code needs none).
3. Scan the QR code from a computer, or type the 6 digits. Both devices must be on the same network.
4. A "How it works" page: "Open the app to sync. Use the Share sheet to send a copy from another
   app." Two short animations. Can be skipped.
5. Optional: iOS Settings > Universal Clipboard > Paste from Other Apps > Allow. With this set, the
   app reads the clipboard when it opens, with no paste prompt.

**Android first run:**
1. Welcome.
2. Permissions, each a fixed row with a refusal path:
   - Notifications (Android 13+), for the required service notification.
   - **Nearby devices / local network (`ACCESS_LOCAL_NETWORK`)**, required at target SDK 37.
     Without it every LAN packet fails, so the phone cannot sync. If refused: the row shows how to
     grant it later.
   - No camera permission: QR scanning uses Google's code scanner, which needs none.
3. Scan the QR code, or type the 6 digits.
4. A "How it works" page: the phone syncs when the app opens, and the Share sheet entry sends a
   copy from another app.
5. A plain note: "Keep battery saving off for this app if the phone misses clips." With a button
   that opens the right settings page.

### 2.7 What users see when something breaks

There is no tray icon, so the rules are strict:
- The app sends an OS notification only when the user must act: permission lost, device removed,
  this device was removed by another device, update needs a manual step, sync blocked for more
  than 24 hours on a network where it worked before. At most one notification per day.
  (iOS has no such notifications: without a push server the app cannot notify while it is closed.)
- All other problems heal on their own (retry with backoff) and are visible only in the settings window.
- Every "Needs attention" state has one sentence and one Fix button.

| Problem | Text in the settings window | Fix button |
|---|---|---|
| macOS Local Network refused | "macOS blocks this app from your network." | Opens the right System Settings page. |
| macOS paste access set to Ask or Deny | "macOS asks before this app can read what you copy." | Opens the right System Settings page. |
| **No path at all** (the LAN is blocked on a device or by the network) | "Your devices cannot reach each other on this network." | "Show help". It links to the cause below. |
| Cause: VPN blocks the LAN | "Can't reach your other device? Your VPN blocks the local network." (desktop screen L12) | Help: "Turn on Local network sharing in your VPN." The help page shows where it is in common VPN apps (for example Mullvad "Local network sharing"; list checked in S1). |
| Cause: Wi-Fi client isolation or a guest network | Help text: "This Wi-Fi keeps devices apart (guest network)." | Help: use the main Wi-Fi, or turn off client isolation in the router. |
| Cause: both Windows PCs refused the firewall row | Help text: "Both PCs block incoming connections." | "Let your devices find this PC" (the firewall row again). |
| This device's clock is wrong | "The date and time on this device are wrong, so its copies are ignored." | Opens the date and time settings. |
| This device was removed by another device | "Another device removed this one." | "Set up again". If it removed all others (stolen device case): "Pair my devices again". |
| Peer offline | "Asleep or away." | None. Not an error. |
| Clip too big | Last activity shows "Image was too large to sync (limit 25 MB)." | None. |
| Agent not running | The window starts it. If it fails twice: "The background service cannot start." | "Copy a diagnostic report". |
| Linux desktop without background clipboard access | "This desktop blocks background clipboard access." | Link to the help page for that desktop. |
| Update failed and was rolled back | Nothing, unless it fails 3 times: "The app could not update itself." | "Download the newest version". |

---

## 3. Architecture

### 3.1 Components

```
+--------------------- one device (desktop) ----------------------+
|  uc-ui (Tauri 2 window)  <-- local IPC -->  uc-agent (headless)  |
|   settings, onboarding                       |                   |
|                                              +-- uc-core   (sync logic, state)        |
|                                              +-- uc-net    (iroh, LAN discovery)      |
|                                              +-- uc-pair   (PAKE, membership)         |
|                                              +-- uc-clip-* (OS clipboard backend)     |
|                                              +-- uc-keys   (OS key store)             |
|                                              +-- uc-update (signed update, rollback)  |
+------------------------------------------------------------------+

Phone: native app (SwiftUI or Kotlin) + extensions  --> uc-mobile (UniFFI) --> uc-core, uc-net, uc-pair

Network: the local network only. No server of ours, no relay, no internet path.
```

- **uc-core** (Rust library): clip model, wire codec, clock, echo and loop rules, size caps, the sync
  state machine. No OS calls. Fully testable with a fake clipboard and a fake network. It has two
  roles from the start: **full member** (desktop, Android service) and **pull-only client** (iOS,
  extensions), so phones do not force a redesign later.
- **uc-net**: wraps iroh (LAN only). Interface classification, LAN discovery, address book, reconnect,
  VPN-blocks-LAN detection.
  mDNS and socket binding sit behind traits, so iOS (`NWBrowser`) and Android (`NsdManager`,
  `Network.bindSocket` with a passed file descriptor) can plug in their own.
- **uc-pair**: pairing protocol and the group membership chain.
- **uc-clip-win, uc-clip-mac, uc-clip-linux**: one clipboard backend per OS behind one `Clipboard` trait.
- **uc-keys**: one trait, one backend per OS key store, plus a file backend for test mode.
- **uc-update**: update check, signature check, staged rollout, health check, rollback.
- **uc-agent**: the headless background program. One per logged-in user. Owns all state.
- **uc-ui**: Tauri 2 app. Only a window. Talks to the agent over local IPC. Holds no secrets.
- **uc-mobile**: UniFFI bindings for Swift and Kotlin.
- **Mobile apps**: native shells. iOS: SwiftUI app, Share Extension, App Intents. Android: Compose
  app, foreground service, share target.

**Why two processes on desktop.** The agent is small (no web view, about 15 to 30 MB of memory),
can be tested headless, and keeps running when the window closes. A hidden Tauri app would keep a
web view runtime loaded for years.

**Why native mobile shells and not Tauri mobile.** The value on phones is in the extensions (Share
sheet, service). Those are native targets that Tauri does not create or manage,
and a web view does not fit an iOS extension's memory limit. The phone UI is three screens. So the
phone apps are native, and all logic stays in the shared Rust core.

### 3.2 Process model per OS

| OS | Agent | Autostart | UI | Notes |
|---|---|---|---|---|
| Windows | `uc-agent.exe`, GUI subsystem, no window, one message-only window for clipboard events | Exe build: `HKCU\...\Run` value, set by the app. Store build: a startup task in the package. Both show in Task Manager "Startup apps". | `Universal Clipboard.exe` (Tauri). Single instance. | Exe build installs per user in `%LOCALAPPDATA%`. |
| macOS | `uc-agent` inside the app bundle (`Contents/MacOS`), runs as a LaunchAgent in the user's GUI session | `SMAppService.agent(plistName:)` with the plist in `Contents/Library/LaunchAgents`. `KeepAlive` on crash. | The app bundle's main binary. Dock icon only while the window is open. | The agent must be signed with the same team and bundle so that macOS shows one app name in every prompt. Verify in spike S2. |
| Linux | `uc-agent --supervise`: a tiny parent that restarts the real agent if it crashes (with backoff) | **XDG autostart `.desktop` file in `~/.config/autostart`, on every desktop.** It always runs inside the session, so `DISPLAY` and `WAYLAND_DISPLAY` are set. No systemd unit: many X11 desktops never start `graphical-session.target`. | `universal-clipboard` (Tauri). | Tested on each supported desktop in M6. |
| Android | Foreground service (type `connectedDevice`, to be confirmed in S7) in the app process | Starts on boot (`RECEIVE_BOOT_COMPLETED`) and when the app opens. | Compose activity. | The service notification is permanent and low priority. |
| iOS, iPadOS | None. The app and its extensions run only when the user acts. | None. | SwiftUI app. | App and extensions share one identity through a shared Keychain group. |

**Local IPC** (desktop): JSON lines, request, reply, and an event stream.
- Windows: named pipe `\\.\pipe\uclip-<user SID>`, ACL for that user only, created with
  `FILE_FLAG_FIRST_PIPE_INSTANCE` so no other program can take the name first. The UI checks that
  the pipe server is our signed agent running under the same SID before it sends anything.
- macOS and Linux: Unix socket, mode 0600, in `~/Library/Application Support/uclip/` or
  `$XDG_RUNTIME_DIR/uclip/`. The UI checks the peer's user ID.
- Calls: `status`, `subscribe`, `pair.start`, `pair.join`, `pair.cancel`, `devices.list`,
  `devices.rename`, `devices.remove`, `pause.set`, `settings.get`, `settings.set`, `diagnostics`,
  `leave`. The same API drives the headless tests through a small CLI, `uclip`.
- Two users logged in on one computer: each has an own agent, pipe or socket, state folder and UDP
  port. If the chosen UDP port is in use, the agent picks a new one and tells its peers.

When the UI starts and no agent answers, the UI starts the agent.

**Profiles.** `UCLIP_PROFILE=<name>` gives the agent and the UI a separate state folder, pipe or
socket name, key store entry and UDP port. Test installs on your PC and Mac use the profile `dev`
until M16 (section 7, rules), so they never fight with the private clipsync prototype.

### 3.3 Transport

**Local network only (2026-11-13).** There is no relay, no hole punching over the internet and no
lookup service.

- **iroh 1.x** (QUIC, TLS 1.3, peers are dialed by their Ed25519 public key), used with the relay
  turned off (`RelayMode::Disabled`) and no address lookup service (no n0 DNS, no pkarr). Today's
  version on docs.rs is 1.4.1. The 1.0 line promises a stable wire protocol and API. If spike S1
  picks the unstable custom-transport feature, the exact iroh version is pinned. If S1 shows that
  iroh adds little for LAN only, plain QUIC (`quinn` with our own TLS key check) is the fallback;
  the device key and the wire protocol stay the same.
- ALPN `uclip/1` for sync, `uclip-pair/1` for pairing.
- One long-lived QUIC connection per pair of online full members. Full mesh. A group has at most 8
  devices, so at most 7 connections per device. Pull-only clients (iOS) connect on demand.
- Only one path type: direct LAN. A peer is dialed by its remembered LAN addresses (section 3.4).
- Keep-alive every 15 s. A dead path is found in under 30 s.
- On wake from sleep, on network change, and on interface change the agent reconnects at once.

### 3.4 Discovery on the LAN

A device finds its peers with three methods. All of them feed one address book
(`uc-net::LanBook`), which iroh uses as an address source.

1. **Remembered addresses.** At pairing, and on every connection, devices exchange their current
   LAN addresses (IPv4 and IPv6 link-local) and their UDP port. The agent keeps a fixed UDP port,
   chosen at first run, so old addresses stay valid after a restart. Reconnect starts by dialing
   every remembered address directly. This needs no multicast.
2. **Address gossip.** When a device's address changes, it tells every connected peer. Peers pass
   the news on to the other members. The update is signed by the device key.
3. **mDNS / DNS-SD** (`_uclip._udp.local`), sent and received **on each physical interface**
   separately (section 3.5). The service instance name is a rotating tag:
   `HMAC(group secret, 10-minute time slot)`, truncated to 12 bytes. Only group members can
   recognise it. A stranger on the same Wi-Fi sees no stable ID and no device name.

If all three fail, the devices cannot find each other. The app then shows the "No path at all"
state and its help (sections 2.7 and 3.5). There is no other path.

There is **no subnet sweep** (cut after review: up to 1,024 probes look like a port scan to
company security tools, and your own PC is onboarded to Defender for Endpoint).

On iOS the app uses `NWBrowser` (Bonjour) for method 3. That needs only the Local Network
permission and the `NSBonjourServices` key, not the special multicast entitlement.
On Android it uses `NsdManager`, which needs `ACCESS_LOCAL_NETWORK` at target SDK 37.

### 3.5 VPN on the same network

The requirement: device A runs a VPN, device B is on the same physical LAN, sync should work when
the VPN allows it.

**The honest picture.** Some VPNs let LAN traffic pass and some block it. We do not know the
default of each VPN (Mullvad, for one, ships with local network sharing off, and Max has it on).
When the VPN blocks the LAN completely, no app can use the LAN, and **this app has no other path**
(scope change of 2026-11-13). So the app must do two things: try the physical LAN path while a VPN
is on, and detect the blocked case and give clear help. Spike S1 checks the default LAN setting of 4
or 5 large consumer VPNs, and the result goes into the help page.

**Detecting "the VPN blocks the LAN".** The app knows a peer's remembered LAN address and that a VPN
interface is up on this device (or on the peer, which it learned at the last connection). If no
direct connection works for 20 seconds, the peer was seen on this LAN before, and a VPN interface is
up, the device row shows "Blocked by a VPN or firewall" and the Fix button opens the VPN help: "Can't
reach your other device? Turn on Local network sharing in your VPN." (Desktop screen L12.) The same
help covers an iOS VPN with `includeAllNetworks` and without `excludeLocalNetworks`. Spike S1 defines
and tests the detection rule (false alarms and misses), per OS.

| Case | What breaks | What we do | Result | Status |
|---|---|---|---|---|
| Full-tunnel VPN that allows local network access | Multicast leaves through the tunnel interface. The default socket picks the VPN's source address. | Use one UDP socket per physical interface address. Send mDNS and sync traffic from those sockets. Dial remembered addresses. | Direct LAN path. | To prove in S1 per OS |
| VPN pushes a route that overlaps the LAN subnet | Unicast to the peer goes into the tunnel. Binding to an address does not beat the route table on macOS and Linux. | Needs a socket tied to the device, which stable iroh cannot do (below). | Direct LAN path if S1 finds a way. Else this case shows the VPN help. | **Unproven** |
| VPN with kill switch or "block LAN" (also iOS VPNs with `includeAllNetworks` and without `excludeLocalNetworks`, and many company VPNs) | All LAN traffic is dropped by the VPN's firewall. No app can pass it. | Detect it (above) and show the help "Turn on Local network sharing in your VPN". | No sync until the user changes the VPN setting. The help is clear. | Help path, sure |
| Wi-Fi client isolation, guest network | Same effect as a LAN block. | Help text for guest networks (section 2.7). | No sync on that network. | Help path, sure |
| Split-tunnel VPN | Usually nothing. | Same code path. | Direct LAN path. | |
| Mesh VPN on both devices (Tailscale, ZeroTier) | Nothing. | The mesh addresses are LAN-like private addresses. The app may dial them if they are remembered. | Direct path. | To check in S1 |

**What iroh can and cannot do (reviewer checked docs.rs for iroh 1.4.1 today).** The endpoint
builder takes socket addresses with a prefix length (`bind_addr_with_opts`). It cannot take a
socket that we made, and it cannot set `IP_BOUND_IF`, `SO_BINDTODEVICE` or `IP_UNICAST_IF`. The
only hook for our own sockets is `add_custom_transport`, behind the feature
`unstable-custom-transports`, which may change without a major version.

**Spike S1 tests three options and picks one, on Windows AND macOS AND Linux, with a real
overlapping route** (the answer can differ per OS):
- (a) Address binding only, with the stable API. Simple. May lose to the route table.
- (b) The unstable custom transport with our own device-bound sockets. Pin the iroh version.
- (c) A small fork or an upstream patch that lets the builder take a pre-bound socket.
If none gives a direct path under an overlapping route, that case shows the VPN help and the plan says so.

**Physical interface detection** (in `uc-net::ifaces`, refreshed on every network change):

| OS | Physical means | How a socket is tied to it (used by options b and c) |
|---|---|---|
| Windows | `GetAdaptersAddresses`: `IfType` Ethernet or IEEE 802.11, `OperStatus` up, not a tunnel type, hardware interface flag set | Bind to the interface address, plus `IP_UNICAST_IF` and `IP_MULTICAST_IF` |
| macOS | `getifaddrs` plus `SCNetworkInterface` type Ethernet or Wi-Fi. `utun*`, `ipsec*`, `ppp*` are never physical. | `IP_BOUND_IF` |
| Linux | `/sys/class/net/<if>/device` exists, type is not tun, wireguard, bridge or veth | `SO_BINDTODEVICE` (allowed without root on kernel 5.7+), else bind to the address |
| Android | `ConnectivityManager` networks with `TRANSPORT_WIFI` or `TRANSPORT_ETHERNET` and without `TRANSPORT_VPN` | `Network.bindSocket()` on a file descriptor that the Rust core hands to Kotlin |
| iOS | `NWPathMonitor`, interface type `.wifi` or `.wiredEthernet` | Network.framework is used **only for the Bonjour browse**. Sync traffic uses iroh's own UDP sockets. Those are tied to Wi-Fi with `IP_BOUND_IF` on the socket if S1 picks option (b) or (c), else by address. Checked in S5. |

When the set of interfaces changes, the agent rebuilds the endpoint. Reconnect takes under a second.

VPN tests: simulated in Linux network namespaces (section 6.3), and on real Windows and macOS in
the lab VMs already in M2, not first in M10 (section 6.5).

### 3.6 Pairing protocol

**The exact protocol is in `docs/protocol.md` (signed off in M0c on 2026-11-14). M3 implements
that page word for word. Where this section and that page differ, that page wins.** This section
is only the summary.

Goal: a code that a person can type, safe against an attacker on the same network, and no way at
all for an attacker on the internet to find or attack a pairing.

| | Short code | QR code |
|---|---|---|
| Form | 6 digits, all secret. No public part. | The inviter's device key, its port, up to 4 LAN addresses, and a 128-bit secret |
| Works | **Same LAN only** | Same LAN only |
| How the new device finds the inviter | mDNS pairing service on each physical interface, one random pairing tag per open pairing; the new device tries each pairing it sees | Needs no discovery: the address is in the QR. The new device checks the inviter's key before it sends anything. |
| Guess limit | 5 tries, 10 minutes | 10 minutes (guessing 128 bits is not possible) |

- **PAKE: SPAKE2**, RustCrypto `spake2` pinned to `=0.4.0` (decided in M0c; CPace has no
  maintained Rust crate and is not an RFC yet). Both device keys are the SPAKE2 identities.
- **Transcript and confirmation:** both device keys (read from the TLS layer), a TLS exporter
  value of this connection, the mode and both SPAKE2 messages are hashed. Each side sends an HMAC
  with its own confirmation key. **The inviter confirms first.** The new device sends nothing
  that depends on the code before it has checked that. So a fake inviter learns nothing, and a
  fake new device gets one guess per try.
- **Limits:** 5 tries per code, counted when the inviter sends its reply (a dropped connection
  stays counted); 10 minutes; one open pairing. After 5 codes in a row die from wrong tries, the
  short code is off for 1 hour on that device and only the QR code is shown. A guess works at most
  5 times in 1,000,000 (1 in 200,000) per code, and only from the same LAN.
- **A 6-digit code is safe only with the PAKE and the try limit together** (protocol page 4.1).
- **LAN only, enforced:** the inviter accepts a pairing only from a private or link-local address
  inside the subnet of one of its physical interfaces. The QR payload is taken only from the
  in-app scanner; the app registers no link scheme.
- **No second confirm code:** the PAKE with key confirmation already proves that both sides know
  the code and see the same keys.

### 3.7 Groups and membership

**The exact rules, byte formats and cases are in `docs/protocol.md` section 5 (signed off in M0c).
Where this section and that page differ, that page wins.** Summary:

- Every device has one Ed25519 key pair, made at first run. Its public key is its ID and its iroh address.
- A group is a **membership chain**: signed changes (genesis, add, remove, rename, rotate). Each
  change names the hashes of its parents, so the changes form a hash-linked graph.
- **Removal always wins.** A removal counts if its signer was a member in the removal's own past.
  Nothing beside it can cancel it. A removed key is never a member again; a removed device makes
  a new key.
- **A removed device cannot add devices.** Every add, rename or rotate signed by a removed device
  that is not in the past of its removal is invalid. That covers changes signed after the removal
  and back-dated ones. Devices added that way are not members.
- **Two devices remove each other at the same moment: both are removed** (changed in M0c: with
  "the older device wins", a stolen old device could cancel its own removal by back-dating).
- All members are equal. Any member can add or remove a device. This fits one person's devices.
- On every connection, membership sync runs first. Clips, secrets and addresses flow only after
  both sides evaluated the result and see each other as members.
- The agent accepts sync connections only from keys that are valid members.
- **Group secret:** 32 random bytes with an epoch, used only for the discovery tags. The device
  that signs a removal also makes the new secret and signs a `rotate` change with a commitment to
  it. Members get the secret over the pairwise channels (`WantSecret`, `Secret`).
- A device wipes itself only when its own evaluation of the signed changes says it is no longer
  a member. It then shows "Another device removed this one" and "Pair my devices again".
- **A stolen device removes all the others.** It gains nothing: the others stop sending to it.
- **Honest limits:** a member that is offline during a removal learns of it at its next connection
  to any other member. Until then it still accepts the removed device, and the removed device can
  use it to make other devices drop out (never to get back in). If A removes B while B adds C at
  the same time, C is not a member and is paired again.
- Maximum 8 devices in v1.

Property tests cover exactly the attacks above (section 6.1 and protocol page section 7).

### 3.8 Message format and sync rules

**Encoding:** `postcard` (compact binary, serde). Every stream starts with a protocol version byte.
Unknown message types are skipped. Devices accept the current and the previous protocol version.

**Messages on `uclip/1`:**

| Message | Content |
|---|---|
| `Hello` | protocol version, app version, device record, role (full member or pull-only), chain heads, group epoch |
| `Membership` | missing membership changes |
| `AddrUpdate` | signed list of a device's LAN addresses and port |
| `Latest` | metadata of the newest clip this device holds (sent on connect) |
| `Clip` | clip header plus parts: id, origin device, clock stamp, list of (format, size, hash), then the bytes |
| `Want` | "send me clip id X" (used after `Latest`, for large clips, and by pull-only clients) |
| `Removed` | "you were removed from the group" (a hint only; the device acts on the signed changes) |
| `WantSecret`, `Secret` | ask for and send the group secret of an epoch (added in M0c, `docs/protocol.md` 5.8) |

**Clip model:** one clip has 1 to 3 parts: `text/plain` (UTF-8), `text/html`, `image/png`.
- **Wire ID:** BLAKE3 over the parts as sent.
- **Content hash:** BLAKE3 over a normalized form: text with line endings turned into LF and
  trailing NUL removed; image as decoded pixels (width, height, RGBA). The HTML part is not in it.
- **Line endings:** text on the wire uses LF. The Windows backend turns LF into CRLF on write and
  CRLF into LF on read. macOS and Linux use LF.

**Sending:** when the local clipboard changes and passes the checks, the agent sends the clip to
every connected peer. Clips up to 256 KB are pushed at once. Larger clips are announced with
`Latest`, and the peer asks for them with `Want`. So a phone on a slow Wi-Fi can skip a big image.

**No queue, short memory:** nothing is written to disk. Each full member keeps only the newest clip
in memory. On connect, both sides send `Latest`.
- **Automatic apply** (a computer or Android phone that wakes or reconnects): the newer clip is
  applied if it is less than **1 hour** old. Reason for a limit: a device that was away should not
  have its own clipboard replaced by something old.
- **Fetch on open** (opening the phone app, or `uclip fetch` on a desktop): always returns
  the group's current clip, **with no age limit**, as long as one always-on member on the same
  network still holds it. This is not a button: it happens when the app opens.
- Age is measured with the hybrid clock below, not with each device's raw wall time.

**Conflict rule:** last writer wins. Every clip carries a hybrid logical clock stamp (wall time
plus a counter, plus the device ID as tie-break). A device applies a clip only if its stamp is
higher than the stamp of the clip it has now. All devices end with the same clip, also when two
copies happen in the same second.
**Wrong clocks:** a device refuses a stamp that is more than 2 minutes ahead of its own clock. So
one device with a clock set far ahead cannot win every conflict. That device sees its clips
refused and shows "The date and time on this device are wrong" (section 2.7). A clock that is
behind does no harm: the hybrid clock never goes below the highest stamp it has seen.

**Echo and loop suppression** (four layers):
1. When the agent writes a received clip to the OS clipboard, it adds a private marker format
   (`UniversalClipboard.Origin` on Windows, `io.github.maxaubert.uclip.origin` on macOS,
   `application/x-uclip-origin` on Linux). A change that carries the marker is never sent.
2. It also records the clipboard sequence number (Windows), `changeCount` (macOS) or its own
   selection ownership (Linux) of that write, and skips that event.
3. Protocol level: a device never sends a wire ID that it received, and never applies one it
   already holds. A small list of the last 64 IDs backs this up.
4. **Content level:** a clipboard manager can rewrite a clip (it drops our marker and re-encodes
   the bytes), which gives a new wire ID. So the agent also keeps the content hashes of the last
   64 clips it applied or sent, and never sends a clip whose content hash is in that list. Without
   this, two devices with clipboard managers could send the same clip back and forth for ever.

**Copied files:** if the clipboard holds a file reference (`CF_HDROP` on Windows, `public.file-url`
on macOS, `text/uri-list` with `file://` on Linux), the whole clip is skipped. The file name text
and the icon image that some apps add next to it are not synced.

**Settle time:** apps often write the clipboard in several steps. The agent waits 150 ms after the
last change event before it reads. If another app holds the clipboard, it retries 5 times over 500 ms.

**Double delivery with OS features.**
- *Our writes:* macOS writes use `prepareForNewContents(with: .currentHostOnly)`, iOS writes use
  `.localOnly`, Windows writes carry `CanUploadToCloudClipboard = 0`. So Apple's Universal
  Clipboard and Microsoft's cloud clipboard do not send our clips again. On Windows they stay
  visible in Win+V history.
- *Clips that arrive through Apple's Universal Clipboard:* a clip that reaches a Mac through
  Apple's Handoff also changes `changeCount`. The agent detects it by the pasteboard type
  `com.apple.is-remote-clipboard` (to verify in S2). Two possible behaviours:
  **skip** it, or **forward it on purpose**. Forwarding gives Apple users automatic
  iPhone to Mac to PC sync with no tap, which is the one thing iOS itself does not let us build.
  Cost: reading it makes macOS fetch the data from the iPhone each time. This is decision 8.
  Default in the plan: a setting, **on if S2 shows the read is fast and silent, else off**.
  The content hash (layer 4) stops double delivery when two Macs in the group both get the clip.
- *Clips that arrive through Windows cloud clipboard:* M4 checks whether such a clip can be told
  apart. If not, the content hash (layer 4) is the protection.

### 3.9 Clipboard backends

| OS | Watch | Read | Write |
|---|---|---|---|
| Windows | `AddClipboardFormatListener` on a message-only window, `WM_CLIPBOARDUPDATE` | `CF_UNICODETEXT`, `HTML Format`, registered `PNG`, else `CF_DIBV5` or `CF_DIB` converted to PNG | All formats in one open/close, plus the marker |
| macOS | Poll `NSPasteboard.general.changeCount` every 250 ms. This reads no content and shows no prompt. | `public.utf8-plain-text`, `public.html`, `public.png`, else TIFF converted to PNG | `public.utf8-plain-text`, `public.html`, PNG and TIFF, plus the marker |
| Linux X11 | XFIXES selection-owner events on `CLIPBOARD` (`x11rb`) | `UTF8_STRING`, `text/html`, `image/png` | The agent becomes selection owner and serves the data |
| Linux Wayland (KDE Plasma 6, Sway, Hyprland, other wlroots, COSMIC) | `ext-data-control-v1`, else `wlr-data-control-unstable-v1` | Same MIME types | Data-control source |
| Linux Wayland (GNOME) | Decided by spike S4, in this fixed order: (1) the X11 path through XWayland, (2) the RemoteDesktop portal clipboard. | | |

**GNOME on Wayland is a go/no-go for the word "Linux".** It is the default desktop of Ubuntu and
Fedora, and recent GNOME releases no longer ship an X11 session (reviewer's knowledge; S4 verifies).
- S4 first tests XWayland: do XFIXES events arrive for every copy, also when no X11 window has
  focus, and can our X11 client set the clipboard for Wayland apps?
- If not, S4 tests the portal: does it work with no prompt after the first grant, across reboots?
- A GNOME Shell extension is **not** an option in v1. It needs a review on extensions.gnome.org and
  breaks at GNOME releases, which is the opposite of "never touch it again".
- If neither path works: the README and the download page say **"Linux: KDE and X11 desktops only.
  GNOME on Wayland is not supported."**, the app says so at first run, and the build stops to ask
  you before it continues (decision 6).

The primary selection (middle-click paste) is not synced.

**macOS pasteboard privacy.** Since macOS 15.4 there is a per-app setting "Paste from Other Apps"
(Ask, Allow, Deny) and an API (`NSPasteboard.accessBehavior`, plus `detect` methods that look at
types without a prompt). Sources disagree on whether "Ask" is the default in macOS 26 and later.
The agent reads `accessBehavior` at start and uses `detect` first; it reads content only when a
sync is due. The app appears in the settings list only after its first read attempt, so onboarding
triggers one read on purpose. **Decision tree, fixed now, answered by S2 on macOS 27.2:**
- No prompt for the agent, or "Allow" can be set by the user with one click in Settings: **go**.
- A prompt on every read and no way to allow for good: **stop and ask you.** Mac to others would
  not be usable in the background, and the Mac would be receive-only.

### 3.10 Private-copy skip

A clip is never sent when the source marks it as private:

| OS | Markers |
|---|---|
| Windows | format `ExcludeClipboardContentFromMonitorProcessing` present; `CanIncludeInClipboardHistory` = 0; `CanUploadToCloudClipboard` = 0 |
| macOS | types `org.nspasteboard.ConcealedType`, `org.nspasteboard.TransientType`, `org.nspasteboard.AutoGeneratedType` |
| Linux | MIME `x-kde-passwordManagerHint` with value `secret` |
| Android | `ClipDescription` extra `android.content.extra.IS_SENSITIVE` |
| iOS | No marker exists. Sends are always a user action, so the user decides. |

If a marker cannot be read (error), the clip counts as private. A password manager that sets no
marker is synced like any text. The help page says so.

### 3.11 Size caps, images, rich text

| Item | Rule |
|---|---|
| Plain text | Up to 1 MB. Larger: not synced, shown in "Last activity". |
| HTML | Up to 2 MB. Larger: the HTML part is dropped, the plain text is still sent. |
| Image | Sent as PNG. Up to 25 MB. Larger: not synced. |
| Image safety | Before decode: check the PNG header, at most 16,384 px per side and 100 megapixels. Decoding uses memory-safe Rust crates only. |
| Files, folders | Never synced in v1 (section 3.8). |
| Mixed clips | Text plus image (for example from Office or a browser): all parts are sent within the caps. |

**Rich text decision:** sync `text/html` with a `text/plain` copy. Windows needs the `CF_HTML`
header with byte offsets, which the backend builds and parses. That parser reads hostile input, so
Opus writes it. RTF is not synced; an RTF-only copy arrives as plain text. The HTML is never
rendered by our app. A setting "Keep formatting" turns the HTML part off. Rich text is milestone
M11, after the desktop beta, and moves to v1.1 if the schedule needs it.

### 3.12 Key storage and state

| OS | Device key | Other state |
|---|---|---|
| Windows | Credential Manager (DPAPI, per user) via the `keyring` crate | `%LOCALAPPDATA%\UniversalClipboard\state.json` |
| macOS | Login Keychain, item limited to the signed app | `~/Library/Application Support/uclip/state.json` |
| Linux | Secret Service (GNOME Keyring, KWallet). If none exists: a 0600 file, and the diagnostic report says so. | `~/.local/share/uclip/state.json` |
| iOS | Keychain, shared access group for app and extensions, `AfterFirstUnlock` | App Group container |
| Android | Key wrapped by an Android Keystore AES key | App-private storage |

`state.json` holds the membership chain, the group secret (wrapped with a key from the key
store), remembered peer addresses, and settings. It never holds clipboard content.
Logs hold kinds, sizes, times and error codes. Never content, never keys. Logs rotate at 256 KB.
Test mode (`UCLIP_PROFILE` plus `UCLIP_TEST_KEYSTORE=file`) uses a file key store, because the
login Keychain is not usable from every test context.

### 3.13 Updates

An invisible app that updates silently needs a safety net: a bad release would break every device
and nobody would see it.

| Channel | Who updates | Notes |
|---|---|---|
| Microsoft Store (if S8 passes) | The Store | Silent. Rollback is "publish a fixed version"; the Store has staged rollout. |
| Windows exe from GitHub, macOS `.dmg` | Our own updater, `uc-update`, inside the agent | Below. |
| Linux `.deb` | The system's package updater, from our apt repository | No updater code of ours. |
| Phones | The stores | |

**How `uc-update` works:**
1. Once a day the agent fetches the static file `latest.json` from the newest GitHub release
   (`releases/latest/download/latest.json`). It does not use the GitHub API, which allows only 60
   requests per hour per IP address.
2. `latest.json` holds the version, file URLs, a signature per file, and a **rollout percentage**.
   A device takes the update only if a hash of its device ID falls inside the percentage. A release
   starts at 10%, then goes to 100% after 2 days without rollbacks reported in our own use.
3. The agent verifies the signature (updater key, section 8.4) before anything runs.
4. When the clipboard has been idle for 60 s, a small update helper takes over. It **stops the
   agent** (Windows locks a running exe; on macOS the bundle must not change under a running
   LaunchAgent), keeps the old version in a `previous` folder, installs the new one (Windows:
   silent per-user installer, no UAC; macOS: the app bundle is swapped), and starts the new agent.
5. **Health check:** within 2 minutes the new agent must start, open its IPC, load its state and
   reconnect to at least one peer that was reachable before the update (or, with no peer online,
   come up healthy on its own: IPC open, state loaded, LAN listener bound). If not, the helper **puts the old version back**, starts it, and marks that
   version as bad so it is not tried again.
6. Three failed updates in a row give the "could not update itself" state (section 2.7).

- **Compatibility:** each release speaks the current and the previous protocol version. A device
  that is too old shows "Update needed" in its row on the other devices.
- The test "version N, then a broken N+1, then rollback to N" is part of M9's acceptance.

### 3.14 No server (formerly the relay service)

**Removed on 2026-11-13.** There is no relay, no VPS, no domain, no lookup service and no monitor.
The only outside contacts are GitHub (or the system's store or package updater) for updates.

- **If the local network cannot be used** (VPN blocks it, guest Wi-Fi, devices on different
  networks): sync does not work, and the app says why and shows help (sections 2.7 and 3.5).
- **Privacy policy:** a short page (`docs/privacy.md`, published on GitHub Pages). It says: no
  account, no content on any server, no server of ours at all, data only moves between your devices
  on your local network, GitHub serves updates. The stores and SignPath need this page. The
  installer and the Welcome screen link to it.

---

## 4. Security and privacy

### 4.1 Model
- Only devices in your group can read or write your clipboard. Membership starts with a PAKE code
  that you typed or scanned yourself.
- All traffic is encrypted and authenticated end to end with the devices' own keys (QUIC, TLS 1.3).
  Anyone on the network sees ciphertext only.
- **There is no server.** Traffic stays on the local network. Nothing of ours is on the internet,
  so there is no relay to attack, misuse or pay for, and no device is reachable from the internet.
- **Never reachable from outside the local network (hard requirements, added 2026-11-14):**
  - No port mapping of any kind: no UPnP, NAT-PMP or PCP. iroh's port mapper, relay, hole punching
    and DNS/pkarr discovery are all OFF (M2 asserts this in a test). The app never asks a router to
    open a port.
  - The LAN rule of `docs/protocol.md` 4.8 applies to the SYNC port too, not only to pairing: a
    connection from an address outside the private ranges and the on-link subnets of a physical
    interface is closed before the TLS handshake finishes. This also covers public IPv6 addresses,
    which some routers let in from the internet.
  - The firewall rule allows inbound only from the local subnet (Windows `RemoteAddress=LocalSubnet`,
    profile Private), never Public. On a public Wi-Fi network the app does not listen at all.
  - Even a device that reaches the port without being paired gets nothing: unknown keys are dropped
    before any clipboard data moves.
- No account, no analytics, no telemetry, no crash upload. The only outside contact is GitHub (or
  the system's store or package updater) for updates.
- Clipboard content exists only in memory and on the OS clipboard. Never on disk, never in logs,
  never in the UI.
- The UI process holds no keys. Only the agent does.

### 4.2 Threats and answers

| # | Threat | Answer |
|---|---|---|
| 1 | Stranger on the same Wi-Fi tries to connect | Not a valid member, so the connection closes before any data. |
| 2 | Stranger on the LAN tries to pair by guessing the short code | PAKE: one guess per try, 5 tries, 10 minutes, dropped connections count. No offline guessing. They can burn the 5 tries; the user sees "Someone else tried this code". |
| 2b | Stranger on the internet tries to find or guess a pairing | Not possible: the app has no internet path and listens only on local interfaces. Pairing is accepted only from the local subnet. |
| 3 | Man in the middle during pairing | Device keys and connection keys are in the PAKE transcript, with key confirmation. The check fails. |
| 4 | Someone on the local network reads traffic | End-to-end encryption (QUIC, TLS 1.3 with device keys). They see addresses, sizes and times only. |
| 5 | Tracking a person by a stable ID in discovery | mDNS names are rotating tags from the group secret. No device name, no key in the clear. |
| 6 | A removed or stolen device keeps reading, or adds a device of its own | Removal closes its connections and rotates the group secret. The membership chain makes every change it signs after the removal invalid (3.7). Limit: a member that is offline, until its next contact. |
| 6b | A stolen device removes all the others | It gains nothing. The others get a notification and "Pair my devices again". |
| 7 | Someone with the unlocked device adds their own device | Out of scope (they already have the clipboard). The devices list shows every member, and a notification says "A device was added". |
| 8 | Passwords get synced | Private-copy skip (3.10). Honest limit: apps that set no marker. Pause switch. |
| 9 | A member device is compromised and sends bad data | Size caps, strict decoding, image limits, memory-safe decoders. HTML is never rendered by us. Future clock stamps are refused. |
| 10 | Another local user or program talks to the agent, or pretends to be it | Pipe and socket are limited to the same user. First-instance pipe creation, and the UI checks the server (3.2). Same-user malware is out of scope (it can read the clipboard anyway). |
| 10b | The firewall allow rule points at an exe in a folder the user can write to | Same-user malware could replace the exe and inherit the rule. Accepted for the exe build (same-user malware is out of scope). The Store build does not have this: its files are read-only. |
| 11 | Bad or forged update | Updates are signed with the updater key. The key is a secret of a protected GitHub environment that runs only after your approval (8.4). Also OS-signed where we have a certificate. Honest limit: a takeover of your GitHub account plus your approval step would defeat it; use 2FA with a hardware key or passkey. Broken (not forged) updates: health check and rollback. |
| 12 | Replay of an old clip | QUIC stops network replay. The clock rule and the ID lists stop an old clip from being applied. |
| 13 | Flooding with pairing requests | One open pairing, only after the user pressed "Add a device". Rate limits. |
| 14 | Supply chain | `Cargo.lock` committed, `cargo-deny` and `cargo-audit` in CI, few dependencies, pinned GitHub Actions. iroh pinned exactly if an unstable feature is used. |
| 15 | Secrets left behind after uninstall | Section 8.5 says per OS what is removed and what stays. |
| 16 | (Removed 2026-11-13: there is no relay.) | The attack surface is smaller: no server, no internet listener. |

A `SECURITY.md` explains how to report a problem.

**Reviews.** Three fixed points:
- **Before M3 (M0c):** the planning model (Fable) reviews the pairing and membership protocol text
  on paper. A protocol error is cheap to fix there.
- **In M3 and in M10:** an Opus security review **with a fresh context**. The reviewer gets only
  this spec and the diff, not the build agent's conversation.

---

## 5. Repo layout and tech choices

```
Universal-Clipboard/
  Cargo.toml                 workspace
  crates/
    uc-core/                 clip model, codec, clock, sync state machine (full and pull-only roles)
    uc-net/                  iroh endpoint, ifaces, LanBook, mDNS (pluggable)
    uc-pair/                 PAKE pairing, membership chain
    uc-keys/                 key store trait + backends
    uc-clip/                 Clipboard trait + fake
    uc-clip-win/  uc-clip-mac/  uc-clip-linux/
    uc-update/               update check, verify, rollout, health check, rollback
    uc-agent/                the background program + IPC server
    uc-cli/                  `uclip` test and debug CLI (talks to the agent)
    uc-mobile/               UniFFI bindings
    uc-testkit/              in-process multi-agent harness, network sim helpers
  apps/
    desktop/                 Tauri 2 app (src-tauri + Svelte frontend)
    android/                 Gradle project (Kotlin, Compose)
    ios/                     Xcode project (SwiftUI app, ShareExtension, AppIntents)
  packaging/                 installer scripts, MSIX manifest, LaunchAgent plist, firewall helper,
                             apt repository script
  spikes/                    throwaway spike code (M0), removed before v1
  tools/                     hidden-desktop test host, netns scripts, lab VM scripts, runners
  docs/
    plan/                    this file
    spikes/                  spike results (facts with date and source)
    help/                    user help pages
    privacy.md               privacy policy
  .github/workflows/         ci.yml, release.yml
  CLAUDE.md  README.md  SECURITY.md  LICENSE-MIT  LICENSE-APACHE
```

Internal names use `uclip` everywhere (crates, bundle ID `io.github.maxaubert.uclip`, pipe names).
The display name is one config value, so the name can change without code changes (section 8.3).

| Area | Choice | Version | Checked |
|---|---|---|---|
| Language | Rust, edition 2024 | 1.99 on the PC | installed |
| Async | tokio | 1.x | |
| Transport | iroh | 1.4.x, pinned exactly if S1 picks the unstable custom transport | docs.rs today: 1.4.1 |
| mDNS | `mdns-sd` (per-interface control). `iroh-mdns-address-lookup` 0.5 is the alternative if it can be limited to chosen interfaces. | latest | decide in S1 |
| PAKE | `spake2` (RustCrypto) with our transcript and confirmation (`docs/protocol.md`) | `=0.4.0` | decided in M0c |
| Hash, MAC, KDF | `blake3`, `hmac` + `sha2`, `hkdf` | | verify |
| Codec | `postcard` + `serde` | 1.x | verify |
| Key store | `keyring` | 3.x | verify |
| Windows API | `windows` crate | latest | |
| macOS API | `objc2`, `objc2-app-kit`, `objc2-service-management` | latest | |
| Linux | `x11rb` 0.13, `wayland-client` 0.31, `wayland-protocols` (ext-data-control) | verify | |
| Images | `png`, `image` (PNG and DIB/TIFF only, other features off) | latest | |
| Desktop UI | Tauri 2, Svelte 5, TypeScript, Vite | tauri-cli 2.12.4 installed | installed |
| Tauri plugins | `single-instance`, `notification`. Not the Tauri updater (we use `uc-update`, with the same minisign signature format). | 2.x | |
| Mobile bindings | UniFFI | latest | verify in S5 |
| Android | Kotlin, Jetpack Compose, min SDK 29, target SDK 37, Google code scanner | | SDK installed in M0a |
| iOS | Swift 6, SwiftUI, App Intents, min iOS 18 | Xcode 27.2 | installed on the Mac |
| Tests | `cargo nextest`, `proptest`, n0 `patchbay` (Linux network namespaces), Vitest, Playwright | | |
| CI | GitHub Actions, free for the public repo | | |
| License | MIT OR Apache-2.0 (same as iroh and Tauri) | | |

"verify" means: the build agent runs `cargo add`, takes the newest stable version, and pins it in
`Cargo.lock`. Exact versions are written to `docs/spikes/versions.md` in M0.

---

## 6. Testing strategy

Rule from CLAUDE.md: nothing a test starts may take the screen or the foreground. Every test below
is headless. No test may touch your real clipboard or cut your PC's network.

### 6.0 The test lab (built in M0a)

There is one PC and one Mac. Three of the five required pairs need a second machine of the same
OS, and Linux needs a real desktop. So the lab has virtual machines, all with a **bridged** network
adapter (they sit on your real LAN, like separate computers), all run without a window:

| VM | Host | Role |
|---|---|---|
| `win-b`: Windows 11 | Hyper-V on the PC | Second Windows device. Clean-install target. Reboot target. WireGuard and consumer VPN tests on Windows. Timed cold start. |
| `mac-b`: macOS | Apple Virtualization on the Mac Mini (tart or UTM) | Second Mac. Reboot target. VPN tests on macOS. |
| `lin-gnome`: Ubuntu, GNOME on Wayland | Hyper-V on the PC | Linux release gate, the GNOME path. |
| `lin-kde`: Kubuntu, KDE Plasma 6 on Wayland | Hyper-V on the PC | Linux release gate, the KDE path, Linux to Linux. |

- VPN tests that break the network run **in the VMs, never on your own PC or Mac**. A kill switch
  on your PC would cut your network, and the PC is managed by Defender for Endpoint.
- The VMs use their real clipboards (nobody works on them). Your PC and Mac use isolated
  clipboards (6.4).
- WSL and Docker stay for the fast inner loop (unit tests, namespaces, headless compositors).
- **This needs your yes once (decision 9):** a bridged adapter needs a Hyper-V "external switch" on
  the PC. Creating it interrupts the PC's network for a few seconds and changes the adapter setup.
  The macOS VM needs a one-time click through its setup assistant and permission prompts (over
  screen sharing to the VM, not on your screen).

### 6.1 Unit tests (every crate, every OS, in CI)
- Codec round trips and fuzzing with `proptest` (every message, random bytes must not panic).
- Clock and conflict rule: random event orders must converge. A stamp far in the future is refused.
- Echo and loop rules with the fake clipboard, including a fake clipboard manager that strips the
  marker and re-encodes the clip on two devices (must not ping-pong).
- Line-ending conversion, file-clip skip, size caps, HTML drop rule, image limit checks,
  `CF_HTML` header build and parse (fuzzed).
- Membership chain, property tests: merge is order-free; bad signatures are refused; **a removed
  device signs "add Y" after its removal, or back-dates it, and hands it to a member that was
  offline: Y never becomes a member on any device once the removal is known**; changes signed by
  such a Y are invalid (a removal signed by Y still counts: removal always wins); two devices
  remove each other (both are removed); a device removes all others. Full list: `docs/protocol.md` 7.
- Pairing state machine: wrong code, 5 tries, dropped connection counts as a try, timeout, man in
  the middle (different keys on each side), transcript test vectors, **a pairing is
  refused from a non-local address, and a client outside the LAN finds nothing.**
- Private-marker detection per OS with recorded format lists.
- Updater: bad signature refused, rollout bucket, health check fail gives rollback.

### 6.2 Integration tests on one machine (in CI)
`uc-testkit` starts 2 to 5 agents in one process or as child processes, each with a fake clipboard
and its own state folder, on loopback.
Scenarios: pair two (short code, QR payload), add a third, copy on each and check all
others, two copies at the same instant, big image, private clip, pause, remove a device (and check
it is refused after), restart an agent, protocol version N with N-1, a pull-only client that
fetches the current clip with no age limit, a pull-only client with no full member online
(must get the clear "no computer is on" result).

### 6.3 Simulated networks (Linux: WSL on the PC, and CI)
Network namespaces through `patchbay`, or plain `ip netns` scripts in `tools/netsim/`.
M0a checks first that the WSL kernel has WireGuard and user namespaces; if not, these tests run in
`lin-kde`.

| Scenario | Setup | Must pass |
|---|---|---|
| Same LAN | 3 namespaces on one bridge | Direct path, under 1 s |
| Multicast blocked | drop 224.0.0.0/4 and ff02::/16 | Found by remembered address; a never-seen address is not found, and the app shows "No path at all" help |
| VPN, LAN allowed | WireGuard interface with default route in one namespace | Direct LAN path, not through the tunnel |
| VPN, overlapping route | tunnel route covers the LAN subnet | The result S1 promised for Linux (direct, or the VPN help) |
| VPN, kill switch | drop all non-tunnel traffic | The "No path at all" state with the VPN help (detection rule from S1) |
| Client isolation | drop traffic between the two namespaces on the bridge | The "No path at all" state with the guest-network help |
| Both sides inbound-blocked | drop inbound on both | The "No path at all" state, no crash, heals when one side opens |
| Address change | change one device's IP | Reconnect under 5 s through gossip or mDNS |
| Sleep and wake | `SIGSTOP` 5 minutes, `SIGCONT` | Reconnect, newest clip arrives |
| Lossy Wi-Fi | `tc netem` 5% loss, 100 ms delay | Sync still correct |

### 6.4 Real clipboard tests per OS
- **Windows (your PC):** the tests run inside a separate, non-visible window station and desktop
  made by `tools/winhost` (extends the `HiddenDesktop.cs` pattern from Prism). A window station has
  its own clipboard, so your clipboard is untouched and nothing appears on screen. Spike S3 proves
  this. Fallback: the `win-b` VM only.
- **macOS (your Mac):** a process started from `ssh mac` is not the same as the real app: macOS
  may block its LAN traffic, it cannot use the login Keychain, and it may not reach the pasteboard
  server. So the runner installs a test LaunchAgent and starts the agent with
  `launchctl bootstrap gui/<uid>`, which is the real LaunchAgent path. Test builds use a named
  private pasteboard (`UCLIP_TEST_PASTEBOARD`) in place of the general one and the file key store.
  On `mac-b` the tests use the real pasteboard and the real Keychain.
- **Linux:** fast loop in Docker or WSL: X11 in `Xvfb`, wlroots in headless Sway
  (`WLR_BACKENDS=headless`), KDE in `kwin_wayland --virtual`. Release gate in `lin-gnome` and
  `lin-kde` with their real sessions.

### 6.5 Cross-OS end-to-end (PC as the driver)

A runner script on the PC builds and starts agents on the PC (hidden window station), the Mac
(LaunchAgent), and the four VMs, pairs them with the `uclip` CLI through the real pairing
protocol, and runs the copy matrix in all directions with text and an image (and HTML after M11).
This is the release gate.

**Pair table.** Every cell says how that pair is tested. "Auto" = automated, no person needed.

| | Windows | macOS | Linux | Android | iPhone, iPad |
|---|---|---|---|---|---|
| **Windows** | Auto: PC + `win-b` | Auto: PC + Mac (real machines) | Auto: PC + `lin-gnome`, `lin-kde` | Auto: emulator on the PC with a forwarded direct address. LAN discovery: by hand on a real phone. | Auto: Simulator on the Mac (shares the Mac's LAN). By hand: your iPhone and iPad. |
| **macOS** | | Auto: Mac + `mac-b` | Auto: Mac + `lin-kde` | Auto: emulator, forwarded address. | Auto: Simulator. By hand: your iPhone and iPad. |
| **Linux** | | | Auto: `lin-gnome` + `lin-kde` | Auto: emulator, forwarded address. | Auto: Simulator. By hand: your iPhone with `lin-kde`. |
| **Android** | | | | Auto: two emulators, forwarded addresses, with the PC as a member. | Auto: emulator + Simulator, forwarded address. |
| **iPhone, iPad** | | | | | Auto: two Simulators with the Mac as the always-on member. By hand: your iPhone to your iPad. Also tested: no computer on gives the clear message. |

Honest gaps: Android LAN discovery and the real Android background behaviour are tested by hand
only (decision 10 proposes a cheap test phone on USB, which would make them automatic). iOS on real
hardware is always by hand, by you.

**Soak test (M10):** a generator makes 1 clip every 30 s on a random device for 24 hours (2,880
clips; 90% text, 10% images). After each clip, every device must hold the same content hash within
5 s. It runs on the four VMs plus the hidden window station on the PC. Never on your clipboard.

**Reboot tests** run in `win-b`, `mac-b` and the Linux VMs (the agent must be back and connected
within 60 s of login). A reboot of your own PC would kill the build session, so you do one manual
reboot check of the PC and the Mac at M10.

### 6.6 UI tests
- Frontend: Vitest for logic, Playwright (headless Chromium) against the Vite build with a mocked
  agent API. Screenshots of every screen and every error state are saved as test output.
- **"Matches the mockups" has a number:** after you approve M8b's screens once, they become
  baseline images. Later runs fail if more than 1% of the pixels differ.
- App level: `tauri-driver` on Windows (hidden desktop, and `win-b`) and Linux (`Xvfb`). It does
  not support macOS, so macOS is covered by the frontend tests plus the agent tests.
- **Window open time:** the median of 10 cold starts on `win-b` is under 1.5 s.

### 6.7 Mobile
- **Shared core:** Rust tests of `uc-mobile` on the host, including the pull-only role.
- **Android:** emulator on the PC with `-no-window`, API 29 and API 37 images. JVM unit tests,
  instrumented tests for the service, the share target and the clipboard write.
  `ACCESS_LOCAL_NETWORK` is tested granted and refused on API 37. The emulator sits behind its own
  NAT, so its network tests use a forwarded direct address (`adb forward` and
  the emulator's UDP redirect). **Real LAN discovery and OEM background limits are tested by hand:**
  by the 12 testers that Google Play's closed test needs anyway (they get a 10-minute checklist),
  or automatically if you add a test phone (decision 10).
- **iOS:** `xcodebuild test` on the Mac over ssh, on a headless Simulator (`simctl`, Simulator.app
  not opened). The Simulator shares the Mac's network, so pairing and LAN sync with the PC are
  tested for real. XCTest covers the app, the Share Extension and the App Intents.
  The Simulator does **not** enforce pasteboard and Local Network privacy, so those are never
  accepted from Simulator results.
- **By hand, by you:** spike S5 on your real iPhone connected to the Mac (your yes is needed), and
  the TestFlight checklist (about 15 minutes per device): permissions, QR pairing, Share sheet,
  Share sheet, app-open sync, VPN on the phone with and without local network sharing, "no computer is on".

---

## 7. Implementation plan

Rules for every milestone:
- One milestone is one branch and one PR. Small follow-up fixes get their own PR.
- CI must be green. The branch is built and installed on the PC (and the Mac when it applies) and
  the version is confirmed before the merge question. You approve each merge.
- **Test installs on your PC and Mac use the profile `dev`** (own state folder, pipe name, key and
  port) and run against the hidden window station or the named pasteboard, never your real
  clipboard. So the app and the private clipsync prototype cannot echo each other. The prototype
  keeps running until M16. If a test must use the real clipboard, the prototype is stopped for
  that test and started again after.
- New facts found on the way go to `docs/spikes/` or `docs/` in the same PR.
- "Opus" and "Sonnet" name the model that does the task. "Fable" appears only for the paper review
  in M0c (planning work).

Order and parallel work:

```
M0a -> M0b (7 spikes in parallel) -> M0c -> M1 -> [M2 | M3 | M4 | M5 | M6 | M8a] in parallel
     -> M7 -> [M8b | M9] in parallel -> M10 (desktop beta)
     -> [M11 | M12 -> (M13 | M14)] in parallel -> M15 -> M16
```

### M0a. Repo bootstrap and test lab
- Depends on: approval of this plan. Machine: PC, Mac. Model: Sonnet.
- Tasks: Cargo workspace with empty crates; `ci.yml` (fmt, clippy, nextest on Windows, macOS,
  Ubuntu; `cargo-deny`); `CLAUDE.md` (build, test, release block via the `shipping` skill);
  README stub; licenses; `SECURITY.md`; privacy policy draft; install Rust and Node on the Mac;
  install the Android SDK, the emulator and the API 29 and API 37 images on the PC; **create the
  four lab VMs** (6.0) with scripts in `tools/lab/`; check the WSL kernel for WireGuard and user
  namespaces; name check for Copybridge and Clipbridge; start the account and signing
  applications that you approved (decision 4).
- Accept: CI green on all three OS. `cargo build` works on the PC, the Mac, WSL and in each VM.
  Each VM is reachable over ssh on the LAN and reboots to a logged-in desktop session without a
  person. The emulator boots headless on API 37. The WSL check has a written result.

### M0b. Spikes (parallel, throwaway code in `spikes/`, results in `docs/spikes/`)
Each spike ends with a one-page result and a go, change or stop decision.
Model: Opus, except S3 and the Docker session images in S4 (Sonnet).

| Spike | Question | Machine | Go criterion |
|---|---|---|---|
| S1 | **iroh and interfaces:** test options (a) address bind, (b) unstable custom transport, (c) fork or patch, each with full-tunnel WireGuard, a kill switch and a **real overlapping route**, on Windows, macOS and Linux. `mdns-sd` or the iroh mDNS crate? **Windows firewall:** which alerts appear and when; does the rule for Private and Public profiles work; outbound-only mode; two PCs that both refused. **VPN defaults:** the LAN setting of 4 or 5 large consumer VPNs. | `win-b`, `mac-b`, `lin-kde` | One option chosen per OS. The exact result for the overlapping-route row. Clear VPN-blocks-LAN detection and help proven under a kill switch (detection rule written down, tested for false alarms and misses). Prompt list written. |
| S2 | **macOS 27.2:** does the agent started by `SMAppService` get a pasteboard prompt? What does `accessBehavior` return? Apply the decision tree in 3.9. Local Network prompt: does it show the app's name for the agent? Keychain and notifications from the agent. Do the grants survive an app update? Is `com.apple.is-remote-clipboard` present on Handoff clips, and is reading them fast and silent? | Mac and `mac-b` (you click the prompts once on the Mac; this needs your yes because it is on screen) | The exact prompt list for 2.2. Go, or stop and ask you. |
| S3 | Does a private window station give an isolated clipboard for tests on Windows? | PC | A test copies and reads in the hidden station; your clipboard is unchanged. |
| S4 | **GNOME Wayland go/no-go:** (1) XWayland: do XFIXES events arrive for every copy with no X11 window focused, and can we write? (2) the portal. Does GNOME still ship an X11 session? Also confirm headless Sway and KWin in Docker. | `lin-gnome`, PC (Docker) | One path that needs no babysitting. Else: "KDE and X11 only" and a question to you. |
| S5 | **iOS:** does the Rust core with iroh build through UniFFI for iOS? Does it fit a Share Extension (memory, start under 2 s)? **On a real iPhone:** does the app read the clipboard and write the newest clip when it opens (with paste setting Ask and Allow)? Local Network prompt and iroh socket binding to Wi-Fi with a VPN on the phone. The "no computer is on" state. | Mac + your iPhone on USB (needs your yes) | A Share Extension and sync on app open work against the PC. The rewrite of 2.6 is confirmed or corrected. |
| S6 | (Dropped 2026-11-13: relay and long code are out of scope.) | | |
| S7 | **Android:** does a background `setPrimaryClip` from the foreground service work on API 29 to 37, and does it show the clipboard preview overlay each time? Play policy for foreground service type `connectedDevice` (declaration, review video). Clipboard read with window focus when the app opens. `ACCESS_LOCAL_NETWORK` granted and refused. Doze and OEM killing (from documents). UniFFI build for Android. | PC (emulator) | Automatic computer to phone is silent and allowed. Else the fallback is fixed: a notification "Tap to copy". |
| S8 | **Microsoft Store (MSIX):** can an individual in Norway open a developer account, and what does it cost? Does an MSIX package give Store signing, silent updates, a firewall rule from the manifest, a startup task and a clean uninstall, with our two-process design? | PC, `win-b` | A sideloaded MSIX on `win-b` that syncs after a reboot with no prompt. Account question answered. |

- Accept: seven result pages (S6 is dropped). Sections 2 and 3 of this plan are updated where a spike changed a
  decision. Any "stop" result goes to you before M1 starts.

### M0c. Protocol review on paper
- Depends on: S1. Model: Fable (the planning model). One short session.
- Tasks: review the final text of 3.6 (pairing, transcript, confirmation) and
  3.7 (membership chain, validity rule, removal cases) for protocol errors; decide SPAKE2 or
  CPace; write the test vectors' format.
- Accept: a signed-off protocol page in `docs/`, which M3 implements word for word.
- **Done 2026-11-14: `docs/protocol.md`.** S1 had no result yet; the page works with iroh and
  with plain QUIC (both give the peer key and a TLS exporter).

### M1. Core library (`uc-core`, `uc-clip` trait and fake, `uc-testkit` base)
- Depends on: M0. Machine: PC. Model: Opus.
- Tasks: clip model and parts; wire ID and content hash; line-ending rule; `postcard` messages
  with version byte; hybrid logical clock with the future-stamp limit; conflict rule; four echo
  and loop layers; size caps; "latest clip" memory with the 1-hour automatic rule and the
  no-limit explicit fetch; the sync state machine against abstract `Clipboard` and `Peer` traits,
  **in both roles (full member, pull-only client)**; in-memory fake network.
- Accept: unit and property tests from 6.1 pass. Three fake devices converge in every random
  order. The clipboard-manager ping-pong test passes.

### M2. Networking (`uc-net`)  [parallel with M3 to M6, M8a]
- Depends on: M1, S1. Machine: PC, WSL, `win-b`, `mac-b`. Model: Opus.
- Tasks: iroh endpoint wrapper with the option S1 chose; physical interface detection for Windows,
  macOS, Linux; per-interface sockets; **traits for mDNS and for socket binding, so iOS and
  Android can plug in (results of S5 and S7 feed these)**; `LanBook`; remembered addresses; signed
  address gossip; mDNS with rotating tags; reconnect on wake and network change; LAN only (no relay, no internet path);
  path state reporting (LAN, blocked by VPN or firewall, no path); the VPN-blocks-LAN detection rule from S1.
- Accept: every row of the 6.3 table passes in WSL or `lin-kde`. **On `win-b` and on `mac-b`:
  WireGuard full tunnel with LAN allowed, kill switch, and overlapping route each give the result
  that S1 wrote down.** Interface detection has unit tests with recorded adapter lists.

### M3. Pairing, membership, keys (`uc-pair`, `uc-keys`)  [parallel]
- Depends on: M1, M0c. Machine: PC, Mac for the Keychain backend. Model: Opus.
- Tasks: short code and QR payload; PAKE with the transcript and confirmation from the
  protocol page; try and time limits; LAN-only rule for all pairing;
  membership chain with signatures, validity rule and merge; removal and secret rotation; key
  store backends (Windows, macOS, Linux with file fallback, test file store).
- Accept: pairing and membership tests from 6.1 pass, including the man-in-the-middle test, the
  off-LAN short-code test and the removed-device attack tests. A fresh-context Opus security
  review (spec plus diff only) finds no open high or medium issue.

### M4. Windows clipboard backend (`uc-clip-win`) and `tools/winhost`  [parallel]
- Depends on: M1, S3. Machine: PC. Model: Opus.
- Tasks: listener window; read and write of text, PNG and DIB; private markers; origin marker;
  `CanUploadToCloudClipboard = 0` on writes; file-clip skip; CRLF rule; retry when the clipboard
  is busy; check whether cloud-clipboard arrivals can be detected; the hidden window-station host.
- Accept: real clipboard tests pass in the hidden station. Your clipboard and screen are untouched.
  Tested with recorded format sets (Notepad-like text, a browser copy, a screenshot-like bitmap,
  a copied file from Explorer).

### M5. macOS clipboard backend (`uc-clip-mac`)  [parallel]
- Depends on: M1, S2. Machine: Mac, `mac-b`. Model: Opus.
- Tasks: `changeCount` watcher; `detect` then read; text, PNG and TIFF; nspasteboard.org markers;
  origin marker; `.currentHostOnly` writes; `accessBehavior` check and the on-purpose first read;
  remote-clipboard detection with the skip or forward setting; file-clip skip; test mode.
- Accept: tests pass on the Mac through the LaunchAgent path on the named pasteboard, and on
  `mac-b` on the real pasteboard. One manual check of the real prompts, written into `docs/spikes/`.

### M6. Linux clipboard backends and autostart (`uc-clip-linux`)  [parallel]
- Depends on: M1, S4. Machine: PC (Docker, WSL), `lin-gnome`, `lin-kde`. Model: Opus for the
  backends, Sonnet for the Docker images.
- Tasks: X11 backend; data-control backend (ext and wlr); the GNOME path from S4; session
  detection; KDE password hint; file-clip skip; XDG autostart entry and the supervisor loop.
- Accept: tests pass in Xvfb, headless Sway and KWin, and in the real sessions of `lin-kde` and
  `lin-gnome` (or GNOME is marked unsupported after your answer). Autostart and crash restart work
  on GNOME, KDE and one X11 desktop (XFCE in Docker or a VM snapshot).

### M7. The agent (`uc-agent`, `uc-cli`)
- Depends on: M1 to M6. Machine: PC, Mac, VMs. Model: Opus for the agent, Sonnet for the CLI.
- Tasks: wire core, net, pair, keys and the OS backend; state file; IPC server with the hardening
  in 3.2; profiles; pause; settings; diagnostics report; logs without content; autostart register
  and unregister per OS; single agent per user, two users on one machine; port-in-use handling;
  Windows firewall order rule and outbound-only mode; sleep and wake hooks; "leave" that tells
  the group; the `uclip` CLI.
- Accept: the cross-OS runner (6.5) pairs the PC, the Mac and the four VMs through the CLI and
  passes the copy matrix for text and images, for every desktop cell of the pair table. Agents
  come back after a reboot in `win-b`, `mac-b` and both Linux VMs. Idle CPU under 0.5%, memory
  under 40 MB.

### M8a. Mockups  [parallel with M2 to M6]
- Depends on: M0. Machine: PC. Model: Sonnet.
- Tasks: ask you which design skill and image tool to use (recommendation: `impeccable`). Mockups of
  every screen in sections 2.2 to 2.7 for desktop, and the phone screens. Opened in Zen for review.
- Accept: you approve the mockups.

### M8b. Desktop UI (`apps/desktop`)  [parallel with M9]
- Depends on: M7, M8a. Machine: PC, `win-b`. Model: Sonnet.
- Tasks: Tauri 2 app with Svelte; onboarding with the permission rows per OS; add-device screen
  with QR and short code; settings window; every state in 2.7 with its Fix button, including the VPN help screen (L12);
  single instance; start the agent when missing; the IPC server check; English strings in one
  file (ready for Norwegian later).
- Accept: you approve the Playwright screenshots once; they become the baselines (1% pixel
  threshold). `tauri-driver` flows pass on Windows and Linux. Median cold start on `win-b` under 1.5 s.

### M9. Installers, firewall helper, updater, release workflow  [parallel with M8b]
- Depends on: M7, S8. Machine: PC, Mac, VMs, CI.
- Model: **Opus** for the elevated firewall helper, `uc-update` (verify, rollout, health check,
  rollback), the uninstall logic and the release signing set-up. **Sonnet** for packaging scripts
  (NSIS, MSIX, DMG, `.deb`, apt repository on GitHub Pages), `release.yml` plumbing and docs.
- Tasks: NSIS per-user installer; MSIX package for the Store if S8 passed; elevated one-time
  firewall rule helper (Private and Public) and its removal; notarized DMG (Apple silicon);
  `.deb` plus a signed apt repository that the package registers; `uc-update`; updater key and
  apt key in a protected GitHub environment (8.4); `release.yml`; the uninstall paths in 8.5.
- Accept: `win-b`, `mac-b` and `lin-kde` install from the release files with only the expected
  warnings. An update from N to N+1 happens with no user action on Windows, macOS and through apt.
  **"N, then a broken N+1, then automatic rollback to N" passes on Windows and macOS.** The
  uninstall test per OS matches the table in 8.5.

### M9b. (Dropped 2026-11-13)
The relay service is out of scope. Its one useful task, the final privacy policy, moves to M15.

### M10. Hardening and the release gate for desktop
- Depends on: M8b, M9. Machine: PC, Mac, VMs. Model: Opus.
- Tasks: full pair table for desktops with groups of 3 to 5 devices; VPN matrix on `win-b` and
  `mac-b` with WireGuard, **plus one consumer VPN client with a WFP kill switch on Windows and one
  Network Extension VPN on macOS**; sleep and wake; network switch; removal; the 24-hour soak test
  (6.5); second fresh-context Opus security review of the whole code; fix what is found.
- By you, once: reboot the PC and the Mac and check that sync is back (10 minutes).
- Accept: all matrices green. No crash and no missed clip in the soak test. Review issues closed.
  **Result: desktop beta (v0.9), plain text and images.**

### M11. Rich text  [after the beta, parallel with M12 to M14; may move to v1.1]
- Depends on: M10. Machine: PC, Mac, VMs. Model: **Opus writes the `CF_HTML` parser and builder**
  (hostile input), Sonnet does the rest.
- Tasks: HTML part on all three desktop backends; caps; the "Keep formatting" setting.
- Accept: byte-level fixtures recorded from real browsers (`CF_HTML`, `public.html`, `text/html`)
  make a round trip through every backend pair and come out equal after normalization. The parser
  passes fuzzing. One manual check by you: copy from a browser on one OS, paste into a rich editor
  on another.

### M12. Mobile bindings (`uc-mobile`)
- Depends on: M10, S5, S7. Machine: PC + Mac. Model: Opus.
- Tasks: UniFFI API (pair, send clip, fetch latest, status, members, "no always-on member");
  the iOS and Android implementations of the mDNS and socket-binding traits from M2; build
  scripts for XCFramework and AAR; shared Keychain and Keystore backends.
- Accept: host tests pass. A sample call from Swift and from Kotlin pairs with a desktop agent.

### M13. Android app  [parallel with M14]
- Depends on: M12. Machine: PC. Model: Sonnet for UI, Opus for the service and clipboard.
- Tasks: Compose UI (onboarding with the two permission rows, QR scan with Google's code scanner,
  devices, settings); foreground service with background clipboard write, or the "Tap to copy"
  notification if S7 said so; share target (a Share sheet entry that sends a copy); read the clipboard once when the app opens;
  boot start; sensitive flag; battery page; a clear state when the local network permission is
  refused (the phone cannot sync until it is granted).
- Accept: emulator tests pass headless on API 29 and API 37, with the local network permission
  granted and refused. Computer to phone works as S7 decided. Each phone to computer path works in
  one tap. The LAN checklist is ready for the Play testers (or passes on the test phone).

### M14. iOS and iPadOS app  [parallel with M13]
- Depends on: M12. Machine: Mac. Model: Opus for the extension, Sonnet for SwiftUI.
- Tasks: SwiftUI app (onboarding, QR scan with the camera prompt, devices, settings, "How it
  works" page); on open: fetch the newest clip and write it (`.localOnly`), and read the clipboard
  (`UIPasteControl` or the Allow setting) and share it if it is new; Share Extension that sends a
  copy; the "No computer is on" state; shared Keychain group. No App Intents, no Shortcuts, no
  Control Center control.
- Accept: Simulator tests pass headless and pair with the PC over the LAN. Your real-device
  checklist passes through TestFlight on the iPhone and the iPad.

### M15. Stores, docs, beta
- Depends on: M13, M14. Machine: PC, Mac. Model: Sonnet.
- Tasks: App Store Connect, Play Console and Microsoft Store listings, privacy labels, privacy
  policy link, the Play foreground-service declaration, screenshots; TestFlight build; Play closed
  test (12 testers, 14 days); README (with the `readme` skill) with the honest platform list; help
  pages for each permission and each error, the VPN help page from S1 (including where "Local network sharing" is in Mullvad and other VPNs), the final privacy policy; a short page "What phones
  can and cannot do".
- Accept: all store builds are in review or approved. The README covers install and pairing in
  under one screen per OS.

### M16. v1.0
- Depends on: M15 and at least two weeks of your own daily use on PC, Mac, iPhone and iPad.
- Tasks: fix beta findings; remove `spikes/`; tag v1.0; replace the private `clipsync` prototype on
  your machines with the app (default profile) and update the `device-transfer` skill.
- Accept: release files signed and published. The prototype's scheduled task and LaunchAgent are removed.

**Time estimate (raised after review, with the cuts in section 11 applied):**
- M0 (bootstrap, lab, 7 spikes, paper review): 1.5 to 2 weeks.
- Desktop beta (M10): 6 to 8 weeks from the start (no relay work).
- Phones and rich text (M11 to M14): 4 to 5 weeks more.
- Stores and your two weeks of daily use (M15, M16): 3 to 4 weeks, partly waiting time we do not
  control (store reviews, the Play 14-day test, signing approvals).
- **Plan for 13 to 17 weeks to v1.0** (one week less than before, because there is no relay or spike S6). The first estimate (8 to 12 weeks) did not include the
  VM lab, two more spikes, the Store package or the store review loops.
- Running cost after launch: no server cost (there is no server). Only the Apple fee and the one-time or yearly store fees in section 8.2.

---

## 8. Distribution

### 8.1 Installers and signing

| Platform | Package | Signing | Channel |
|---|---|---|---|
| Windows | MSIX | Signed by the Store | **Microsoft Store: first choice, if S8 passes.** No SmartScreen warning, silent updates, firewall rule in the package. |
| Windows | NSIS per-user `.exe` | Updater signature. OS signature when SignPath (or Certum) accepts us; **unsigned until then**. | GitHub Releases |
| macOS | `.dmg`, Apple silicon | Developer ID, hardened runtime, notarized and stapled | GitHub Releases |
| Linux | `.deb` with our apt repository | Repository signed with our apt key | GitHub Pages (apt), GitHub Releases |
| iOS, iPadOS | App Store build | Apple distribution certificate | TestFlight, then App Store |
| Android | AAB | Play App Signing | Google Play only |

Not in the first release: `.rpm` with a dnf repository (a v1.1 candidate; cheap to add once the
apt repository works), AppImage (it cannot be a set-and-forget install: autostart would point at a
file in Downloads), GitHub APK (no auto-update, second key), Intel Mac.

**Windows signing from Norway.** Microsoft's Artifact Signing does not take new individual
developers outside the USA and Canada. SignPath Foundation (free for open source) requires that
the project is **already released** in the form to be signed, has "a certain verifiable
reputation", a code signing policy on its home page and a privacy policy shown at install (their
terms, checked by the reviewer today). A new, empty repo will likely be refused at first. Certum's
open-source certificate sits on a smart card or in SimplySign, which is hard to use from CI.
So, in this order:
1. **Microsoft Store (MSIX)** as the main Windows channel, if S8 passes.
2. The GitHub `.exe` ships unsigned (SmartScreen warning) and says so on the download page.
3. Apply to **SignPath Foundation** after the desktop beta is public, with the policy pages ready.
4. **Certum** only if 1 and 3 both fail.

### 8.2 Costs (to recheck when buying)

| Item | Cost | Needed for |
|---|---|---|
| Apple Developer Program | about 99 USD per year | Mac notarization, iOS and iPadOS |
| Google Play Console | about 25 USD once | Android on Play. New personal accounts must run a closed test (12 testers, 14 days) before going public. |
| Microsoft Store developer account | free or a small one-time fee for individuals (unverified, S8) | Windows Store channel |
| Windows signing for the exe | 0 (SignPath Foundation, later) or roughly 70 to 120 EUR per year (Certum, unverified) | No SmartScreen warning on the GitHub exe |
| Android test phone (optional) | about 100 EUR once, used | Automatic Android LAN and background tests (decision 10) |
| GitHub Actions, Releases, Pages | 0 for a public repo | CI, downloads, apt repository, privacy page |

### 8.3 The name risk

"Universal Clipboard" is the name of Apple's own feature. App Review guideline 5.2.5 and Apple's
trademark rules make a rejection of that app name likely on the App Store, and the name could also
confuse Mac users ("is this Apple's?").

Decision in this plan: all internal names are neutral (`uclip`), and the display name is one config
value. Fallback display name: **Copybridge** (second choice: **Clipbridge**). Neither name has been
checked for trademarks or existing apps yet; that check is a task in M0a.
My recommendation is in section 9: use the fallback name on every platform from the start, so the
product has one name.

### 8.4 Release signing and auto-update

**Decision: CI signs. The key is not "offline".** Revision 1 said both "held offline" and "CI
builds and signs", which cannot both be true.
- The updater key and the apt repository key are secrets of a GitHub **environment** named
  `release`. That environment needs your approval for every run (required reviewer) and is limited
  to tags on the default branch.
- A release is: tag, `release.yml` builds all packages, **waits for your click**, signs, uploads
  the files and `latest.json` at 10% rollout. A second small workflow run raises the rollout to 100%.
- The threat table says what this does not protect against (row 11).
- A printed or offline copy of the updater key is kept by you as a backup. Losing the key would
  mean no device can ever update again.
- Updating itself: section 3.13. Phones and the Store build update through the stores.

### 8.5 Uninstall: what is removed, what stays, what the user sees

The clean way on every OS is **"Remove this device and forget everything"** in the app first. It
tells the group that the device left and wipes the key, the state and the autostart entry. The
table says what happens when the user skips that and only uninstalls.

| OS | How | Removed | Stays | What the user sees |
|---|---|---|---|---|
| Windows (Store) | Uninstall in Settings | App, agent, startup task, firewall rule, package data | The Credential Manager entry (an unusable key). The device stays in the group's list as "Asleep or away" until someone removes it. | Nothing more. |
| Windows (exe) | Uninstall in Settings | App, agent, Run value, state folder, Credential Manager entry | The firewall rule, unless the user accepts **one UAC prompt** that the uninstaller offers ("Also remove the firewall rule"). | One optional UAC prompt. The uninstaller tries to tell the group first (2 s limit). |
| macOS | Drag the app to the Trash. There is no uninstaller. | The app. macOS drops the login item when the bundle is gone. | The Keychain item, `~/Library/Application Support/uclip`, the Local Network and Paste settings entries. | The help page and the README say: use "Remove this device and forget everything" before you delete the app. |
| Linux (`.deb`) | `apt remove` | App, agent, the apt source entry | Per-user files: the autostart entry, state folder, keyring entry (a package cannot safely touch home folders). The agent's supervisor exits when the binary is gone; the dead autostart entry is harmless. | Same advice as macOS. `apt purge` prints that advice. |
| Android | Uninstall | Everything | The device in the group's list. | Nothing. |
| iOS, iPadOS | Delete the app | App and its data | **Keychain items survive app deletion.** On the next install the app finds the old key; it asks "Set up again or continue as <name>?". | Nothing. |

In every "stays in the list" case, the other devices show the device as "Asleep or away", and the
user can remove it there. M9's acceptance has an uninstall test per desktop OS that checks this table.

---

## 9. Risks and open decisions for Max

### Decisions (please answer these with the approval)

| # | Decision | Recommendation |
|---|---|---|
| 1 | **Product name.** "Universal Clipboard" will probably be refused by Apple. | Use one neutral name everywhere (Copybridge, after a name check). Keep the repo name for now. |
| 2 | (Closed 2026-11-13.) There is no internet path. Local network only. | Done. A VPN that blocks the LAN gets clear help instead. |
| 3 | **iOS keyboard extension and Android accessibility service.** They could cut one tap, but they need "Full Access" or a sensitive permission, and store review is strict. | Not in v1. Decide after real use of the Share sheet and app-open sync. |
| 4 | **Accounts to open:** Apple Developer Program (99 USD per year), Google Play Console (25 USD), Microsoft Store developer account (after S8 says it is possible). SignPath comes later (8.1). | Open Apple and Google at the start of M0, because approval takes days to weeks. |
| 5 | **Mockup tools** (your rule: you choose). | `impeccable` for the screens. A simple logo with the local image stack. |
| 6 | **GNOME on Wayland** if spike S4 finds no clean path. This is the default desktop of Ubuntu and Fedora. | The build stops and asks you. My advice then: ship with "Linux: KDE and X11 desktops only" stated plainly, and revisit after v1. |
| 7 | (Closed 2026-11-13.) There is no relay. | Done. |
| 8 | **Forward clips that arrive on a Mac through Apple's own Universal Clipboard?** It would give automatic iPhone to Mac to PC sync for Apple users, with no tap. | Yes, as a setting that is on by default, but only if S2 shows the read is fast and silent. Else off. |
| 9 | **The VM lab.** A Hyper-V external switch on the PC (a few seconds of network interruption, once), four VMs (about 150 GB disk, memory only while tests run), a macOS VM on the Mac. | Yes. Without it, three of your five pairs are tested only with fakes. |
| 10 | **A cheap Android test phone on USB** (about 100 EUR used). You have no Android device, and the emulator cannot test the real LAN or OEM background limits. | Yes, if Android matters to you. Else the Play testers do a checklist by hand. |
| 11 | **Windows main channel: Microsoft Store**, with the GitHub exe unsigned at first. | Yes, if S8 passes. |
| 12 | **Your iPhone on the Mac for spike S5**, and one on-screen prompt session on the Mac for S2. | Yes. Both are short, and both remove the largest unknowns before code is written. |

### Risks

| Risk | Effect | What we do |
|---|---|---|
| iroh cannot tie sockets to a device with its stable API | The overlapping-route VPN case has no direct path | S1 tests three options on three OS. Worst case: that case shows the VPN help, and the user turns on local network sharing. |
| Many VPNs block the LAN by default | With such a VPN, devices cannot sync until the user turns on local network sharing | Clear detection (S1), one Fix button, a help page with real VPN settings (Mullvad "Local network sharing" and others). Limit stated plainly: no internet fallback. |
| Devices on different networks never sync (by design) | A laptop away from home does not get clips | Stated in the README and the app. A possible later version may add an internet path; it is out of scope now. |
| macOS pasteboard prompt on every read | Mac to others not usable | S2 with a fixed decision tree. Stop and ask you if it fails. |
| GNOME Wayland has no clean path | "Linux" means KDE and X11 only | S4 is a go/no-go. Honest text. Your decision. |
| Android background write is blocked, noisy or refused by Play | Computer to Android is one tap, not automatic | S7 before any Android code. Fallback: "Tap to copy" notification. |
| iOS gives no background access | iPhone syncs only when the app is open or through the Share sheet | Stated plainly in the app (section 2.6). S5 on your real iPhone checks app-open sync. |
| Phone experience disappoints (one tap, not zero; needs a computer that is on) | Users expect Apple-like magic | Say it plainly in the app and the store text. Decision 8 gives Apple users a no-tap path. |
| Store review rejects the app (name, background service type, permissions) | Delay for phones | Neutral name. Smallest permission set. Desktop ships first and does not depend on the stores. |
| The GitHub exe is unsigned for months | SmartScreen warning | The Store is the main Windows channel. SignPath after the public beta. |
| A bad silent update | Every device breaks, nobody sees it | Staged rollout, health check, automatic rollback, tested in M9. |
| Password managers that set no private marker | A password is synced | Honest help text. Pause switch. A per-app ignore list is a candidate for v1.1. |
| Competing apps improve (UniClipboard, CrossPaste) | Less reason to build | Our edge is setup, no tray, VPN handling and honesty. Recheck them before M15. |
| Size of v1 (five platforms, a lab) | Schedule slips | Desktop beta is a full product at M10. Phones follow. Rich text can move to v1.1. |

---

## 10. What was checked, and what was not

Checked on 2026-11-13 by the planner:
- iroh is past 1.0. docs.rs shows 1.4.1. The builder has `bind_addr`, `bind_addr_with_opts`,
  `clear_ip_transports`, `address_lookup`, `relay_mode`. (https://docs.rs/iroh/latest/iroh/endpoint/struct.Builder.html)
- iroh local discovery lives in separate crates, `iroh-mdns-address-lookup` 0.5 and
  `iroh-mdns-peer-lookup` 0.1. (https://docs.iroh.computer/connecting/local-discovery)
- macOS: `NSPasteboard.accessBehavior` and `detect` methods exist; the general pasteboard's
  documented default is "ask"; a third-party report says the prompt is opt-in on macOS 26.
  (https://developer.apple.com/documentation/appkit/nspasteboard/accessbehavior-swift.enum/default.md,
  https://mjtsai.com/blog/2025/05/12/pasteboard-privacy-preview-in-macos-15-4)
- Wayland: `ext-data-control-v1` is the standard protocol; KDE supports it; no source confirms
  GNOME support. (https://wayland.app/protocols/ext-data-control-v1, https://gitlab.gnome.org/GNOME/mutter/-/issues/524)
- Tauri has no built-in support for iOS extension targets; community plugins need manual Xcode
  targets. (https://docs.rs/crate/tauri-plugin-mobile-sharetarget/latest)
- Artifact Signing: individuals only in the USA and Canada; Basic plan 9.99 USD per month.
  (https://learn.microsoft.com/en-us/answers/questions/5810735/cant-create-a-new-trusted-signing-individual-ident,
  https://azure.microsoft.com/en-gb/pricing/details/artifact-signing/)
- iOS VPN flags `includeAllNetworks` and `excludeLocalNetworks` decide whether LAN traffic passes.
  (https://developer.apple.com/forums/thread/132477, https://mullvad.net/en/blog/why-we-still-dont-use-includeallnetworks)
- Apple rejects app names that use Apple product names under guideline 5.2.5 (examples for
  "iMessage" and "Mac"). No report about "Universal Clipboard" itself was found; the risk is an
  inference. (https://developer.apple.com/forums/thread/129822)

Checked on 2026-11-13 by the reviewer (taken over into this revision, not fetched again by me):
- iroh 1.4.1 binds to addresses only; no pre-bound sockets and no device binding; the custom
  transport hook is behind `unstable-custom-transports`. (docs.rs page above)
- n0 public relays are "suitable for development and testing", rate-limited, with no uptime or
  performance guarantee. (https://docs.iroh.computer/concepts/relays) This no longer matters: the
  app uses no relay since 2026-11-13.
- Android: `ACCESS_LOCAL_NETWORK` is a required runtime permission at target SDK 37; without it
  LAN packets fail. (https://developer.android.com/privacy-and-security/local-network-permission)
- SignPath Foundation terms: project already released, verifiable reputation, code signing policy,
  privacy policy. (https://signpath.org/terms)

Not checked, from general knowledge, to verify in the named spike or milestone:
- Exact crate versions other than iroh and tauri-cli (M0a).
- Windows Security Alert behaviour, block rules after a dismissal, firewall profiles (S1).
- Default LAN settings of consumer VPNs (S1).
- Whether address binding beats the route table per OS (S1).
- macOS: process attribution for Local Network privacy when started from ssh (Apple TN3179 did not
  load for the reviewer), Local Network prompt naming for an `SMAppService` agent, pasteboard
  default on 27.2, the `com.apple.is-remote-clipboard` type (S2).
- Clipboard isolation of a private window station (S3).
- Whether GNOME still ships an X11 session, and whether Mutter passes all Wayland copies to
  XWayland clients (S4).
- iOS: extension memory limits against the iroh core, pasteboard access from background intents,
  clipboard read and write on app open, Local Network behaviour (S5).
- That iroh works with the relay disabled and no address lookup (S1). The S6 items (own relay,
  long-code rendezvous, paid plan) are dropped.
- Android: clipboard preview overlay on background writes, Play policy for `connectedDevice`,
  OEM background killing (S7).
- Microsoft Store: individual accounts in Norway, firewall rules and startup tasks in MSIX (S8).
- (Checked in M0c: RustCrypto `spake2` is the python-spake2 variant, not the RFC 9382 wire format; no maintained CPace crate exists. See `docs/protocol.md` 2.1.)
- Certum terms and prices, Play Console test rule, store fees (M0a, M15).
- Name availability of Copybridge and Clipbridge (M0a).

---

## 11. Review notes

An independent review of revision 1 gave 43 points. This revision applies them. Short record:

**Applied as written (by review number):** 1 (short code LAN only), 2 (own relay as a v1
deliverable, privacy policy), 3 (iroh binding marked unproven, S1 with three options on three OS),
4 (GNOME Wayland go/no-go, fixed order), 5 (VM lab, pair table), 6 (membership chain with
validity rule and attack tests), 7 (no age limit on explicit fetch, 1 hour automatic, clock
limit), 8 (iPhone-only groups), 9 (Android local network permission), 10 (iOS Shortcuts
redesign), 11 (spike S7), 12 (VPN default claim removed), 13 (LaunchAgent test path), 14 (macOS
decision tree), 15 (Apple and Windows cloud double delivery), 16 (firewall order, both profiles),
17 (XDG autostart everywhere), 19 (rollback, staged rollout, static manifest), 20 (Store spike S8,
exe unsigned at first), 21 (minimum OS versions), 22 (iOS binding text), 23 (camera permission, no
iOS notification row), 25 (no AppImage), 26 (no GitHub APK), 27 (uninstall table), 28 (no sweep),
29 ("No path at all" state, client isolation), 30 (`dev` profile), 31 (real VPN tests in M2),
32 (reboot tests in VMs), 33 (byte-level fixtures), 34 (measures for screenshots, start time and
soak), 36 (WSL kernel check, lab in M0a), 37 (transcript, confirmation, dropped connection),
38 (content hash, line endings, file clips), 39 (IPC hardening), 40 (fresh-context reviews, paper
review as M0c), 41 (model split in M9 and M11), 42 (cuts), 43 (estimate raised to 14 to 18 weeks
AND the cuts applied).

**Applied with a change:**
- **1 and 2, long code and lookup service.** The reviewer asked for an unguessable rendezvous key
  and said the DNS/pkarr service has the same problem as the public relays. This revision goes one
  step further and uses no lookup service at all: every device uses our one relay, and the long
  code derives a temporary endpoint key that both sides can compute. S6 must prove it; the
  fallback is our own `iroh-dns-server`.
- **5, Linux VMs.** The reviewer named Fedora KDE. This revision uses Kubuntu for the KDE VM,
  because `.rpm` is not in the first release (point 24), so a Fedora VM could not install the app.
- **7, wrong clocks.** A refused future stamp would make a device with a wrong clock fail silently.
  So this revision adds a visible state with a Fix button for it.
- **18, updater key.** Chosen: CI signs inside a protected GitHub environment that needs Max's
  approval. Not chosen: local signing by Max, because it makes every release a manual task on one
  machine.
- **24, Linux updates.** Chosen: `.deb` with an apt repository in v1; `.rpm` with a dnf repository
  moves to v1.1.
- **35, Android LAN test.** A bridged Linux VM does not help: the Android emulator keeps its own
  NAT inside any host. So this revision names the manual test (Play closed-test testers, with a
  checklist) and offers a USB test phone as decision 10. The other half of point 35 (mobile traits
  in M1 and M2) is applied as written.

**Not applied:**
- **4 and 42, "GNOME Shell extension last".** Point 4 lists the extension as the last option;
  point 42 cuts it. This revision follows 42: no extension in v1.
- **21, Windows 10.** The reviewer offered "Windows 10 1809+ or Windows 11 only". This revision
  supports Windows 11 and lets Windows 10 22H2 install without a promise, because there is no
  Windows 10 test machine and Windows 10 is out of support.
- Nothing else was rejected.

**Added beyond the review:** decision 12 (your iPhone for S5), the relay misuse threat (row 16),
the "update failed" state, and the backup copy of the updater key.

**Revision 3 (2026-11-13, scope change to local network only).** Everything about the relay, the
internet path, the long code and the iOS Shortcuts was removed or rewritten. The review notes above
describe revision 2 and are kept as history. Where they mention a relay, the long code or
"long-code rendezvous" (points 1, 2 and 18 and the "Applied with a change" entry), read them as
superseded by the section "Decisions after approval".
