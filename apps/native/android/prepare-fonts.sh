#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
REPOSITORY="$(cd "$ROOT/../../.." && pwd)"
CACHE="$REPOSITORY/shared/fonts/cache"
GENERATED="$ROOT/app/build/generated/caper-fonts"

python3 "$REPOSITORY/scripts/native_fonts.py"

mkdir -p "$GENERATED/res/font" "$GENERATED/assets"
for weight in Regular Medium Bold Black; do
  source="$CACHE/Satoshi-$weight.otf"
  [[ -f "$source" ]] || { echo "Missing official Satoshi font: $source" >&2; exit 1; }
  cp "$source" "$GENERATED/res/font/satoshi_${weight,,}.otf"
done

license="$CACHE/Satoshi-FFL.txt"
[[ -f "$license" ]] || { echo "Missing official Fontshare license: $license" >&2; exit 1; }
cp "$license" "$GENERATED/assets/NOTICE-Satoshi-Fontshare.txt"
