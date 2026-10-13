#!/usr/bin/env bash
# Build and run the Apple clients (apps/native/apple) against the loopback
# fixture on a Mac with Xcode: the iPhone app in the iOS Simulator, or the Mac app.
#
#   scripts/emulators/apple.sh ios [FIXTURE] [OUT]    # boots CAPER_IOS_DEVICE (default iPhone 16)
#   scripts/emulators/apple.sh macos [FIXTURE] [OUT]
#   scripts/emulators/apple.sh stop
#
# The app starts signed in as the fixture owner (CAPER_TEST_MODE=parity, as in
# the parity UI tests). FIXTURE is an optional CAPER_UI_FIXTURE name such as
# reaction-chips or voice-roster. OUT is a PNG screenshot path. In an Amp Mac orb
# the booted simulator is also served as an interactive portal page.
# Simulator runs are not physical iPhone acceptance.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
apple="$root/apps/native/apple"
device="${CAPER_IOS_DEVICE:-iPhone 16}"
ios_bundle="${CAPER_IOS_BUNDLE_ID:-chat.caper.ios}"
artifacts="$root/.amp/in/artifacts"

require_mac() {
  [[ "$(uname -s)" == Darwin ]] && command -v xcodebuild >/dev/null 2>&1 || {
    echo "The Apple clients need macOS with Xcode (an Amp Mac orb or a Mac runner)." >&2
    exit 2
  }
}

# Fixture launch environment, mirroring CaperParityUITests.configuredApp.
fixture_env() {
  local prefix="$1" fixture="$2"
  printf '%s\n' "${prefix}CAPER_TEST_MODE=parity" "${prefix}CAPER_API_BASE_URL=http://127.0.0.1:3001" \
    "${prefix}CAPER_TEST_BEARER=fixture-owner-token" "${prefix}CAPER_TEST_SPACE_ID=space0000001"
  [[ -z "$fixture" ]] || printf '%s\n' "${prefix}CAPER_UI_FIXTURE=$fixture"
}

ios() {
  local fixture="${1:-}" out="${2:-$artifacts/ios.png}" app
  "$root/scripts/emulators/fixture.sh" start
  "$apple/prepare.sh"
  xcodebuild -project "$apple/CaperApple.xcodeproj" -scheme CaperIOS -configuration Debug \
    -destination "platform=iOS Simulator,name=$device,OS=latest" -derivedDataPath "$apple/DerivedData-Emulator" \
    CAPER_IOS_BUNDLE_ID="$ios_bundle" CODE_SIGNING_ALLOWED=NO build
  app="$apple/DerivedData-Emulator/Build/Products/Debug-iphonesimulator/Caper.app"
  xcrun simctl boot "$device" 2>/dev/null || true
  xcrun simctl bootstatus "$device" -b
  open -a Simulator 2>/dev/null || true
  xcrun simctl install "$device" "$app"
  xcrun simctl terminate "$device" "$ios_bundle" 2>/dev/null || true
  # simctl passes SIMCTL_CHILD_* variables to the app without the prefix.
  env $(fixture_env SIMCTL_CHILD_ "$fixture") xcrun simctl launch "$device" "$ios_bundle"
  sleep "${CAPER_SHOT_WAIT:-8}"
  mkdir -p "$(dirname "$out")"
  xcrun simctl io "$device" screenshot "$out"
  echo "$out"
  if command -v amp >/dev/null 2>&1 && amp orb service list >/dev/null 2>&1; then
    amp orb service start caper-ios-simulator --command "amp orb simulator serve --device '$device'" --portal
  fi
}

macos() {
  local fixture="${1:-}" out="${2:-$artifacts/macos.png}" app
  "$root/scripts/emulators/fixture.sh" start
  "$apple/prepare.sh"
  xcodebuild -project "$apple/CaperApple.xcodeproj" -scheme CaperMacOS -configuration Debug \
    -destination "platform=macOS,arch=$(uname -m)" -derivedDataPath "$apple/DerivedData-Emulator" build
  app="$apple/DerivedData-Emulator/Build/Products/Debug/Caper.app"
  pkill -x Caper 2>/dev/null || true
  env $(fixture_env "" "$fixture") nohup "$app/Contents/MacOS/Caper" -ApplePersistenceIgnoreState YES \
    >"${TMPDIR:-/tmp}/caper-macos.log" 2>&1 &
  sleep "${CAPER_SHOT_WAIT:-8}"
  mkdir -p "$(dirname "$out")"
  screencapture -x "$out"
  echo "$out"
}

case "${1:-}" in
  ios | macos | stop) require_mac ;;
esac
case "${1:-}" in
  ios) ios "${2:-}" "${3:-}" ;;
  macos) macos "${2:-}" "${3:-}" ;;
  stop)
    xcrun simctl terminate "$device" "$ios_bundle" 2>/dev/null || true
    pkill -x Caper 2>/dev/null || true
    ;;
  *) awk 'NR > 1 && !/^#/ { exit } NR > 1' "$0" >&2; exit 2 ;;
esac
