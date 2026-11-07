#!/usr/bin/env bash
# Device benchmarks for the UIKit client and the shipping SwiftUI app, both
# built Release from bench-project.yml as TEST FIXTURE apps (chat.caper.bench.*)
# that can only talk to the fixture. Nothing here ships or touches caper.chat.
#
#   CAPER_FIXTURE_HOST=$(ipconfig getifaddr en0) \
#   CAPER_BENCH_DESTINATION='platform=iOS,id=<device UDID>' DEVELOPMENT_TEAM=<team> \
#     ./apps/native/apple/bench.sh test uikit|swiftui [xcodebuild arguments...]
#   ./apps/native/apple/bench.sh build uikit|swiftui   # compile only, no fixture
#
# `test` starts scripts/native-parity-fixture.mjs on CAPER_FIXTURE_HOST (default
# 127.0.0.1, for simulators; a device needs the Mac's private-LAN address) and
# CAPER_FIXTURE_PORT (default 3101), with 3,000 #general rows and
# CAPER_BENCH_FORMATTED_MESSAGES (default 400) #bench-formatting rows.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$ROOT/../../.." && pwd)"
MODE="${1:-}"
APP="${2:-}"
if [[ "$MODE" != build && "$MODE" != test ]] || [[ "$APP" != uikit && "$APP" != swiftui ]]; then
  echo "Usage: $0 build|test uikit|swiftui [xcodebuild arguments...]" >&2
  exit 2
fi
shift 2
scheme=$([[ "$APP" == uikit ]] && echo CaperBenchUIKit || echo CaperBenchSwiftUI)
destination="${CAPER_BENCH_DESTINATION:-platform=iOS Simulator,name=iPhone 16,OS=latest}"
host="${CAPER_FIXTURE_HOST:-127.0.0.1}"
port="${CAPER_FIXTURE_PORT:-3101}"
if [[ "$destination" != *Simulator* && "$host" == 127.* ]]; then
  echo "A device cannot reach the Mac's loopback: set CAPER_FIXTURE_HOST to its private-LAN address." >&2
  exit 2
fi
signing=(CODE_SIGNING_ALLOWED=NO)
if [[ "$destination" != *Simulator* ]]; then
  : "${DEVELOPMENT_TEAM:?Set DEVELOPMENT_TEAM to sign the chat.caper.bench.* apps for the device}"
  signing=(-allowProvisioningUpdates "DEVELOPMENT_TEAM=$DEVELOPMENT_TEAM")
fi

"$ROOT/prepare.sh"
xcodegen=("$ROOT"/.build/xcodegen-*)
swift run --package-path "${xcodegen[0]}" xcodegen --spec "$ROOT/bench-project.yml" --project "$ROOT"
build=(xcodebuild -project "$ROOT/CaperBench.xcodeproj" -scheme "$scheme" -configuration Release
  -destination "$destination" -derivedDataPath "$ROOT/DerivedData-Bench" "${signing[@]}")

if [[ "$MODE" == build ]]; then
  "${build[@]}" "$@" build-for-testing
  exit 0
fi

CAPER_FIXTURE_HOST="$host" CAPER_FIXTURE_PORT="$port" CAPER_FIXTURE_GATEWAY_PORT="$((port + 1))" \
  CAPER_BENCH_MESSAGES=3000 CAPER_BENCH_FORMATTED_MESSAGES="${CAPER_BENCH_FORMATTED_MESSAGES:-400}" \
  node "$REPO/scripts/native-parity-fixture.mjs" >"${TMPDIR:-/tmp}/caper-bench-fixture.log" 2>&1 &
FIXTURE_PID=$!
trap 'kill "$FIXTURE_PID" 2>/dev/null || true' EXIT
fixture="http://$host:$port"
for _ in {1..50}; do curl --silent --fail "$fixture/health" >/dev/null && break; sleep .1; done
curl --silent --fail "$fixture/health" >/dev/null || { cat "${TMPDIR:-/tmp}/caper-bench-fixture.log" >&2; exit 1; }

export TEST_RUNNER_CAPER_FIXTURE_URL="$fixture"
results="$ROOT/BenchResults-$APP.xcresult"
rm -rf "$results"
"${build[@]}" -resultBundlePath "$results" -test-timeouts-enabled YES \
  -default-test-execution-time-allowance 900 -maximum-test-execution-time-allowance 1800 "$@" test
