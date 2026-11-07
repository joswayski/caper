#!/usr/bin/env bash
# Caper Linux hardware check: what an orb can't test. Run it from a desktop
# session on real hardware (see HARDWARE-CHECK.md). It records the machine,
# runs the scripted performance scenarios on every GTK renderer and display
# backend against the local fixture (and the egui desktop for comparison),
# then walks through the manual checks, saving everything to one report.
set -u
cd "$(dirname "$0")/../../.."
REPO=$PWD
STAMP=$(date +%Y%m%d-%H%M%S)
REPORT=${REPORT:-$HOME/caper-hardware-check-$STAMP.md}
WORK=$(mktemp -d /tmp/caper-hwcheck.XXXXXX)
PORT=${CAPER_CHECK_PORT:-3931}
RUNS=${RUNS:-3}
TARGET=$REPO/apps/native/desktop/target/release
LINUX=$TARGET/caper-linux
EGUI=$TARGET/caper-desktop
FIXTURE_PID=""

say() { printf '\n\033[1m%s\033[0m\n' "$*"; }
log() { printf '%s\n' "$*" >>"$REPORT"; }
cleanup() { [ -n "$FIXTURE_PID" ] && kill "$FIXTURE_PID" 2>/dev/null; rm -rf "$WORK"; }
trap cleanup EXIT

# Asks one question; the answer and an optional note go to the report.
ask() {
  local answer note
  printf '\n%s\n' "$1"
  read -r -p "  Works? [y/n/s=skip] " answer
  read -r -p "  Note (Enter for none): " note
  log "- [$( [ "$answer" = y ] && echo x || echo ' ')] $1 — ${answer:-s}${note:+ — $note}"
}

# A clean profile, so the fixture account never touches your real one. The
# keyring entry is per server, so your caper.chat session stays as it is.
profile() {
  export XDG_CONFIG_HOME=$WORK/$1/config XDG_DATA_HOME=$WORK/$1/data XDG_CACHE_HOME=$WORK/$1/cache
  mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"
}

# bench <label> <binary> <scenario> [seconds]: one scripted run, appended to
# the report.
bench() {
  local label=$1 bin=$2 scenario=$3 limit=${4:-90} out=$WORK/bench.jsonl
  rm -f "$out"
  BENCH=$scenario BENCH_OUT=$out timeout "$limit" "$bin" --api-url "http://127.0.0.1:$PORT" >>"$WORK/app.log" 2>&1
  if [ -s "$out" ]; then
    log "$label $(cat "$out")"
  else
    log "$label {\"scenario\":\"$scenario\",\"error\":\"no result (crashed, timed out or never signed in)\"}"
  fi
}

say "Caper Linux hardware check → $REPORT"
{
  echo "# Caper Linux hardware check, $(date '+%Y-%m-%d %H:%M')"
  echo
  echo "## Machine"
  echo '```'
  echo "commit: $(git rev-parse --short HEAD 2>/dev/null)"
  echo "kernel: $(uname -sr)"
  grep -m1 PRETTY_NAME /etc/os-release 2>/dev/null
  echo "session: ${XDG_SESSION_TYPE:-?} desktop: ${XDG_CURRENT_DESKTOP:-?}"
  echo "gtk4: $(pkg-config --modversion gtk4 2>/dev/null || echo '?')"
  echo "cpu: $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2-)"
  echo "memory: $(free -h | awk '/Mem:/ {print $2}')"
  command -v glxinfo >/dev/null && glxinfo -B 2>/dev/null | grep -E 'renderer string|version string'
  command -v vulkaninfo >/dev/null && vulkaninfo --summary 2>/dev/null | grep -E 'deviceName|driverName' | head -4
  echo "input method: GTK_IM_MODULE=${GTK_IM_MODULE:-} XMODIFIERS=${XMODIFIERS:-} $(pgrep -l -x 'ibus-daemon|fcitx5|fcitx' | tr '\n' ' ')"
  command -v pactl >/dev/null && pactl info 2>/dev/null | grep -E 'Server Name|Default Sink|Default Source'
  echo "orca: $(command -v orca || echo 'not installed')"
  echo '```'
  echo
} >"$REPORT"

say "Building release binaries (skip with SKIP_BUILD=1)"
if [ -z "${SKIP_BUILD:-}" ]; then
  if [ -z "${LK_CUSTOM_WEBRTC:-}" ]; then
    LK_CUSTOM_WEBRTC=$(python3 apps/native/desktop/voice-spike/fetch_libwebrtc.py --platform linux | tail -1)
    export LK_CUSTOM_WEBRTC
  fi
  python3 scripts/native_fonts.py >/dev/null
  cargo build --manifest-path apps/native/desktop/Cargo.toml --release -p caper-linux -p caper-desktop || {
    echo "Build failed; fix it or rerun with SKIP_BUILD=1."
    exit 1
  }
fi

say "Starting the fixture on 127.0.0.1:$PORT (3,000 messages in #general)"
CAPER_BENCH_MESSAGES=3000 CAPER_FIXTURE_PORT=$PORT CAPER_FIXTURE_GATEWAY_PORT=$((PORT + 1)) \
  node scripts/native-parity-fixture.mjs >"$WORK/fixture.log" 2>&1 &
FIXTURE_PID=$!
sleep 2

say "Signing in to the fixture"
echo "Each client opens once. If it shows sign-in, use fixture@example.test and"
echo "code ABC234. It closes by itself about 2 s after #general's messages show."
read -r -p "Press Enter to start." _
profile linux
bench "sign-in caper-linux" "$LINUX" startup 600
profile egui
bench "sign-in caper-desktop" "$EGUI" startup 600

# GTK's renderers on the session's own backend, plus GL through XWayland.
# A renderer GTK was built without falls back to its default (see the log).
native=x11
[ "${XDG_SESSION_TYPE:-}" = wayland ] && native=wayland
configs="$native:gl $native:ngl $native:vulkan $native:cairo"
[ "$native" = wayland ] && configs="$configs x11:gl"
count=$(( $(wc -w <<<"$configs") + 1 ))
say "Performance: $RUNS runs of each scenario per renderer (hands off the mouse and keyboard)"
echo "Each run opens Caper, waits for #general, then scrolls, types or idles by itself."
read -r -p "Press Enter to start (about $((RUNS * 4 * count * 15 / 60)) minutes)." _
log ""
log "## Performance"
log ""
log "One JSON line per run (the Windows bench's metrics). The label is the client, GDK_BACKEND and GSK_RENDERER."
log ""
log '```'
for scenario in startup idle scroll typing; do
  for run in $(seq 1 "$RUNS"); do
    profile linux
    for config in $configs; do
      GDK_BACKEND=${config%:*} GSK_RENDERER=${config#*:} \
        bench "caper-linux ${config%:*} ${config#*:}" "$LINUX" "$scenario"
    done
    profile egui
    bench "caper-desktop wgpu" "$EGUI" "$scenario"
  done
done
log '```'
log ""

say "Manual checks. Answer y, n or s for each; notes go into the report."
log "## Manual checks (${XDG_CURRENT_DESKTOP:-?}, ${XDG_SESSION_TYPE:-?})"
log ""
echo "Caper opens on the fixture now. Leave it open for the next questions."
profile linux
"$LINUX" --api-url "http://127.0.0.1:$PORT" >>"$WORK/app.log" 2>&1 &
APP=$!
sleep 3
ask "Display: the window opens with the Caper icon and title in the dock or task bar, and text is sharp at your display scale (also try 125% or 150% if you use fractional scaling)."
ask "Display: maximize, restore, resize below 760 px wide (narrow layout) and back, and drag the pane edges. Nothing flickers, jumps or leaves stale pixels."
ask "Clipboard: copy message text into another app and paste text and a screenshot from another app into the composer."
ask "Drag and drop: drag a file from the file manager onto the conversation; it shows the drop overlay and attaches."
echo
echo "Notifications: in Settings, set Notify me about to All messages, then put"
echo "another window in front of Caper. A message arrives in #general 5 s after Enter."
read -r -p "Press Enter." _
sleep 5
curl -s -XPOST "localhost:$PORT/__fixture/control" -H 'content-type: application/json' \
  -d '{"incomingMessage":{"channelId":"chan00000001","text":"hardware check ping"}}' >/dev/null
ask "Notifications: a desktop notification for the message appears, and clicking it brings Caper to the front on #general."
log ""
log "### Input methods"
echo
echo "Switch to an IME (ibus or fcitx5, for example Japanese/Mozc, Chinese/Pinyin or Korean/Hangul) and type in the composer."
ask "IME: the preedit text shows underlined in the composer, and the candidate window sits next to the cursor (not at a screen corner)."
ask "IME: committing a candidate inserts it once; Enter while composing picks the candidate instead of sending."
ask "IME: Ctrl+Z after a commit undoes the whole commit; Ctrl+Shift+Z redoes it."
ask "Markers: type **bold**, _italic_, ~~struck~~ and \`code\` by hand (with and without the IME on). Each shows styled with its markers hidden, and sends as typed."
ask "Compose and dead keys: type é and ñ with your compose key or a dead-key layout."
ask "Drafts: type a draft, switch channels and back, and quit and reopen Caper; the draft is still there, formatting included."
kill "$APP" 2>/dev/null
log ""
log "### Screen reader (Orca)"
echo
echo "Start Orca (Super+Alt+S on GNOME, or run 'orca') and open Caper again; it opens now."
"$LINUX" --api-url "http://127.0.0.1:$PORT" >>"$WORK/app.log" 2>&1 &
APP=$!
sleep 3
ask "Orca: Tab from the start goes rail → channel list → messages → composer → member list, and each stop is announced by name and role."
ask "Orca: the space rail announces each space's name and the selected one; the channel list announces names, unread and mention counts."
ask "Orca: the composer is announced as 'Message #general' with its text; Insert emoji, GIF, Attach files and the microphone are named buttons."
ask "Orca: message rows read their author, time and text (rows are known to have no accessible name before GTK 4.12; note what Orca says)."
ask "Orca: Settings, Audio test, the space menu and an owner dialog (Space settings) announce their title and focus their first control; Escape closes them."
ask "Orca: a context menu (right-click or Menu key on a message) reads each item, and arrow keys move through it."
kill "$APP" 2>/dev/null
log ""
log "### Voice (real server)"
echo
echo "Caper opens on caper.chat with your own account now (your normal profile)."
echo "Have a second device in the same voice channel."
unset XDG_CONFIG_HOME XDG_DATA_HOME XDG_CACHE_HOME
"$LINUX" >>"$WORK/app.log" 2>&1 &
APP=$!
sleep 3
ask "Audio test (account bar settings → Audio test): your microphone and speakers are listed, Test speakers plays, and Test microphone records and plays back both the natural and enhanced takes."
ask "Join a voice channel: the other device hears you clearly, and you hear it, with no echo, crackle or growing delay over two minutes."
ask "Mute and deafen: the other device sees and hears each change; the speaking ring follows whoever talks."
ask "Devices: switch the input and output device (Input Options, Output Options) during the call; audio moves without leaving the call. Unplug a headset; Caper falls back to the default."
ask "Network: turn Wi-Fi off for 10 s and back on (or switch networks) during the call; Caper reconnects to the call by itself."
ask "Voice messages: record one with the microphone button, play it back, and send it; the other device plays it."
kill "$APP" 2>/dev/null

say "Done. Report: $REPORT"
echo "Run this again in the other desktop (GNOME and KDE, Wayland sessions) and send both reports."
