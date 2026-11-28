#!/bin/sh
# Spike S5: build the Rust core for the iOS Simulator (and device), the Swift bindings and
# UcS5.xcframework. Uses the shared CARGO_TARGET_DIR.
set -eu
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:$PATH"
T="${CARGO_TARGET_DIR:-$(cd ../.. && pwd)/target}"
export IPHONEOS_DEPLOYMENT_TARGET=18.0
(cd rust
 cargo build --release --target aarch64-apple-ios-sim
 cargo build --release --target aarch64-apple-ios
 cargo run -q --bin uniffi-bindgen -- generate --library "$T/aarch64-apple-ios-sim/release/libuc_s5.a" \
   --language swift --out-dir ../ios/Generated)
mkdir -p ios/Generated/headers
mv ios/Generated/uc_s5FFI.h ios/Generated/headers/
mv ios/Generated/uc_s5FFI.modulemap ios/Generated/headers/module.modulemap
rm -rf ios/UcS5.xcframework
xcodebuild -create-xcframework \
  -library "$T/aarch64-apple-ios-sim/release/libuc_s5.a" -headers ios/Generated/headers \
  -library "$T/aarch64-apple-ios/release/libuc_s5.a" -headers ios/Generated/headers \
  -output ios/UcS5.xcframework
