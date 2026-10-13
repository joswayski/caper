#!/usr/bin/env bash
# Build and run the Windows/Linux Rust desktop client (apps/native/desktop) on
# Linux against the loopback fixture, either on a visible display (the Amp orb
# Desktop tab, or your own session) or on a private Xvfb for screenshots.
#
#   scripts/emulators/desktop.sh build
#   scripts/emulators/desktop.sh show [FIXTURE]        # default parity-mentions
#   scripts/emulators/desktop.sh shot [FIXTURE] [OUT]  # PNG of the window only
#   scripts/emulators/desktop.sh stop
#
# FIXTURE is any `--fixture` name from apps/native/desktop/README.md (`login`,
# `parity-channel`, `parity-narrow`, ...). Narrow fixtures open a 390x844 window.
# CAPER_SHOT_SCALE (default 2) sets the Xvfb scale factor and CAPER_SHOT_WAIT
# (default 8 seconds) how long to let the window settle before capturing.
# This is Linux rendering only: it is not Windows or macOS acceptance.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
native="$root/apps/native/desktop"
state="${CAPER_EMULATOR_STATE:-${XDG_CACHE_HOME:-$HOME/.cache}/caper-emulators}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$native/target}"
binary="$CARGO_TARGET_DIR/debug/caper-desktop"
mkdir -p "$state"

build() {
  python3 "$root/scripts/native_fonts.py"
  # The orb's default linker can exceed its memory limit on this binary; lld does not.
  CC="${CC:-clang-21}" CXX="${CXX:-clang++-21}" CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" \
    LK_CUSTOM_WEBRTC="${LK_CUSTOM_WEBRTC:-$(python3 "$native/voice-spike/fetch_libwebrtc.py" --platform linux)}" \
    RUSTFLAGS="${RUSTFLAGS:--Clink-arg=-fuse-ld=lld}" \
    cargo build --manifest-path "$native/Cargo.toml" -p caper-desktop --locked
}

ensure_built() { [[ -x "$binary" ]] || build; }

show() {
  local fixture="${1:-parity-mentions}"
  ensure_built
  "$root/scripts/emulators/fixture.sh" start
  stop
  if [[ -x /opt/amp-desktop/run-script ]]; then
    # Amp orb: run inside the Desktop session so it appears in the Desktop tab.
    amp orb desktop ensure >/dev/null
    printf 'cd %q\nexec %q --fixture %q\n' "$root" "$binary" "$fixture" >"$state/desktop-show.sh"
    /opt/amp-desktop/run-script "$state/desktop-show.sh"
    echo "Caper desktop ($fixture) is open in the orb Desktop tab."
  else
    [[ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]] || { echo "No display: use '$0 shot' instead." >&2; exit 1; }
    nohup "$binary" --fixture "$fixture" >"$state/desktop.log" 2>&1 &
    echo "Caper desktop ($fixture) started; log: $state/desktop.log"
  fi
}

shot() {
  local fixture="${1:-parity-mentions}" out="${2:-$root/.amp/in/artifacts/desktop-${1:-parity-mentions}.png}"
  local scale="${CAPER_SHOT_SCALE:-2}" display=90 window
  xvfb="" app="" # global: the EXIT trap outlives this function
  ensure_built
  "$root/scripts/emulators/fixture.sh" start
  while [[ -e "/tmp/.X$display-lock" ]]; do display=$((display + 1)); done
  Xvfb ":$display" -screen 0 "$((1440 * scale))x$((900 * scale))x24" -nolisten tcp >"$state/xvfb.log" 2>&1 &
  xvfb=$!
  # Force X11 (winit prefers Wayland when WAYLAND_DISPLAY is set) at a fixed scale.
  env -u WAYLAND_DISPLAY DISPLAY=":$display" WINIT_X11_SCALE_FACTOR="$scale" \
    "$binary" --fixture "$fixture" >"$state/desktop-shot.log" 2>&1 &
  app=$!
  trap 'kill "$app" "$xvfb" 2>/dev/null || true' EXIT
  window=""
  for _ in {1..120}; do
    window="$(DISPLAY=":$display" xdotool search --name '^Caper$' 2>/dev/null | head -n 1)" && [[ -n "$window" ]] && break
    kill -0 "$app" 2>/dev/null || { cat "$state/desktop-shot.log" >&2; exit 1; }
    sleep 0.5
  done
  [[ -n "$window" ]] || { echo "The Caper window never appeared." >&2; exit 1; }
  sleep "${CAPER_SHOT_WAIT:-8}"
  mkdir -p "$(dirname "$out")"
  DISPLAY=":$display" import -window "$window" "$out"
  echo "$out"
}

stop() {
  pkill -x caper-desktop 2>/dev/null || true
}

case "${1:-}" in
  build) build ;;
  show) show "${2:-}" ;;
  shot) shot "${2:-}" "${3:-}" ;;
  stop) stop ;;
  *) awk 'NR > 1 && !/^#/ { exit } NR > 1' "$0" >&2; exit 2 ;;
esac
