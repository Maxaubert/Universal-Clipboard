# Installers, firewall helper, updater and releases (M9)

Date: 2026-11-14. Plan: sections 2.1, 3.2, 3.13, 4.1, 7 (M9), 8. Spike: `docs/spikes/s8-msix.md`.

Everything is **unsigned** for now: no Windows code signing, no Apple Developer ID. Only the
update manifest is signed (with our own updater key). Nothing in this milestone was installed
on Max's PC or Mac: install and uninstall tests run on CI runners and in Docker.

## What is built

| OS | Files | Script | Where it is tested |
|---|---|---|---|
| Windows | `UniversalClipboard-<v>-windows-x64-setup.exe` (NSIS, per user, no admin) | `packaging/windows/build.ps1` | CI: silent install, uninstall, update over the top, name rule, firewall rule (`packaging/windows/test-install.ps1`) |
| Windows | `UniversalClipboard-<v>-windows-x64.msix` (Store build, unsigned) | same | CI: signed with a throwaway certificate, installed, alias run, removed |
| macOS | `UniversalClipboard-<v>-macos-aarch64.dmg` and `...app.tar.gz` (the update archive) | `packaging/macos/build-app.sh` | CI: DMG verified and mounted read-only, bundle signature checked. The Mac mini: bundle built over ssh (no DMG, `SKIP_DMG=1`, nothing opened) |
| Linux | `universal-clipboard_<v>_amd64.deb` | `packaging/linux/build-deb.sh` | Docker: `packaging/linux/test-deb.sh` in `ubuntu:22.04`, `ubuntu:24.04`, `debian:12` |

The window (M8b) is packaged when its binary is in the bin folder (`Universal Clipboard.exe`,
`universal-clipboard`). Without it the packages hold the agent and `uc` only: the macOS bundle
then runs the agent as its main program (`LSUIElement`), and the MSIX has no visible app.

`rpm` and AppImage are cut from v1 (plan 8.1). The signed apt repository (plan 8.1) is not
built yet: it needs an apt key and GitHub Pages. Until then, apt updates do not happen; a new
`.deb` is installed by hand.

## Package contents

- **Windows (NSIS):** `%LOCALAPPDATA%\Programs\Universal Clipboard\`: `uc-agent.exe`, `uc.exe`
  (or `uclip.exe`), the window, `Uninstall.exe`, `cli-name.txt`. Uninstall entry in
  `HKCU\...\Uninstall\UniversalClipboard`. The folder goes on the user PATH
  (`HKCU\Environment`). Switches: `/S`, `/UPDATE` (the updater: no stop, nothing opened),
  `/D=<folder>`, uninstaller `/REMOVEFIREWALL`.
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
  (XDG autostart, made by `uc setup` or the window), never by the package.

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

## Firewall helper (Windows exe build)

`uc-agent --firewall add|remove|check [--exe PATH]`. One inbound rule "Universal Clipboard" for
the agent program: UDP, **Private profile only, remote LocalSubnet only**, never Public (plan
4.1). Add and remove need an administrator: when not elevated, the helper starts itself with
`runas` (one UAC prompt) and waits. Exit codes: 0 done, 1 failed, 3 the user said no.

- `uc setup` runs it when the user says yes to "Let your devices find this PC". A "No" in the
  UAC prompt counts as no: the agent runs in outbound-only mode. The agent itself never shows
  a UAC prompt. The window (M8b) should do the same as `uc setup`.
- Test profiles never elevate and use their own rule name (`Universal Clipboard test <name>`).
  The real add, check and remove run only on CI (`UCLIP_TEST_FIREWALL=1`, elevated runner).
- The MSIX manifest **cannot** limit remote addresses: `RemoteAddress` on `desktop2:Rule` is
  rejected by makeappx ("not defined in the DTD/Schema", checked 2026-11-14). The Store build
  relies on the agent's own LAN rule (protocol 4.8: connections from outside the LAN are closed
  before the handshake).

## Self-update (plan 3.13)

Only the Windows exe build and the macOS DMG build update themselves (cargo feature
`self-update`, on by default; the `.deb` is built with `--no-default-features`; the MSIX turns
it off at run time). Code: `crates/uc-update`, `crates/uc-agent/src/update.rs`.

1. 10 minutes after start, then once a day, the agent fetches
   `https://github.com/Maxaubert/Universal-Clipboard/releases/latest/download/latest.json`
   with the system `curl` (HTTPS only; no TLS library in the agent, no GitHub API).
2. `latest.json` = `{"manifest": "<json>", "signature": "<minisign>"}`. The signature covers the
   manifest (version, rollout, files with URL, size, SHA-256 and a signature each). Trusted
   comments bind each signature to the file name and the version.
3. Take it only if: newer than this version, not on the bad list, fewer than 3 failures in a
   row, and the device's rollout bucket (BLAKE3 of the device ID, 0 to 99) is under the
   rollout percentage. Then download, check size, hash and signature, and unpack (system `tar`).
4. After 60 s with no clipboard activity, the agent copies itself to
   `<state>/update/helper/` and starts the copy with `--apply-update` (detached, own process
   group). The helper: note the connected peers, ask the agent to quit (IPC `quit`, a service
   call with no `uc` command), copy the install folder to `<state>/update/previous`, install
   (Windows: `setup.exe /S /UPDATE /D=...`; macOS: swap the bundle contents), start the new
   agent, and wait up to 2 minutes for: the new version answers, the network is up, and at
   least one peer is back if one was connected before.
5. On failure: stop it, copy `previous` back, start the old agent, mark the version bad.
   Three failures in a row: status "Needs attention: The app could not update itself."
6. The report (`uc diagnostics`) shows the last result, failures in a row and bad versions.
   The helper keeps the last 50 results in `<state>/update/helper.log`.

**Acceptance test "N, then a broken N+1, then rollback to N"**: `crates/uc-cli/tests/update.rs`,
headless and offline (a `file://` feed signed with a test key, test profile, fake clipboard).
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
  everything, then the `publish` job (environment `release`, waits for Max) signs
  `latest.json` at 10% and makes a **draft** release. Without the secret there is no
  `latest.json`, so nothing auto-updates to that release.
- `rollout.yml` (run by hand): re-signs `latest.json` of a tag with a new percentage, for
  example 100 after two quiet days.
- Local signing, if ever needed: `uc-release manifest --key <file> --version V --rollout 10
  --base-url URL --out latest.json windows-x86_64:nsis:<setup.exe> macos-aarch64:tar:<app.tar.gz>`.

## Signing and notarization (documented, not done)

- **macOS (needs Max's Developer ID, not used tonight):**
  1. `codesign --force --options runtime --timestamp --sign "Developer ID Application: <name> (<team>)"`
     each of `Contents/MacOS/*`, then the bundle.
  2. `xcrun notarytool submit <dmg> --keychain-profile <profile> --wait`
     (profile made once with `xcrun notarytool store-credentials`).
  3. `xcrun stapler staple <dmg>` (and the app).
  In CI the certificate and an App Store Connect API key would be secrets of the `release`
  environment.
- **Windows:** the exe stays unsigned until SignPath Foundation (or Certum) accepts the project
  (plan 8.1). The Store signs the MSIX on submit; its `Publisher` must then be the one Partner
  Center gives (`build.ps1 -Publisher`).

## Not done here

- The signed check that the pipe server is our agent (plan 3.2) needs code signing; the SID
  check from M7 stays.
- The apt repository and its key.
- Real-VM tests (`win-b`, `mac-b`, `lin-kde`): install with only the expected warnings, the
  update N to N+1 with no user action, the macOS uninstall table row.
- Icons are placeholders (`packaging/assets`, made with ImageMagick): replace them with the
  real ones from the design work.
