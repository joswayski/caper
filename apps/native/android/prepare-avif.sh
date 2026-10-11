#!/usr/bin/env bash
# Builds the AVIF photo encoder's native inputs (docs/media.md "Client compression and
# previews"): libaom (encoder only, 8-bit) as a static library per ABI, plus the libavif
# sources that app/src/main/cpp compiles into libcaper_avif.so with a small JNI wrapper.
# Both are pinned (SHA-256 tarball, exact commit) and built once into app/build/native-inputs.
# Their licences are checked in as third_party/NOTICE-libaom.txt and NOTICE-libavif-encoder.txt.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
[[ -n "$SDK" ]] || { echo 'Set ANDROID_HOME to the Android SDK.' >&2; exit 1; }
NDK="$SDK/ndk/27.2.12479018"
CMAKE="$SDK/cmake/3.22.1/bin/cmake"
NINJA="$SDK/cmake/3.22.1/bin/ninja"
for tool in "$NDK/build/cmake/android.toolchain.cmake" "$CMAKE" "$NINJA"; do
  [[ -e "$tool" ]] || { echo "Missing $tool: install ndk;27.2.12479018 and cmake;3.22.1 with sdkmanager first." >&2; exit 1; }
done
OUT="$ROOT/app/build/native-inputs"
AVIF="$OUT/avif"
mkdir -p "$AVIF"

AOM_VERSION=3.15.1
LIBAVIF_VERSION=1.4.2
LIBAVIF_COMMIT=c5240fc79fe5c2407e10afd35f5505ef6333ea49

AOM_TAR="$AVIF/libaom-$AOM_VERSION.tar.gz"
if [[ ! -f "$AVIF/aom/CMakeLists.txt" ]]; then
  curl -fLsS --retry 6 --retry-delay 5 --retry-all-errors "https://storage.googleapis.com/aom-releases/libaom-$AOM_VERSION.tar.gz" -o "$AOM_TAR.tmp"
  mv "$AOM_TAR.tmp" "$AOM_TAR"
  echo "8ca0c52746174603500f0adb6f2a215d69c9ca2aab2acb3caa06fb791d8d01bf  $AOM_TAR" | sha256sum -c -
  rm -rf "$AVIF/aom" && mkdir -p "$AVIF/aom"
  tar -xzf "$AOM_TAR" -C "$AVIF/aom" --strip-components=1
  rm -f "$AOM_TAR"
fi

if [[ ! -f "$AVIF/libavif/src/write.c" ]]; then
  rm -rf "$AVIF/libavif" && mkdir -p "$AVIF/libavif"
  git -C "$AVIF/libavif" init -q
  git -C "$AVIF/libavif" fetch -q --depth 1 https://github.com/AOMediaCodec/libavif.git "$LIBAVIF_COMMIT"
  git -C "$AVIF/libavif" checkout -q FETCH_HEAD
  [[ "$(git -C "$AVIF/libavif" rev-parse HEAD)" == "$LIBAVIF_COMMIT" ]] || { echo "libavif $LIBAVIF_VERSION commit mismatch" >&2; exit 1; }
  rm -rf "$AVIF/libavif/.git"
fi

# Still-image encoding only: no decoder (AvifCompat's AOMedia artifact decodes), no 10/12-bit
# paths, no WebM/libyuv/denoiser. x86 and x86_64 (emulators, Chromebooks) build portable C
# because aom's x86 SIMD needs nasm, which the build machines do not have.
JOBS="$(( $(nproc 2>/dev/null || echo 2) < 4 ? $(nproc 2>/dev/null || echo 2) : 4 ))"
for abi in arm64-v8a armeabi-v7a x86 x86_64; do
  lib="$AVIF/lib/$abi/libaom.a"
  [[ -f "$lib" ]] && continue
  build="$AVIF/build-$abi"
  cpu=()
  [[ "$abi" == x86* ]] && cpu=(-DAOM_TARGET_CPU=generic)
  rm -rf "$build"
  "$CMAKE" -S "$AVIF/aom" -B "$build" -G Ninja -DCMAKE_MAKE_PROGRAM="$NINJA" \
    -DCMAKE_TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake" -DANDROID_ABI="$abi" -DANDROID_PLATFORM=android-26 \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS="-ffunction-sections -fdata-sections" -DCMAKE_CXX_FLAGS="-ffunction-sections -fdata-sections" \
    -DCONFIG_AV1_DECODER=0 -DCONFIG_AV1_HIGHBITDEPTH=0 -DCONFIG_WEBM_IO=0 -DCONFIG_LIBYUV=0 -DCONFIG_DENOISE=0 \
    -DCONFIG_PIC=1 -DCONFIG_MULTITHREAD=1 -DCONFIG_RUNTIME_CPU_DETECT=1 \
    -DENABLE_DOCS=0 -DENABLE_EXAMPLES=0 -DENABLE_TESTS=0 -DENABLE_TESTDATA=0 -DENABLE_TOOLS=0 -DENABLE_APPS=0 \
    "${cpu[@]}" >/dev/null
  "$CMAKE" --build "$build" --target aom -j "$JOBS" >/dev/null
  mkdir -p "$AVIF/lib/$abi"
  cp "$build/libaom.a" "$lib.tmp" && mv "$lib.tmp" "$lib"
  rm -rf "$build"
done
mkdir -p "$AVIF/include/aom"
cp "$AVIF"/aom/aom/*.h "$AVIF/include/aom/"

