# Implementation check (generated)

Made by `apps/desktop/tests/e2e/visual.spec.ts` (`npm run e2e`). Each picture shows three
parts, from left to right: the reference, the app after its motion has settled, and the
difference in red. The reference is the locked design, except Home (L07), which is the end
frame of the H3 "You are here" motion mockup (picked 2026-11-14), and Paused (L10), the update
screens (U1, U3) and the file sync parts (F1 to F3), which are pictures of the app
(`tests/e2e/baselines`) because no design shows them in the H3 layout. Home (L07) and Settings
(L09) hide the file sync parts when they are compared with the design.
The QR code is hidden in both (the design has a fixed picture, the app draws the real code).
Limit: 1% of pixels.

| Screen | File | Reference | Different pixels | Picture |
|---|---|---|---|---|
| L09 | L09-settings | locked design | 1.62% | ![L09](L09-settings.png) |
| F3 | F3-settings-files | app baseline (Copy smaller files automatically rows) | 0.00% | ![F3](F3-settings-files.png) |
