#!/usr/bin/env bash
# Installs this caper-linux tester build for the current user (no root):
# ~/.local/lib/caper-linux, the caper-linux command in ~/.local/bin and a
# "Caper (GTK preview)" launcher. PREFIX picks another prefix.
#   ./install.sh              install or update
#   ./install.sh --uninstall  remove it again
# It needs GTK 4.8 or later (libgtk-4-1) and PulseAudio or PipeWire's Pulse;
# link preview players also need WebKitGTK (libwebkit2gtk-4.1-0), else they
# open in the browser.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
prefix=${PREFIX:-$HOME/.local}
lib="$prefix/lib/caper-linux"
bin="$prefix/bin/caper-linux"
desktop="$prefix/share/applications/chat.caper.linux.desktop"
icon="$prefix/share/icons/hicolor/scalable/apps/chat.caper.linux.svg"

refresh() {
  command -v update-desktop-database >/dev/null && update-desktop-database -q "$prefix/share/applications" || true
  command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" || true
}

if [ "${1:-}" = --uninstall ]; then
  rm -rf "$lib"
  rm -f "$bin" "$desktop" "$icon"
  refresh
  echo "Removed caper-linux from $prefix."
  exit 0
fi

mkdir -p "$lib" "$(dirname "$bin")" "$(dirname "$desktop")" "$(dirname "$icon")"
# FFmpeg and ONNX Runtime stay beside the binary, where it looks for them.
install -m 0755 "$here/lib/caper-linux" "$here/lib/ffmpeg" "$here/lib/libonnxruntime.so.1.23.2" "$lib/"
# The link preview player helper, when the build has it.
if [ -f "$here/lib/caper-web-player" ]; then
  install -m 0755 "$here/lib/caper-web-player" "$lib/"
else
  rm -f "$lib/caper-web-player"
fi
ln -sf "$lib/caper-linux" "$bin"
# Desktop entries unescape strings before parsing the quoted command. Keep
# prefixes with spaces and reserved characters as one literal executable.
exec_path=${lib//\\/\\\\\\\\}/caper-linux
exec_path=${exec_path//\"/\\\\\"}
exec_path=${exec_path//\$/\\\\\$}
exec_path=${exec_path//\`/\\\\\`}
exec_path=${exec_path//%/%%}
while IFS= read -r line; do
  if [[ "$line" == Exec=* ]]; then
    # GIO checks the executable before expanding %%; env keeps a percent
    # in the install path in an argument, which is expanded before launch.
    printf 'Exec=env "%s"\n' "$exec_path"
  else
    printf '%s\n' "$line"
  fi
done <"$here/share/chat.caper.linux.desktop" >"$desktop"
install -m 0644 "$here/share/chat.caper.linux.svg" "$icon"
rm -rf "$lib/licenses"
cp -R "$here/licenses" "$lib/licenses"
[ -f "$here/REVISION" ] && install -m 0644 "$here/REVISION" "$lib/REVISION"
refresh
echo "Installed caper-linux $(cat "$here/REVISION" 2>/dev/null | cut -c1-8) in $lib."
echo "Run caper-linux, or open \"Caper (GTK preview)\" from your applications."
