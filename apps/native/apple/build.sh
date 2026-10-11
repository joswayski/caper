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
# macOS builds and tests target this Mac's architecture unless CAPER_MACOS_ARCH
# names another; x86_64 on Apple silicon runs the tests under Rosetta.
mac_arch="${CAPER_MACOS_ARCH:-$(uname -m)}"
case "$mac_arch" in arm64) artifact_arch=arm64 ;; x86_64) artifact_arch=x64 ;; *) echo "Unsupported macOS architecture: $mac_arch" >&2; exit 2 ;; esac
if [[ "$mac_arch" == x86_64 && "$(uname -m)" == arm64 ]] && ! /usr/bin/arch -x86_64 /usr/bin/true 2>/dev/null; then
  echo "x86_64 tests on Apple silicon need Rosetta: softwareupdate --install-rosetta --agree-to-license" >&2
  exit 2
fi

export CAPER_MACOS_BUNDLE_ID="$MAC_BUNDLE_ID" CAPER_IOS_BUNDLE_ID="$IOS_BUNDLE_ID"
"$ROOT/prepare.sh"
xcodebuild -project "$ROOT/CaperApple.xcodeproj" -scheme CaperMacOS -configuration Debug \
  -destination "platform=macOS,arch=$mac_arch" -derivedDataPath "$ROOT/DerivedData-Tests" \
  CAPER_MACOS_BUNDLE_ID="$MAC_BUNDLE_ID" CAPER_IOS_BUNDLE_ID="$IOS_BUNDLE_ID" \
  -test-timeouts-enabled YES -default-test-execution-time-allowance 300 -maximum-test-execution-time-allowance 600 test

case "$MODE" in
  macos)
    rm -rf "$ROOT/DerivedData" "$ROOT/dist/Caper.app"
    # Release builds carry the release number the self-updater compares.
    versions=()
    if [[ -n "${CAPER_BUILD_NUMBER:-}" ]]; then
      [[ "$CAPER_BUILD_NUMBER" =~ ^[1-9][0-9]*$ ]] || { echo "CAPER_BUILD_NUMBER must be a positive integer." >&2; exit 2; }
      versions=(CURRENT_PROJECT_VERSION="$CAPER_BUILD_NUMBER" MARKETING_VERSION="0.1.$CAPER_BUILD_NUMBER")
    fi
    xcodebuild -project "$ROOT/CaperApple.xcodeproj" -scheme CaperMacOS -configuration Release \
      -destination "platform=macOS,arch=$mac_arch" -derivedDataPath "$ROOT/DerivedData" \
      CAPER_MACOS_BUNDLE_ID="$MAC_BUNDLE_ID" ARCHS="$mac_arch" ONLY_ACTIVE_ARCH=YES ${versions[@]+"${versions[@]}"} build
    cp -R "$ROOT/DerivedData/Build/Products/Release/Caper.app" "$ROOT/dist/Caper.app"
    test -x "$ROOT/dist/Caper.app/Contents/MacOS/Caper"
    test -f "$ROOT/dist/Caper.app/Contents/Frameworks/WebRTC.framework/WebRTC"
    test -f "$ROOT/dist/Caper.app/Contents/Frameworks/CaperRTCBridge.framework/CaperRTCBridge"
    test -f "$ROOT/dist/Caper.app/Contents/Frameworks/libonnxruntime.1.23.2.dylib"
    test -f "$ROOT/dist/Caper.app/Contents/Frameworks/CaperRTCBridge.framework/Resources/dpdfnet8_48khz_hr.onnx"
    test -f "$ROOT/dist/Caper.app/Contents/Resources/WebRTC-LICENSE.txt"
    test -f "$ROOT/dist/Caper.app/Contents/Resources/WebRTC-PATENTS.txt"
    for license in libwebp-LICENSE libavif-LICENSE libaom-LICENSE libaom-PATENTS; do test -f "$ROOT/dist/Caper.app/Contents/Resources/$license.txt"; done
    test -f "$ROOT/dist/Caper.app/Contents/Resources/Satoshi-FFL.txt"
    for font in Regular Medium Bold Black; do test -f "$ROOT/dist/Caper.app/Contents/Resources/Satoshi-$font.otf"; done
    lipo -archs "$ROOT/dist/Caper.app/Contents/MacOS/Caper" | tr ' ' '\n' | grep -qx "$mac_arch"
    lipo -archs "$ROOT/dist/Caper.app/Contents/Frameworks/WebRTC.framework/WebRTC" | tr ' ' '\n' | grep -qx "$mac_arch"
    lipo -archs "$ROOT/dist/Caper.app/Contents/Frameworks/CaperRTCBridge.framework/CaperRTCBridge" | tr ' ' '\n' | grep -qx "$mac_arch"
    lipo -archs "$ROOT/dist/Caper.app/Contents/Frameworks/libonnxruntime.1.23.2.dylib" | tr ' ' '\n' | grep -qx "$mac_arch"
    otool -l "$ROOT/dist/Caper.app/Contents/MacOS/Caper" | grep -q '@executable_path/../Frameworks'
    codesign --verify --deep --strict "$ROOT/dist/Caper.app"
    rm -f "$ROOT/dist/Caper-macos-$artifact_arch.zip"
    ditto -c -k --keepParent "$ROOT/dist/Caper.app" "$ROOT/dist/Caper-macos-$artifact_arch.zip"
    echo "$ROOT/dist/Caper-macos-$artifact_arch.zip"
    ;;
  ios)
    xcodebuild -project "$ROOT/CaperApple.xcodeproj" -scheme CaperIOS -configuration Debug \
      -destination 'platform=iOS Simulator,name=iPhone 16,OS=latest' -derivedDataPath "$ROOT/DerivedData-iOS-Tests" \
      CAPER_MACOS_BUNDLE_ID="$MAC_BUNDLE_ID" CAPER_IOS_BUNDLE_ID="$IOS_BUNDLE_ID" \
      -test-timeouts-enabled YES -default-test-execution-time-allowance 300 -maximum-test-execution-time-allowance 600 test
    rm -rf "$ROOT/DerivedData" "$ROOT/dist/Caper.app"
    xcodebuild -project "$ROOT/CaperApple.xcodeproj" -scheme CaperIOS -configuration Release \
      -destination 'generic/platform=iOS Simulator' -derivedDataPath "$ROOT/DerivedData" \
      CAPER_IOS_BUNDLE_ID="$IOS_BUNDLE_ID" ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO build
    product="$ROOT/DerivedData/Build/Products/Release-iphonesimulator"
    cp -R "$product/Caper.app" "$ROOT/dist/Caper.app"
    test -x "$ROOT/dist/Caper.app/Caper"
    test -f "$ROOT/dist/Caper.app/Frameworks/WebRTC.framework/WebRTC"
    test -f "$ROOT/dist/Caper.app/Frameworks/CaperRTCBridgeIOS.framework/CaperRTCBridgeIOS"
    test -f "$ROOT/dist/Caper.app/Frameworks/CaperRTCBridgeIOS.framework/dpdfnet8_48khz_hr.onnx"
    test -f "$ROOT/dist/Caper.app/WebRTC-LICENSE.txt"
    test -f "$ROOT/dist/Caper.app/WebRTC-PATENTS.txt"
    for license in libwebp-LICENSE libavif-LICENSE libaom-LICENSE libaom-PATENTS; do test -f "$ROOT/dist/Caper.app/$license.txt"; done
    test -f "$ROOT/dist/Caper.app/Satoshi-FFL.txt"
    for font in Regular Medium Bold Black; do test -f "$ROOT/dist/Caper.app/Satoshi-$font.otf"; done
    lipo -archs "$ROOT/dist/Caper.app/Caper" | tr ' ' '\n' | grep -qx arm64
    lipo -archs "$ROOT/dist/Caper.app/Frameworks/WebRTC.framework/WebRTC" | tr ' ' '\n' | grep -qx arm64
    lipo -archs "$ROOT/dist/Caper.app/Frameworks/CaperRTCBridgeIOS.framework/CaperRTCBridgeIOS" | tr ' ' '\n' | grep -qx arm64
    rm -f "$ROOT/dist/Caper-ios-simulator-arm64.zip"
    ditto -c -k --keepParent "$ROOT/dist/Caper.app" "$ROOT/dist/Caper-ios-simulator-arm64.zip"
    echo "$ROOT/dist/Caper-ios-simulator-arm64.zip (simulator only; not installable on a physical device)"
    ;;
  *) echo "Usage: $0 macos|ios" >&2; exit 2 ;;
esac
