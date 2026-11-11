#!/usr/bin/env bash
# The development-only UIKit iPhone client (uikit-project.yml). Nothing here
# ships: release.yml, TestFlight and build.sh use CaperApple.xcodeproj.
#
#   ./apps/native/apple/uikit.sh build   # simulator Debug build
#   ./apps/native/apple/uikit.sh test    # UI tests against the bench fixture
#
# `test` starts scripts/native-parity-fixture.mjs with CAPER_BENCH_MESSAGES=3000
# on 127.0.0.1:3001 and exports screenshots into ignored uikit-artifacts/.
# The SwiftUI screenshot pairs run only with CAPER_PARITY_SWIFTUI=1, after the
# shipping iOS app (chat.caper.ios) is installed on the same simulator.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$ROOT/../../.." && pwd)"
MODE="${1:-}"
if [[ "$MODE" != build && "$MODE" != test ]]; then
  echo "Usage: $0 build|test [xcodebuild arguments...]" >&2
  exit 2
fi
shift
# CI creates an iPhone 16 (native.yml). Elsewhere, when this Xcode has none,
# use the first available iPhone instead of failing to find a destination.
if [[ -n "${CAPER_IOS_SIMULATOR_DESTINATION:-}" ]]; then
  destination="$CAPER_IOS_SIMULATOR_DESTINATION"
elif xcrun simctl list devices available | grep -q '^ *iPhone 16 ('; then
  destination="platform=iOS Simulator,name=iPhone 16,OS=latest"
else
  udid="$(xcrun simctl list devices available | sed -n 's/^ *iPhone[^(]* (\([0-9A-F-]*\)).*/\1/p' | head -1)"
  [[ -n "$udid" ]] || { echo "No iPhone simulator is available; set CAPER_IOS_SIMULATOR_DESTINATION." >&2; exit 2; }
  destination="platform=iOS Simulator,id=$udid"
fi

"$ROOT/prepare.sh"
xcodegen=("$ROOT"/.build/xcodegen-*)
swift run --package-path "${xcodegen[0]}" xcodegen --spec "$ROOT/uikit-project.yml" --project "$ROOT"

if [[ "$MODE" == build ]]; then
  xcodebuild -project "$ROOT/CaperUIKit.xcodeproj" -scheme CaperUIKit -configuration Debug \
    -destination "$destination" -derivedDataPath "$ROOT/DerivedData-UIKit" CODE_SIGNING_ALLOWED=NO "$@" build
  exit 0
fi

CAPER_BENCH_MESSAGES=3000 node "$REPO/scripts/native-parity-fixture.mjs" >"${TMPDIR:-/tmp}/caper-uikit-fixture.log" 2>&1 &
FIXTURE_PID=$!
trap 'kill "$FIXTURE_PID" 2>/dev/null || true' EXIT
for _ in {1..50}; do curl --silent --fail http://127.0.0.1:3001/health >/dev/null && break; sleep .1; done
curl --silent --fail http://127.0.0.1:3001/health >/dev/null
# The composer tests use the software keyboard, which a connected hardware keyboard hides.
defaults write com.apple.iphonesimulator ConnectHardwareKeyboard -bool false
# The attach drawer test picks a photo from the simulator's library.
"$ROOT/seed-simulator-photos.sh" "$destination"

[[ "${CAPER_PARITY_SWIFTUI:-}" == 1 ]] && export TEST_RUNNER_CAPER_PARITY_SWIFTUI=1
rm -rf "$ROOT/UIKitResults.xcresult" "$ROOT/uikit-artifacts"
set +e
xcodebuild -project "$ROOT/CaperUIKit.xcodeproj" -scheme CaperUIKit -configuration Debug \
  -destination "$destination" -derivedDataPath "$ROOT/DerivedData-UIKit" -resultBundlePath "$ROOT/UIKitResults.xcresult" \
  -test-timeouts-enabled YES -default-test-execution-time-allowance 300 "$@" test
status=$?
set -e
if [[ -d "$ROOT/UIKitResults.xcresult" ]]; then
  xcrun xcresulttool export attachments --path "$ROOT/UIKitResults.xcresult" --output-path "$ROOT/uikit-artifacts" \
    || echo "warning: could not export UIKit test attachments" >&2
fi
exit "$status"
