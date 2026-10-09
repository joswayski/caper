#!/bin/sh
# Chromium that reports a mouse (hover, fine pointer) for the desktop hover
# fixtures: MESSAGE_TEST_CHROME="$PWD/scripts/fine-pointer-chromium.sh".
# Stock headless Chromium reports no hover device. Set CAPER_CHROMIUM to pick a
# browser; otherwise the newest Playwright Chromium is used.
set -eu
browser=${CAPER_CHROMIUM:-$(ls -d "${PLAYWRIGHT_BROWSERS_PATH:-$HOME/.cache/ms-playwright}"/chromium-*/chrome-linux/chrome 2>/dev/null | tail -n 1)}
[ -x "$browser" ] || { echo "Set CAPER_CHROMIUM to a Chromium executable" >&2; exit 1; }
exec "$browser" --blink-settings=primaryHoverType=2,availableHoverTypes=2,primaryPointerType=4,availablePointerTypes=4 "$@"
