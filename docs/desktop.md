# Desktop window (M8b)

The settings window lives in `apps/desktop`. It is a Tauri 2 shell with a Svelte 5 page
(TypeScript, Vite). It is only a window. It holds no keys, no clipboard content and no state
of its own. The agent does the work (`docs/agent.md`).

## What is in it

| Part | Where |
|---|---|
| 12 screens, from the locked design `docs/design/desktop/L01` to `L12` | `src/screens/` |
| Title bar, 5-step indicator, back bar, logos, QR drawing | `src/components/` |
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
| L01 Welcome, L02 Permissions | `setup.status`, `setup.step` (a row, then `done`) |
| L03 Add a device | none |
| L04 Show a code | `pair.start`, `pair.cancel`; events `pair.added`, `pair.tried`, `pair.closed` |
| L05 Enter a code | `pair.join` (asks "Leave your other devices and join these?" on `in-group`) |
| L06 Connected | none (name from the event) |
| L07 Home, L10 Paused | `status`, `devices.list`, `pause.set` |
| L08 Device | `devices.rename`, `devices.remove` |
| L09 Settings | `settings.get`, `settings.set` (`start-at-login`), `leave` |
| L11 Needs attention | `status` (reason and fix code), `setup.step` for the firewall |
| L12 VPN help | none |

Window rules: the window opens only when the user starts the app. Closing it ends the window
process and leaves the agent running. A second start brings the first window to the front.
The title bar is our own (no native frame) with a drag strip.

## Tests

Run from `apps/desktop`:

- `npm test`: Vitest. Logic and components against the mock agent.
- `npm run check`: `svelte-check` (types).
- `npm run e2e`: Playwright in headless Chromium against the Vite build with the mock agent.
  It covers the whole setup flow, show and enter a code, wrong code, lockout notice, home,
  device detail, rename and remove, settings, paused, needs attention, VPN help, the title bar
  and the agent-down screen. Nothing opens on screen.
- `visual.spec.ts` takes a picture of each screen and compares it with the locked design
  rendered in the same browser (limit 1% of pixels). The side-by-side pictures are in
  `docs/design/desktop/impl-check/`.
- `cargo test` in `src-tauri`: the call list matches the agent.

The Tauri shell is built but never started in tests (it would open a window):
`npx tauri build --no-bundle`. `tauri-driver` flows come with M9 (they need a hidden desktop
or a VM).

## Not done here

- Installers, signing, the "is it our signed agent" pipe check, the firewall helper: M9.
- A real-agent run of the shell (needs a window). The same IPC client is tested through `uc`.
- Norwegian text: all strings are in `strings.ts`, ready.
- Settings beyond the two in the locked design (image size, formatting, name): use `uc settings`.
