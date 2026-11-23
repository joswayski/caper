#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
REPOSITORY="$(cd "$ROOT/../../.." && pwd)"
FONTS="$REPOSITORY/shared/fonts"
GENERATED="$ROOT/app/build/generated/caper-fonts"

mkdir -p "$GENERATED/res/font" "$GENERATED/assets"
for weight in Regular Medium Bold Black; do
  # tr, not ${weight,,}: macOS ships bash 3.2.
  cp "$FONTS/Figtree-$weight.ttf" "$GENERATED/res/font/figtree_$(printf %s "$weight" | tr "[:upper:]" "[:lower:]").ttf"
done
cp "$FONTS/Nunito-Bold.ttf" "$GENERATED/res/font/nunito_bold.ttf"
cp "$FONTS/Nunito-ExtraBold.ttf" "$GENERATED/res/font/nunito_extrabold.ttf"
cp "$FONTS/OFL.txt" "$GENERATED/assets/NOTICE-Fonts-OFL.txt"
