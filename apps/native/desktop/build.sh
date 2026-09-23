#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -m)" != "x86_64" ]]; then
  echo "Caper-linux-x64 packages require an x86_64 Linux build host." >&2
  exit 2
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
native="$root/apps/native/desktop"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export CARGO_TARGET_DIR="$native/target"

cargo fmt --manifest-path "$native/Cargo.toml" -- --check
cargo test --manifest-path "$native/Cargo.toml" --locked
cargo clippy --manifest-path "$native/Cargo.toml" --locked --all-targets -- -D warnings
cargo build --manifest-path "$native/Cargo.toml" --locked --release

rm -rf "$native/dist" "$native/target/package"
mkdir -p "$native/dist" "$native/target/package/Caper-linux-x64"
install -m 0755 "$native/target/release/caper-desktop" "$native/target/package/Caper-linux-x64/caper-desktop"
install -m 0644 "$root/LICENSE" "$native/README.md" "$native/THIRD-PARTY-NOTICES.md" "$native/target/package/Caper-linux-x64/"
tar -C "$native/target/package" -czf "$native/dist/Caper-linux-x64.tar.gz" Caper-linux-x64

deb="$native/target/package/deb"
mkdir -p "$deb/DEBIAN" "$deb/usr/bin" "$deb/usr/share/applications" "$deb/usr/share/icons/hicolor/scalable/apps" "$deb/usr/share/doc/caper-desktop"
install -m 0755 "$native/target/release/caper-desktop" "$deb/usr/bin/caper-desktop"
install -m 0644 "$native/resources/caper.desktop" "$deb/usr/share/applications/caper.desktop"
install -m 0644 "$native/resources/caper.svg" "$deb/usr/share/icons/hicolor/scalable/apps/caper.svg"
install -m 0644 "$root/LICENSE" "$native/README.md" "$native/THIRD-PARTY-NOTICES.md" "$deb/usr/share/doc/caper-desktop/"
size="$(du -sk "$deb" | cut -f1)"
cat > "$deb/DEBIAN/control" <<EOF
Package: caper-desktop
Version: 0.1.0
Section: net
Priority: optional
Architecture: amd64
Installed-Size: $size
Maintainer: Caper <noreply@caper.chat>
Depends: libc6 (>= 2.39), libgcc-s1, libdbus-1-3, libwayland-client0, libx11-6, libxkbcommon0, libxkbcommon-x11-0, libvulkan1 | libgl1
Recommends: gnome-keyring
Description: Native Caper conversation client
 A browser-free eframe/egui client for Caper spaces and text channels.
EOF
dpkg-deb --root-owner-group --build "$deb" "$native/dist/Caper-linux-x64.deb"
printf 'Built:\n  %s\n  %s\n' "$native/dist/Caper-linux-x64.tar.gz" "$native/dist/Caper-linux-x64.deb"
