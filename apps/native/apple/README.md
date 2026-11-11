# Caper for Apple platforms

The shipping macOS app is the AppKit client (`CaperMacAppKit`, below); the shipping iPhone app is the UIKit client (`CaperIOSUIKit`, built from `Sources/CaperUIKit` with the `chat.caper.ios` identity, entitlements and notification extension; `build.sh` and `upload-testflight.sh` always build it). They contain no Electron, Tauri, WebView, or browser content (apart from the isolated KLIPY ad view on iPhone). The earlier SwiftUI apps (`CaperIOS`, `CaperMacOS`) still build but are no longer released.

## Features

- Email sign-in and onboarding with no Guest/General fallback. Sessions restore through the origin-namespaced Keychain; logout returns to sign-in.
- The caper.chat shell at desktop and narrow widths: space rail, channel sidebar, responsive member panel, Browse navigation, Satoshi typography and the web color/spacing tokens.
- Owner workflows: create/delete/rename spaces and channels, add/remove existing accounts by username, private channels and grants.
- Invite links: owners create (expiry, use limit, optional landing channel), copy and revoke them in Manage space; anyone can join from **Join with invite link**. iPhone opens `https://caper.chat/invite/<code>` universal links (`applinks:caper.chat` Associated Domains entitlement); both apps register `caper://invite/<code>`. The Mac app ships as a Developer ID DMG without a provisioning profile, so it opens only `caper://` links. Universal links need the Associated Domains capability on the `chat.caper.ios` App ID (automatic signing with `-allowProvisioningUpdates` in `upload-testflight.sh` normally enables it; otherwise enable it in the Apple Developer portal) and `/.well-known/apple-app-site-association` on caper.chat.
- Chat history and pagination, idempotent sends, ordered gateway replay, reconnect state, typing, paginated presence, and deleting your own messages (`DELETE /api/chat/channels/<channel>/messages/<message>`; the row disappears everywhere).
- Direct messages, message requests (Accept/Decline/Block), blocking with collapsed "⊘ N blocked messages" runs, and **Privacy and blocked accounts** settings.
- `:` emoji and `@` people suggestions; mention pills (square-cornered, since SwiftUI `Text` cannot pad or round inline runs) open a profile card with **Message**. Inline URLs get a chain-link symbol and tint drawn with TextKit, never inserted into the text.
- Caper team names (`@feedback`, `@caper`, `@admin`, …) notify the team; a team name in **New message** opens the Caper DM. `feedbackTeam` accounts get a **Feedback** inbox with archive/restore and visits.
- Attachments (`docs/media.md`, "Uploads and attachments"): images, video (AVKit), audio and file cards; an in-app media viewer; the attach drawer (`AttachDrawer.swift`) on iPhone/iPad and an "Upload a file" menu on the Mac; paste and drag and drop; up to ten files per message per conversation or thread. Uploads are available when `GET /api/assets/usage` succeeds. On-device compression uses the server's `compression` settings: lossless sources as indexed PNG or lossless WebP (libwebp), photos as AVIF (libavif 1.4.2 with aom 3.15.1, speed 6 all-intra, `tune=ssim`) or WebP/JPEG, and videos as SDR H.264/AAC MP4 when needed. Originals lose location metadata losslessly.
- Attachment editor (`AttachmentEdit`, geometry shared with `shared/messages/edit-cases.json`): spoiler, crop, rotate, draw and text with undo/redo. Stills render with CoreGraphics/CoreText; videos transcode to H.264 with marks composited through Core Image.
- Dictation: a tap or click on the microphone ("Dictate") records until Done, then `POST /api/dictation` inserts the words at the caret; the bar's "Voice note" keeps the recording as a voice message instead, and a failed transcription keeps the audio for Retry or "Keep as voice note".
- Voice messages: press-and-hold (touch) or the dictation bar's "Voice note"; recording up to 10:00, processed by the call's DPDFNet-8/RNNoise worker and voice contour (`CaperVoiceNoteProcessor`) and encoded as AAC/MP4 (mono, 48 kHz, 64 kbps), with an unprocessed AVAudioRecorder fallback. Recordings stage as a chip and go with the next Send, or send at once with "Send voice messages as soon as I stop recording" (Mac Settings → Voice messages; iPhone account menu → Audio settings). Players show waveforms and transcripts.
- Custom profile pictures and space icons (GIFs up to 5 MB/1024 px unchanged, larger ones shrunk with `GIFShrink`; others 512 px WebP up to 1 MB). The `profiles` gateway feed applies `profile.updated` and `space.updated` live.
- KLIPY GIFs and stickers (`docs/media.md#gifs-and-stickers-klipy`) when `GET /api/klipy` answers; ads only on iPhone, in a non-persistent WKWebView.
- Voice in the channel list follows the shared layout in [apps/native](../README.md#layout-rules). The account bar keeps microphone, deafen, device menus and settings.
- Packaged macOS releases check for updates (account settings menu, Settings, and **Caper › Check for Updates…**) and show an **Update available** button and modal. iPhone updates come from TestFlight/the App Store.
- Generation fences for account/channel transitions, separate in-memory chat/media capabilities, and same-origin-only credential redirects.

## Notifications

- **Controls (iOS and macOS).** The space menu and each channel's menu set **Notifications** (Default, All messages, Only @mentions, Nothing) and **Mute** (15 minutes, 1, 8 or 24 hours, or until turned back on). DM rows have **Turn off/on notifications** and **Mute/Unmute conversation**. **Account settings ▸ Notifications** holds "Notify me about". Changes apply at once and revert with an inline error if saving fails.
- **iOS push.** When the server advertises this build's APNs environment in `GET /api/push/config`, settings show "Send to this phone". After opt-in the app registers its APNs token with `appId` (the bundle ID) for the current sign-in session. Taps open the channel (`spaceId` + `channelId`), DM (`conversationId`), or, for kind `feedback`, a feedback visit at `messageId` in its thread. `Configuration/iOS-Debug.entitlements` selects the APNs sandbox (`apnsSandbox`) and `iOS-Release.entitlements` production (`apns`). Unsigned simulator builds cannot receive tokens, so opt-in stays hidden.
- **Invitations and message requests** are regular alerts (requests without message content); tapping opens Accept/Decline review.
- **Deleted messages.** On iPhone the server's passive `message.deleted` alert replaces the original by collapse id, and the notification service removes any delivered original with that `messageId`. While running, the `profiles` feed's `notification.retracted` removes it on both platforms.
- macOS receives local notifications from the account gateway while running; there is no APNs delivery after quit.

## Fonts and licenses

`prepare.sh` runs the repository's verified `scripts/native_fonts.py`, then bundles the original, unmodified Satoshi Regular/Medium/Bold/Black OTF files and unchanged Fontshare license. Generated font inputs are ignored. Both app archives also contain, from `Resources/`:

- `WebRTC-LICENSE.txt`;
- `libwebp-LICENSE.txt` (libwebp 1.6.0 via `SDWebImage/libwebp-Xcode`, BSD-3-Clause plus the WebM patent grant);
- `libavif-LICENSE.txt` (libavif 1.4.2, BSD-2-Clause, including its bundled libyuv scaling code);
- `libaom-LICENSE.txt` (aom 3.15.1, BSD-2-Clause, plus its compiled-in fastfeat, vector and x86inc code) and `libaom-PATENTS.txt` (AOMedia Patent License 1.0).

`prepare.sh` downloads libavif and the aom AV1 encoder as one static `.build/libavif-1.4.2/libavif.xcframework` (module `libavif`; macOS arm64/x86_64, iOS arm64, simulator arm64/x86_64) from a SHA-256-pinned [joswayski/libavif-apple](https://github.com/joswayski/libavif-apple) release, which builds it from pinned upstream sources (Neon on arm64, SSE/AVX on x86_64) and smoke-tests every slice. It checks that the licence files match that release. Upgrading means a new release there and a new tag and checksum here.

`prepare.sh` also builds `.build/CaperMobile.xcframework` (module `CaperMobile`, imported implementation-only by CaperCore), the shared Rust logic in [`../mobile`](../mobile/README.md), as a static library for the same five slices. It needs rustup; `mobile/build-apple.sh` adds the pinned toolchain's Apple targets.

The DPDFNet-8 HR model, ONNX Runtime (macOS 1.23.2; iOS uses the official static 1.23.0 XCFramework with arm64 device and arm64/x86_64 simulator slices) and the RNNoise fallback are bundled offline with their licenses; `prepare.sh` verifies source hashes.

## Build

Requires the Xcode 16 toolchain (GitHub `macos-15` / `macos-15-intel`). XcodeGen is built automatically from pinned commit `21ac9944b0ab546a07422dbed86f33dd2ebd76f8`; WebRTC is pinned to `stasel/WebRTC` 153.0.0.

```sh
./apps/native/apple/build.sh macos
./apps/native/apple/build.sh ios
```

On Apple silicon, `CAPER_MACOS_ARCH=x86_64 ./apps/native/apple/build.sh macos` builds the Intel app and runs its tests under Rosetta.

Each command first runs the macOS XCTest suite. Outputs are ignored and credential-free:

- `dist/Caper-macos-arm64.zip` or `dist/Caper-macos-x64.zip` with an ad-hoc-signed `.app`
- `dist/Caper-ios-simulator-arm64.zip` (unsigned simulator app; not a device package)

The script verifies app/framework architectures, embedded WebRTC, runpaths/signature on macOS, and embedded font/WebRTC license resources. Bundle IDs default to `chat.caper.macos` and `chat.caper.ios` and may be overridden with `CAPER_MACOS_BUNDLE_ID` / `CAPER_IOS_BUNDLE_ID`.

Signed Mac releases use `dmgbuild==1.6.7` (locked in the `dmg` dependency group) to create the app → Applications Finder window. `sign-and-notarize.sh` notarizes and staples the app, keeps the ZIP self-update payload, then signs, notarizes, staples and assesses the DMG. From the repository root, with the signing environment set:

```sh
uv run --locked --only-group dmg bash apps/native/apple/sign-and-notarize.sh <label>
uv run --locked --only-group dmg python -m unittest discover -s tests -p test_apple_release_signing.py -v
```

The test builds and mounts a disposable fixture DMG on a Mac and skips that check elsewhere.

macOS AppIcon PNGs (16–1024px, transparent) are generated from `apps/web/public/caper-face.svg` by `node scripts/generate-favicons.mjs` (ImageMagick 7 and librsvg; `--check` detects drift). The opaque iOS AppIcon is not generated by this command.

## AppKit Mac client

`native-project.yml` includes the shipping project and adds `CaperAppKit`, whose workspace (navigation, timeline, composer, member list, thread pane and voice panels) uses AppKit controls, layer-backed views and Core Text measurement. Dialogs, the forward picker, pins, the activity inbox, Settings and What's New host the shared `CaperCore` SwiftUI views through `AppKitHosting`. It reuses `CaperCore`'s account, API, gateway and chat models. `NativeTimelineLayout` supplies estimated geometry and stable-ID scroll anchors; the host measures text only for realized rows and exposes those rows to accessibility.

It covers sign-in and onboarding, space/channel/DM navigation and requests, history, sending/retry, editing and deletion, replies, threads, pins, forwarding, reactions, attachments, GIFs, voice notes and voice channels, the member list, every space/channel/account management dialog, notifications, the activity inbox, Settings and What's New. It also handles `caper://` links and includes the shared self-updater. `NativePresentation` and `NativeTimelineLayout` support both macOS and iOS; controls and observation remain host-specific.

Release builds use the `CaperMacAppKit` target, which keeps the SwiftUI app's identity: bundle ID `chat.caper.macos`, product name Caper, its Keychain session, login item, `caper://` URL type and update channel, so existing installs move over on their next self-update and stay signed in. `build.sh` always builds it. The `CaperAppKit` development scheme keeps the bundle ID `chat.caper.appkit-development` and a separate Keychain service. Build and run the development host from the repository root on a Mac:

```sh
./apps/native/apple/prepare.sh
apps/native/apple/.build/xcodegen-21ac9944b0ab546a07422dbed86f33dd2ebd76f8/.build/debug/xcodegen generate \
  --spec apps/native/apple/native-project.yml --project apps/native/apple
xcodebuild -project apps/native/apple/CaperNative.xcodeproj -scheme CaperAppKit \
  -configuration Debug -destination 'platform=macOS' \
  -derivedDataPath apps/native/apple/.build/NativeDerivedData test
open apps/native/apple/.build/NativeDerivedData/Build/Products/Debug/CaperAppKit.app
```

For UI automation, run `node scripts/native-parity-fixture.mjs` on the Mac (or forward both ports 3001 and 3002), then run the same `xcodebuild` command with `-scheme CaperAppKitTests`. Those tests use the explicit loopback fixture and an in-memory fixture session, never a production account. They are functional smoke tests, not a screenshot-parity or hardware-performance acceptance suite.

The `Native development builds` workflow has a separate AppKit job: it runs shared/core and native-view tests, compiles the Release development executable, and checks its isolated bundle ID and ad-hoc signature. It uploads test/build logs, not an app for distribution. The job does not gate or replace the shipping Apple builds.

## Parity captures

The scripts launch only the explicit loopback fixture; fixture data can never be selected in a production launch.

```sh
./apps/native/apple/parity-screenshots.sh macos
./apps/native/apple/parity-screenshots.sh ios
```

The macOS suite captures populated, members-hidden, login, login error, Manage space, and private channel overview states; iOS adds narrow conversation and Browse-open states on an `iPhone 16` simulator. The `reaction-chips` fixture captures who reacted (hover card on macOS, press-and-hold sheet on iOS). The `voice-roster`, `voice-crowd` and `voice-elsewhere` fixtures show local call rosters without capture or media. Screenshots are exported into ignored `apps/native/apple/parity-artifacts/`.

## Device benchmarks

`bench-project.yml` includes the shipping project and builds two TEST FIXTURE apps in Release: `chat.caper.bench.uikit` (the UIKit client) and `chat.caper.bench.swiftui` (the shipping SwiftUI app). Their `CaperCore` is compiled with `CAPER_DEVICE_FIXTURE`, the only build that accepts a fixture on a private-LAN IPv4 address (10/8, 172.16/12, 192.168/16) as well as loopback. Without a valid `CAPER_TEST_MODE=parity` and `CAPER_API_BASE_URL`, a bench app stops at launch instead of connecting to caper.chat, and its session stays in memory. The apps have no URL schemes, associated domains, push or extensions, so they cannot replace or impersonate `chat.caper.ios`. Shipping builds still accept only a loopback fixture.

```sh
CAPER_FIXTURE_HOST="$(ipconfig getifaddr en0)" CAPER_BENCH_DESTINATION='platform=iOS,id=<device UDID>' \
  DEVELOPMENT_TEAM=<team> ./apps/native/apple/bench.sh test uikit   # or swiftui
```

`bench.sh` starts the fixture on `CAPER_FIXTURE_HOST:CAPER_FIXTURE_PORT` (default `127.0.0.1:3101`, for simulators). The fixture binds only that address and uses it in every absolute URL. It serves 3,000 `#general` rows and, in an opt-in `#bench-formatting` channel, `CAPER_BENCH_FORMATTED_MESSAGES` (default 400) long, formatted, link-heavy rows. `Tests/CaperBenchTests` measures launch (with and without the UIKit startup cache), history scrolling in both channels, and typing a formatted draft that is never sent. Without these variables the fixture behaves as before.

## Voice

Join and audio preferences are available in normal builds. Joining requests microphone permission; publication stays silent until the remote answer, transport readiness and generation checks complete. Signaling sends pending local SDP immediately, like the web client. The implementation supports gateway and lease snapshots, roster subscribe/close, local mute/deafen/leave, bounded rejoin, TURN renewal with restart/ack, and per-participant local mute/gain. Browsing keeps the call; Join elsewhere, logout, deletion and access loss tear it down locally first.

- **macOS:** the pinned WebRTC binary's custom audio factory and AUHAL route each call to the chosen input/output without changing system defaults.
- **iPhone:** an M153 custom audio device backed by VoiceProcessingIO follows the current `AVAudioSession` route (Apple's communication-route UI). `RTCAudioSession` interruption/route handling is implemented, and the audio background mode lets an active call continue while locked. Interaction sounds play mixed with other audio (ambient session) and never change a call's session.
- **Processing (both):** the microphone callback enqueues gain-adjusted PCM to a bounded worker that resamples to 48 kHz, runs DPDFNet-8 HR (or native RNNoise), applies the voice contour, and resamples back; it never runs inference on the callback and sends silence until processed output is ready or if both engines fail. Settings offer 0–200% input gain, contour strength, and a local natural/enhanced comparison (up to 30 seconds; publication suspended meanwhile). Connection statistics omit addresses, candidate IDs and credentials; denoiser counters are shown only to debug-enabled accounts.

There is no PushKit/CallKit incoming-call behavior because the server has no incoming-call signaling.
