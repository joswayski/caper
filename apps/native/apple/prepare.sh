#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$ROOT/../../.." && pwd)"

python3 "$REPO/scripts/native_fonts.py"
FONT_CACHE="$REPO/shared/fonts/cache"
FONT_RESOURCES="$ROOT/Resources/Fonts"
rm -rf "$FONT_RESOURCES"
mkdir -p "$FONT_RESOURCES"
for file in Satoshi-Regular.otf Satoshi-Medium.otf Satoshi-Bold.otf Satoshi-Black.otf Satoshi-FFL.txt; do
  test -s "$FONT_CACHE/$file"
  cp "$FONT_CACHE/$file" "$FONT_RESOURCES/$file"
done

XCODEGEN_COMMIT=21ac9944b0ab546a07422dbed86f33dd2ebd76f8
XCODEGEN="$ROOT/.build/xcodegen-$XCODEGEN_COMMIT"
if [[ ! -d "$XCODEGEN/.git" ]]; then
  rm -rf "$XCODEGEN"
  git clone --quiet https://github.com/yonaskolb/XcodeGen.git "$XCODEGEN"
fi
git -C "$XCODEGEN" checkout --quiet --detach "$XCODEGEN_COMMIT"
test "$(git -C "$XCODEGEN" rev-parse HEAD)" = "$XCODEGEN_COMMIT"
swift run --package-path "$XCODEGEN" xcodegen --spec "$ROOT/project.yml" --project "$ROOT"
