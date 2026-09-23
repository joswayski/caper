# Caper for Apple platforms

Native Swift clients for macOS and iPhone. They use SwiftUI backed by AppKit/UIKit—not Electron, Tauri, WebView, or embedded browser content. Shared `CaperCore` code owns account, spaces, chat, gateway, Keychain, and WebRTC behavior.

The macOS client follows Captures' native direction, but it is a shared SwiftUI implementation backed by AppKit—not a direct port of Captures' custom AppKit views.

## Implemented slice

- Passwordless email code sign-in using bearer transport, profile onboarding, logout, and a Keychain session (`AfterFirstUnlockThisDeviceOnly`).
- Authenticated spaces/channels and native navigation.
- HTTP history/pagination and idempotent message writes, with the unified `/api/chat/events` gateway for ordered delivery and reconnect visibility.
- Account credentials only in `Authorization`; chat/media capabilities remain in memory and use their dedicated headers. Cross-origin redirects are rejected before credentials can be forwarded.
- An experimental audio-only native WebRTC path implements join/publish/subscribe, mute/deafen, roster polling, departed-track cleanup, and leave. It is disabled by default; developers can expose it with `CAPER_EXPERIMENTAL_VOICE=1`. It is not a supported calling claim.

The visual tokens match caper.chat. Satoshi is not bundled because the repository contains no redistributable font files; the clients use the native system font instead.

## Build and test

Requires Xcode 16.x (the default on GitHub's `macos-15` images). WebRTC is pinned exactly to `stasel/WebRTC` 153.0.0. XcodeGen is built from pinned commit `21ac9944b0ab546a07422dbed86f33dd2ebd76f8`; no global installation is needed.

```sh
swift test --package-path apps/native/apple
./apps/native/apple/build.sh macos
./apps/native/apple/build.sh ios
```

Outputs (never committed):

- `dist/Caper-macos-arm64.zip` on Apple Silicon
- `dist/Caper-macos-x64.zip` on Intel (`x86_64` is normalized)
- `dist/Caper-ios-simulator-arm64.zip`, explicitly simulator-only

The macOS app is ad-hoc signed, requiring no developer identity. The simulator package is unsigned. Bundle IDs default to `chat.caper.macos` and `chat.caper.ios`; override packaging with `CAPER_MACOS_BUNDLE_ID` and `CAPER_IOS_BUNDLE_ID`.

GitHub's `macos-15` and `macos-15-intel` runners should be sufficient. Both macOS jobs can run tests and `build.sh macos`; run `build.sh ios` on `macos-15` (arm64).

## Device signing

The deterministic `ios` target is simulator-only and credential-free. Physical-device archive/export configuration belongs in the repository-level distribution runbook once the Apple team and profile exist.

## Supported vs. missing

The supported first slice is account login/onboarding/logout, spaces/channel navigation, history/pagination, message sending, and reconnecting gateway delivery. It validates canonical cursors/content, rejects cross-origin credential redirects, isolates channel generations, clears revoked channel data, and preserves idempotent sends only for unknown outcomes.

Space/channel/member administration, presence display, typing indicators, notification delivery, and polished mobile navigation remain missing.

## Required voice hardening and validation before claiming parity

The experimental native WebRTC path is **not verified in this Linux orb** (there is no Swift/Xcode SDK, Apple audio device, signing identity, or iPhone). The UI is disabled by default and must not be marketed as calling support yet. Before release:

1. Compile on both macOS runners and resolve any WebRTC 153 header/API drift.
2. Run a real two-party call through Cloudflare on macOS and a physical iPhone, including TURN-only/multi-network coverage.
3. Lock the physical iPhone during an active call and verify uninterrupted capture/playback for a sustained period. A plist declaration and audio-session category are necessary but are not proof.
4. Add the browser client's gateway media snapshots, TURN credential renewal/ICE restart, connection-state recovery, deployment handoff behavior, and robust audio interruption/route handling. This implementation polls snapshots and has no ICE restart; long-running/recovering-call parity is therefore still outstanding.
5. Verify route changes, Bluetooth/wired output, interruptions, permission denial, and app termination behavior. Caper intentionally does not add incoming-call PushKit/CallKit infrastructure because the server has no incoming-call signaling.

Unit coverage exercises URL transport headers, invalid history, idempotent delivery state, sequence precision, message boundaries, ICE decoding, and cross-origin redirect rejection. A full controllable WebSocket lifecycle fixture remains missing. The Linux orb cannot execute XCTest; `build.sh` runs it on the macOS pipeline before producing either artifact.
