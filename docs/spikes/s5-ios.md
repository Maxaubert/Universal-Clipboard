# Spike S5: the Rust core on iOS, Simulator part (2026-11-28)

**Result: GO.** The Rust core builds for iOS, runs in an app and in a Share Extension, and
syncs with a desktop agent. Part 2 (a real iPhone) is still open. It waits until Max plugs in
the phone (plan `docs/plan/2026-11-28-ios-ipad.md`, row 7).

Code: `spikes/s5-ios` (throwaway). Run it with `build.sh`, `run.sh`, and the UI test in
`ios/UITests`. The raw output is not committed, because it names Max's devices and the repo is
public. The numbers below come from that output.

## Setup

- Mac mini M6, macOS 27.2, Xcode 27.2, iPhone 18 Pro Simulator (iOS 27.2). The Simulator was
  booted with `simctl`. Simulator.app was never opened, so nothing appeared on screen.
- Rust 1.99, targets `aarch64-apple-ios` and `aarch64-apple-ios-sim`, release profile
  (`opt-level = "s"`, thin LTO). UniFFI 0.32.2, proc macros. The XCFramework is a static
  library.
- The spike links the **whole desktop agent** (`uc-agent`, `--no-default-features`) into the
  app. It runs the agent on its own thread with a test profile, and talks to it through its
  IPC. This is not the design for the real app (see "What changes for M12"). It was the fastest
  way to prove that the transport, pairing, membership and sync work inside an iOS process.
- Peer: a `uc-agent` on the Mac with a test profile, fake clipboard, file key store and
  `UCLIP_TEST_LOOPBACK=127.0.0.1`. The Simulator agent also used loopback. Loopback needs no
  Local Network permission and no firewall rule, so no macOS prompt could appear.

## Results

| Question | Answer |
|---|---|
| Does quinn, rustls with ring, and the agent build for iOS? | **Yes**, for the device and the Simulator. The only code change: `uc-keys` now takes `keyring` (`apple-native`) on iOS too. |
| Link flags | `-lc++ -lresolv -framework Network -framework Security -framework SystemConfiguration`. `netdev` 0.47 needs Network.framework on iOS (gateways) and `libresolv` (DNS servers, `os/ios/dns.rs`). |
| Start time in the app | 65 ms from `start` to the first IPC answer. |
| Pairing (6-digit code) | 0.86 s, joiner in the Simulator, inviter on the Mac. |
| Copy Simulator to Mac | Arrived. |
| Copy Mac to Simulator | Arrived. |
| Keychain in the Simulator | Works through `keyring` (no access group). The file key store also works. |
| Named `UIPasteboard` write and read | Works. The general pasteboard was never touched. |
| App memory (`phys_footprint`, Swift debug build) | 19.1 MB before start, 20.6 MB when ready, 26.5 MB after pairing and a sync. So the core adds about 7.5 MB. |
| Share Extension: start | 5 ms to start the core. The UI test opens the Share sheet in the host app and taps the entry (`ios/UITests/ShareTests.swift`). |
| Share Extension: memory | 14.3 MB before start, 16.0 MB after start, 25.7 MB after 1 s with the network up. The target was 60 MB. The extension limit is about 120 MB. |
| Name in the Share sheet | The **containing app's** display name ("UC S5"), not the extension's `CFBundleDisplayName`. This matches plan 2.6 ("with the app's own name"). |
| Size (device, release) | App binary 10.7 MB, extension binary 10.7 MB. The Rust code is linked twice. |

## What changes for M12 (the real `uc-mobile`)

1. **No IPC socket on iOS.** A Unix socket path may be at most 104 bytes, and the app container
   path is longer (about 165 bytes to `agent.sock`). The spike used a short folder outside the
   container, which works only in the Simulator. `uc-mobile` calls the core in the same process,
   as the plan already says.
2. **Device type and name.** The agent says type 3 (Linux) and "My computer" on iOS.
   `uc-mobile` must send 4 (iPhone) or 5 (iPad) and the name from `UIDevice`.
3. **Role.** The spike ran a full member, which stays up and listens. The app and the extension
   must be `Role::PullOnly`: they connect on demand, sync once, and stop.
4. **mDNS.** `mdns-sd` sends raw multicast. On a real iPhone that needs the restricted
   multicast entitlement (`com.apple.developer.networking.multicast`). `uc-mobile` uses
   `NWBrowser` from Swift instead, as planned. Not testable in the Simulator.
5. **Keychain access group.** `keyring` cannot set an access group, so the app and the
   extension would not share the device key. `uc-keys` gets its own iOS backend
   (`SecItem` with `kSecAttrAccessGroup`, `AfterFirstUnlock`).
6. **Binary size.** The Rust code goes into one dynamic framework, which the app embeds and the
   extension links. This saves about 10 MB.
7. **Deployment target.** The C parts (`blake3`, `ring`) were built for 27.2 even with
   `IPHONEOS_DEPLOYMENT_TARGET=18.0` (linker warning). The M12 build script must pass the target
   through to `cc` (`CFLAGS_aarch64_apple_ios=-mios-version-min=18.0`).

## Not checked here (part 2, a real iPhone)

These need a real device, because the Simulator does not enforce them: the paste prompt
(Ask and Allow), the Local Network prompt and a refusal, Bonjour through `NWBrowser` on a real
LAN, a VPN on the phone with and without local network sharing, the "no computer is on" state
on a real network, and the memory limit of a real extension.
Real LAN in the Simulator and a sync with the PC were also not run. The LAN run could show a
macOS Local Network prompt on the Mac. SSH to the PC was not tried, because the link has been
unreliable.
