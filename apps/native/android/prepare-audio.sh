#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$ROOT/../../.." && pwd)"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
[[ -n "$SDK" ]] || { echo 'Set ANDROID_HOME to the Android SDK.' >&2; exit 1; }
for component in 'ndk/27.2.12479018' 'cmake/3.22.1'; do
  [[ -d "$SDK/$component" ]] || { echo "Install $component with sdkmanager first." >&2; exit 1; }
done
OUT="$ROOT/app/build/native-inputs"
mkdir -p "$OUT/assets" "$OUT/ort" "$OUT/rnnoise"
MODEL="$REPO/apps/web/public/audio/dpdfnet8-v2/dpdfnet8_48khz_hr.onnx"
echo "7b3afbb260a08fe9af3d16e3bda992971be1e7e951d1dee7c2d235f5c43f5631  $MODEL" | sha256sum -c -
cp "$MODEL" "$OUT/assets/dpdfnet8_48khz_hr.onnx"
cp "$(dirname "$MODEL")/LICENSE-APACHE-2.0" "$OUT/assets/LICENSE-DPDFNet.txt"

AAR="$OUT/onnxruntime-android-1.23.2.aar"
if [[ ! -f "$AAR" ]]; then
  curl -fLsS --retry 2 'https://repo.maven.apache.org/maven2/com/microsoft/onnxruntime/onnxruntime-android/1.23.2/onnxruntime-android-1.23.2.aar' -o "$AAR.tmp"
  mv "$AAR.tmp" "$AAR"
fi
echo "82048d1f462218adae4ba76477089ab0ba76093d84f733540066db1a8ba6b827  $AAR" | sha256sum -c -
unzip -oq "$AAR" 'headers/*' 'jni/*/libonnxruntime.so' -d "$OUT/ort"
for notice in LICENSE ThirdPartyNotices.txt; do
  if [[ ! -f "$OUT/ort/$notice" ]]; then
    curl -fLsS --retry 2 "https://raw.githubusercontent.com/microsoft/onnxruntime/v1.23.2/$notice" -o "$OUT/ort/$notice"
  fi
done
echo "2f07c72751aed99790b8a4869cf2311df85a860b22ded05fa22803587a48922c  $OUT/ort/LICENSE" | sha256sum -c -
echo "e9e90971a8e75a9a8ac0c6412e29c1202d079998389915aa485f46c816c3b4cc  $OUT/ort/ThirdPartyNotices.txt" | sha256sum -c -
cp "$OUT/ort/LICENSE" "$OUT/assets/LICENSE-ONNXRuntime.txt"
cp "$OUT/ort/ThirdPartyNotices.txt" "$OUT/assets/NOTICE-ONNXRuntime.txt"

RN="$OUT/rnnoise"
if [[ ! -f "$RN/src/rnnoise_data.c" ]]; then
  SRC="$OUT/rnnoise-source.tar.gz"
  WEIGHTS="$OUT/rnnoise-model.tar.gz"
  curl -fLsS --retry 2 'https://github.com/xiph/rnnoise/archive/70f1d256acd4b34a572f999a05c87bf00b67730d.tar.gz' -o "$SRC"
  echo "f61ee0b3f4c4cd337303e003d333357c5eaf25ef5d75a742109ee59e9a0a3932  $SRC" | sha256sum -c -
  tar -xzf "$SRC" -C "$RN" --strip-components=1
  curl -fLsS --retry 2 'https://media.xiph.org/rnnoise/models/rnnoise_data-0a8755f8e2d834eff6a54714ecc7d75f9932e845df35f8b59bc52a7cfe6e8b37.tar.gz' -o "$WEIGHTS"
  echo "0a8755f8e2d834eff6a54714ecc7d75f9932e845df35f8b59bc52a7cfe6e8b37  $WEIGHTS" | sha256sum -c -
  tar -xzf "$WEIGHTS" -C "$RN" src/rnnoise_data.c src/rnnoise_data.h
fi
test -s "$RN/COPYING"
cp "$RN/COPYING" "$OUT/assets/LICENSE-RNNoise.txt"
