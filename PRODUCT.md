# Product
<!-- impeccable:product-schema 1 -->

Written 2026-11-13 by a mockup agent with no way to ask Max questions. Every fact comes from Max's
request and `docs/plan/2026-11-13-spec-and-plan.md`. Lines marked (inferred) are assumptions for Max to confirm.

## Platform
adaptive

## Stack
Desktop: Tauri 2 window (Svelte 5, TypeScript) over a headless Rust agent. iPhone and iPad: SwiftUI.
Android: Jetpack Compose. Mockups: static HTML, no build step.

## Users
Normal, non-technical people who own more than one device (a Windows PC, a Mac, a Linux laptop, an
iPhone, an iPad, an Android phone) and want to copy on one and paste on another. They set it up once
and never think about it again.

## Product Purpose
Universal Clipboard shares one clipboard across a person's own devices. Setup is a few minutes, one time:
install, accept a few permissions, add a device with a code. After that the app is invisible.
Success is a user who forgets the app exists because copy and paste simply work across devices.

## Positioning
Works across every mix of Windows, macOS and Linux, plus phones and tablets, with no account, no cloud
storage and end-to-end encryption. It keeps working when a device is on a VPN on the same network.

## Operating Context
- No system tray icon and no Dock icon. A background agent starts at login. Opening the app shows one
  settings window (about 420 x 560 px).
- Pairing with a 9-digit short code (same network), a QR code, or a 20-character long code (anywhere).
- Phones need one tap per sync, because iOS and Android do not allow background clipboard access.
- A group needs at least one computer (or Android phone) that is on.
- The OS notifies only when the user must act, at most once a day.

## Capabilities and Constraints
- Syncs plain text and one image. Formatted text may come in v1.1. No files. No history.
- Groups of 2 to 8 devices, one person's devices only.
- The UI never shows clipboard content, only facts ("Text, from Max's Mac, 2 minutes ago").
- Skips copies that password managers mark as private.

## Brand Commitments
- Name: Universal Clipboard.
- Voice: few words, common words, short sentences, no technical terms (Max's ASD-STE100 style rule).
- No em-dashes in any copy.
- Looks native-ish on each OS without copying any OS exactly (from the mockup brief).
- No logo exists yet (inferred: one will be made later with the local image stack).

## Evidence on Hand
None. No users, reviews, or metrics exist yet. Do not invent any.

## Product Principles
1. Set up once, then disappear.
2. One sentence and one button for every problem.
3. Say platform limits plainly; never hide them.
4. Show facts about clips, never their content.
5. Every permission is asked only after the user presses a button that explains it.

## Accessibility & Inclusion
Max is nearsighted and reads with a magnifier, but wants UIs that simply look good: no forced big
text or forced high contrast. Meet WCAG AA contrast. Respect reduced motion.
