# Desktop window (M8b)

The settings window lives in `apps/desktop`. It is a Tauri 2 shell with a Svelte 5 page
(TypeScript, Vite). It is only a window. It holds no keys, no clipboard content and no state
of its own. The agent does the work (`docs/agent.md`).

## What is in it

| Part | Where |
|---|---|
| 12 screens, from the locked design `docs/design/desktop/L01` to `L12` | `src/screens/` |
| Title bar, 4-step indicator, back bar, logos, QR drawing | `src/components/` |
| Pictures taken from the design files (generated once) | `src/lib/art.ts`, `scripts/gen-art.mjs` |
| All English text in one file | `src/lib/strings.ts` |
| The only way to talk to the agent | `src/lib/ipc.ts` |
| A pretend agent for tests and for a browser | `src/lib/mock.ts` |
| Screen state (route, status, devices) | `src/lib/store.svelte.ts` |
| Tauri shell: IPC proxy, events, single instance | `src-tauri/` |

The font is Figtree, bundled from `@fontsource/figtree`. Nothing loads from the internet.

## How it talks to the agent

- The page calls `ipc.call(name, params)`. In the app, that is the Tauri command `ipc_call`.
  It connects to the agent pipe or socket with the same client as `uc` (the same peer checks)
  and starts the agent if none answers. `UCLIP_NO_START=1` turns the start off.
- A background task in the shell subscribes to the agent and passes each event to the page as
  `uc-event`. If the agent restarts, it subscribes again.
- Only the calls of plan 3.2 pass (`ALLOWED` in `src-tauri/src/lib.rs`, `ALLOWED_CALLS` in
  `src/lib/ipc.ts`). A test checks the Rust list against the agent. There is no send, get or
  fetch call, and no `debug.*` call.
- `UCLIP_PROFILE` works as for the agent. Test installs use `dev`.

## Screens and the calls behind them

| Screen | Calls |
|---|---|
| L01 Welcome, L02 Permissions | `setup.status`, `setup.step` (a row, then `done`). No OS asks anything in setup (decision 2026-11-14), so Get started marks setup done and goes to L03, and the step indicator has 4 steps instead of the design's 5 on every OS. L02 shows only when macOS blocks the local network or paste access (the agent sends those rows only then); it is part of step 1. |
| L03 Add a device | none |
| L04 Show a code | `pair.start`, `pair.cancel`; events `pair.added`, `pair.tried`, `pair.closed` |
| L05 Enter a code | `pair.join` (asks "Leave your other devices and join these?" on `in-group`) |
| L06 Connected | none (name from the event) |
| L07 Home, L10 Paused | `status`, `devices.list`, `pause.set` |
| L08 Device | `devices.rename`, `devices.remove` |
| L09 Settings | `settings.get`, `settings.set` (`start-at-login`), `leave` |
| L11 Needs attention | `status` (reason and fix code). Fix for `firewall`: the shell command `firewall_add` (the helper, one UAC prompt, the same code as `uc firewall add`), then `firewall.check` |
| L12 VPN help | none |
| Update (placeholder, `src/screens/Update.svelte`) | `update.status`, `update.apply`, `update.later`; event `update`. "Version X is ready." with Update now and Later, plus the applying and failed states. Plain on purpose: the final look and motion come from Max's mockup pick |

Window rules: the window opens when the user starts the app, and once when the agent finds a
newer version at its start (the agent starts it with `--route update`; the shell's
`launch_route` command tells the page, and a second start forwards the route through the
single-instance handler as the page event `open`). While the update helper has the agent
stopped, the update screen stays instead of "agent is down". Closing it ends the window
process and leaves the agent running. A second start brings the first window to the front.
Windows and Linux: the title bar is our own (no native frame) with a drag strip and drawn
minimize, maximize and close buttons. macOS (2026-11-14): a native window
(`src-tauri/tauri.macos.conf.json`: `titleBarStyle` Overlay, hidden title, traffic lights at
18, 24: the 16 px buttons then sit 14 px from the top, centered on the 36 px header band), so rounded corners, shadow and the traffic lights are the system's. `html.mac`
(`src/lib/platform.ts`, from the webview's user agent) hides the drawn buttons, makes the drag
strip full width and moves headers right by `--tl` (72 px; the lights end 79 px from the left) so the back button clears the
lights; "Start at sign-in" reads "Open at login". Preview it in the browser build with
`?platform=mac` (the lights are drawn there). Setup has no permissions step on any OS (4 steps,
not 5): macOS lets the agent, a launchd job, use the local network and the clipboard without a
prompt (checked 2026-11-14), the Windows installer adds the firewall rule, and start at login is
a setting, on by default, turned on by the agent's first run on every OS (`docs/agent.md`).
"Get started" sends `setup.step done`. The macOS network and paste rows appear only when macOS
blocks them. The locked design screenshots are the Windows frame.

## Tests

Run from `apps/desktop`:

- `npm test`: Vitest. Logic and components against the mock agent.
- `npm run check`: `svelte-check` (types).
- `npm run e2e`: Playwright in headless Chromium against the Vite build with the mock agent.
  It covers the whole setup flow, show and enter a code, wrong code, lockout notice, home,
  device detail, rename and remove, settings, paused, needs attention, VPN help, the title bar
  and the agent-down screen. Nothing opens on screen.
- `visual.spec.ts` takes a picture of each screen after its motion has settled and compares it
  with its reference rendered in the same browser (limit 1% of pixels): the locked design, except
  Home (L07, compared with the last frame of the H3 mockup, mock scenario `h3`) and Paused (L10,
  compared with a picture of the app in `tests/e2e/baselines`, because no design shows Paused in
  the H3 layout; `UPDATE_BASELINES=1` writes it again). The setup screens show 4 step dots where
  the design has 5 (a few pixels). L02 uses the mock's `mac-blocked` (macOS blocks the network
  and paste access) and shows those two rows, not the design's Windows rows, so its limit is 2%. The side-by-side pictures are in
  `docs/design/desktop/impl-check/`.
- `motion.spec.ts`: setup, pairing success, wrong code and dead code, pause and resume, and a
  copy moving, each with full motion and with reduced motion (`emulateMedia`).
- `cargo test` in `src-tauri`: the call list matches the agent.
- `caret.spec.ts`: the real text caret of the code field is in the box that takes the next
  digit (0, 3, 6 digits, backspace, paste). It paints the invisible input's caret red and
  finds it in a screenshot.

## Motion (2026-11-14)

Max picked these from the mockups in `docs/design/desktop/motion` (timings, easings and art
are taken from them):

| Pick | Where | Code |
|---|---|---|
| S1 "One shape" | Setup: the black button that was pressed grows into the next black part (Show a code button, code box); on pairing it shrinks to the node between the devices, turns green and the check draws (Connected) | `src/lib/flow.ts`, `src/lib/art-motion.ts` |
| E1 "Shake, then clear" | Wrong code: one damped shake, red outlines, boxes empty from the right, caret back in box 1. Dead code: boxes fade grey in a wave, typing starts a new code. Show a code: a wrong try shakes the code; a lockout fades the digits, then the notice rises. Error texts rise in everywhere | `EnterCode.svelte`, `ShowCode.svelte` |
| H3 "You are here" | Home: this device is a dark bar at the bottom with the last copy (facts only); build-in from the top; a device that comes on pops its dot | `Home.svelte` |
| A3 "Ticker" | A copy that moves: the line under the headline rolls to it for 2.8 s (arrow up sent, green arrow down received), the sender's tile edge turns green, the bar's line runs across and the logo rings | `Home.svelte` |
| A4 "Hush" | Pause and Resume: headline and line roll, a colour wave runs from the button across the tiles | `Home.svelte` |

How it works: each screen is a `.layer` in a `{#key}` in `App.svelte`, so the old screen leaves
while the new one arrives. `store.go(route, { from })` passes the pressed button. Back,
settings and the other screens cross-fade (S1's reduced variant). Reduced motion (the OS
setting): every change is a fade, no shake, no shape, no wave lag. The leaving layer is inert
and hidden from screen readers; focus moves to the new screen's heading (Enter a code keeps
its own focus on the code field). Headline and line are a `role="status"` region; errors are
`role="alert"`. Web Animations and CSS transitions only, no library. Mostly transform and
opacity; the A4 wave and E1 outlines also move colours, as in the mockups.
The last copy comes from the agent's `status.activity` and the `activity` event (kind,
direction, device, time), never the content.

## The code field and the caret (2026-11-14)

L05 draws 6 boxes and covers them with one invisible `<input>`. Screen magnifiers (Max uses
Wind) follow the caret the webview reports to the OS, so that caret must be where the next
digit goes, not at the left of the row. The input uses a monospace font at 30 px with
`letter-spacing: calc(60px - 1ch)` (one box pitch per digit) and a left padding computed
from the digit count, so the caret after n digits is in the middle of box n (after 6, in the
right part of the last box). The input stays at opacity 0, so the screen looks the same.
ShowCode has no input; the rename field in L08 is a normal visible input.

The Tauri shell is built but never started in tests (it would open a window):
`npx tauri build --no-bundle`. `tauri-driver` flows come with M9 (they need a hidden desktop
or a VM).

## Not done here

- Installers, signing, the "is it our signed agent" pipe check: M9.
- A real-agent run of the shell (needs a window). The same IPC client is tested through `uc`.
- Norwegian text: all strings are in `strings.ts`, ready.
- Settings beyond the two in the locked design (image size, formatting, name): use `uc settings`.
