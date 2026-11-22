#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$ROOT/../../.." && pwd)"

# Release and TestFlight builds share this generated configuration. Debug builds
# deliberately keep an empty revision and never fetch production release notes.
revision="${CAPER_RELEASE_SHA:-$(git -C "$REPO" rev-parse HEAD)}"
[[ "$revision" =~ ^[0-9a-f]{40}$ ]] || { echo "CAPER_RELEASE_SHA must be a full lowercase source SHA." >&2; exit 2; }
mkdir -p "$ROOT/.build"
printf 'CAPER_SOURCE_REVISION = %s\n' "$revision" > "$ROOT/.build/SourceRevision.xcconfig"

FONT_RESOURCES="$ROOT/Resources/Fonts"
rm -rf "$FONT_RESOURCES"
mkdir -p "$FONT_RESOURCES"
for file in Figtree-Regular.ttf Figtree-Medium.ttf Figtree-Bold.ttf Figtree-Black.ttf BricolageGrotesque-Bold.ttf BricolageGrotesque-ExtraBold.ttf; do
  cp "$REPO/shared/fonts/$file" "$FONT_RESOURCES/$file"
done
cp "$REPO/shared/fonts/OFL.txt" "$FONT_RESOURCES/Fonts-OFL.txt"

# Native CPU inference runs from the app bundle, never a runtime network fetch.
# Keep the exact ORT headers and macOS binaries from the same release: Microsoft's
# 1.23.2 NuGet package, whose arm64 and x86_64 dylibs are joined into one
# universal library. The license and notices are the copies vendored for web.
ORT_NAME=onnxruntime-osx-universal2-1.23.2
ORT_DIR="$ROOT/.build/$ORT_NAME"
ORT_ARCHIVE="$ROOT/.build/microsoft.ml.onnxruntime.1.23.2.nupkg"
ORT_SHA=25fe172f7fcdf34f2b5b02c8b997f6b88abc282956849d4af4335af23d0c4e4a
ORT_NOTICES="$REPO/apps/web/public/audio/dpdfnet8-v2"
if [[ ! -f "$ORT_DIR/lib/libonnxruntime.1.23.2.dylib" ]]; then
  mkdir -p "$ROOT/.build"
  curl -fLsS --retry 6 --retry-delay 5 --retry-all-errors 'https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/1.23.2/microsoft.ml.onnxruntime.1.23.2.nupkg' -o "$ORT_ARCHIVE"
  echo "$ORT_SHA  $ORT_ARCHIVE" | shasum -a 256 -c -
  rm -rf "$ORT_DIR" "$ORT_DIR.unpack"
  unzip -q "$ORT_ARCHIVE" 'runtimes/osx-*/native/libonnxruntime.dylib' 'build/native/include/*' -d "$ORT_DIR.unpack"
  echo "d306d2bc768540766c7ed8a1e0ff05d2870c77a934ebeee4a7bafa1b732ef299  $ORT_DIR.unpack/runtimes/osx-arm64/native/libonnxruntime.dylib" | shasum -a 256 -c -
  echo "8c9c78de65ea3786f987c0d980e9c1b13a3a5fbc6b3e2965ba05b450e6e4c054  $ORT_DIR.unpack/runtimes/osx-x64/native/libonnxruntime.dylib" | shasum -a 256 -c -
  mkdir -p "$ORT_DIR/lib"
  mv "$ORT_DIR.unpack/build/native/include" "$ORT_DIR/include"
  lipo -create "$ORT_DIR.unpack/runtimes/osx-arm64/native/libonnxruntime.dylib" \
    "$ORT_DIR.unpack/runtimes/osx-x64/native/libonnxruntime.dylib" -output "$ORT_DIR/lib/libonnxruntime.1.23.2.dylib"
  rm -rf "$ORT_DIR.unpack"
fi
cp "$ORT_NOTICES/ORT-LICENSE" "$ORT_DIR/LICENSE"
cp "$ORT_NOTICES/ORT-ThirdPartyNotices.txt" "$ORT_DIR/ThirdPartyNotices.txt"
test -f "$ORT_DIR/include/onnxruntime_cxx_api.h"
test -f "$ORT_DIR/LICENSE"
test -f "$ORT_DIR/ThirdPartyNotices.txt"
test "$(shasum -a 256 "$REPO/apps/web/public/audio/dpdfnet8-v2/dpdfnet8_48khz_hr.onnx" | cut -d ' ' -f 1)" = \
  7b3afbb260a08fe9af3d16e3bda992971be1e7e951d1dee7c2d235f5c43f5631

# iOS uses the static XCFramework (arm64 device, arm64/x86_64 simulator) from
# the same pinned 1.23.2 NuGet package as macOS; Microsoft's former
# download.onnxruntime.ai pod archive host no longer serves it. Headers come
# from the framework itself, so they always match its binaries.
IOS_ORT="$ROOT/.build/onnxruntime-ios-1.23.2"
if [[ ! -f "$IOS_ORT/onnxruntime.xcframework/Info.plist" ]]; then
  mkdir -p "$ROOT/.build"
  if ! echo "$ORT_SHA  $ORT_ARCHIVE" | shasum -a 256 -c - >/dev/null 2>&1; then
    curl -fLsS --retry 6 --retry-delay 5 --retry-all-errors 'https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/1.23.2/microsoft.ml.onnxruntime.1.23.2.nupkg' -o "$ORT_ARCHIVE"
    echo "$ORT_SHA  $ORT_ARCHIVE" | shasum -a 256 -c -
  fi
  rm -rf "$IOS_ORT" "$IOS_ORT.unpack"
  unzip -q "$ORT_ARCHIVE" 'runtimes/ios/native/onnxruntime.xcframework.zip' -d "$IOS_ORT.unpack"
  echo "60cd467b88cbe61142dd3f9f6c1e5f4e1d88e3c48a943335aea7587324d48bb6  $IOS_ORT.unpack/runtimes/ios/native/onnxruntime.xcframework.zip" | shasum -a 256 -c -
  mkdir -p "$IOS_ORT"
  unzip -q "$IOS_ORT.unpack/runtimes/ios/native/onnxruntime.xcframework.zip" -d "$IOS_ORT"
  cp -R "$IOS_ORT/onnxruntime.xcframework/ios-arm64/onnxruntime.framework/Headers" "$IOS_ORT/Headers"
  rm -rf "$IOS_ORT.unpack"
fi
test -f "$IOS_ORT/Headers/onnxruntime_cxx_api.h"
test -f "$IOS_ORT/onnxruntime.xcframework/ios-arm64/onnxruntime.framework/onnxruntime"
test -f "$IOS_ORT/onnxruntime.xcframework/ios-arm64_x86_64-simulator/onnxruntime.framework/onnxruntime"
cp "$ORT_NOTICES/ORT-LICENSE" "$IOS_ORT/LICENSE"
cp "$ORT_NOTICES/ORT-ThirdPartyNotices.txt" "$IOS_ORT/ThirdPartyNotices.txt"

# The native RNNoise fallback is built from exact upstream C sources and its
# matching model. Neither engine downloads code or weights when the app runs.
RN_DIR="$ROOT/.build/rnnoise"
if [[ ! -f "$RN_DIR/src/rnnoise_data.c" ]]; then
  mkdir -p "$RN_DIR"
  curl -fLsS --retry 6 --retry-delay 5 --retry-all-errors 'https://github.com/xiph/rnnoise/archive/70f1d256acd4b34a572f999a05c87bf00b67730d.tar.gz' -o "$ROOT/.build/rnnoise-source.tar.gz"
  echo "f61ee0b3f4c4cd337303e003d333357c5eaf25ef5d75a742109ee59e9a0a3932  $ROOT/.build/rnnoise-source.tar.gz" | shasum -a 256 -c -
  tar -xzf "$ROOT/.build/rnnoise-source.tar.gz" -C "$RN_DIR" --strip-components=1
  curl -fLsS --retry 6 --retry-delay 5 --retry-all-errors 'https://media.xiph.org/rnnoise/models/rnnoise_data-0a8755f8e2d834eff6a54714ecc7d75f9932e845df35f8b59bc52a7cfe6e8b37.tar.gz' -o "$ROOT/.build/rnnoise-model.tar.gz"
  echo "0a8755f8e2d834eff6a54714ecc7d75f9932e845df35f8b59bc52a7cfe6e8b37  $ROOT/.build/rnnoise-model.tar.gz" | shasum -a 256 -c -
  tar -xzf "$ROOT/.build/rnnoise-model.tar.gz" -C "$RN_DIR" src/rnnoise_data.c src/rnnoise_data.h
fi
test -f "$RN_DIR/COPYING"

# AVIF photo encoding: libavif 1.4.2 with aom 3.15.1's encoder as one static
# XCFramework, built from SHA-256-pinned upstream sources and smoke-tested on
# every slice by joswayski/libavif-apple (Neon on arm64, SSE/AVX on x86_64).
# The bundled licence texts must be the ones that release ships.
AVIF_RELEASE=libavif-1.4.2-aom-3.15.1-r2
AVIF_ZIP_SHA=d42a48c0dc0aa2c4efbbb213dc925fcb4bfdc8f9865ed8f4074ab4dc348fd579
AVIF_DIR="$ROOT/.build/libavif-1.4.2"
AVIF_ZIP="$ROOT/.build/$AVIF_RELEASE.zip"
if [[ ! -f "$AVIF_DIR/libavif.xcframework/Info.plist" || "$(cat "$AVIF_DIR/release.txt" 2>/dev/null)" != "$AVIF_RELEASE" ]]; then
  mkdir -p "$ROOT/.build"
  curl -fLsS --retry 6 --retry-delay 5 --retry-all-errors "https://github.com/joswayski/libavif-apple/releases/download/$AVIF_RELEASE/libavif.xcframework.zip" -o "$AVIF_ZIP"
  echo "$AVIF_ZIP_SHA  $AVIF_ZIP" | shasum -a 256 -c -
  rm -rf "$AVIF_DIR"
  mkdir -p "$AVIF_DIR"
  unzip -q "$AVIF_ZIP" -d "$AVIF_DIR"
  echo "$AVIF_RELEASE" >"$AVIF_DIR/release.txt"
fi
test -f "$AVIF_DIR/libavif.xcframework/Info.plist"
for license in libavif-LICENSE.txt libaom-LICENSE.txt libaom-PATENTS.txt; do
  cmp -s "$AVIF_DIR/$license" "$ROOT/Resources/$license" || { echo "Resources/$license differs from $AVIF_RELEASE's; copy it from $AVIF_DIR." >&2; exit 1; }
done

# Shared Rust logic (apps/native/mobile) for every Apple slice, linked statically into CaperCore.
bash "$REPO/apps/native/mobile/build-apple.sh" "$ROOT/.build/CaperMobile.xcframework"

XCODEGEN_COMMIT=21ac9944b0ab546a07422dbed86f33dd2ebd76f8
XCODEGEN="$ROOT/.build/xcodegen-$XCODEGEN_COMMIT"
if [[ ! -d "$XCODEGEN/.git" ]]; then
  rm -rf "$XCODEGEN"
  git clone --quiet https://github.com/yonaskolb/XcodeGen.git "$XCODEGEN"
fi
git -C "$XCODEGEN" checkout --quiet --detach "$XCODEGEN_COMMIT"
test "$(git -C "$XCODEGEN" rev-parse HEAD)" = "$XCODEGEN_COMMIT"
swift run --package-path "$XCODEGEN" xcodegen --spec "$ROOT/project.yml" --project "$ROOT"
