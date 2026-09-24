#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"

"$ROOT/prepare-fonts.sh"
./gradlew --no-daemon \
  -PcaperFixtureMode=true \
  -PcaperApiBaseUrl=http://localhost:3001 \
  assembleDebug
mkdir -p dist
cp app/build/outputs/apk/debug/app-debug.apk dist/Caper-android-fixture-debug.apk
printf 'Loopback fixture-only debug APK: %s\n' "$ROOT/dist/Caper-android-fixture-debug.apk"
