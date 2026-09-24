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

This is **development calling, not accepted physical-device support**. The UI fixture deliberately returns media 503 and cannot validate Cloudflare. Required acceptance remains: exact-head Apple compilation/tests, two-party Cloudflare calls, TURN-only/network handoff, route/interruption recovery, and sustained locked-iPhone capture/playback on physical hardware. iOS uses Apple's communication-route UI; macOS follows System Settings because the pinned WebRTC binary exposes no safe per-device switch. Output gain and per-participant local mute/gain are implemented. Input gain, natural/enhanced mic playback and processing strength, safe prejoin/in-call mic testing, and connection/audio diagnostics are not yet parity-complete. There is no incoming-call PushKit/CallKit behavior because the server has no incoming-call signaling.
