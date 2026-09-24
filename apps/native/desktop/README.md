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
- Guest General, account spaces, private-channel grants, pagination, typing,
  paginated member presence, and owner space/channel/member management.
- Experimental native voice: raw Google libwebrtc with platform audio devices,
  Caper SFU offer/answer publication and subscription, voice roster, lease
  snapshots, TURN refresh and replay-safe ICE restart/ACK. Browsing leaves the
  active call running; explicit replacement, logout and access denial silence
  locally. Mute/deafen and input/output selection use native ADM.

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
# clang-21/lld-21 are not in the stock Noble repositories. Configure the
# signed apt.llvm.org llvm-toolchain-noble-21 repository first.
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libwayland-dev \
  libxkbcommon-dev libx11-dev libxi-dev libxcursor-dev libxrandr-dev \
  libdbus-1-dev dbus-x11 libglib2.0-dev clang-21 lld-21
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
X11/Wayland, D-Bus, PulseAudio/ALSA and Vulkan-or-GL runtime dependencies. It is not a static or
distribution-independent Linux binary. Linux session persistence also requires
an unlocked Secret Service provider such as GNOME Keyring.

Run the unpacked binary directly, or install the Debian package with
`sudo apt install ./Caper-linux-x64.deb`. The production endpoint is
`https://caper.chat`; development may use `--api-url http://localhost:PORT` or
`CAPER_API_URL`. Plain HTTP is rejected for non-loopback hosts.

## Calling and known parity gaps

Voice is **experimental**, not live/physical-device accepted. Source and local
tests exercise signaling, cancellation and native ICE gathering; they do not
prove two-client SFU/TURN or actual mic/speaker quality. The pinned binding
does not provide safe ADM input/output gain, per-remote gain, or a local
natural/enhanced mic-monitor path, so these browser controls are not shown.
Mic test, processing strength and full connection diagnostics are also absent.
Device selection is available only after an active session enumerates hardware.
No camera, screen sharing, native notifications, installers, signing or updates.
IME/accessibility and sustained multi-network voice need separate acceptance.
The `.deb` and archives are unsigned release artifacts, not installers.

An earlier opt-in, ignored live smoke used two locally muted/deafened NativeSessions
with a private PulseAudio null sink/monitor. On 2026-09-24 both published and
reached connected transports, but the combined snapshot/reconcile step returned HTTP 502 on
one run and later `unauthorized` on another; two-way subscription/RTP was not
verified. This is not evidence of physical capture or listening. Do not run the
public smoke without isolated virtual devices and explicit authorization.

A separate **local-only** connected-peer test found that ADM disabled plus a
disabled device track emitted 0 RTP bytes; enabling ADM without the track also
emitted 0, while a virtual null-source input and synthetic zero-PCM track each
emitted RTP. Production starts with synthetic silence, sends the pending local
SDP immediately after setting the offer (like the web client), and only opens
the selected device after the current gateway and roster are ready. Cancel
closes the local peer and disables ADM before remote leave finishes.

The owned-session-only live smoke was rerun on 2026-09-24 at 04:39:57–04:40:12
UTC with private virtual devices. Both peers connected and subscribed, but
the ten-second receive check failed: A sent/received **451/0 bytes**, B
**1396/0 bytes**. No unrelated roster was observed. This is a real unresolved
receive-path failure, not working voice. The failed process closed local peers;
remote leave completion was not independently confirmed (server leases expire).
The ignored test now awaits remote leave before asserting its RTP result and
reports safe direction/track counts. It has not been rerun with that diagnostic.

A non-ignored, local-only native test receives actual RTP after adding a
subscription to an existing transport and after ICE renewal. It caught a separate
binding-default bug that changed receive transceivers to inactive during renewal;
restart offers now retain existing receive directions without adding an unused
receiver to a publish-only session. This does **not** diagnose or resolve the
immediate live receive failure above. Further public tests require explicit
authorization and the same isolation controls.

## Validation

```sh
cargo fmt --manifest-path apps/native/desktop/Cargo.toml -p caper-desktop -- --check
CC=clang-21 CXX=clang++-21 LK_CUSTOM_WEBRTC="$(python3 apps/native/desktop/voice-spike/fetch_libwebrtc.py --platform linux)" CARGO_BUILD_JOBS=2 cargo test --manifest-path apps/native/desktop/Cargo.toml -p caper-desktop --locked
CC=clang-21 CXX=clang++-21 LK_CUSTOM_WEBRTC="$PWD/apps/native/desktop/target/libwebrtc/linux-x64-release" CARGO_BUILD_JOBS=2 cargo clippy --manifest-path apps/native/desktop/Cargo.toml -p caper-desktop --locked --all-targets --no-deps -- -D warnings
```

For deterministic visual inspection without live accounts, launch
`caper-desktop --fixture login` or `--fixture parity-channel`. These are
explicitly labeled static previews; the chat fixture at loopback port 3001
does not provide live SFU media. Use normal `--api-url` for networked chat tests.
`parity-voice-joining` and `parity-voice-connected` preview Cancel/Leave and the
audio bar without starting a media transport.

After `npm ci`, run `node scripts/native-icons.mjs --check` to verify the bundled
vectors match the web client's pinned Lucide package. Omit `--check` to regenerate.
