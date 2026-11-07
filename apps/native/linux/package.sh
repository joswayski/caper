#!/usr/bin/env bash
# Packages a built caper-linux as a tester tarball: the binary with Caper's
# trimmed FFmpeg and ONNX Runtime beside it (where it looks for them), a
# .desktop file, the icon and install.sh. Not a release: nothing ships it.
#   apps/native/linux/package.sh <caper-linux binary> <output directory>
# writes <output directory>/caper-linux-x64.tar.gz.
set -euo pipefail
binary=$(realpath "$1")
out=$(mkdir -p "$2" && realpath "$2")
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# The same pinned, checksummed downloads as apps/native/desktop/build.sh.
ort_archive="$work/onnxruntime.tgz"
curl -fL --retry 6 --retry-delay 5 --retry-all-errors -o "$ort_archive" \
  'https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-linux-x64-1.23.2.tgz'
echo "1fa4dcaef22f6f7d5cd81b28c2800414350c10116f5fdd46a2160082551c5f9b  $ort_archive" | sha256sum --check --status
mkdir "$work/ort"
tar -xzf "$ort_archive" -C "$work/ort" --strip-components=1
ort="$work/ort/lib/libonnxruntime.so.1.23.2"
echo "13ab8084954fa4a47c777880180b90810d6020f021441395712b48a75b74c68b  $ort" | sha256sum --check --status
ffmpeg_archive="$work/ffmpeg.zip"
curl -fL --retry 6 --retry-delay 5 --retry-all-errors -o "$ffmpeg_archive" \
  'https://github.com/joswayski/ffmpeg-desktop/releases/download/ffmpeg-9.0.2-r2/ffmpeg-desktop-linux-x64.zip'
echo "adeede9c943a94f7d32b682057c7e59a6bce2ab8da8335c4d70288faada3b7ab  $ffmpeg_archive" | sha256sum --check --status
python3 -m zipfile -e "$ffmpeg_archive" "$work/ffmpeg"

stage="$work/caper-linux-x64"
mkdir -p "$stage/lib" "$stage/share"
install -m 0755 "$binary" "$stage/lib/caper-linux"
install -m 0755 "$work/ffmpeg/ffmpeg" "$stage/lib/ffmpeg"
install -m 0755 "$ort" "$stage/lib/libonnxruntime.so.1.23.2"
install -m 0644 "$root/apps/native/desktop/resources/caper.svg" "$stage/share/chat.caper.linux.svg"
install -m 0755 "$here/install.sh" "$stage/install.sh"
# Named for the app id, so the desktop matches its windows to it.
cat >"$stage/share/chat.caper.linux.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=Caper (GTK preview)
Comment=Caper's GTK 4 client, in development
Exec=caper-linux
Icon=chat.caper.linux
Terminal=false
Categories=Network;Chat;
StartupNotify=true
StartupWMClass=chat.caper.linux
EOF
git -C "$root" rev-parse HEAD >"$stage/REVISION" 2>/dev/null || true
tar -C "$work" -czf "$out/caper-linux-x64.tar.gz" caper-linux-x64
echo "$out/caper-linux-x64.tar.gz"
