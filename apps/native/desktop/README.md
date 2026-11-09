# Caper Desktop for Windows and Linux

Linux release packages (`build.sh`) carry the GTK 4 client
(`apps/native/linux`) in this client's package layout; egui ships on Windows
and still builds and runs on Linux for development. Package and install notes
below apply to both.

Browser-free native Caper client built with Rust, eframe/egui, wgpu and winit.
It does not embed Electron, Tauri or a browser engine, and its own UI never
uses a WebView or JavaScript runtime. The one exception is third-party video
players in link previews (YouTube, Vimeo, Twitch): after Play, the operating
system's own web engine (WebView2 on Windows, WebKitGTK on Linux) shows
just that player in the card.

## Features

- Email sign-in and profile onboarding; no Guest/General fallback.
- Bearer sessions in Windows Credential Manager or the Linux Secret Service,
  isolated by canonical API origin. If the vault is unavailable the session
  stays in memory only and the UI warns; there is no plaintext fallback.
  Logout revokes the server session and removes the OS credential.
- Spaces, channels (including private), owner space/channel/member management,
  confirmed leave-space, paginated presence, typing, history pagination,
  last-channel restoration, and hover prefetch with access rechecks.
- Idempotent HTTP sends: a timeout keeps the same client message UUID for
  retry; a matching gateway delivery confirms a pending write. The unified
  `/api/chat/events` gateway provides ordered replay, deduplication, gap
  resync, heartbeats and reconnect status.
- Resizable channel (default 280px) and Members (220–440px) sidebars: drag,
  Left/Right/Home/End, double-click to reset; widths save locally.
- Ctrl/Cmd + Plus, Minus or 0 changes or resets interface zoom.
- Invite links: owners create (expiry, use limit, optional landing channel),
  copy and revoke them in Manage space; anyone can join from **Join with invite
  link**. The `caper://` handler brings the existing window forward. Windows
  open maximized.
- Direct messages, message requests (Accept/Decline/Block), blocking with
  collapsed "N blocked messages" runs, and "Who can start a DM with you".
- `:` emoji and `@` people suggestions in channel, DM and thread composers;
  mention pills open a profile card with an inline DM composer.
- Reactions (hover cards name who reacted), pins, forwarding, threads.
- `@feedback` and other reserved team names; `feedbackTeam` accounts get a
  **Feedback** inbox (archive/restore, visits with `visitor: true`).
- Notifications: per-space/channel/DM levels and mutes, and local OS
  notifications from the account gateway while the app runs (including
  minimized). Quitting exits; there is no tray mode.
- Custom profile pictures and space icons (GIFs up to 5 MB/1024 px unchanged,
  larger ones shrunk; others become 512 px WebP; HEIC/AVIF refused).
- Attachments when the server has storage configured: up to 10 files per
  message from the picker, drag and drop, or Ctrl+V. The picker is Win32 on
  Windows and the XDG desktop portal on Linux (needs a file-chooser backend
  such as xdg-desktop-portal-gtk, -gnome or -kde). Clipboard paste of files and
  images works on Windows, X11/XWayland, and Wayland compositors with the
  data-control protocol.
- On-device compression with the server's `GET /api/assets/usage`
  `compression` settings (rules in `docs/media.md`): lossless stills as
  indexed PNG or lossless WebP (libwebp), photos as AVIF (rav1e) or WebP,
  videos as H.264/AAC MP4 through the bundled FFmpeg (Windows tries Media
  Foundation's hardware encoder first, then x264). Originals have metadata
  stripped losslessly. Without FFmpeg, videos upload as originals.
- Attachment editor (`attachment_editor.rs`, `attachment_edit.rs`): spoiler,
  crop, rotate, draw and text with undo/redo. Draw and Text are unavailable for
  videos because the bundled FFmpeg lacks the `overlay` filter. HEIC/AVIF are
  not editable.
- Media viewer (`viewer.rs`) and video player (`player.rs`: FFmpeg decode,
  rodio audio, signed URLs streamed through the loopback range proxy
  `media_proxy.rs`). AVIF/HEIC decode through the bundled FFmpeg. Signed URLs
  refresh through `POST /api/assets/urls` before expiry and after a 403/404.
- Link preview cards (`embeds.rs`). YouTube/Vimeo/Twitch Play loads Caper's
  `play` page (HTTPS from `embed.caper.chat` or `embed.staging.caper.chat`
  only) in a native web view over the card (`web_player.rs`): WebView2 in
  process on Windows; on Linux the separate `caper-web-player` helper
  (`web-player/`) runs WebKitGTK so `caper-desktop` never links GTK. On X11 the
  view is a child window; on Wayland it opens a small "Caper — YouTube" window.
  The view has ephemeral storage, no app bridge, and opens other links in the
  system browser. Without the engine the card names what to install
  (`libwebkit2gtk-4.1-0` or the WebView2 runtime). YouTube on Linux also needs
  `gstreamer1.0-libav` and `gstreamer1.0-plugins-bad` (recommended by the
  `.deb`).
- Voice messages (`voice_note.rs`): hands-free recording up to 10:00 through
  a private libwebrtc device module and the call's microphone processing,
  encoded as AAC/MP4 with the bundled FFmpeg (16 kHz WAV without it), staged
  as a chip and sent with the next message, or sent at once with **Settings →
  Voice messages → Send voice messages as soon as I stop recording**. Players
  show waveforms and transcripts.
- Updates (release builds only): checks 20 seconds after launch and every
  minute, **Settings… → Updates → Check for updates**, an **Update available**
  button and modal with release notes, and restart-to-update (the `.deb` offers
  a download instead). **Settings… → Updates → What's new** shows release
  highlights for the running source revision, which builds take from
  `CAPER_RELEASE_SHA` (40 lowercase hex characters) or the current Git HEAD.
- Experimental native voice (raw Google libwebrtc, see
  [voice-spike](voice-spike/README.md)): Cloudflare SFU publish/subscribe,
  roster, lease snapshots, TURN refresh and replay-safe ICE restart. Browsing
  keeps the call running. Voice in the channel list follows the shared layout
  in [apps/native](../README.md#layout-rules).
- Audio: device selection and saved preferences, 0–200% input gain and
  per-person playback gain, local mute, speaking rings, voice-contour strength,
  DPDFNet-8 HR denoising with RNNoise fallback, an Audio test (speaker test and
  up to 30 seconds of local natural/enhanced microphone comparison, kept in
  memory only), connection statistics, and the web's bundled interaction sounds.

The account bearer is sent only in the HTTP/WebSocket `Authorization` header.
The short-lived chat capability is held only in memory and sent only in
`x-caper-chat-token` for message writes. Neither appears in URLs or logs.
HTTP redirects are disabled.

## Build dependencies

The workspace pins Rust **1.99.1** and eframe **0.37.0** in its independent
`Cargo.lock`. Builds default to `CARGO_BUILD_JOBS=2` for 4 GB runners.

Ubuntu 24.04 build host:

```sh
# clang-21/lld-21 are not in the stock Noble repositories. Configure the
# signed apt.llvm.org llvm-toolchain-noble-21 repository first.
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libwayland-dev \
  libxkbcommon-dev libx11-dev libxi-dev libxcursor-dev libxrandr-dev \
  libdbus-1-dev dbus-x11 libglib2.0-dev libasound2-dev clang-21 lld-21 \
  libgtk-3-dev libwebkit2gtk-4.1-dev
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

The `.deb` targets Ubuntu 24.04 (glibc 2.39) and depends on X11/Wayland, D-Bus,
PulseAudio/ALSA, Vulkan-or-GL and WebKitGTK (`libwebkit2gtk-4.1-0`). Session
persistence needs an unlocked Secret Service provider such as GNOME Keyring.
Only the `caper-web-player` helper (beside the app in the archive,
`/usr/lib/caper-desktop/` in the `.deb`) links GTK and WebKitGTK; `build.sh`
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
and the background worker) is the `caper-client` crate in `apps/native/client`,
a member of this workspace.

```sh
cargo fmt --manifest-path apps/native/desktop/Cargo.toml -p caper-desktop -p caper-client -- --check
CC=clang-21 CXX=clang++-21 LK_CUSTOM_WEBRTC="$(python3 apps/native/desktop/voice-spike/fetch_libwebrtc.py --platform linux)" CARGO_BUILD_JOBS=2 cargo test --manifest-path apps/native/desktop/Cargo.toml -p caper-desktop -p caper-client --locked
CC=clang-21 CXX=clang++-21 LK_CUSTOM_WEBRTC="$PWD/apps/native/desktop/target/libwebrtc/linux-x64-release" CARGO_BUILD_JOBS=2 cargo clippy --manifest-path apps/native/desktop/Cargo.toml -p caper-desktop -p caper-client --locked --all-targets --no-deps -- -D warnings
cargo fmt --manifest-path apps/native/desktop/Cargo.toml -p caper-web-player -- --check
cargo test --manifest-path apps/native/desktop/Cargo.toml -p caper-web-player --features engine --locked
cargo clippy --manifest-path apps/native/desktop/Cargo.toml -p caper-web-player --features engine --locked --all-targets --no-deps -- -D warnings
```

## Fixture previews

`caper-desktop --fixture <name>` opens a labelled static preview that makes no
network requests, starts no media transport or updater, and never uploads.
Use `--api-url` with the loopback parity fixture for networked chat. Fixtures:

- Sign-in and chat: `login`, `parity-channel`, `parity-new-messages`,
  `parity-mentions`, `parity-reactions`, `parity-opening`,
  `parity-opening-narrow`, `parity-opening-error`, `parity-loading`.
- Voice: `parity-voice-joining`, `parity-voice-connected`,
  `parity-voice-speaking`, `parity-voice-elsewhere`, `parity-voice-crowded`
  (and `-narrow`), `parity-voice-list` (and `-narrow`),
  `parity-voice-strip-narrow`, `parity-voice-join-narrow`,
  `parity-voice-error`, `parity-audio-error`.
- Direct messages: `parity-direct`, `parity-direct-new`, `parity-direct-emoji`,
  `parity-direct-no-spaces`, `parity-requests`, `parity-requests-outgoing`,
  `parity-blocked`, `parity-blocked-dm`, `parity-muted`.
- Files and media: `parity-attachments`, `parity-pictures` (and `-profile`,
  `-space`), `parity-embeds`, `parity-attachment-editor` (and `-crop`, `-text`,
  `-narrow`, `-chip`, `-preview`), `parity-voice-messages`,
  `parity-voice-recording`, `parity-voice-profile` (and `-recording`),
  `parity-voice-staged`.
- Threads: `parity-thread`, `parity-thread-loading`,
  `parity-thread-attachments`, `parity-thread-attachments-ready`,
  `parity-thread-attachments-rejected` (append `-narrow` for 390px).
- Feedback: `parity-feedback`, `parity-feedback-zero`.
- Updates: `parity-settings`, `parity-settings-checking`,
  `parity-settings-current`, `parity-settings-deferred`,
  `parity-settings-offline`, `parity-update`, `parity-update-narrow`,
  `parity-update-download`, `parity-update-error`, `parity-update-incomplete`,
  `parity-whats-new`, `parity-whats-new-narrow`, `parity-whats-new-loading`,
  `parity-whats-new-empty`, `parity-whats-new-error`.

Debug builds started with `CAPER_DEV_PLAY_ORIGIN=http://127.0.0.1:PORT`
(loopback HTTP only) load `play` pages from a stand-in server; release builds
ignore it. `CAPER_WEB_PLAYER` names a helper outside the usual places.

## Icons

After `npm ci`, `node scripts/native-icons.mjs --check` verifies the bundled
vectors match the web client's pinned Lucide package; omit `--check` to
regenerate.

Static desktop icons use `apps/web/public/caper-face.svg`. With ImageMagick 7
and librsvg installed, `node scripts/generate-favicons.mjs` regenerates the web
fallback PNGs, Linux SVG, desktop fallback PNG, multi-size Windows ICO, and
macOS AppIcon PNGs (`--check` detects drift without writing). The executable,
installer and uninstaller use that ICO. Linux packages the SVG as
`hicolor/scalable/apps/caper.svg` and uses `app_id=caper` to match
`caper.desktop` and `Icon=caper`.
