# Caper Desktop for Windows and Linux

This directory packages the desktop clients. `build.ps1` ships the Win32 client
(`apps/native/windows`) as `Caper.exe`, and `build.sh` ships the GTK 4 client
(`apps/native/linux`) as `caper-desktop`, keeping the earlier client's install
layout so installs, saved sessions, launch at login, `caper://` links and
self-updates carry over. Its Cargo workspace holds both clients, the UI-free
`caper-client` core (`apps/native/client`), the link-preview player
(`web-player/`) and the vendored libwebrtc (`voice-spike/`). Package and
install notes below apply to both.

The clients do not embed Electron, Tauri or a browser engine, and their own UI
never uses a WebView or JavaScript runtime. The one exception is third-party
video players in link previews (YouTube, Vimeo, Twitch): after Play, the
operating system's own web engine (WebView2 on Windows, WebKitGTK on Linux)
shows just that player in the card.

The account bearer is sent only in the HTTP/WebSocket `Authorization` header.
The short-lived chat capability is held only in memory and sent only in
`x-caper-chat-token` for message writes. Neither appears in URLs or logs.
HTTP redirects are disabled.

## Build dependencies

The workspace pins Rust **1.99.1** in its independent `Cargo.lock`. Builds default to `CARGO_BUILD_JOBS=2` for 4 GB runners.

Ubuntu 24.04 build host:

```sh
# clang-21/lld-21 are not in the stock Noble repositories. Configure the
# signed apt.llvm.org llvm-toolchain-noble-21 repository first.
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libwayland-dev \
  libxkbcommon-dev libx11-dev libxi-dev libxcursor-dev libxrandr-dev \
  libdbus-1-dev dbus-x11 libglib2.0-dev libasound2-dev clang-21 lld-21 \
  libgtk-3-dev libgtk-4-dev libwebkit2gtk-4.1-dev
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy
./apps/native/desktop/build.sh
```

Windows Server 2025 / Windows 11 build host:

1. Install Visual Studio 2022 or newer Build Tools with **Desktop development with C++**
   and a Windows 10/11 SDK.
2. Install rustup and the stable `1.99.1-x86_64-pc-windows-msvc` toolchain.
3. Install NSIS 3.11 (`choco install nsis --version=3.11 -y`).
4. Open the x64 Visual Studio developer environment, then run
   `powershell -ExecutionPolicy Bypass -File apps/native/desktop/build.ps1`.
   Packaging uses its `VCToolsRedistDir` to locate the signed app-local runtime.

Both scripts check formatting, run the tests and Clippy, then build and package.
`CAPER_DESKTOP_STAGE=checks` or `CAPER_DESKTOP_STAGE=package` runs only that
half; a release runs the two as parallel jobs and needs both to pass.

Outputs are unsigned:

- `dist/Caper-linux-x64.tar.gz`
- `dist/Caper-linux-x64.deb`
- `dist/Caper-windows-x64-Setup.exe` (normal Windows download)
- `dist/Caper-windows-x64.zip` (self-update payload / optional portable archive)

Windows Setup installs to `%LOCALAPPDATA%\Programs\Caper\app`, creates Start
and desktop shortcuts, registers an uninstaller in Installed apps, then opens
Caper (silent `/S` installs do not launch it). It needs no administrator access.
The updater replaces only `app`. `test-installer.ps1` checks the GUI subsystem,
install/reinstall, file hashes, shortcuts and removal; run locally, it refuses
to touch an existing install.

The `.deb` targets Ubuntu 24.04 (glibc 2.39) and depends on GTK 4.8 or later,
D-Bus, PulseAudio/ALSA and WebKitGTK (`libwebkit2gtk-4.1-0`). Session
persistence needs an unlocked Secret Service provider such as GNOME Keyring.
Only the `caper-web-player` helper (beside the app in the archive,
`/usr/lib/caper-desktop/` in the `.deb`) links GTK 3 and WebKitGTK; `build.sh`
fails if `caper-desktop` does.

Linux packages carry the native ONNX Runtime and its licenses. Windows also
stages the four Microsoft-signed app-local VC++ DLLs that ONNX Runtime imports,
taken from the active Visual Studio toolchain (not hash-pinned).

Run the unpacked binary, or `sudo apt install ./Caper-linux-x64.deb`. The
production endpoint is `https://caper.chat`; development may use
`--api-url http://localhost:PORT` or `CAPER_API_URL`. Plain HTTP is rejected
for non-loopback hosts.

## Audio processing

Live input and local comparison run gain → bundled DPDFNet-8 HR (or RNNoise) →
voice contour. Natural replay is post-gain/post-denoise; enhanced replay adds
the contour. The model and native ONNX Runtime 1.23.2 match the web assets. The
contour approximates the browser filters/compressor, not bit-for-bit Web Audio.
Playback gain uses WebRTC's software track volume, not system volume. A missing
saved device fails explicitly rather than opening another microphone.

## Windows microphone access and firewall prompts

Windows capture uses WebRTC's WASAPI **shared mode**, so another app's call
does not normally block the microphone. Caper cannot bypass Windows microphone
privacy settings or another application's exclusive access. **System default**
resolves the normal Windows default device, not the communications default.

Voice and local audio tests open WebRTC sockets and can trigger a Windows
Firewall prompt; Caper does not change firewall rules. The unsigned executable
can show **Publisher: Unknown**. See the
[Windows audio and firewall runbook](../../../docs/media.md#windows-microphone-access-and-firewall-prompts).

Not supported: camera, screen sharing, notifications after quit, and
Windows/Linux code signing.

## Checks

The UI-free code (API, gateway, model, uploads, voice media, notifications
and the background worker) is the `caper-client` crate in `apps/native/client`.
`build.sh` and `build.ps1` run the full checks for their client; on Linux:

```sh
cargo fmt --manifest-path apps/native/desktop/Cargo.toml -p caper-linux -p caper-client -- --check
CC=clang-21 CXX=clang++-21 LK_CUSTOM_WEBRTC="$(python3 apps/native/desktop/voice-spike/fetch_libwebrtc.py --platform linux)" CARGO_BUILD_JOBS=2 cargo test --manifest-path apps/native/desktop/Cargo.toml -p caper-linux -p caper-client --locked
CC=clang-21 CXX=clang++-21 LK_CUSTOM_WEBRTC="$PWD/apps/native/desktop/target/libwebrtc/linux-x64-release" CARGO_BUILD_JOBS=2 cargo clippy --manifest-path apps/native/desktop/Cargo.toml -p caper-linux -p caper-client --locked --all-targets --no-deps -- -D warnings
cargo fmt --manifest-path apps/native/desktop/Cargo.toml -p caper-web-player -- --check
cargo test --manifest-path apps/native/desktop/Cargo.toml -p caper-web-player --features engine --locked
cargo clippy --manifest-path apps/native/desktop/Cargo.toml -p caper-web-player --features engine --locked --all-targets --no-deps -- -D warnings
```

## Link preview players

Debug builds started with `CAPER_DEV_PLAY_ORIGIN=http://127.0.0.1:PORT`
(loopback HTTP only) load `play` pages from a stand-in server; release builds
ignore it. `CAPER_WEB_PLAYER` names a helper outside the usual places.

## Icons

After `npm ci`, `node scripts/native-icons.mjs --check` verifies the bundled
vectors match the web client's pinned Lucide package; omit `--check` to
regenerate.

Static desktop icons use `apps/web/public/caper-face.svg`. With ImageMagick 7
and librsvg installed, `node scripts/generate-favicons.mjs` regenerates the web
fallback PNGs, Linux SVG, multi-size Windows ICO, macOS and iOS AppIcon PNGs,
and the Android launcher PNG
(`--check` detects drift without writing). The executable, installer and
uninstaller use that ICO. Linux packages the SVG as
`hicolor/scalable/apps/caper.svg`; `caper.desktop` names it with `Icon=caper`
and matches the GTK application ID with `StartupWMClass=chat.caper.linux`.
