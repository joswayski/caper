#!/usr/bin/env bash
# Builds .build/libavif-<version>/libavif.xcframework: the libavif AVIF encoder
# with aom's AV1 encoder merged into one static library per platform, plus
# avif.h and a module map, so CaperCore can `import libavif`. Both come from
# exact upstream release tarballs pinned by SHA-256; prepare.sh runs this and
# it does nothing when the same recipe was already built.
#
# Encoder only and 8-bit only (photos are 8-bit 4:2:0): no AV1 decoder, high
# bit depth, apps, tests, examples, libyuv, libsharpyuv or libvmaf. arm64 slices
# use aom's Neon intrinsics (dot-product/i8mm paths chosen at run time; no SVE,
# which no Apple CPU has). x86_64 slices use aom's SSE/AVX code when nasm or
# yasm is installed, else aom's portable C (much slower; a warning says so).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
AVIF_VERSION=1.4.2
AVIF_SHA=2b645287340ba5a631d268b551dc2d72bd73ac33335962dd36dcdb6d8366921d
AOM_VERSION=3.15.1
AOM_SHA=8ca0c52746174603500f0adb6f2a215d69c9ca2aab2acb3caa06fb791d8d01bf
MACOS_MIN=14.0
IOS_MIN=17.0

OUT="$ROOT/.build/libavif-$AVIF_VERSION"
XCFRAMEWORK="$OUT/libavif.xcframework"
RECIPE_FILE="$OUT/recipe.txt"

X86_CPU=x86_64 AOM_NASM=0
if command -v nasm >/dev/null 2>&1; then
  AOM_NASM=1
elif ! command -v yasm >/dev/null 2>&1; then
  X86_CPU=generic
fi
# Rebuild whenever this script, a pinned version or the x86_64 code path changes.
RECIPE="libavif $AVIF_VERSION aom $AOM_VERSION x86_64 $X86_CPU script $(shasum -a 256 "$0" | cut -d ' ' -f 1)"
if [[ -f "$XCFRAMEWORK/Info.plist" && -f "$RECIPE_FILE" && "$(cat "$RECIPE_FILE")" == "$RECIPE" ]]; then
  exit 0
fi

for tool in cmake xcrun xcodebuild libtool lipo; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "build-libavif.sh: '$tool' is required to build libavif/aom for the Apple clients." >&2
    [[ "$tool" == cmake ]] && echo "Install CMake 3.22 or newer (for example 'brew install cmake') and rerun." >&2
    exit 1
  fi
done
if [[ "$X86_CPU" == generic ]]; then
  echo "warning: build-libavif.sh: neither nasm nor yasm is installed, so the x86_64 (Intel Mac and" >&2
  echo "warning: x86_64 simulator) aom slices are portable C without SSE/AVX and encode several times" >&2
  echo "warning: slower. Install nasm ('brew install nasm') and rerun to rebuild them with SIMD." >&2
fi
if command -v ninja >/dev/null 2>&1; then GENERATOR=Ninja; else GENERATOR="Unix Makefiles"; fi
JOBS="$(sysctl -n hw.ncpu 2>/dev/null || echo 4)"

mkdir -p "$ROOT/.build"
fetch() { # url archive sha256
  if [[ ! -f "$2" ]] || ! echo "$3  $2" | shasum -a 256 -c - >/dev/null 2>&1; then
    curl -fLsS --retry 6 --retry-delay 5 --retry-all-errors "$1" -o "$2"
  fi
  echo "$3  $2" | shasum -a 256 -c -
}
AOM_ARCHIVE="$ROOT/.build/libaom-$AOM_VERSION.tar.gz"
AVIF_ARCHIVE="$ROOT/.build/libavif-$AVIF_VERSION.tar.gz"
# aom publishes signed release tarballs; libavif's release is its GitHub tag archive.
fetch "https://storage.googleapis.com/aom-releases/libaom-$AOM_VERSION.tar.gz" "$AOM_ARCHIVE" "$AOM_SHA"
fetch "https://github.com/AOMediaCodec/libavif/archive/refs/tags/v$AVIF_VERSION.tar.gz" "$AVIF_ARCHIVE" "$AVIF_SHA"

WORK="$ROOT/.build/libavif-work"
rm -rf "$WORK" "$OUT"
mkdir -p "$WORK/src" "$OUT"
tar -xzf "$AOM_ARCHIVE" -C "$WORK/src"
tar -xzf "$AVIF_ARCHIVE" -C "$WORK/src"
AOM_SRC="$WORK/src/libaom-$AOM_VERSION"
AVIF_SRC="$WORK/src/libavif-$AVIF_VERSION"
test -f "$AOM_SRC/LICENSE" && test -f "$AOM_SRC/PATENTS" && test -f "$AVIF_SRC/LICENSE"
# The bundled licence texts must contain these releases' own, verbatim.
for pair in "$AVIF_SRC/LICENSE:libavif-LICENSE.txt" "$AOM_SRC/LICENSE:libaom-LICENSE.txt" \
  "$AOM_SRC/third_party/fastfeat/LICENSE:libaom-LICENSE.txt" "$AOM_SRC/third_party/vector/LICENSE:libaom-LICENSE.txt" \
  "$AOM_SRC/third_party/x86inc/LICENSE:libaom-LICENSE.txt" "$AOM_SRC/PATENTS:libaom-PATENTS.txt"; do
  if ! python3 -c 'import sys; sys.exit(open(sys.argv[1]).read() not in open(sys.argv[2]).read())' "${pair%%:*}" "$ROOT/Resources/${pair#*:}"; then
    echo "build-libavif.sh: Resources/${pair#*:} does not contain ${pair%%:*}; update it for this release." >&2
    exit 1
  fi
done

# One configure/build/install of aom and then libavif for one platform and arch.
build_slice() { # name sdk arch clang-target aom-cpu
  local name="$1" sdk="$2" arch="$3" target="$4" cpu="$5"
  local sysroot prefix flags
  sysroot="$(xcrun --sdk "$sdk" --show-sdk-path)"
  prefix="$WORK/$name/prefix"
  # The -target triple sets the platform and minimum OS (device or simulator),
  # so CMake's own -m*-version-min flag stays off (empty deployment target).
  flags="-target $target -fvisibility=hidden"
  local common=(
    -G "$GENERATOR" -Wno-dev
    -DCMAKE_BUILD_TYPE=Release
    -DCMAKE_INSTALL_PREFIX="$prefix"
    -DCMAKE_SYSTEM_NAME=Darwin
    -DCMAKE_SYSTEM_PROCESSOR="$arch"
    -DCMAKE_OSX_ARCHITECTURES="$arch"
    -DCMAKE_OSX_SYSROOT="$sysroot"
    -DCMAKE_OSX_DEPLOYMENT_TARGET=
    -DCMAKE_C_COMPILER="$(xcrun --find clang)"
    -DCMAKE_CXX_COMPILER="$(xcrun --find clang++)"
    -DCMAKE_C_FLAGS="$flags"
    -DCMAKE_CXX_FLAGS="$flags"
    -DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY
    -DBUILD_SHARED_LIBS=OFF
  )
  cmake -S "$AOM_SRC" -B "$WORK/$name/aom" "${common[@]}" \
    -DAOM_TARGET_CPU="$cpu" \
    -DCONFIG_AV1_DECODER=0 -DCONFIG_AV1_ENCODER=1 -DCONFIG_AV1_HIGHBITDEPTH=0 \
    -DCONFIG_LIBYUV=0 -DCONFIG_WEBM_IO=0 -DCONFIG_TUNE_VMAF=0 -DCONFIG_TUNE_BUTTERAUGLI=0 \
    -DENABLE_APPS=0 -DENABLE_EXAMPLES=0 -DENABLE_TOOLS=0 -DENABLE_TESTS=0 -DENABLE_TESTDATA=0 -DENABLE_DOCS=0 \
    -DENABLE_SVE=0 -DENABLE_SVE2=0 -DENABLE_NASM="$AOM_NASM"
  cmake --build "$WORK/$name/aom" --parallel "$JOBS"
  cmake --install "$WORK/$name/aom"
  cmake -S "$AVIF_SRC" -B "$WORK/$name/avif" "${common[@]}" \
    -DAVIF_CODEC_AOM=SYSTEM -DAVIF_CODEC_AOM_ENCODE=ON -DAVIF_CODEC_AOM_DECODE=OFF \
    -DAOM_INCLUDE_DIR="$prefix/include" -DAOM_LIBRARY="$prefix/lib/libaom.a" \
    -DCMAKE_DISABLE_FIND_PACKAGE_PkgConfig=ON \
    -DAVIF_LIBYUV=OFF -DAVIF_LIBSHARPYUV=OFF -DAVIF_LIBXML2=OFF -DAVIF_JPEG=OFF -DAVIF_ZLIBPNG=OFF \
    -DAVIF_BUILD_APPS=OFF -DAVIF_BUILD_TESTS=OFF -DAVIF_BUILD_EXAMPLES=OFF
  cmake --build "$WORK/$name/avif" --parallel "$JOBS"
  cmake --install "$WORK/$name/avif"
  # One archive per slice: libavif plus the aom encoder it calls.
  libtool -static -no_warning_for_no_symbols -o "$WORK/$name/libavif.a" "$prefix/lib/libavif.a" "$prefix/lib/libaom.a"
  lipo -archs "$WORK/$name/libavif.a" | grep -qx "$arch"
}

build_slice macos-arm64 macosx arm64 "arm64-apple-macos$MACOS_MIN" arm64
build_slice macos-x86_64 macosx x86_64 "x86_64-apple-macos$MACOS_MIN" "$X86_CPU"
build_slice ios-arm64 iphoneos arm64 "arm64-apple-ios$IOS_MIN" arm64
build_slice sim-arm64 iphonesimulator arm64 "arm64-apple-ios$IOS_MIN-simulator" arm64
build_slice sim-x86_64 iphonesimulator x86_64 "x86_64-apple-ios$IOS_MIN-simulator" "$X86_CPU"

mkdir -p "$WORK/universal/macos" "$WORK/universal/sim"
lipo -create "$WORK/macos-arm64/libavif.a" "$WORK/macos-x86_64/libavif.a" -output "$WORK/universal/macos/libavif.a"
lipo -create "$WORK/sim-arm64/libavif.a" "$WORK/sim-x86_64/libavif.a" -output "$WORK/universal/sim/libavif.a"

# Headers/libavif/ keeps this module map from colliding with other libraries'
# in the shared include directory Xcode copies XCFramework headers into.
HEADERS="$WORK/Headers"
mkdir -p "$HEADERS/libavif/avif"
cp "$WORK/macos-arm64/prefix/include/avif/avif.h" "$HEADERS/libavif/avif/avif.h"
cat >"$HEADERS/libavif/module.modulemap" <<'EOF'
module libavif {
    header "avif/avif.h"
    export *
}
EOF
xcodebuild -create-xcframework \
  -library "$WORK/universal/macos/libavif.a" -headers "$HEADERS" \
  -library "$WORK/ios-arm64/libavif.a" -headers "$HEADERS" \
  -library "$WORK/universal/sim/libavif.a" -headers "$HEADERS" \
  -output "$XCFRAMEWORK" >/dev/null
test -f "$XCFRAMEWORK/Info.plist"
rm -rf "$WORK"
echo "$RECIPE" >"$RECIPE_FILE"
echo "Built $XCFRAMEWORK (libavif $AVIF_VERSION, aom $AOM_VERSION, x86_64: $X86_CPU)"
