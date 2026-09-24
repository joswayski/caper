#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$ROOT/../../.." && pwd)"

python3 "$REPO/scripts/native_fonts.py"
FONT_CACHE="$REPO/shared/fonts/cache"
FONT_RESOURCES="$ROOT/Resources/Fonts"
rm -rf "$FONT_RESOURCES"
mkdir -p "$FONT_RESOURCES"
for file in Satoshi-Regular.otf Satoshi-Medium.otf Satoshi-Bold.otf Satoshi-Black.otf Satoshi-FFL.txt; do
  test -s "$FONT_CACHE/$file"
  cp "$FONT_CACHE/$file" "$FONT_RESOURCES/$file"
done

# Native CPU inference runs from the app bundle, never a runtime network fetch.
# Keep the exact ORT headers and universal macOS binary from the same release.
ORT_NAME=onnxruntime-osx-universal2-1.23.2
ORT_DIR="$ROOT/.build/$ORT_NAME"
ORT_ARCHIVE="$ROOT/.build/$ORT_NAME.tgz"
ORT_SHA=49ae8e3a66ccb18d98ad3fe7f5906b6d7887df8a5edd40f49eb2b14e20885809
if [[ ! -f "$ORT_DIR/lib/libonnxruntime.1.23.2.dylib" ]]; then
  mkdir -p "$ROOT/.build"
  curl -fLsS --retry 2 "https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/$ORT_NAME.tgz" -o "$ORT_ARCHIVE"
  echo "$ORT_SHA  $ORT_ARCHIVE" | shasum -a 256 -c -
  tar -xzf "$ORT_ARCHIVE" -C "$ROOT/.build"
fi
test -f "$ORT_DIR/include/onnxruntime_cxx_api.h"
test -f "$ORT_DIR/LICENSE"
test -f "$ORT_DIR/ThirdPartyNotices.txt"
test "$(shasum -a 256 "$REPO/apps/web/public/audio/dpdfnet8-v2/dpdfnet8_48khz_hr.onnx" | cut -d ' ' -f 1)" = \
  7b3afbb260a08fe9af3d16e3bda992971be1e7e951d1dee7c2d235f5c43f5631

# Official onnxruntime-c CocoaPods archive. 1.23.0 is the latest 1.23 iOS
# artifact (macOS retains 1.23.2); its static XCFramework has arm64 device and
# arm64/x86_64 simulator slices. Headers are from this same pinned archive.
IOS_ORT="$ROOT/.build/onnxruntime-c-ios-1.23.0"
IOS_ORT_ARCHIVE="$ROOT/.build/onnxruntime-c-ios-1.23.0.zip"
IOS_ORT_SHA=756a78e0168f29840bc614b43aeb03e63673f44022e0221d21698a2c8ed747ef
if [[ ! -f "$IOS_ORT/onnxruntime.xcframework/Info.plist" ]]; then
  mkdir -p "$IOS_ORT"
  curl -fLsS --retry 2 https://download.onnxruntime.ai/pod-archive-onnxruntime-c-1.23.0.zip -o "$IOS_ORT_ARCHIVE"
  echo "$IOS_ORT_SHA  $IOS_ORT_ARCHIVE" | shasum -a 256 -c -
  unzip -q "$IOS_ORT_ARCHIVE" -d "$IOS_ORT"
fi
test -f "$IOS_ORT/Headers/onnxruntime_cxx_api.h"
test -f "$IOS_ORT/LICENSE"
test -f "$IOS_ORT/onnxruntime.xcframework/ios-arm64/onnxruntime.framework/onnxruntime"
test -f "$IOS_ORT/onnxruntime.xcframework/ios-arm64_x86_64-simulator/onnxruntime.framework/onnxruntime"
IOS_ORT_NOTICES="$IOS_ORT/ThirdPartyNotices.txt"
if [[ ! -f "$IOS_ORT_NOTICES" ]]; then
  curl -fLsS --retry 2 https://raw.githubusercontent.com/microsoft/onnxruntime/v1.23.0/ThirdPartyNotices.txt -o "$IOS_ORT_NOTICES"
fi
echo "e9e90971a8e75a9a8ac0c6412e29c1202d079998389915aa485f46c816c3b4cc  $IOS_ORT_NOTICES" | shasum -a 256 -c -

# The native RNNoise fallback is built from exact upstream C sources and its
# matching model. Neither engine downloads code or weights when the app runs.
RN_DIR="$ROOT/.build/rnnoise"
if [[ ! -f "$RN_DIR/src/rnnoise_data.c" ]]; then
  mkdir -p "$RN_DIR"
  curl -fLsS --retry 2 'https://github.com/xiph/rnnoise/archive/70f1d256acd4b34a572f999a05c87bf00b67730d.tar.gz' -o "$ROOT/.build/rnnoise-source.tar.gz"
  echo "f61ee0b3f4c4cd337303e003d333357c5eaf25ef5d75a742109ee59e9a0a3932  $ROOT/.build/rnnoise-source.tar.gz" | shasum -a 256 -c -
  tar -xzf "$ROOT/.build/rnnoise-source.tar.gz" -C "$RN_DIR" --strip-components=1
  curl -fLsS --retry 2 'https://media.xiph.org/rnnoise/models/rnnoise_data-0a8755f8e2d834eff6a54714ecc7d75f9932e845df35f8b59bc52a7cfe6e8b37.tar.gz' -o "$ROOT/.build/rnnoise-model.tar.gz"
  echo "0a8755f8e2d834eff6a54714ecc7d75f9932e845df35f8b59bc52a7cfe6e8b37  $ROOT/.build/rnnoise-model.tar.gz" | shasum -a 256 -c -
  tar -xzf "$ROOT/.build/rnnoise-model.tar.gz" -C "$RN_DIR" src/rnnoise_data.c src/rnnoise_data.h
fi
test -f "$RN_DIR/COPYING"

XCODEGEN_COMMIT=21ac9944b0ab546a07422dbed86f33dd2ebd76f8
XCODEGEN="$ROOT/.build/xcodegen-$XCODEGEN_COMMIT"
if [[ ! -d "$XCODEGEN/.git" ]]; then
  rm -rf "$XCODEGEN"
  git clone --quiet https://github.com/yonaskolb/XcodeGen.git "$XCODEGEN"
fi
git -C "$XCODEGEN" checkout --quiet --detach "$XCODEGEN_COMMIT"
test "$(git -C "$XCODEGEN" rev-parse HEAD)" = "$XCODEGEN_COMMIT"
swift run --package-path "$XCODEGEN" xcodegen --spec "$ROOT/project.yml" --project "$ROOT"
