# App icon: Paper Layers

Source: given by Max on 2026-11-15. Made with Codex (output `Universal-Clipboard-Paper-Layers.zip`).
A dark gradient tile (#34373f to #101116) with white stacked layers and a grey clip.

## Files

- `source/paper-layers.svg`: the original 1024 full-bleed square. Do not edit; everything else is derived from it.
- `source/paper-layers.preview.svg`: the rounded preview from the same zip.
- `source/layers/01-tile.svg`, `source/layers/02-sync-symbol.svg`: the two layers on their own.
- Derived masters (written by `render.mjs`, same art):
  - `tile.svg`: Windows and Linux. Rounded tile with a 32/1024 margin.
  - `tile-small.svg`: 16 to 32 px. Two layers instead of three, bigger, thicker stroke (the three
    layers run together below 48 px; see `docs/design/icon/contact-sheet.png`).
  - `macos.svg`, `macos-small.svg`: the macOS icon grid. 824/1024 body at (100,100),
    continuous corners (radius 185.4, 60% smoothing), soft drop shadow in the margin.
- `render.mjs`: renders every file below. `sheet.mjs`: the contact sheet.

## Where the outputs go

| Platform | Files | Used by |
|---|---|---|
| Windows | `packaging/assets/icon.ico` (16-64 BMP, 256 PNG) | NSIS installer, `icon.ico` in the install folder, Start menu shortcut, uninstall entry |
| Windows exe | `apps/desktop/src-tauri/icons/icon.ico` (same file) | embedded in `Universal Clipboard.exe` by tauri-build |
| MSIX | `packaging/assets/msix/*.png` (scale-100..400, targetsize 16..256 plated, unplated, lightunplated) | `packaging/windows/build.ps1` + makepri `resources.pri` |
| macOS | `packaging/assets/macos/AppIcon.iconset` | `packaging/macos/build-app.sh` (iconutil) |
| Linux | `packaging/assets/linux/hicolor/{16..512,scalable}` | `packaging/linux/build-deb.sh`, `Icon=universal-clipboard` |
| Tauri | `apps/desktop/src-tauri/icons/*` | window icon (Linux, macOS dev), `bundle.icon` |

## Re-render

```sh
# deps in a scratch folder (not in the repo)
cd <scratch> && npm i @resvg/resvg-js@2.6.2 sharp@0.33.5
cd <repo>
node packaging/assets/icon/render.mjs <scratch>
node packaging/assets/icon/sheet.mjs <scratch>
cd apps/desktop
npx tauri icon ../../packaging/assets/icon/build/tauri-source.png
npx tauri icon ../../packaging/assets/icon/build/tauri-source-macos.png -o ../../packaging/assets/icon/build/mac
cd src-tauri/icons
rm -rf android ios
cp ../../../../packaging/assets/icon/build/icon.ico icon.ico
cp ../../../../packaging/assets/icon/build/mac/icon.icns icon.icns
cp ../../../../packaging/assets/linux/hicolor/32x32/apps/universal-clipboard.png 32x32.png
```

`build/` is ignored by git.
