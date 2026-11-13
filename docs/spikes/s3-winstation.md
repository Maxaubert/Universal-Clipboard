# Spike S3: private window station for clipboard tests

Date: 2026-11-13. Result: **GO**.

## Question
Can a test get its own isolated Windows clipboard, with nothing on screen and no change to the real clipboard?

## What we ran
Code: `spikes/s3-winstation/S3.cs` (C#, built with the in-box `csc.exe`, no SDK needed).
1. The host reads `GetClipboardSequenceNumber()` (number only, no content): 1893.
2. It calls `CreateWindowStation(NULL, 0, ...)` and `CreateDesktop("S3Desktop")` inside that station.
3. It starts a child with `STARTUPINFO.lpDesktop = "<station>\S3Desktop"` (CREATE_NO_WINDOW).
4. The child makes a message-only window, then writes `CF_UNICODETEXT` and `CF_DIB` (2x2 image) and reads both back.
5. The host reads the sequence number again.

## Result
- Child ran in station `Service-0x0-26a5251$`, desktop `S3Desktop`. Its clipboard started at sequence 0 (a fresh clipboard).
- Text and image round trips: both OK. The child sequence went 0 to 8.
- Real clipboard sequence: 1893 before, 1893 after. Unchanged.
- Nothing appeared on screen (the desktop is not the input desktop, so it cannot be shown).

## Gotchas
- A named station fails with error 5 (access denied) for a normal user. Pass **NULL as the name**. Windows then makes a unique name. Read it back with `GetUserObjectInformation(UOI_NAME)`.
- Create the desktop while the new station is the process station (`SetProcessWindowStation`), then switch back.
- The clipboard needs a window owner. A message-only window (`HWND_MESSAGE`) is enough.
- Delayed rendering needs the child to run a message loop.
- Only processes started with that `lpDesktop` see this clipboard. Tests must start the app under test (the agent) this way.

## How tests should use it
Write a small test helper (Rust, `windows` crate, same calls) that creates the station and desktop with a NULL name, spawns the agent or test binary with `lpDesktop` set, waits, and closes the handles. Each test run gets its own station, so tests are isolated from each other and from the user session. In local runs, also assert that the host sequence number is unchanged. Use this for Windows clipboard integration tests. Keep the fake clipboard for unit tests.

## Not tested
Clipboard history and cloud clipboard (Win+V), and UI Automation inside the private desktop. Those features tie to the user session and are expected to be off there.
