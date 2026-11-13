# Spike S7: Android background clipboard (2026-11-13)

**Result: GO, with one change to the plan text.** A foreground service of type `connectedDevice`
can write the clipboard in the background on API 29 and API 37. The write is not blocked.
But on Android 13+ (API 33+) the system shows its clipboard preview popup on every write.
So "automatic" is true, "silent" is not. The "Tap to copy" notification is not needed as the
main path, because it does not avoid the popup either (see 3).

Code: `spikes/s7-android` (throwaway). Raw evidence: `docs/spikes/s7/api29/` and `docs/spikes/s7/api37/`
(`steps.txt`, `logcat.txt`, `logcat-tap.txt`, screenshots).

## Setup

- Emulators, headless (`-no-window -no-audio`): `uc-api29` (Android 10, API 29) and `uc-api37`
  (Android 17, API 37), Google APIs x86_64 images.
- App: Kotlin, plain `android.app` classes, AGP 9.5.1, Gradle 9.9.1 (wrapper in the folder),
  compileSdk 37, minSdk 29, targetSdk 37. Build: in `rust/`, run
  `cargo ndk -t x86_64 -t arm64-v8a -o ../app/src/main/jniLibs build --release` (the `.so` files
  are not committed), then `gradlew assembleDebug`.
- A broadcast to the service stands in for "a clip arrived from the network".
  `run-spike.ps1 -Serial <emulator> -Out <folder>` runs all checks.
- Max's clipboard: the emulator has no window, so it has no Qt clipboard bridge to Windows.
  The script reads the Windows clipboard sequence number (read only) before and after. It did
  not change (1893 before and after, on both runs).

## 1. Background write from the foreground service

The app opens once (starts the service), then goes to the home screen. The service then writes.

| Case | API 29 | API 37 |
|---|---|---|
| Service is foreground, type `connectedDevice` | yes (`isForeground=true`) | yes (`types=0x10`) |
| Write, app in background | ok | ok |
| Write while the screen is off | ok | ok |
| Write in forced Doze (`deviceidle force-idle`) | ok | ok |
| Clip really landed (read back by the tile activity) | yes, `bg-doze-29` | yes, `bg-doze-37` |
| Preview popup on screen | no (popup is API 33+) | **yes, every write** |

Popup details on API 37 (screenshots `api37/02` to `api37/06`):

- It shows on every write: plain clip, with `EXTRA_IS_REMOTE_DEVICE`, and with `IS_SENSITIVE`.
- `EXTRA_IS_REMOTE_DEVICE` does **not** hide it.
- `IS_SENSITIVE` hides the text (shows dots). We must set it for password-manager clips.
- It was still there 5 s after the write (`03b`).
- A write while the screen is off shows the popup when the screen turns on (`06`).
  The emulator has no lock screen PIN, so a locked real phone is not tested.
- Note: AOSP SystemUI hides the popup on emulators unless the clip has the extra
  `com.android.systemui.SHOW_CLIPBOARD_OVERLAY`. On this API 37 image it showed even without the
  extra, so the result matches a phone. Samsung and other OEM skins may look different.

## 2. Reading the clipboard

| Where | API 29 | API 37 |
|---|---|---|
| Service, app in background | **blocked**: `null`, log "Denying clipboard access to dev.uc.s7, application is not in focus" | **blocked**, same log |
| Tile opens a transparent activity: read in `onCreate` | blocked | blocked |
| Same activity: read in `onResume` | blocked | blocked |
| Same activity: read in `onWindowFocusChanged(true)` | **works** | **works** |
| Time from `onCreate` to a good read | 193 ms | 289 ms |

- The tile uses `startActivityAndCollapse(PendingIntent)` on API 34+ and the `Intent` form below.
  Both worked (`cmd statusbar add-tile` and `click-tile` drove it headless).
- Rule for M13: read only after window focus, then `finish()`. Never read in `onCreate` or `onResume`.
- Android 12+ may show the "pasted from your clipboard" toast on this read. Expected and fine.

## 3. Fallback "Tap to copy" notification

Built and tested anyway: a high-priority notification, a tap opens the transparent activity,
which writes the clip. Works on both APIs (`logcat-tap.txt`, screenshots `11` and `12`).
On API 37 the same popup appears after the tap (`api37/12`). So the fallback only helps if a
future Android blocks background writes. It does not make the write quieter.

**Decision:** keep the automatic write as the main path. Keep "Tap to copy" in the code as a
setting ("Ask before copying") and as the automatic fallback if `setPrimaryClip` throws or the
service cannot run. M13 does not need it for launch.

## 4. Rust library in the app

A minimal crate (`spikes/s7-android/rust`) built with `cargo ndk -t x86_64 -t arm64-v8a`
(NDK 30.0.16248370, cargo-ndk 4.1.2), copied to `jniLibs`.

- Plain JNI (`Java_dev_uc_s7_Native_add`, no helper crates): `add(40,2)=42`. Works.
- UniFFI 0.32.2, Kotlin bindings from `uniffi-bindgen generate --library libuc_s7.so`, JNA
  5.19.1 `@aar`: `greet` and `core_info` return `"hello android from Rust"` and
  `"uc-s7 0.1.0 arch=x86_64 os=android"`. Works from the activity and from the service.
- Size: about 530 KB per ABI for this tiny lib (not stripped). AGP could not strip it
  ("Unable to strip", cause not checked). Set `strip = true` in the Rust release profile in M12.

## 5. Play policy for `connectedDevice`

Sources (read 2026-11-13):
- Play Console Help, "Understanding foreground service and full-screen intent requirements",
  https://support.google.com/googleplay/android-developer/answer/13392821
- Android developers, "Foreground service types",
  https://developer.android.com/develop/background-work/services/fg-service-types

Facts:
- The type covers "Interactions with external devices that require a Bluetooth, NFC, IR, USB, or
  network connection." Play's use case is "Continuous Data Transfer to an External Device"
  (examples: wearables, baby monitors, headsets, cars).
- Runtime prerequisite on API 34+: declare one of `CHANGE_NETWORK_STATE`, `CHANGE_WIFI_STATE`,
  `CHANGE_WIFI_MULTICAST_STATE`, `NFC`, `TRANSMIT_IR`, or hold a Bluetooth/UWB/USB permission.
  We declare `CHANGE_WIFI_MULTICAST_STATE`, which mDNS needs anyway. It worked on API 37.
- Play Console declaration (App content page), per type: what the feature does, user impact if
  the system defers or stops it, a **video link**, and a use case. The video must "Show the user
  pairing and connecting to an external device directly inside the app" and show "the in-app
  setup and subsequent device interaction, not system settings".
- The user must be aware of the service: a "clear and accurate notification" counts.
- The types page marks dataSync, mediaPlayback, phoneCall and mediaProjection as not allowed to
  start from `BOOT_COMPLETED` (Android 15+), and camera and microphone through the while-in-use
  rule. It has no such note for `connectedDevice`, so boot start should stay allowed. Not tested here.
- The docs suggest the companion device manager as an alternative. It targets Bluetooth/Wi-Fi
  companion hardware, not a PC on the LAN, so it does not fit.

Risk: a reviewer may say a PC is not an "external device". Our case is close to the wording
(network connection to the user's own computer). Mitigation: the review video shows QR pairing
with the PC in the app, then a copy on the PC landing on the phone. Backup type if refused:
`specialUse` with a written explanation. Real review is only proven at the first Play upload.

## Changes to the plan

- Section 2.6, "Computer to phone": replace "silent" with "automatic; on Android 13+ the system
  shows its small copy preview for a few seconds each time (it cannot be turned off; sensitive
  clips show dots)". The "Tap to copy" notification becomes a setting and a fallback, not the plan B.
- M13: set `IS_SENSITIVE` on sensitive clips; read only on window focus in the tile activity;
  declare `CHANGE_WIFI_MULTICAST_STATE`; plan the Play review video.
- Not covered by this spike (still open from the S7 row): `ACCESS_LOCAL_NETWORK` granted and
  refused, OEM battery killing, and behaviour on a real phone with a lock screen.
