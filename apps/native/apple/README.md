# Caper for Apple platforms

Native SwiftUI clients backed by AppKit on macOS and UIKit on iPhone. They contain no Electron, Tauri, WebView, or browser content. This follows the Captures native direction, but is not a direct port of Captures' custom AppKit views.

## Current implementation

- Guest access to public General plus passwordless email sign-in, onboarding/profile, logout, and origin-namespaced Keychain sessions.
- The caper.chat shell at desktop and narrow widths: space rail, channel sidebar, toggleable responsive member panel, Browse navigation, conversation stage, Satoshi typography, and the web color/spacing tokens.
- Space/channel/member owner workflows: create/delete/rename spaces and channels, add/remove existing accounts by username, private-channel toggle and grants.
- HTTP chat history and pagination, idempotent sends, ordered gateway delivery/replay, reconnect/error state, typing, and paginated member presence.
- Generation fences for account/channel transitions, immediate revoked-data clearing, separate in-memory chat/media capabilities, and same-origin-only credential redirects.

The native UI and platform projects still require exact-head Apple CI before they are considered build-verified. The Linux orb used for implementation has no Swift or Xcode installation.

## Fonts and licenses

`prepare.sh` runs the repository's verified `scripts/native_fonts.py`, then bundles the original, unmodified Satoshi Regular/Medium/Bold/Black OTF files and unchanged Fontshare license. Generated font inputs are ignored. Both app archives also contain `WebRTC-LICENSE.txt`.

## Build

Requires the Xcode 16 toolchain on GitHub `macos-15` / `macos-15-intel`. XcodeGen is built automatically from pinned commit `21ac9944b0ab546a07422dbed86f33dd2ebd76f8`; WebRTC is pinned to `stasel/WebRTC` 153.0.0.

```sh
./apps/native/apple/build.sh macos
./apps/native/apple/build.sh ios
```

Each command first runs the macOS XCTest suite. Outputs are ignored and credential-free:

- `dist/Caper-macos-arm64.zip` or `dist/Caper-macos-x64.zip` with an ad-hoc-signed `.app`
- `dist/Caper-ios-simulator-arm64.zip` (unsigned simulator app; not a device package)

The script verifies app/framework architectures, embedded WebRTC, runpaths/signature on macOS, and embedded font/WebRTC license resources. Bundle IDs default to `chat.caper.macos` and `chat.caper.ios` and may be overridden with `CAPER_MACOS_BUNDLE_ID` / `CAPER_IOS_BUNDLE_ID`.

## Deterministic parity captures

The scripts launch only the explicit loopback fixture; fixture data can never be selected in a production launch.

```sh
./apps/native/apple/parity-screenshots.sh macos
./apps/native/apple/parity-screenshots.sh ios
```

The macOS suite captures populated, members-hidden, login, actionable login error, Manage space, and private Channel overview states. The iOS suite adds narrow conversation and Browse-open states on an `iPhone 16` simulator. Screenshots are exported from the XCTest result into ignored `apps/native/apple/parity-artifacts/`. Tests also assert fixture content and required controls before capture.

## Voice in development builds

Join and audio preferences are available in normal builds, with no environment flag. Joining requests microphone permission before capture; publication stays locally silent until the remote answer, transport readiness, and generation checks complete. The implementation supports gateway snapshots, lease snapshots, roster subscribe/close, immediate local mute/deafen/leave, bounded rejoin, TURN credential renewal with restart/ack, and iOS `RTCAudioSession` interruption/route handling. The iOS app declares the audio background mode so an active audio session can continue while locked.

Like the existing web client, native signaling sends pending local SDP immediately and gates audio on transport readiness, not completion of every STUN/TURN probe. The previous combination of continuous gathering and waiting for `.complete` made every initial join time out. The native transport regression creates two real peers, gives only the server-side stand-in gathered candidates, connects DTLS/SCTP, and repeats negotiation with a fresh ICE generation. This tests transport, not microphone capture or Cloudflare audio. Separate tests exercise delayed join/stop/replacement and preserve media error codes so provider retry is not mistaken for membership revocation.

Browsing another space or text channel preserves the active call and its displayed `space / channel` context. Opening that context navigates back to it; an explicit Join in another channel synchronously tears down the old transport before starting the replacement. Logout, active-context deletion, and active-channel access loss also tear down locally before any network cleanup.

This is **development calling, not accepted physical-device support**. The UI fixture deliberately returns media 503 and cannot validate Cloudflare. Required acceptance remains: exact-head Apple compilation/tests, two-party Cloudflare calls, TURN-only/network handoff, route/interruption recovery, and sustained locked-iPhone capture/playback on physical hardware. iOS uses Apple's communication-route UI. On macOS the pinned WebRTC binary's custom audio factory and AUHAL route each call to the chosen input/output without changing system defaults; selected devices still require physical acceptance. Output gain is persisted and per-participant local mute/gain is implemented. There is no incoming-call PushKit/CallKit behavior because the server has no incoming-call signaling.

macOS audio preferences include input gain, a live voice contour, and an explicitly started local microphone comparison (up to 30 seconds, also available in-call). The microphone callback enqueues gain-adjusted PCM to a bounded worker that resamples the actual hardware rate to 48 kHz, runs the pinned DPDFNet-8 HR model or native RNNoise fallback, then resamples to the hardware rate. The callback never runs inference and sends silence before processed output is ready or after both engines fail. Natural replay is post-gain/post-denoise; enhanced replay adds the same contour sent live. During in-call recording and replay, publication stays suspended until the sheet closes; dismissal or joining another call removes the temporary recording. The sheet also shows aggregate connection statistics without addresses, candidate IDs, or credentials. The model, native runtime, and fallback are bundled offline with licenses; `prepare.sh` verifies source hashes. Linux synthetic speech exercises the native model and 44.1/48 kHz worker paths, but actual recording, quality, route timing, and Cloudflare calling still require Apple CI and physical-device acceptance.

iPhone now has the same local gain (0–200%), on-device DPDFNet-8/RNNoise worker and live contour strength (0–100%) through an M153 custom audio device backed by iOS VoiceProcessingIO. It follows the current `AVAudioSession` route rather than selecting desktop-style HAL devices. Its explicitly started natural/enhanced microphone comparison records up to 30 seconds locally and keeps publication suspended through replay; gain zero is exact silence. Debug-enabled accounts can view local numeric denoiser counters, while aggregate connection statistics remain available in the sheet. The official pinned ONNX Runtime 1.23.0 static iOS XCFramework supplies arm64 device and arm64/x86_64 simulator slices; macOS keeps its pinned 1.23.2 runtime. No interaction sounds are bundled for iPhone, matching web mobile suppression. Synthetic peer and UI fixture tests do not establish physical iPhone microphone, speaker, Bluetooth route, interruption, background, or live SFU acceptance.
