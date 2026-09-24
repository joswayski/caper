#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -m)" != "x86_64" ]]; then
  echo "Caper-linux-x64 packages require an x86_64 Linux build host." >&2
  exit 2
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
native="$root/apps/native/desktop"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$native/target}"
export CC="${CC:-clang-21}"
export CXX="${CXX:-clang++-21}"
export LK_CUSTOM_WEBRTC="${LK_CUSTOM_WEBRTC:-$(python3 "$native/voice-spike/fetch_libwebrtc.py" --platform linux)}"

# Official ONNX Runtime 1.23.2, pinned independently of the Rust ort wrapper.
# Never execute or package an unchecked download. Keep it out of git.
ort_archive="$CARGO_TARGET_DIR/onnxruntime-linux-x64-1.23.2.tgz"
ort_dir="$CARGO_TARGET_DIR/onnxruntime-linux-x64-1.23.2"
mkdir -p "$CARGO_TARGET_DIR"
if [[ ! -f "$ort_archive" ]]; then
  curl -fL --retry 2 -o "$ort_archive" 'https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-linux-x64-1.23.2.tgz'
fi
echo '1fa4dcaef22f6f7d5cd81b28c2800414350c10116f5fdd46a2160082551c5f9b  '"$ort_archive" | sha256sum --check --status
mkdir -p "$ort_dir"
tar -xzf "$ort_archive" -C "$ort_dir" --strip-components=1
echo '13ab8084954fa4a47c777880180b90810d6020f021441395712b48a75b74c68b  '"$ort_dir/lib/libonnxruntime.so.1.23.2" | sha256sum --check --status
export CAPER_ONNXRUNTIME_LIBRARY="$ort_dir/lib/libonnxruntime.so.1.23.2"
echo '7b3afbb260a08fe9af3d16e3bda992971be1e7e951d1dee7c2d235f5c43f5631  '"$root/apps/web/public/audio/dpdfnet8-v2/dpdfnet8_48khz_hr.onnx" | sha256sum --check --status

python3 "$root/scripts/native_fonts.py"
cargo fmt --manifest-path "$native/Cargo.toml" --package caper-desktop -- --check
cargo test --manifest-path "$native/Cargo.toml" --locked --package caper-desktop
cargo test --manifest-path "$native/Cargo.toml" --locked --package caper-desktop native_inference_is_finite_and_owns_fresh_state -- --ignored
cargo clippy --manifest-path "$native/Cargo.toml" --locked --package caper-desktop --all-targets --no-deps -- -D warnings
cargo build --manifest-path "$native/Cargo.toml" --locked --release --package caper-desktop

package_dir="$native/target/package"
rm -rf "$native/dist" "$package_dir"
mkdir -p "$native/dist" "$package_dir/Caper-linux-x64"
install -m 0755 "$CARGO_TARGET_DIR/release/caper-desktop" "$package_dir/Caper-linux-x64/caper-desktop"
install -m 0644 "$root/LICENSE" "$native/README.md" "$native/THIRD-PARTY-NOTICES.md" "$package_dir/Caper-linux-x64/"
install -m 0644 "$root/shared/fonts/cache/Satoshi-FFL.txt" "$package_dir/Caper-linux-x64/"
install -m 0644 "$native/voice-spike/licenses/"* "$package_dir/Caper-linux-x64/"
install -m 0755 "$CAPER_ONNXRUNTIME_LIBRARY" "$package_dir/Caper-linux-x64/"
install -m 0644 "$ort_dir/LICENSE" "$package_dir/Caper-linux-x64/ONNX-RUNTIME-LICENSE"
install -m 0644 "$ort_dir/ThirdPartyNotices.txt" "$package_dir/Caper-linux-x64/ONNX-RUNTIME-THIRD-PARTY-NOTICES.txt"
install -m 0644 "$root/apps/web/public/audio/dpdfnet8-v2/LICENSE-APACHE-2.0" "$package_dir/Caper-linux-x64/DPDFNET-LICENSE"
tar -C "$package_dir" -czf "$native/dist/Caper-linux-x64.tar.gz" Caper-linux-x64

deb="$package_dir/deb"
mkdir -p "$deb/DEBIAN" "$deb/usr/bin" "$deb/usr/lib/caper-desktop" "$deb/usr/share/applications" "$deb/usr/share/icons/hicolor/scalable/apps" "$deb/usr/share/doc/caper-desktop"
install -m 0755 "$CARGO_TARGET_DIR/release/caper-desktop" "$deb/usr/bin/caper-desktop"
install -m 0755 "$CAPER_ONNXRUNTIME_LIBRARY" "$deb/usr/lib/caper-desktop/"
install -m 0644 "$ort_dir/LICENSE" "$deb/usr/share/doc/caper-desktop/ONNX-RUNTIME-LICENSE"
install -m 0644 "$ort_dir/ThirdPartyNotices.txt" "$deb/usr/share/doc/caper-desktop/ONNX-RUNTIME-THIRD-PARTY-NOTICES.txt"
install -m 0644 "$root/apps/web/public/audio/dpdfnet8-v2/LICENSE-APACHE-2.0" "$deb/usr/share/doc/caper-desktop/DPDFNET-LICENSE"
install -m 0644 "$native/resources/caper.desktop" "$deb/usr/share/applications/caper.desktop"
install -m 0644 "$native/resources/caper.svg" "$deb/usr/share/icons/hicolor/scalable/apps/caper.svg"
install -m 0644 "$root/LICENSE" "$native/README.md" "$native/THIRD-PARTY-NOTICES.md" "$deb/usr/share/doc/caper-desktop/"
install -m 0644 "$root/shared/fonts/cache/Satoshi-FFL.txt" "$deb/usr/share/doc/caper-desktop/"
install -m 0644 "$native/voice-spike/licenses/"* "$deb/usr/share/doc/caper-desktop/"
size="$(du -sk "$deb" | cut -f1)"
cat > "$deb/DEBIAN/control" <<EOF
Package: caper-desktop
Version: 0.1.0
Section: net
Priority: optional
Architecture: amd64
Installed-Size: $size
Maintainer: Caper <noreply@caper.chat>
Depends: libc6 (>= 2.39), libgcc-s1, libstdc++6, libdbus-1-3, libwayland-client0, libx11-6, libxkbcommon0, libxkbcommon-x11-0, libpulse0, libasound2, libvulkan1 | libgl1
Recommends: gnome-keyring
Description: Native Caper conversation client
 A browser-free client for Caper spaces, text, and experimental voice.
EOF
dpkg-deb --root-owner-group --build "$deb" "$native/dist/Caper-linux-x64.deb"
printf 'Built:\n  %s\n  %s\n' "$native/dist/Caper-linux-x64.tar.gz" "$native/dist/Caper-linux-x64.deb"
