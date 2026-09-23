#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
MODE="${1:-}"
MAC_BUNDLE_ID="${CAPER_MACOS_BUNDLE_ID:-chat.caper.macos}"
IOS_BUNDLE_ID="${CAPER_IOS_BUNDLE_ID:-chat.caper.ios}"
if [[ "$MODE" != macos && "$MODE" != ios ]]; then
  echo "Usage: $0 macos|ios" >&2
  exit 2
fi
mkdir -p "$ROOT/dist"

export CAPER_MACOS_BUNDLE_ID="$MAC_BUNDLE_ID" CAPER_IOS_BUNDLE_ID="$IOS_BUNDLE_ID"
XCODEGEN_COMMIT=21ac9944b0ab546a07422dbed86f33dd2ebd76f8
XCODEGEN="$ROOT/.build/xcodegen-$XCODEGEN_COMMIT"
if [[ ! -d "$XCODEGEN/.git" ]]; then
  rm -rf "$XCODEGEN"
  git clone --quiet https://github.com/yonaskolb/XcodeGen.git "$XCODEGEN"
fi
git -C "$XCODEGEN" checkout --quiet --detach "$XCODEGEN_COMMIT"
test "$(git -C "$XCODEGEN" rev-parse HEAD)" = "$XCODEGEN_COMMIT"
swift run --package-path "$XCODEGEN" xcodegen --spec "$ROOT/project.yml" --project "$ROOT"
xcodebuild -project "$ROOT/CaperApple.xcodeproj" -scheme CaperMacOS -configuration Debug \
  -destination 'platform=macOS' -derivedDataPath "$ROOT/DerivedData-Tests" \
  CAPER_MACOS_BUNDLE_ID="$MAC_BUNDLE_ID" CAPER_IOS_BUNDLE_ID="$IOS_BUNDLE_ID" test

case "$MODE" in
  macos)
    machine="$(uname -m)"
    case "$machine" in arm64) artifact_arch=arm64 ;; x86_64) artifact_arch=x64 ;; *) echo "Unsupported macOS architecture: $machine" >&2; exit 2 ;; esac
    rm -rf "$ROOT/DerivedData" "$ROOT/dist/Caper.app"
    xcodebuild -project "$ROOT/CaperApple.xcodeproj" -scheme CaperMacOS -configuration Release \
      -destination 'platform=macOS' -derivedDataPath "$ROOT/DerivedData" \
      CAPER_MACOS_BUNDLE_ID="$MAC_BUNDLE_ID" ARCHS="$machine" ONLY_ACTIVE_ARCH=YES build
    cp -R "$ROOT/DerivedData/Build/Products/Release/Caper.app" "$ROOT/dist/Caper.app"
    test -x "$ROOT/dist/Caper.app/Contents/MacOS/Caper"
    test -f "$ROOT/dist/Caper.app/Contents/Frameworks/WebRTC.framework/WebRTC"
    test -f "$ROOT/dist/Caper.app/Contents/Resources/WebRTC-LICENSE.txt"
    otool -l "$ROOT/dist/Caper.app/Contents/MacOS/Caper" | grep -q '@executable_path/../Frameworks'
    codesign --verify --deep --strict "$ROOT/dist/Caper.app"
    rm -f "$ROOT/dist/Caper-macos-$artifact_arch.zip"
    ditto -c -k --keepParent "$ROOT/dist/Caper.app" "$ROOT/dist/Caper-macos-$artifact_arch.zip"
    echo "$ROOT/dist/Caper-macos-$artifact_arch.zip"
    ;;
  ios)
    rm -rf "$ROOT/DerivedData" "$ROOT/dist/Caper.app"
    xcodebuild -project "$ROOT/CaperApple.xcodeproj" -scheme CaperIOS -configuration Release \
      -destination 'generic/platform=iOS Simulator' -derivedDataPath "$ROOT/DerivedData" \
      CAPER_IOS_BUNDLE_ID="$IOS_BUNDLE_ID" ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO build
    product="$ROOT/DerivedData/Build/Products/Release-iphonesimulator"
    cp -R "$product/Caper.app" "$ROOT/dist/Caper.app"
    test -x "$ROOT/dist/Caper.app/Caper"
    test -f "$ROOT/dist/Caper.app/Frameworks/WebRTC.framework/WebRTC"
    test -f "$ROOT/dist/Caper.app/WebRTC-LICENSE.txt"
    rm -f "$ROOT/dist/Caper-ios-simulator-arm64.zip"
    ditto -c -k --keepParent "$ROOT/dist/Caper.app" "$ROOT/dist/Caper-ios-simulator-arm64.zip"
    echo "$ROOT/dist/Caper-ios-simulator-arm64.zip (simulator only; not installable on a physical device)"
    ;;
  *) echo "Usage: $0 macos|ios" >&2; exit 2 ;;
esac
