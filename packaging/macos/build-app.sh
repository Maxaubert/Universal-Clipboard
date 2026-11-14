#!/bin/sh
# Build "Universal Clipboard.app", the DMG and the update archive on macOS (plan 2.1, 3.2, 8.1).
# Ad-hoc signed by default (codesign -s -). Nothing is installed: no copy to /Applications,
# no login item, the DMG is not mounted.
#
#   sh packaging/macos/build-app.sh VERSION BIN_DIR OUT_DIR
#
# Optional (docs/install.md, "Signing and notarization"):
#   SIGN_IDENTITY="Developer ID Application: <name> (<team>)"  sign with a Developer ID
#     (hardened runtime, secure timestamp) instead of ad hoc.
#   NOTARY_PROFILE=<profile>  also notarize and staple the app and the DMG
#     (profile made once with xcrun notarytool store-credentials; needs SIGN_IDENTITY).
#
# BIN_DIR holds uc-agent and uc (cargo build --release), and universal-clipboard (the window,
# M8b) when it is built. Without the window, the bundle runs the agent as its main program
# (LSUIElement, no Dock icon) so it is still a working app.
set -eu

VERSION=${1:?VERSION}
BIN=${2:?BIN_DIR}
OUT=${3:?OUT_DIR}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
ARCH=$(uname -m)
[ "$ARCH" = "arm64" ] && ARCH=aarch64

case "$VERSION" in
  *[!0-9.]* | "") echo "VERSION must be a.b.c" >&2; exit 2 ;;
esac
for f in uc-agent uc; do
  [ -x "$BIN/$f" ] || { echo "$BIN/$f is missing" >&2; exit 2; }
done

mkdir -p "$OUT"
APP="$OUT/Universal Clipboard.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Library/LaunchAgents"
cp "$BIN/uc-agent" "$BIN/uc" "$APP/Contents/MacOS/"
if [ -x "$BIN/universal-clipboard" ]; then
  cp "$BIN/universal-clipboard" "$APP/Contents/MacOS/"
  MAIN=universal-clipboard
  UI_ELEMENT=false
else
  MAIN=uc-agent
  UI_ELEMENT=true
fi

# The icon.
ICONSET=$(mktemp -d)/AppIcon.iconset
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s "$ROOT/packaging/assets/icon-512.png" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  [ $d -le 512 ] && sips -z $d $d "$ROOT/packaging/assets/icon-512.png" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$(dirname "$ICONSET")"

sed -e "s/@VERSION@/$VERSION/g" -e "s/@MAIN@/$MAIN/g" -e "s/@UI_ELEMENT@/$UI_ELEMENT/g" \
  "$HERE/Info.plist.in" > "$APP/Contents/Info.plist"
cp "$HERE/io.github.maxaubert.uclip.agent.plist" "$APP/Contents/Library/LaunchAgents/"
plutil -lint "$APP/Contents/Info.plist" "$APP/Contents/Library/LaunchAgents/io.github.maxaubert.uclip.agent.plist"

# Signature: inner programs first, then the bundle. Hardened runtime on in both modes.
IDENTITY=${SIGN_IDENTITY:--}
TS=--timestamp
[ "$IDENTITY" = "-" ] && TS=--timestamp=none
if [ -n "${NOTARY_PROFILE:-}" ] && [ "$IDENTITY" = "-" ]; then
  echo "NOTARY_PROFILE needs SIGN_IDENTITY (a Developer ID)" >&2; exit 2
fi
for f in "$APP/Contents/MacOS/"*; do
  case "$(basename "$f")" in
    "$MAIN") id=io.github.maxaubert.uclip ;;
    uc-agent) id=io.github.maxaubert.uclip.agent ;;
    *) id=io.github.maxaubert.uclip.cli ;;
  esac
  codesign --force --options runtime $TS --identifier "$id" --sign "$IDENTITY" "$f"
done
codesign --force --options runtime $TS --sign "$IDENTITY" "$APP"
codesign --verify --deep --strict "$APP"

# Notarize the app before it goes into the update archive and the DMG, so both carry the
# stapled ticket.
if [ -n "${NOTARY_PROFILE:-}" ]; then
  ZIP=$(mktemp -d)/app.zip
  ditto -c -k --keepParent "$APP" "$ZIP"
  xcrun notarytool submit "$ZIP" --keychain-profile "$NOTARY_PROFILE" --wait
  rm -rf "$(dirname "$ZIP")"
  xcrun stapler staple "$APP"
fi

# The update archive: the CONTENTS of the bundle (uc-update swaps the contents in place).
TAR="$OUT/UniversalClipboard-$VERSION-macos-$ARCH.app.tar.gz"
tar -czf "$TAR" -C "$APP" .

# The DMG: the app and a link to /Applications. hdiutil mounts a temporary image while it
# works, so on a computer someone is using, set SKIP_DMG=1 (CI makes the DMG).
if [ "${SKIP_DMG:-0}" = "1" ]; then
  echo "app: $APP"
  echo "update archive: $TAR"
  echo "dmg: skipped (SKIP_DMG=1)"
  exit 0
fi
DMG="$OUT/UniversalClipboard-$VERSION-macos-$ARCH.dmg"
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -quiet -volname "Universal Clipboard" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
rm -rf "$STAGE"
if [ "$IDENTITY" != "-" ]; then
  codesign --force $TS --sign "$IDENTITY" "$DMG"
fi
if [ -n "${NOTARY_PROFILE:-}" ]; then
  xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
fi
echo "app: $APP"
echo "dmg: $DMG"
echo "update archive: $TAR"
