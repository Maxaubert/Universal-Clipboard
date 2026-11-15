# Installers, firewall helper, updater and releases (M9)

Date: 2026-11-14. Plan: sections 2.1, 3.2, 3.13, 4.1, 7 (M9), 8. Spike: `docs/spikes/s8-msix.md`.

Windows builds are **unsigned**. macOS builds are ad hoc by default (PR CI, local builds);
release builds are signed with Max's Developer ID and notarized in CI (see "Signing and
notarization"). The update manifest is signed with our own
updater key. Nothing in this milestone was installed
on Max's PC or Mac: install and uninstall tests run on CI runners and in Docker.

## What is built

| OS | Files | Script | Where it is tested |
|---|---|---|---|
| Windows | `UniversalClipboard-<v>-windows-x64-setup.exe` (NSIS, per user; one UAC prompt only for the firewall rule) | `packaging/windows/build.ps1` | CI: silent install with `/ADDFIREWALL` (the rule is made by the installer), install over the top while the agent and a stand-in window run, a locked file fails the install, update over the top (every exe hash-compared with the build), uninstall, name rule, no rule without `/ADDFIREWALL` (`packaging/windows/test-install.ps1`) |
| Windows | `UniversalClipboard-<v>-windows-x64.msix` (Store build, unsigned) | same | CI: signed with a throwaway certificate, installed, alias run, removed |
| macOS | `UniversalClipboard-<v>-macos-aarch64.dmg` and `...app.tar.gz` (the update archive) | `packaging/macos/build-app.sh` | CI: DMG verified and mounted read-only, bundle signature checked. The Mac mini: bundle built over ssh (no DMG, `SKIP_DMG=1`, nothing opened) |
| Linux | `universal-clipboard_<v>_amd64.deb` | `packaging/linux/build-deb.sh` | Docker: `packaging/linux/test-deb.sh` (also checks the window and its libraries) in `ubuntu:22.04`, `ubuntu:24.04`, `debian:12` |

The window (M8b, Tauri 2, `apps/desktop`) is in every installer. `package.yml` builds it on each
OS with `npm ci && npx tauri build --no-bundle` (the shell has its own `Cargo.lock` and is not
in the root workspace) and puts it in the bin folder under the name each script expects:
`Universal Clipboard.exe` (Windows), `universal-clipboard` (macOS, Linux). Linux builds it on
ubuntu-22.04 with `libwebkit2gtk-4.1-dev`, `libsoup-3.0-dev`, `libgtk-3-dev`, `librsvg2-dev` and
`libssl-dev`, so glibc stays 2.35. The `.deb` then depends on `libwebkit2gtk-4.1-0` and
`libgtk-3-0`. The install tests check that the window file is in the installed package; they
never start it. A local build without the window still works: the scripts skip it, the macOS
bundle then runs the agent as its main program (`LSUIElement`), and the MSIX has no visible app.

`rpm` and AppImage are cut from v1 (plan 8.1). The signed apt repository (plan 8.1) is not
built yet: it needs an apt key and GitHub Pages. Until then, apt updates do not happen; a new
`.deb` is installed by hand.

## Package contents

- **Windows (NSIS):** `%LOCALAPPDATA%\Programs\Universal Clipboard\`: `uc-agent.exe`, `uc.exe`
  (or `uclip.exe`), the window, `Uninstall.exe`, `cli-name.txt`. Uninstall entry in
  `HKCU\...\Uninstall\UniversalClipboard`. The folder goes on the user PATH
  (`HKCU\Environment`). Switches: `/S`, `/UPDATE` (the updater: no stop, nothing opened),
  `/ADDFIREWALL` (also add the firewall rule in a silent install), `/D=<folder>`, uninstaller
  `/REMOVEFIREWALL`.
- **Windows (NSIS), replacing a running app (bug and fix 2026-11-14):** a silent install over
  the top used to replace `uc.exe` and the window but not `uc-agent.exe`, with exit code 0.
  Cause: the installer stopped the agent first and closed the window second, and the open
  window started the agent again, so `uc-agent.exe` was in use and NSIS skipped it. Now
  (`packaging/windows/nsis/close-app.ps1`, packed into the setup exe, only processes that run
  from the install folder): 1. close the window (5 s, then end it), 2. stop the agent
  (`uc-agent --stop`, not with `/UPDATE`), 3. wait up to 15 s until no `uc-agent.exe`, `uc.exe`,
  `uclip.exe` or window from the folder runs, then end them (the last resort). 4. Every program
  file must open for writing, else nothing is replaced: silent mode tries 3 times, then exits
  with code 2; interactive mode shows Retry/Cancel. 5. The old program files are deleted
  before the copy and each new one must be there after it, so a failed write is an error
  (exit 2), never a silent mix of old and new. The uninstaller also closes the window before
  `--uninstall-hook` (a restarted agent made the state folder and the Run value again).
- **The self-updater on Windows** is safe from the same race: the helper writes the update
  marker (`<state>/update/applying`, see "How the agent is started on macOS") before it stops
  the agent, on every OS, and the window starts the agent only through `uc_cli::connect`,
  which starts nothing while the marker is there. The setup exe then closes the window to
  replace it. If a file is still in use, the setup exits 2 and the helper rolls back to the
  old version (and marks the new one bad) instead of leaving a mixed install.
- **Windows (MSIX):** manifest `packaging/windows/msix/AppxManifest.xml.in`. Apps: Settings
  (the window), Agent (no list entry, startup task `UcAgentStartup`), Cli (aliases `uc.exe`
  and `uclip.exe`). Firewall rule at package level: inbound UDP, Profile private. `runFullTrust`
  needs Store approval on submit (S8).
- **macOS:** `Universal Clipboard.app/Contents/`: `MacOS/{uc-agent,uc,universal-clipboard}`,
  `Library/LaunchAgents/io.github.maxaubert.uclip.agent.plist` (registered with
  `SMAppService.agent` when start at login is on), `Info.plist` with the Local Network text and
  both Bonjour services. Bundle ID `io.github.maxaubert.uclip`; the agent is signed as
  `io.github.maxaubert.uclip.agent`, the CLI as `...cli`.
- **Linux:** `/usr/lib/universal-clipboard/`, links `/usr/bin/uc-agent` and `/usr/bin/uc` (or
  `uclip`), the icon, and a menu entry when the window is built. Start at login is per user
  (XDG autostart, made by the agent on its first run, decision 2026-11-14), never by the
  package.

## The `uc` name rule (plan 3.2)

The installer names the CLI `uclip` when a `uc` command already exists, and says so.

| Build | How |
|---|---|
| Windows NSIS | `uc-agent --install-hook` looks for `uc.exe`, `uc.cmd`, `uc.bat`, `uc.com`, `uc.ps1` in every PATH folder except ours, renames `uc.exe` to `uclip.exe` if one is found, and prints the line the installer shows. |
| Windows MSIX | No install script exists in MSIX. Both aliases `uc` and `uclip` are registered. The alias folder is at the end of the user PATH, so an existing `uc` keeps working and `uclip` always works. |
| macOS | `uc-agent --install-hook` (from `uc setup` or the window, M8b) links `uc` or `uclip` in `/usr/local/bin` when it is writable, else in `~/.local/bin`, and says when that folder is not on PATH. |
| Linux | `postinst` checks the standard folders for a `uc` that is not ours and links `/usr/bin/uc` or `/usr/bin/uclip`. |

## Uninstall (plan 8.5)

| Build | What happens | Tested |
|---|---|---|
| Windows NSIS | `uc-agent --uninstall-hook`: `leave` (tells the group, 2 s limit), stop the agent, remove the Run value, the state folder, the key store entries and the PATH entry. Then the files and the uninstall entry. The firewall rule: one question and one UAC prompt (`/REMOVEFIREWALL` in silent mode). | CI |
| Windows MSIX | Windows removes the package, the startup task, the firewall rule and the package data. The Credential Manager entry stays (plan 8.5). | CI (install and remove) |
| macOS | Drag the app to the Trash; macOS drops the login item. The help text says to use "Remove this device and forget everything" first. | not testable without a VM |
| Linux | `apt remove`: files and links go, `postrm` prints what stays (autostart entry, state, keyring) and how to clean up. The supervisor of a running agent ends when the binary is gone. | Docker |

## How the agent is started on macOS (2026-11-14, Mac review)

Code: `crates/uc-agent/src/launch.rs`. Checked on the Mac mini (macOS 27.2).

- **Always a launchd job** (real profile, inside the bundle). `uc`, the window and the update
  helper never start `uc-agent` as their own child: macOS would give the Local Network and
  "Paste from Other Apps" permissions to the terminal or the window. The agent always runs as
  the session-only job `io.github.maxaubert.uclip.agent.run`, loaded from `<state>/launchd/`
  (gone after logout): `kickstart` when it is loaded with the same program, else `bootstrap`
  (`bootout` first only when the app moved). An agent that holds `agent.lock` is never
  touched. Test profiles, Windows and Linux start a detached child as before.
- **The login item is only a launcher.** Started by `SMAppService` (`XPC_SERVICE_NAME` is the
  label), `uc-agent` starts the session job and exits 0. So turning start at login off
  (unregister) never ends the running agent. Registering it runs it once at once.
- **"Already running" under launchd exits 0**, so `KeepAlive {SuccessfulExit=false}` does not
  restart a second agent every 10 s.
- **Update marker** `<state>/update/applying` (holds the helper's PID): the helper writes it
  when it stops the old agent and removes it when the new files are in place, right before it
  starts the new agent (also on rollback). While it is there, `uc` and the window start nothing
  and an agent that launchd restarts exits 0. It is stale (and removed) when the helper is gone
  or after 15 min. The window refreshes when its event stream reconnects, so it leaves "agent
  is down" by itself. Remove this device (`--uninstall-hook`) also unloads the session job.
- **App location:** run from a read-only volume under `/Volumes` (the DMG) or App
  Translocation (another drive is fine), the agent refuses
  start at login, the `uc` link and self-update, and the status is "Needs attention"
  (`app-location`): move the app to Applications. The helper also checks that the bundle can
  be written before it stops the agent.

## Firewall helper (Windows exe build)

`uc-agent --firewall add|remove|check [--exe PATH]`. One inbound rule "Universal Clipboard" for
the agent program: UDP, **Private profile only, remote LocalSubnet only**, never Public (plan
4.1). Add and remove need an administrator: when not elevated, the helper starts itself with
`runas` (one UAC prompt) and waits. Exit codes: 0 done, 1 failed, 3 the user said no.

- **Decision 2026-11-14 (Max): the NSIS installer adds the rule at install time.** Windows
  setup asks nothing that shows a system prompt later. After the files and the install hook,
  the installer runs `uc-agent --firewall check`; when the rule is missing it runs
  `--firewall add` (one UAC prompt, the helper elevates only itself; the install stays per
  user). "No" (exit 3) or a failure (exit 1) does not stop the install. A silent install
  (`/S`, also the updater) never prompts unless `/ADDFIREWALL` is given; an update keeps the
  `uc-agent.exe` path the rule is tied to, so it keeps working.
- The agent checks the rule at each network start (`netsh ... show rule`, no administrator, no
  window). There: it accepts incoming connections. Missing: outbound-only mode, and `uc status`
  (and the IPC `status` reply, field `note`) shows a non-blocking note with the fix. Only when
  the device is in a group and no other device is connected does the status become "Needs
  attention" (`fix_code` `firewall`). The network never waits for a setup answer.
- The fix: `uc firewall add` (runs the helper, one UAC prompt, then IPC `firewall.check`) and
  the window's Fix (the shell command `firewall_add`, the same code: `uc_cli::add_firewall_rule`).
  `uc firewall check` shows the state. The agent itself never shows a UAC prompt.
- Old `state.json` files have a `firewall` field (the setup row before 2026-11-14). They still
  load; the field is ignored.
- Test profiles never elevate and use their own rule name (`Universal Clipboard test <name>`).
  The agent in a test profile never looks at the real rule: `UCLIP_TEST_FIREWALL_RULE=absent`
  (or `present`) fakes the check (`crates/uc-cli/tests/agents.rs`).
  The real add, check and remove run only on CI (`UCLIP_TEST_FIREWALL=1`, elevated runner).
- The MSIX manifest **cannot** limit remote addresses: `RemoteAddress` on `desktop2:Rule` is
  rejected by makeappx ("not defined in the DTD/Schema", checked 2026-11-14). The Store build
  relies on the agent's own LAN rule (protocol 4.8: connections from outside the LAN are closed
  before the handshake).

## Self-update (plan 3.13)

Only the Windows exe build and the macOS DMG build update themselves (cargo feature
`self-update`, on by default; the `.deb` is built with `--no-default-features`; the MSIX turns
it off at run time). Code: `crates/uc-update`, `crates/uc-agent/src/update.rs`.

1. **Once per agent start, never again in that agent's life** (decision 2026-11-14, Max): 15 s
   after the agent starts (login, reboot, after an update) it fetches
   `https://github.com/Maxaubert/Universal-Clipboard/releases/latest/download/latest.json`
   with the system `curl` (HTTPS only; no TLS library in the agent, no GitHub API). There is no
   periodic check and no "check now" command.
2. `latest.json` = `{"manifest": "<json>", "signature": "<minisign>"}`. The signature covers the
   manifest (version, rollout, files with URL, size, SHA-256 and a signature each). Trusted
   comments bind each signature to the file name and the version.
3. Take it only if: newer than this version, not on the bad list, fewer than 3 failures in a
   row, and the device's rollout bucket (BLAKE3 of the device ID, 0 to 99) is under the
   rollout percentage. Then download, check size, hash and signature, and unpack (system `tar`).
4. **Nothing installs by itself.** The version is now "available": the agent emits the IPC
   event `update` and opens the window once on its update screen ("Version X is ready.",
   Update now, Later; see "How the agent opens the window" below; a window already open on
   Home shows it as a card there instead). Later (IPC `update.later`,
   `uc update later`) stops the asking until the next agent start. Update now (IPC
   `update.apply`, `uc update --install` or `uc update now`) copies the agent to
   `<state>/update/helper/` and starts the copy with `--apply-update` (detached, own process
   group). The helper: note the connected peers, ask the agent to quit (IPC `quit`, a service
   call with no `uc` command), copy the install folder to `<state>/update/previous`, install
   (Windows: `setup.exe /S /UPDATE /D=...`; macOS: swap the bundle contents), start the new
   agent, and wait up to 2 minutes for: the new version answers, the network is up, and at
   least one peer is back if one was connected before.
5. On failure: stop it, copy `previous` back, start the old agent, mark the version bad, and
   write `<state>/update/apply-failed.json`. On success it writes `<state>/update/apply-ok.json`.
   The agent that runs then reads (and removes) the note on its next tick (it looks every tick,
   because the note comes after the 2 minute health check): `update.status` state `failed` or
   `updated`, and the window opens once to say so ("The update did not work." or "Updated to
   X"). Try again: when the helper did not start, or the failed version is not marked bad (its
   start check stages it again), `available` stays set in state `failed` and `update.apply`
   (`uc update --install`) runs once more. A version marked bad cannot be tried again.
   Three failures in a row: status "Needs attention: The app could not update itself."
6. The report (`uc diagnostics`) shows the last result, failures in a row and bad versions.
   The helper keeps the last 50 results in `<state>/update/helper.log`.

`update.status` = `{enabled, current, available, failed_version, notes_url, state
(none|available|applying|failed|updated), dismissed, checked, checks, last_check_ms, last_error}`.
`uc update` shows it (`--json` for scripts).

**How the agent opens the window** (`open_window` in `crates/uc-agent/src/update.rs`; the agent
itself never shows a dialog). It starts the window as its own app with `--route update`:

| OS | How |
|---|---|
| Windows | `Universal Clipboard.exe` next to `uc-agent.exe`, detached |
| macOS | `/usr/bin/open -a "<the .app>" --args --route update` (LaunchServices: the window is its own responsible process, so no permission moves between it and the agent). Only when `Contents/MacOS/universal-clipboard` exists |
| Linux | `universal-clipboard` next to the agent (`/usr/lib/universal-clipboard/`), detached |

A window that is already open gets `--route update` through its single-instance handler
(Windows, Linux; on macOS `open` brings the running app forward) and also follows the
`update` event. The shell puts the window in the middle of the screen with the mouse pointer
and raises it once (always-on-top on and off again, plus a taskbar flash where focus is
refused). A pairing in progress (Show or Enter a code) is not interrupted: the update screen
follows when it ends. Test profiles never start a window: they append the route to
`<state>/update/window-opens.log`.

**Acceptance test "N, then a broken N+1, then rollback to N"**: `crates/uc-cli/tests/update.rs`,
headless and offline (a `file://` feed signed with a test key, test profile, fake clipboard).
Update now is the `uc update --install` call. A third test checks the start-only rule: one
check, one window request, nothing installed over a quiet run, Later holds until the agent
starts again.
It passes on Windows (PC), macOS (Mac mini over ssh) and in CI on all three. A second test
updates N to a good N+1. The real-VM version (`win-b`, `mac-b`) waits for the lab VMs.

## Keys

- **Updater key:** made 2026-11-14 with `uc-release keygen`. Secret:
  `C:\Users\Admin\.config\uclip-release\updater.key` on the PC (owner-only ACL), **outside the
  repo**. Public key: `UPDATER_PUBLIC_KEY` in `crates/uc-update/src/lib.rs`
  (`RWT+8yLErcCQ5xMELo4ZbtGumYTct/xDw17Bj/pMuQ6KAKYt13um8NKb`).
- To do (Max): make the GitHub environment `release` (required reviewer: Max; deployment
  branches and tags: tags `v*` only), and put the key in it:
  `gh secret set UCLIP_UPDATER_KEY --env release < C:\Users\Admin\.config\uclip-release\updater.key`.
  Keep an offline copy (plan 8.4): losing the key means no installed app can update again.
- apt key: not made (the apt repository is not built).

## Releases

- `release.yml`, on a tag `vX.Y.Z` that matches `Cargo.toml`: `package.yml` builds and tests
  everything, `macos-signed` (environment `release`, waits for Max) builds the signed and
  notarized macOS files (see "Signing and notarization"), then the `publish` job (environment
  `release`, waits for Max again) signs
  `latest.json` at 10% and makes a **draft** release. Without the secret there is no
  `latest.json`, so nothing auto-updates to that release.
- `rollout.yml` (run by hand): re-signs `latest.json` of a tag with a new percentage, for
  example 100 after two quiet days. Start it from the tag, because the `release` environment
  only accepts tags `v*`: `gh workflow run rollout.yml --ref v0.1.0 -f tag=v0.1.0 -f percent=100`
  (from main it fails with "Branch main is not allowed to deploy to release"). v0.1.0 went to
  100% on 2026-11-15.
- Local signing, if ever needed: `uc-release manifest --key <file> --version V --rollout 10
  --base-url URL --out latest.json windows-x86_64:nsis:<setup.exe> macos-aarch64:tar:<app.tar.gz>`.

## Signing and notarization

- **macOS:** `build-app.sh` signs ad hoc unless told otherwise (checked on the Mac mini 2026-11-14):
  `SIGN_IDENTITY="Developer ID Application: MAX AUBERT (WV7BY8CUM8)"` signs every program and
  the bundle with hardened runtime and a secure timestamp (and the DMG). `NOTARY_PROFILE=notary`
  also notarizes and staples the app (before the update archive and the DMG are made) and the
  DMG. The profile is made once: `xcrun notarytool store-credentials notary --apple-id <id>
  --team-id WV7BY8CUM8`. The certificate is in the Mac mini's login keychain. Without
  notarization, `spctl` says "Unnotarized Developer ID" and a downloaded copy is blocked.
  A Developer ID also keeps Keychain access and privacy grants across updates (an ad-hoc
  signature changes with every build). `NOTARY_KEYCHAIN=<path>` names the keychain that
  holds the profile when it is not the login keychain (CI uses this). `build-app.sh` fails
  when Apple does not answer "Accepted" (notarytool itself can exit 0 for "Invalid").
- **macOS in CI (2026-11-14):** `package.yml` (every PR) stays ad hoc and uses no secrets. In
  `release.yml`, the job `macos-signed` (environment `release`, so Max approves it; it starts
  next to `package`) builds the app again, imports the certificate into a temporary keychain,
  stores the API key as a notarytool profile in that keychain, and runs `build-app.sh` with
  `SIGN_IDENTITY` and `NOTARY_PROFILE`. It then checks the app, the DMG, the app inside the DMG
  and the unpacked update archive: `codesign --verify --deep --strict`, the Developer ID
  authority, `spctl -a -vv` must say `source=Notarized Developer ID`, `stapler validate`. A
  missing secret or any failed step fails the release: there is no ad-hoc fallback. The
  keychain and the key files are deleted at the end, also after a failure. `publish` takes the
  macOS files only from this job. With two jobs in `release`, Max approves twice per release.
  No rust or npm cache in this job (a job with signing keys does not restore caches).
- **Secrets of the `release` environment** (Max sets them; nothing is in the repo):

  | Secret | Value |
  |---|---|
  | `MACOS_CERT_P12` | the Developer ID Application certificate with its private key, `.p12`, base64 |
  | `MACOS_CERT_PASSWORD` | the password set when the `.p12` was exported |
  | `APPLE_API_KEY_P8` | the App Store Connect API key file `AuthKey_<id>.p8`, base64 |
  | `APPLE_API_KEY_ID` | its Key ID (10 characters) |
  | `APPLE_API_ISSUER_ID` | the Issuer ID (a UUID) on the same page |

- **To do (Max), once:**
  1. Export the certificate on the Mac mini. Keychain Access > login > My Certificates >
     "Developer ID Application: MAX AUBERT (WV7BY8CUM8)". Open the arrow and check that a
     private key is under it. Right-click the certificate > Export > format "Personal
     Information Exchange (.p12)" > save as `~/uc-cert.p12`, and set a strong password.
     (Terminal: `security export -k ~/Library/Keychains/login.keychain-db -t identities -f pkcs12
     -P '<password>' -o ~/uc-cert.p12` also works, but it exports every identity in the
     keychain, so the Keychain Access way is better.)
  2. Make the API key. App Store Connect > Users and Access > Integrations > App Store Connect
     API > Team Keys > "+" (the first time, the Account Holder must request API access). Name:
     `uclip-notary`, Access: **Developer**. Download `AuthKey_<id>.p8` (only possible once).
     Note the Key ID in the list and the Issuer ID above it.
  3. Set the secrets (on the Mac mini, in a clone of the repo or with `--repo
     Maxaubert/Universal-Clipboard`):

     ```sh
     base64 -i ~/uc-cert.p12 -o ~/uc-cert.p12.b64
     gh secret set MACOS_CERT_P12 --env release < ~/uc-cert.p12.b64
     gh secret set MACOS_CERT_PASSWORD --env release        # paste the password at the prompt
     base64 -i ~/Downloads/AuthKey_<id>.p8 -o ~/uc-notary.p8.b64
     gh secret set APPLE_API_KEY_P8 --env release < ~/uc-notary.p8.b64
     gh secret set APPLE_API_KEY_ID --env release            # paste the Key ID
     gh secret set APPLE_API_ISSUER_ID --env release         # paste the Issuer ID
     rm ~/uc-cert.p12 ~/uc-cert.p12.b64 ~/uc-notary.p8.b64
     ```

     On the PC instead (PowerShell, absolute paths): `[Convert]::ToBase64String([IO.File]::ReadAllBytes("C:\Users\Admin\Downloads\AuthKey_<id>.p8")) | gh secret set APPLE_API_KEY_P8 --env release`.
  4. Keep the `.p8` in a password manager or delete it; a lost key can be revoked and a new
     one made. Check with `gh secret list --env release` (six names with `UCLIP_UPDATER_KEY`).
- **Windows:** the exe stays unsigned until SignPath Foundation (or Certum) accepts the project
  (plan 8.1). The Store signs the MSIX on submit; its `Publisher` must then be the one Partner
  Center gives (`build.ps1 -Publisher`).

## App icon (2026-11-15)

"Paper Layers", given by Max (made with Codex). Sources, derived masters and the render script:
`packaging/assets/icon/README.md`. Contact sheet: `docs/design/icon/contact-sheet.png`.
Windows `.ico` and MSIX logos (with `resources.pri` from makepri), the macOS iconset (icon grid,
not full-bleed), Linux hicolor 16 to 512 plus scalable SVG, and the Tauri icons all come from it.
16 to 32 px use simpler two-layer art.

## Not done here

- The signed check that the pipe server is our agent (plan 3.2) needs code signing; the SID
  check from M7 stays.
- The apt repository and its key.
- Real-VM tests (`win-b`, `mac-b`, `lin-kde`): install with only the expected warnings, the
  update N to N+1 with no user action, the macOS uninstall table row.
