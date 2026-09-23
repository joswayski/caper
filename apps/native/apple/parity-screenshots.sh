#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$ROOT/../../.." && pwd)"
PLATFORM="${1:-}"
if [[ "$PLATFORM" != macos && "$PLATFORM" != ios ]]; then
  echo "Usage: $0 macos|ios" >&2
  exit 2
fi

export CAPER_MACOS_BUNDLE_ID="${CAPER_MACOS_BUNDLE_ID:-chat.caper.macos}"
export CAPER_IOS_BUNDLE_ID="${CAPER_IOS_BUNDLE_ID:-chat.caper.ios}"

node "$REPO/scripts/native-parity-fixture.mjs" >"${TMPDIR:-/tmp}/caper-native-parity-fixture.log" 2>&1 &
FIXTURE_PID=$!
trap 'kill "$FIXTURE_PID" 2>/dev/null || true' EXIT
for _ in {1..50}; do curl --silent --fail http://127.0.0.1:3001/api/spaces -H 'authorization: Bearer fixture-owner-token' >/dev/null && break; sleep .1; done
curl --silent --fail http://127.0.0.1:3001/api/spaces -H 'authorization: Bearer fixture-owner-token' >/dev/null

"$ROOT/prepare.sh"
rm -rf "$ROOT/ParityResults.xcresult" "$ROOT/parity-artifacts"
if [[ "$PLATFORM" == macos ]]; then
  scheme=CaperMacOSParityTests
  destination='platform=macOS'
else
  scheme=CaperIOSParityTests
  destination='platform=iOS Simulator,name=iPhone 16,OS=latest'
fi
xcodebuild -project "$ROOT/CaperApple.xcodeproj" -scheme "$scheme" -configuration Debug \
  -destination "$destination" -derivedDataPath "$ROOT/DerivedData-Parity" \
  -resultBundlePath "$ROOT/ParityResults.xcresult" test
xcrun xcresulttool export attachments --path "$ROOT/ParityResults.xcresult" --output-path "$ROOT/parity-artifacts"
echo "$ROOT/parity-artifacts"
