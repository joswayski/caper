# Caper Desktop for Windows and Linux

Browser-free native Caper client built with Rust, eframe/egui, wgpu and winit.
It does not embed Electron, Tauri, a WebView, or a JavaScript runtime.

## Available now

- Passwordless email request/verification and first-account profile onboarding.
- Bearer sessions stored in Windows Credential Manager or the Linux Secret
  Service. If the vault is unavailable, the session remains in memory only and
  the UI warns that sign-in will not survive restart. There is no plaintext
  credential fallback. Vault entries are isolated by canonical API origin, so a
  production credential is never restored for a staging/custom HTTPS origin.
- Logout revokes the server session and removes the OS credential.
- Account space/channel navigation, including accessible private channels.
- HTTP message history and idempotent writes. A timeout or lost response retains
  the same client message UUID and original text for retry; a definitive
  validation rejection unlocks editing and the next send gets a new UUID. A
  matching gateway delivery confirms the pending write even if HTTP finishes
  late or fails.
- Unified `/api/chat/events` gateway subscriptions with ordered replay,
  deduplication, gap resync, heartbeat handling, reconnect status, and stale
  result isolation across logout/channel changes.

The account bearer is sent only in the HTTP/WebSocket `Authorization` header.
The short-lived chat capability is separate, held only in memory, and sent only
in `x-caper-chat-token` for message writes. Neither appears in URLs or logs.
HTTP redirects are disabled, so credentials cannot be forwarded to another
origin.

## Build dependencies

The workspace pins Rust **1.94.0** and eframe **0.33.3** in its independent
`Cargo.lock`. Builds default to `CARGO_BUILD_JOBS=2` for 4 GB runners.

Ubuntu 24.04 CI/build host:

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libwayland-dev \
  libxkbcommon-dev libx11-dev libxi-dev libxcursor-dev libxrandr-dev \
  libdbus-1-dev dbus-x11
rustup toolchain install 1.94.0 --profile minimal --component rustfmt --component clippy
./apps/native/desktop/build.sh
```

Windows Server 2025 / Windows 11 build host:

1. Install Visual Studio 2022 Build Tools with **Desktop development with C++**
   and a Windows 10/11 SDK.
2. Install rustup and the stable `1.94.0-x86_64-pc-windows-msvc` toolchain.
3. Run `powershell -ExecutionPolicy Bypass -File apps/native/desktop/build.ps1`.

Outputs are unsigned:

- `dist/Caper-linux-x64.tar.gz`
- `dist/Caper-linux-x64.deb`
- `dist/Caper-windows-x64.zip`

The `.deb` is built against Ubuntu 24.04 (glibc 2.39) and declares native
X11/Wayland, D-Bus and Vulkan-or-GL runtime dependencies. It is not a static or
distribution-independent Linux binary. Linux session persistence also requires
an unlocked Secret Service provider such as GNOME Keyring.

Run the unpacked binary directly, or install the Debian package with
`sudo apt install ./Caper-linux-x64.deb`. The production endpoint is
`https://caper.chat`; development may use `--api-url http://localhost:PORT` or
`CAPER_API_URL`. Plain HTTP is rejected for non-loopback hosts.

## Calling and known parity gaps

**Voice calling is unavailable in this client and no inert call control is
shown.** Caper's existing media path depends on native microphone capture,
playback, WebRTC transceivers, ICE/TURN renewal, and Cloudflare Realtime SFU SDP
negotiation. Rust WebRTC libraries exist, but this workstream could not safely
verify interoperable capture/playback and the existing SFU state machine on
physical Windows/Linux devices. Shipping a partial signaling-only button would
risk leaking tracks or presenting a false connected state.

Other current gaps versus the browser are older-history pagination, typing and
member presence, space/channel/member administration, voice, native
notifications, accessibility/IME acceptance, installers, signing and updates.
The `.deb` and archives are portable release artifacts, not signed installers.

## Validation

```sh
cargo fmt --manifest-path apps/native/desktop/Cargo.toml -- --check
CARGO_BUILD_JOBS=2 cargo test --manifest-path apps/native/desktop/Cargo.toml --locked
CARGO_BUILD_JOBS=2 cargo clippy --manifest-path apps/native/desktop/Cargo.toml --locked --all-targets -- -D warnings
```

For deterministic visual inspection without live accounts, launch
`caper-desktop --fixture signed-out` or `--fixture error`. These are explicitly
labeled test states and contain no fake conversation data.
