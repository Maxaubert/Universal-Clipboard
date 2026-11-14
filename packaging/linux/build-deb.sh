#!/bin/sh
# Build the .deb (plan 2.1, 3.2, 8.1, 8.5). rpm and AppImage are cut from v1.
#
#   sh packaging/linux/build-deb.sh VERSION BIN_DIR OUT_DIR [ARCH]
#
# Build on Ubuntu 22.04 (glibc 2.35, the plan minimum) so the package runs on 22.04 and newer
# and on Debian 12. BIN_DIR holds uc-agent and uc (built WITHOUT the self-update feature: apt updates this
# build), and universal-clipboard (the window, M8b) when it is built.
#
# Layout:
#   /usr/lib/universal-clipboard/{uc-agent,uc,universal-clipboard}
#   /usr/bin/uc-agent                       link
#   /usr/bin/uc or /usr/bin/uclip           link made by postinst (plan 3.2 name rule)
#   /usr/share/applications/...desktop      only with the window
#   /usr/share/icons/hicolor/256x256/apps/universal-clipboard.png
# Start at login is per user (XDG autostart in ~/.config/autostart), set by "uc setup" or the
# window, never by the package: a package must not write into home folders.
set -eu

VERSION=${1:?VERSION}
BIN=${2:?BIN_DIR}
OUT=${3:?OUT_DIR}
ARCH=${4:-$(dpkg --print-architecture)}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
PKG=universal-clipboard

for f in uc-agent uc; do
  [ -x "$BIN/$f" ] || { echo "$BIN/$f is missing" >&2; exit 2; }
done

mkdir -p "$OUT"
D=$(mktemp -d)/$PKG
LIB=$D/usr/lib/$PKG
mkdir -p "$LIB" "$D/usr/bin" "$D/DEBIAN" "$D/usr/share/doc/$PKG" \
  "$D/usr/share/icons/hicolor/256x256/apps"
install -m 0755 "$BIN/uc-agent" "$BIN/uc" "$LIB/"
strip "$LIB/uc-agent" "$LIB/uc" 2>/dev/null || true
ln -s ../lib/$PKG/uc-agent "$D/usr/bin/uc-agent"
if [ -x "$BIN/universal-clipboard" ]; then
  install -m 0755 "$BIN/universal-clipboard" "$LIB/"
  ln -s ../lib/$PKG/universal-clipboard "$D/usr/bin/universal-clipboard"
  mkdir -p "$D/usr/share/applications"
  cat > "$D/usr/share/applications/io.github.maxaubert.uclip.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Universal Clipboard
Comment=Copy on one device, paste on another
Exec=/usr/bin/universal-clipboard
Icon=universal-clipboard
Categories=Utility;
Terminal=false
EOF
fi
install -m 0644 "$ROOT/packaging/assets/icon-256.png" \
  "$D/usr/share/icons/hicolor/256x256/apps/universal-clipboard.png"
cat > "$D/usr/share/doc/$PKG/copyright" <<EOF
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: Universal Clipboard
Source: https://github.com/Maxaubert/Universal-Clipboard

Files: *
License: MIT or Apache-2.0
EOF

DEPENDS="libc6 (>= 2.35), libdbus-1-3"
# The window: WebKitGTK 4.1 and GTK 3 (Ubuntu 22.04+, Debian 12) and the tray library.
[ -x "$BIN/universal-clipboard" ] && DEPENDS="$DEPENDS, libwebkit2gtk-4.1-0, libgtk-3-0"
SIZE=$(du -sk "$D/usr" | cut -f1)
cat > "$D/DEBIAN/control" <<EOF
Package: $PKG
Version: $VERSION
Architecture: $ARCH
Maintainer: Universal Clipboard contributors <noreply@users.noreply.github.com>
Installed-Size: $SIZE
Depends: $DEPENDS
Section: utils
Priority: optional
Homepage: https://github.com/Maxaubert/Universal-Clipboard
Description: Copy on one device, paste on another
 Clipboard sync between your own computers and phones on the same local
 network. No account and no server. Use "uc setup" and "uc pair show" to start.
EOF
install -m 0755 "$HERE/debian/postinst" "$HERE/debian/prerm" "$HERE/debian/postrm" "$D/DEBIAN/"

DEB="$OUT/${PKG}_${VERSION}_${ARCH}.deb"
dpkg-deb --root-owner-group --build "$D" "$DEB"
rm -rf "$(dirname "$D")"
echo "deb: $DEB"
