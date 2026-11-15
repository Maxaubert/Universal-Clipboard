# iPhone mockups (2026-11-15)

Animated mockups for the iPhone app, made for Max's brief of 2026-11-15: iPhone first, native
iOS structure (SF font, large titles, sheets, system controls, Dynamic Type sizes, 44 pt targets,
safe areas, Dark Mode) with the desktop's warm colors and the motion Max picked for the desktop.
iPad follows after the iPhone picks, with the same screens in its wider layout. Android comes
later, in its own round. They replace `docs/mockups/10-iphone-setup.html` and
`11-iphone-ipad-daily.html` (2026-11-13: 9-digit and long codes, no motion).

Overview: `overview.png` (one frame per option).

## How to view

Open `index.html` in a browser. Everything is local and works offline. Each page has Replay all,
Reduced motion, Pause all, Dark Mode and Actual size at the top. URL switches: `?reduce=1`,
`?dark=1`, `?t=1200` (freeze every option at 1200 ms).

| Page | Options | What it shows |
|---|---|---|
| `setup.html` | F1 to F3 | First run and pairing: Welcome, Local Network, scan the QR code, Connected |
| `howto.html` | W1 to W3 | How it works: open the app to sync, the Share sheet, the paste setting |
| `home.html` | H1 to H3 | Home, the sync moment when the app opens, this iPhone shown apart, the ticker |
| `share.html` | X1 to X3 | Sending a copy from another app through the Share sheet |
| `pause.html` | P1 to P3 | Pause and Resume (Hush) |
| `errors.html` | E1 to E3 | Wrong and old code, no computer on, Local Network off, VPN |
| `settings.html` | T1 to T3 | Settings, device detail, remove, About |

## Shared files

- `ios.css`: the phone frame (393 x 852 pt), iOS parts and tokens for light and dark.
- `ios.js`: status bar, Dynamic Island and home indicator on every screen, icons, Dark Mode.
- Copied from the desktop: `kit.css` (gallery), `kit.js` (replay, reduced motion, seek), `art.js`
  (device logos, QR) and `fonts/`, from `../desktop/motion/`. Copies, because Safari does not
  load files from a parent folder when a page is opened from disk.

## Carried over

S1 One shape, E1 Shake then clear, H3 You are here, A3 Ticker, A4 Hush (desktop picks of
2026-11-14). 6 digits plus QR, local network only. No copy history: facts, never content. UI text
cannot be selected. iPhone limits from plan section 2.6: a copy moves when the app opens or
through the Share sheet. Updates come from the App Store. Real paste prompts wait for spike S5.

## Notes for building

- Page animations use forwards fill only: kit's default `fill: 'both'` makes a later animation
  hold its start value from time 0. Initial states live in CSS.
- Screenshots are headless browser renders, not the Simulator. They show the design, not iOS.

## How to choose

Reply with the ids you like, for example "F1, W2, H1, X3, P1, E1, T2". Mixing is fine.
