# Caper Desktop for Windows and Linux

Browser-free native Caper client built with Rust, eframe/egui, wgpu and winit.
It does not embed Electron, Tauri, a WebView, or a JavaScript runtime.

## Available now

- Signed-out first launch opens passwordless email request/verification and
  first-account profile onboarding, with no Guest/General fallback.
- Packaged releases check for signed updates 20 seconds after launch and every
  minute. **User Settings → Settings… → Updates → Check for updates** checks
  immediately with progress and success/failure feedback. Writable installs
  offer a restart to update; the `.deb` offers a download for manual installation.
- Bearer sessions stored in Windows Credential Manager or the Linux Secret
  Service. If the vault is unavailable, the session remains in memory only and
  the UI warns that sign-in will not survive restart. There is no plaintext
  credential fallback. Vault entries are isolated by canonical API origin, so a
  production credential is never restored for a staging/custom HTTPS origin.
- Logout revokes the server session and removes the OS credential.
- Account space/channel navigation, including accessible private channels.
- Bounded read-only hover prefetch, retained conversation cursors and last-channel
  restoration. Navigation rechecks access; mutations and revocation discard
  cached data and fence in-flight completions. Pending navigation and initial
  history use message-pane skeletons, not a layout-shifting banner. Navigation
  failures offer in-pane retry/dismiss; drafts and scroll state remain retained,
  and the composer pauses until navigation finishes or is cancelled/dismissed.
- HTTP message history and idempotent writes. A timeout or lost response retains
  the same client message UUID and original text for retry; a definitive
  validation rejection unlocks editing and the next send gets a new UUID. A
  matching gateway delivery confirms the pending write even if HTTP finishes
  late or fails.
- Unified `/api/chat/events` gateway subscriptions with ordered replay,
  deduplication, gap resync, heartbeat handling, reconnect status, and stale
  result isolation across logout/channel changes.
- Account spaces, private-channel grants, pagination, typing,
  paginated member presence, owner space/channel/member management, and confirmed
  non-owner leave-space with immediate call teardown and conversation clearing.
- File attachments, when the server has storage configured (otherwise the
  attach control is hidden). Inline images show the uploaded preview, else
  the PNG/JPEG/WebP/GIF original, decoded off the UI thread and cached by
  attachment id; clicking opens the full file in the system browser. AVIF
  and HEIC are never decoded in the app. Videos show their poster with a play
  button. Audio and other files show cards that open in the system browser
  or player. Desktop has **no in-app video or audio playback**. Removed files
  read "File removed". The parked server-processing fields stay supported
  (absent `status` means ready): processing placeholders with the
  `attachment.progress` percent, "Couldn't process this file", animated
  ("GIF") posters, and `message.attachments` updates sequenced like
  reactions. Signed URLs are
  refreshed (`POST /api/assets/urls`) before they expire and once after a
  403/404 image load, so a window left open for days keeps working.
- Sending up to 10 files per message from the paperclip file dialog (Win32 on
  Windows; on Linux the XDG desktop portal, which needs a file-chooser backend
  such as xdg-desktop-portal-gtk, -gnome or -kde) or by dropping files on the
  window. Files are compressed on the device, off the UI thread, with the
  server's `GET /api/assets/usage` `compression` settings: lossless stills
  (PNG, BMP, TIFF, lossless WebP) stay pixel-exact at full size, as the
  smaller of an exact-palette indexed PNG (when the colours fit
  `paletteColors`) and lossless WebP (libwebp, `method` 3 like the web
  client), kept only when smaller; photos (JPEG, lossy WebP) are scaled to
  `imageMaxEdge`, upright, without EXIF/GPS, and become AVIF at `avifQuality`
  when `imageFormat` is `"avif"` (rav1e, 8-bit 4:2:0, sRGB or Display P3
  signalled in `nclx`), else lossy WebP at `imageQuality` (libwebp), kept
  only when at least 10% smaller. AVIF falls back to WebP for translucent
  photos, other colour profiles and encoder errors, and WebP to JPEG. A
  missing `imageFormat` means WebP. HEIC, AVIF, GIF,
  SVG, audio, documents and videos upload as the original (no transcoder;
  MP4/QuickTime size and duration come from the header). Originals lose their
  metadata losslessly: JPEG APPn/comments except JFIF, ICC and Adobe (a
  non-default orientation is kept), PNG text/eXIf chunks, and MP4/QuickTime
  `udta`/`meta` boxes under `moov` and each `trak`, zero-filled as `free`
  boxes of the same size. Large images get a JPEG preview (≤ `previewEdge`,
  ≤ 512 KiB). The client reserves the exact stored size, PUTs the preview
  and then the file straight to storage with exactly the presigned headers
  and no Caper credentials, confirms (retrying while storage has not seen it
  yet), and sends. Chips show the name, saving ("1.6 MB → 143 KB"), progress,
  errors such as storage full, and remove, with a local thumbnail the sent
  message keeps. Pasting images from the clipboard is not supported (egui
  does not deliver image pastes).
- Global two-person direct messages, including exact-username conversation
  creation, unread state, account-wide read cursors, paging, typing, retry, and gateway
  replay. Desktop does not provide OS push notifications in this stage.
- The channel/DM composer suggests after `:` (emoji) and `@` (people) in one
  popup; the thread reply box and message editor do not yet. Space channels
  offer the open space's loaded members except you, then `@everyone`/`@here`.
  DMs, self-notes included, offer the people you share a space or DM with
  (`GET /api/people`, fetched when a DM opens); until that loads, only the
  other participant. Names the API resolved in
  `content.mentions` render as pills: egui background spans with square corners
  rather than 4px rounded ones, including in thread replies and edited messages.
  Messages that mention you get a terracotta tint with a 2px leading edge.
  Clicking a person's pill, or Enter/Space on a focused pill, opens a card with
  their avatar, name and `@username` from loaded data, and a **Message** button
  that opens or creates the DM (your own card says "You"). Mentions do not
  notify anyone.
- Message requests: a DM from someone you share no space with waits under
  "Message requests" (with a count, never unread dots or sounds) and opens
  read-only with Accept, Decline or Block; your own pending requests say
  "Waiting for @name to accept". Blocking (confirmed first) from a DM header,
  a request or a message's actions hides that account's messages everywhere as
  collapsed "⊘ N blocked messages — Show" runs, including thread panels; a
  blocked DM replaces the composer with Unblock. Settings lists blocked
  accounts and "Who can start a DM with you" (Anyone, People in my spaces, No
  one new), saved on change and reverted if saving fails. Pinned-message lists
  still show blocked authors' pins.
- Message reactions. Hovering or keyboard-focusing a reaction chip shows who
  reacted (large emoji plus the shared summary wording); names load on demand
  with history's read access and are cached per message reaction revision.
- Experimental native voice: raw Google libwebrtc with platform audio devices,
  Caper SFU offer/answer publication and subscription, voice roster, lease
  snapshots, TURN refresh and replay-safe ICE restart/ACK. Browsing leaves the
  active call running; explicit replacement, logout and access denial silence
  locally. Mute/deafen and input/output selection use native ADM.
- Prejoin device discovery, saved device/output-volume preferences, per-person
  local mute and 0–200% software playback gain, and aggregate connection
  statistics. Participant mute survives microphone-track replacement and takes
  effect locally even while signaling is waiting for HTTP.
- Speaking rings in your own call's roster and voice stack, from the web
  client's rule (32 ms RMS ≥ 0.004, 180 ms release, never while muted) applied
  to your published microphone PCM and each subscription's decoded PCM.
- Persisted 0–200% input gain and voice-contour strength, with on-device DPDFNet-8
  HR denoising and RNNoise fallback. Account-enabled audio diagnostics show only
  local numeric processing counters; ordinary connection details remain public.
- Native playback of the web client's bundled interaction sounds; no network
  audio fetch and no sounds in static fixtures.
- Audio test, as on web: microphone and speaker pickers with 0–200% volumes; a
  speaker test looping the join sound through the selected output's own ADM at
  the live speaker volume until stopped or the dialog closes; explicit local
  microphone recording (30 seconds maximum) with a live input meter, then
  natural and enhanced playback (natural first, enhanced when it ends), a new
  comparison after volume or enhancement changes, and silent-recording
  detection. Samples stay in memory, never go to the API, and are discarded
  when the test ends. During a recording, live microphone publication is
  suspended; newer mute intent is retained.

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
  libdbus-1-dev dbus-x11 libglib2.0-dev libasound2-dev clang-21 lld-21
rustup toolchain install 1.94.0 --profile minimal --component rustfmt --component clippy
./apps/native/desktop/build.sh
```

Windows Server 2025 / Windows 11 build host:

1. Install Visual Studio 2022 or newer Build Tools with **Desktop development with C++**
   and a Windows 10/11 SDK.
2. Install rustup and the stable `1.94.0-x86_64-pc-windows-msvc` toolchain.
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
and desktop shortcuts, and registers an uninstaller in Installed apps. Opening
Setup immediately shows installation progress, then closes and opens Caper;
there are no Welcome/Next/Finish clicks. Silent `/S` installs do not launch Caper.
Neither installation nor launch requires a terminal or administrator access. The updater
replaces only `app`, preserving the uninstaller and shortcuts. Close Caper before
installing or uninstalling. CI runs `test-installer.ps1` on a clean Windows host
to check the PE GUI subsystem, install/reinstall, file hashes, shortcuts and removal.

The `.deb` is built against Ubuntu 24.04 (glibc 2.39) and declares native
X11/Wayland, D-Bus, PulseAudio/ALSA and Vulkan-or-GL runtime dependencies. It is not a static or
distribution-independent Linux binary. Linux session persistence also requires
an unlocked Secret Service provider such as GNOME Keyring.

Run the unpacked binary directly, or install the Debian package with
`sudo apt install ./Caper-linux-x64.deb`. The production endpoint is
`https://caper.chat`; development may use `--api-url http://localhost:PORT` or
`CAPER_API_URL`. Plain HTTP is rejected for non-loopback hosts.

## Windows microphone access and firewall prompts

Windows capture uses WebRTC's WASAPI **shared mode**. A Discord call does not
normally prevent Caper from using the same microphone. Caper cannot bypass
Windows microphone privacy settings or another application's exclusive access.
**System default** consistently resolves the normal Windows input/output
default, not WebRTC's implicit communications default. Failed startup closes
the capture gate and reports the failing stage.

Voice and local audio tests open WebRTC sockets and can trigger a Windows
Firewall prompt. This is separate from microphone permission; Caper does not
automatically change firewall rules or suppress security prompts. The unsigned
Windows executable can also show **Publisher: Unknown**. See the
[Windows audio and firewall runbook](../../../docs/media.md#windows-microphone-access-and-firewall-prompts)
for troubleshooting, network-profile guidance, and Windows acceptance gaps.

## Calling and known parity gaps

Voice is **experimental**, not live/physical-device accepted. Source and local
tests exercise signaling, cancellation and native ICE gathering; they do not
prove two-client SFU/TURN or actual mic/speaker quality. Playback gain uses a
narrow bridge to WebRTC's software track volume, not system volume. Independent
device enumeration does not start capture or alter an active call's ADM. A
missing saved device fails explicitly rather than silently opening another mic.
Explicit/default input and output selection applies synchronously to the local
call, including while signaling is pending. The Linux pinned Pulse ADM exposes
one dynamic default pseudo-device in the orb, not two explicit monitor GUIDs:
the two-null-sink regression verifies A→B by changing the private server's default
and resetting the input, not by selecting two explicit GUIDs.

Mute, route changes, comparison, and zero gain fence publication synchronously.
Reopening live capture replaces the private ADM, peer pair, and decoder before
accepting a new publication epoch, then resets denoiser/contour state. Local
delayed-PCM tests cover that boundary; hardware-driver buffering still requires
physical-device validation.

Live input and local comparison use gain → bundled DPDFNet-8 HR (or RNNoise)
→ voice contour. Natural replay is post-gain/post-denoise; enhanced replay adds
the live contour. The model and native ONNX Runtime 1.23.2 are pinned to the web
assets/runtime version. FFT/OLA and recurrent-state reference tests pass; the
contour approximates the browser filters/compressor, not bit-for-bit Web Audio.
Private null-device speech tests exercise actual capture and positive decoded
local-peer PCM, replay, and stop. They do not establish physical quality or
remote SFU reception. Pure sine capture was suppressed while speech succeeded;
the suppression mechanism is not established.

Linux packages carry the checked native runtime and licenses. Windows also
stages the four Microsoft-signed app-local VC++ DLLs imported by ORT; these come
from the active Visual Studio toolchain and are not immutable hash-pinned. Windows
package execution remains an exact-head CI/platform acceptance requirement.
No camera, screen sharing, native push notifications, or Windows/Linux code signing.
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
**1396/0 bytes**. No unrelated roster was observed. This test fed a silent null
source, not speech, and checked aggregate RTP bytes rather than decoded audio.
It is an unresolved silent-transport observation, not proof that normal speech
cannot be received. Silence handling versus a relay/receive fault was not
distinguished. The failed process closed local peers;
remote leave completion was not independently confirmed (server leases expire).
The ignored test now awaits remote leave before asserting its RTP result and
reports safe direction/track counts. It has not been rerun with that diagnostic.

A physical microphone is not required for meaningful automated audio tests.
The private null-device regression injects locally generated speech into the
virtual microphone, requires nonzero captured and denoised PCM, and checks that
a second local peer decodes it. On September 24, a separately authorized,
20-second-bounded live test also passed **two-way decoded speech through the
public SFU**, using two owned native sessions and generated virtual-microphone
input. Both remote leaves were acknowledged and both clients disappeared from
General. See [the acceptance record](../../../docs/media.md#native-generated-speech-acceptance)
for exact boundaries and results; physical devices and sustained/TURN calls
remain unverified.

A non-ignored, local-only native test receives actual RTP after adding a
subscription to an existing transport and after ICE renewal. It caught a separate
binding-default bug that changed receive transceivers to inactive during renewal;
restart offers now retain existing receive directions without adding an unused
receiver to a publish-only session. This does **not** diagnose or resolve the
silent live-test observation above. Further public tests require explicit
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
audio bar without starting a media transport; `parity-voice-speaking` adds fixed
speaking rings for you and Maya.
`parity-voice-error` and `parity-audio-error` preview capture failure guidance in
the voice dock and Audio test dialog without starting capture or a transport.
`parity-direct` and `parity-direct-new` preview a two-person DM and its
exact-username dialog without a live account or notification provider.
`parity-mentions` previews mention pills, an unresolved `@name`, and rows that
mention you; click `@alex` or `@fixture_owner` for the cards, or type `@` in its
composer for space suggestions.
`parity-direct-no-spaces` previews the first-space page's Direct messages entry
and the global list without any space membership.
`parity-attachments` previews ready, processing (with poster and percent, and
without a preview), failed, animated ("GIF") and removed files, file/audio
cards and upload chips from synthetic local images; it never fetches media or
uploads.
`parity-requests` previews an open incoming request and the requests list;
`parity-requests-outgoing` an unaccepted request you sent; `parity-blocked`
Maya's #general messages collapsed after blocking her; `parity-blocked-dm` the
blocked-DM composer.
`parity-reactions` adds reaction chips whose hover cards name the labelled
fixture members locally instead of requesting who reacted.
`parity-opening`, `parity-opening-narrow`, and `parity-opening-error` preview
pending navigation and its retry state with retained conversation chrome and a
labelled fixture draft; `parity-loading` previews the initial history skeleton.
These previews do not start a navigation request or a media transport.
`parity-settings`, `parity-settings-checking`, `parity-settings-current`, and
`parity-settings-offline` preview the update control and check feedback. These
are static states: their check buttons are disabled and no updater process runs.
`parity-update` and `parity-update-narrow` preview long, version-grouped release
notes with a fixed restart/Later footer. `parity-update-download` shows the
manual installer fallback; `parity-update-error` and `parity-update-incomplete`
show launch failure and unavailable older history. These fixtures never install
updates or start update checks.

After `npm ci`, run `node scripts/native-icons.mjs --check` to verify the bundled
vectors match the web client's pinned Lucide package. Omit `--check` to regenerate.

Static desktop icons use the main `apps/web/public/caper-face.svg` artwork.
With ImageMagick 7 and librsvg installed, run
`node scripts/generate-favicons.mjs` from the repository root to regenerate the
web fallback PNGs, Linux SVG, desktop fallback PNG, multi-size Windows ICO,
and macOS AppIcon PNGs. Use
`node scripts/generate-favicons.mjs --check` to detect asset drift without writes.
The executable, installer, and uninstaller all use that ICO; Start and desktop
shortcuts use the executable's icon. Linux packages the same main SVG under
`hicolor/scalable/apps/caper.svg`; macOS uses matching transparent PNGs from
16 to 1024px. Runtime window icons also stay on the original green mascot;
daily characters rotate only in in-app wordmarks. The opaque iOS icon is unchanged.
Linux uses `app_id=caper` to match `caper.desktop` and its `Icon=caper` lookup;
the app does not write per-user icon overrides.
After installing an updated Windows build, Search may retain a cached icon;
sign out and back in before checking again. Asset checks in Linux do not verify
Windows Search, macOS Finder/Dock or pinned-shortcut cache behavior.
