#!/usr/bin/env bash
# Builds CaperMobile.xcframework, the static library CaperCore links on every
# Apple build: iOS devices, the iOS simulator (arm64 and x86_64) and macOS
# (arm64 and x86_64). apps/native/apple/prepare.sh runs it.
# usage: build-apple.sh OUTPUT.xcframework
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
OUTPUT="${1:?usage: build-apple.sh OUTPUT.xcframework}"
[[ "$OUTPUT" = /* ]] || OUTPUT="$PWD/$OUTPUT"
TARGET_DIR="$ROOT/target"
# rustup picks the pinned toolchain (rust-toolchain.toml) from the working directory.
cd "$ROOT"
# The Rust objects must not require newer systems than the apps (project.yml).
export IPHONEOS_DEPLOYMENT_TARGET=17.0
export MACOSX_DEPLOYMENT_TARGET=14.0

targets=(aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios aarch64-apple-darwin x86_64-apple-darwin)
rustup target add "${targets[@]}" >/dev/null
for target in "${targets[@]}"; do
  cargo build --manifest-path "$ROOT/Cargo.toml" --locked --release --lib --target "$target"
done

library() { echo "$TARGET_DIR/$1/release/libcaper_mobile.a"; }
STAGE="$TARGET_DIR/apple"
rm -rf "$STAGE"
mkdir -p "$STAGE/ios-simulator" "$STAGE/macos"
lipo -create "$(library aarch64-apple-ios-sim)" "$(library x86_64-apple-ios)" -output "$STAGE/ios-simulator/libcaper_mobile.a"
lipo -create "$(library aarch64-apple-darwin)" "$(library x86_64-apple-darwin)" -output "$STAGE/macos/libcaper_mobile.a"

rm -rf "$OUTPUT"
xcodebuild -create-xcframework \
  -library "$(library aarch64-apple-ios)" -headers "$ROOT/include" \
  -library "$STAGE/ios-simulator/libcaper_mobile.a" -headers "$ROOT/include" \
  -library "$STAGE/macos/libcaper_mobile.a" -headers "$ROOT/include" \
  -output "$OUTPUT" >/dev/null
test -f "$OUTPUT/Info.plist"
echo "Built $OUTPUT"
