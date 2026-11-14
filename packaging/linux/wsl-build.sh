#!/bin/sh
# Developer helper (PC + WSL): copy this worktree into WSL, build the Linux release binaries
# (without self-update: apt updates the .deb) and the .deb, and copy it to ./dist.
#
#   wsl -d Ubuntu -- sh packaging/linux/wsl-build.sh VERSION
#
# Run from the worktree root. Uses ~/uc-src/<name> and ~/uc-targets/<name> in WSL.
set -eu
VERSION=${1:?VERSION}
NAME=$(basename "$(pwd)")
SRC=$HOME/uc-src/$NAME
HERE=$(pwd)
export CARGO_TARGET_DIR=$HOME/uc-targets/$NAME
rm -rf "$SRC"
mkdir -p "$SRC"
tar --exclude=./target --exclude=./dist -cf - . | (cd "$SRC" && tar -xf -)
cd "$SRC"
. "$HOME/.cargo/env" 2>/dev/null || true
cargo build --release --no-default-features -p uc-agent -p uc-cli
mkdir -p "$SRC/dist"
sh packaging/linux/build-deb.sh "$VERSION" "$CARGO_TARGET_DIR/release" "$SRC/dist"
# Back to the worktree, for the Docker test from Windows:
#   docker run --rm -v "${PWD}:/src" -w /src ubuntu:24.04 sh packaging/linux/test-deb.sh dist/<deb>
mkdir -p "$HERE/dist"
cp dist/*.deb "$HERE/dist/"
ls "$HERE"/dist/*.deb
