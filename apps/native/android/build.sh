#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"

if [[ -z "${JAVA_HOME:-}" ]] && command -v java >/dev/null 2>&1; then
  JAVA_BIN="$(readlink -f "$(command -v java)")"
  export JAVA_HOME="$(dirname "$(dirname "$JAVA_BIN")")"
fi

./gradlew --no-daemon testDebugUnitTest lintDebug assembleDebug
mkdir -p dist
cp app/build/outputs/apk/debug/app-debug.apk dist/Caper-android-debug.apk
unzip -Z1 dist/Caper-android-debug.apk | grep -Fxq 'assets/NOTICE-webrtc-sdk.txt'
printf 'Development-only debug APK: %s\n' "$ROOT/dist/Caper-android-debug.apk"
