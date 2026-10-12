#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"

if [[ -z "${JAVA_HOME:-}" ]] && command -v java >/dev/null 2>&1; then
  JAVA_BIN="$(readlink -f "$(command -v java)")"
  export JAVA_HOME="$(dirname "$(dirname "$JAVA_BIN")")"
fi

"$ROOT/prepare-fonts.sh"
bash "$ROOT/prepare-audio.sh"
# minifyReleaseWithR8 shrinks the release build's code (no signing needed) so the
# JNI check below sees what Play and APK users actually run.
CMAKE_BUILD_PARALLEL_LEVEL=1 ./gradlew --no-daemon --max-workers=1 testDebugUnitTest lintDebug assembleDebug minifyReleaseWithR8
mkdir -p dist
cp app/build/outputs/apk/debug/app-debug.apk dist/Caper-android-debug.apk
archive_entries="$(mktemp)"
trap 'rm -f "$archive_entries"' EXIT
unzip -Z1 dist/Caper-android-debug.apk > "$archive_entries"
grep -Fxq 'assets/NOTICE-webrtc-sdk.txt' "$archive_entries"
grep -Fxq 'assets/NOTICE-Lucide.txt' "$archive_entries"
grep -Fxq 'assets/NOTICE-libavif.txt' "$archive_entries"
grep -Fxq 'assets/NOTICE-libavif-encoder.txt' "$archive_entries"
grep -Fxq 'assets/NOTICE-libaom.txt' "$archive_entries"
grep -Fxq 'assets/NOTICE-Satoshi-Fontshare.txt' "$archive_entries"
for asset in effects/channel-join.wav effects/disconnect.wav dpdfnet8_48khz_hr.onnx LICENSE-DPDFNet.txt LICENSE-ONNXRuntime.txt NOTICE-ONNXRuntime.txt LICENSE-RNNoise.txt; do
  grep -Fxq "assets/$asset" "$archive_entries"
done
for abi in arm64-v8a armeabi-v7a x86 x86_64; do
  grep -Fxq "lib/$abi/libcaper_audio.so" "$archive_entries"
  grep -Fxq "lib/$abi/libonnxruntime.so" "$archive_entries"
  grep -Fxq "lib/$abi/libavif_android.so" "$archive_entries"
  grep -Fxq "lib/$abi/libcaper_avif.so" "$archive_entries"
done
# Debug and release package the same native libraries.
python3 "$ROOT/check-jni-keeps.py" app/build/outputs/mapping/release/mapping.txt app/build/outputs/mapping/release/usage.txt dist/Caper-android-debug.apk
printf 'Development-only debug APK: %s\n' "$ROOT/dist/Caper-android-debug.apk"
