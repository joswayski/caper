#!/usr/bin/env bash
# Run the Android client (apps/native/android) in the Android Emulator against
# the loopback fixture. Needs hardware acceleration: Linux with a writable
# /dev/kvm (e.g. a GitHub-hosted Ubuntu runner or a Linux PC) or a Mac.
#
#   scripts/emulators/android.sh install   # SDK, emulator, API 36 image, NDK, AVD "caper"
#   scripts/emulators/android.sh start     # boot the AVD and wait for boot_completed
#   scripts/emulators/android.sh app       # build + install the fixture APK, open it
#   scripts/emulators/android.sh shot [OUT]
#   scripts/emulators/android.sh smoke     # apps/native/android/smoke.py
#   scripts/emulators/android.sh stop
#
# Amp orbs have no KVM. Software emulation (CAPER_ANDROID_SOFTWARE=1) is refused
# unless 7 GB of RAM is available: on a 4 GB orb it exhausted memory, and on an
# 8 vCPU/16 GB orb API 36 took 27 minutes to boot, taps took ~5 s, screenshots
# 30-60 s and system_server restarted. Set CAPER_ANDROID_WINDOW=1 to show the
# emulator window on your display. Emulator runs are not physical-device acceptance.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
android="$root/apps/native/android"
state="${CAPER_EMULATOR_STATE:-${XDG_CACHE_HOME:-$HOME/.cache}/caper-emulators}"
mkdir -p "$state"
case "$(uname -s)" in
  Darwin) default_sdk="$HOME/Library/Android/sdk" tools_os=mac ;;
  *) default_sdk="$HOME/.android-sdk" tools_os=linux ;;
esac
export ANDROID_HOME="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$default_sdk}}"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export PATH="$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/emulator:$ANDROID_HOME/platform-tools:$PATH"
case "$(uname -m)" in arm64 | aarch64) abi=arm64-v8a ;; *) abi=x86_64 ;; esac
image="${CAPER_ANDROID_IMAGE:-system-images;android-36;google_apis;$abi}"
avd="${CAPER_ANDROID_AVD:-caper}"
package=chat.caper.android.debug
activity="$package/chat.caper.android.MainActivity"
apk="$android/dist/Caper-android-fixture-debug.apk"

in_orb() { command -v amp >/dev/null 2>&1 && amp orb service list >/dev/null 2>&1; }

install() {
  if ! command -v sdkmanager >/dev/null 2>&1; then
    local download
    download="$(mktemp -d)"
    # Same command-line tools release as the Android CI job.
    curl -fsSL --retry 3 -o "$download/tools.zip" \
      "https://dl.google.com/android/repository/commandlinetools-$tools_os-13114758_latest.zip"
    unzip -q "$download/tools.zip" -d "$download"
    mkdir -p "$ANDROID_HOME/cmdline-tools"
    rm -rf "$ANDROID_HOME/cmdline-tools/latest"
    mv "$download/cmdline-tools" "$ANDROID_HOME/cmdline-tools/latest"
    rm -rf "$download"
  fi
  # `yes` exits on SIGPIPE once sdkmanager stops reading; that is not a failure.
  yes | sdkmanager --licenses >/dev/null || true
  echo "Installing Android SDK packages (several GB on first run; log: $state/sdkmanager.log)"
  sdkmanager platform-tools emulator "platforms;android-36" "build-tools;36.0.0" \
    "ndk;27.2.12479018" "cmake;3.22.1" "$image" >"$state/sdkmanager.log" 2>&1 \
    || { tail -n 20 "$state/sdkmanager.log" >&2; exit 1; }
  # Pixel 7 Pro matches the CI emulator profile used by smoke.py.
  echo no | avdmanager create avd -n "$avd" -k "$image" -d "${CAPER_ANDROID_DEVICE:-pixel_7_pro}" --force >/dev/null
  echo "AVD $avd ready ($image) in $ANDROID_HOME"
}

accelerated() {
  [[ "$(uname -s)" == Darwin ]] || [[ -r /dev/kvm && -w /dev/kvm ]]
}

start() {
  command -v emulator >/dev/null 2>&1 || install
  if adb get-state >/dev/null 2>&1; then
    echo "An emulator is already connected."
    return 0
  fi
  local accel=on timeout="${CAPER_ANDROID_BOOT_TIMEOUT:-900}" available command
  if ! accelerated; then
    available="$(awk '/MemAvailable/ { print int($2 / 1024) }' /proc/meminfo)"
    if [[ "${CAPER_ANDROID_SOFTWARE:-}" != 1 ]] || ((available < 7000)); then
      echo "No hardware acceleration (/dev/kvm) here, so the Android Emulator is unusable:" >&2
      echo "run this on a KVM Linux host or a Mac. See 'Running the clients' in docs/media.md." >&2
      echo "To try software emulation anyway: CAPER_ANDROID_SOFTWARE=1 with >= 7000 MB available (now ${available} MB)." >&2
      exit 2
    fi
    accel=off timeout="${CAPER_ANDROID_BOOT_TIMEOUT:-3600}"
  fi
  command="$ANDROID_HOME/emulator/emulator -avd $avd -accel $accel -no-audio -no-snapshot -no-boot-anim -no-metrics -gpu swiftshader_indirect"
  [[ "${CAPER_ANDROID_WINDOW:-}" == 1 ]] || command+=" -no-window"
  if in_orb; then
    amp orb service start caper-android-emulator --command "$command" >/dev/null
  else
    nohup $command >"$state/emulator.log" 2>&1 &
  fi
  local started=$SECONDS
  # Query only an online device: adb shell can block on one that is still offline.
  until [[ "$(adb get-state 2>/dev/null)" == device &&
    "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == 1 ]]; do
    ((SECONDS - started < timeout)) || { echo "Emulator did not finish booting in ${timeout}s." >&2; exit 1; }
    sleep 5
  done
  for scale in window_animation_scale transition_animation_scale animator_duration_scale; do
    adb shell settings put global "$scale" 0
  done
  echo "Emulator booted in $((SECONDS - started))s (accel $accel)."
}

app() {
  bash "$android/fixture-build.sh"
  "$root/scripts/emulators/fixture.sh" start
  # The fixture APK calls http://localhost:3001 inside the emulator.
  adb reverse tcp:3001 tcp:3001
  adb install -r "$apk"
  adb shell am start -n "$activity"
}

shot() {
  local out="${1:-$root/.amp/in/artifacts/android.png}"
  mkdir -p "$(dirname "$out")"
  adb exec-out screencap -p >"$out"
  echo "$out"
}

smoke() {
  [[ -f "$apk" ]] || bash "$android/fixture-build.sh"
  "$root/scripts/emulators/fixture.sh" start
  # smoke.py reverses port 3001 and installs the APK itself.
  python3 "$android/smoke.py"
}

stop() {
  adb emu kill 2>/dev/null || true
  if in_orb; then amp orb service stop caper-android-emulator >/dev/null 2>&1 || true; fi
}

case "${1:-}" in
  install) install ;;
  start) start ;;
  app) app ;;
  shot) shot "${2:-}" ;;
  smoke) smoke ;;
  stop) stop ;;
  *) awk 'NR > 1 && !/^#/ { exit } NR > 1' "$0" >&2; exit 2 ;;
esac
