# Caper for Android

Native Android client (Kotlin and platform Jetpack Compose; no WebView or cross-platform UI). The provisional application ID is `chat.caper.android`; debug builds install as `chat.caper.android.debug` and are labeled **Caper (Development)**.

## Reproducible build

Prerequisites:

- JDK 17
- Android SDK Platform 36
- Android SDK Build Tools 36.0.0
- Android platform-tools
- Python 3 and network access for the pinned Fontshare acquisition step

One reproducible command-line SDK setup is:

```bash
export ANDROID_HOME="$HOME/android-sdk"
mkdir -p "$ANDROID_HOME/cmdline-tools"
# Install Google's command-line tools under $ANDROID_HOME/cmdline-tools/latest first.
yes | "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --licenses >/dev/null
"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" \
  "platform-tools" "platforms;android-36" "build-tools;36.0.0"
export PATH="$ANDROID_HOME/platform-tools:$PATH"
```

The checked-in wrapper pins Gradle 8.13 and verifies the official distribution with `distributionSha256Sum`. Android Gradle Plugin 8.13.0, Kotlin 2.2.20, Compose BOM 2025.09.01, OkHttp 5.1.0 and `io.github.webrtc-sdk:android:150.7871.01` are pinned.

```bash
./apps/native/android/build.sh
```

The build first invokes `scripts/native_fonts.py`, which downloads and verifies the official, unmodified Satoshi Regular/Medium/Bold/Black OTF files and Fontshare license. Generated font inputs remain ignored; the original fonts and unchanged license are bundled into the APK. It then runs JVM unit tests, Android lint, and `assembleDebug`, asserts that the Satoshi and WebRTC notices are packaged, and writes `apps/native/android/dist/Caper-android-debug.apk`. The APK is installable and debug-signed by the Android toolchain for development only. It is not a production release.

The API defaults to `https://caper.chat`. Override it at build time with `-PcaperApiBaseUrl=https://host.example` (HTTPS is required by the manifest).

Release tasks fail when signing is absent instead of producing an unsigned or debug-signed release. To sign a release, set all four variables: `CAPER_ANDROID_KEYSTORE`, `CAPER_ANDROID_KEYSTORE_PASSWORD`, `CAPER_ANDROID_KEY_ALIAS`, and `CAPER_ANDROID_KEY_PASSWORD`. Never commit those values.

## Implemented

- Guest access to the public General conversation before sign-in, followed by the web-equivalent account entry, verification, profile onboarding, session restore, and logout flows.
- Account bearer token encrypted with AES-GCM; the non-exportable AES-256 key lives in Android Keystore. Chat and media capabilities stay in process memory.
- Adaptive space rail, channel navigation, toggleable member panel, owner space/channel/member/private-grant management, paginated presence, typing, grouped messages, history pagination, and narrow-layout Browse navigation. Colors, dimensions, and bundled Satoshi typography follow the working web client.
- Idempotent HTTP sends and multiplexed gateway chat with heartbeat watchdog, durable cursor replay, reconnect backoff, fresh-history resync, and visible connection/error state.
- Access revocation and channel/logout generation guards clear prior messages and reject late results. HTTP send confirmation does not advance the gateway replay cursor. Unknown send outcomes retain their UUID and text; a matching gateway event confirms them, while definitive rejection unlocks a new operation.
- An experimental native WebRTC implementation covers Cloudflare's SFU join/publish/subscribe contract, live roster snapshots, lease renewal, mute/deafen state with prior mute intent restored after undeafen, Android communication-route selection, global output gain and per-remote local mute/gain, connection statistics, TURN renewal, restart-ICE/ack recovery, explicit call replacement, and an ongoing microphone foreground service. It is compiled but **disabled and absent from the default UI** because its device lifecycle has not been verified. Maintainers can build an explicitly experimental APK with `-PcaperEnableNativeVoice=true`; this is not a parity or production-readiness claim.

## Known voice gaps and validation boundary

The voice implementation compiles against the pinned native WebRTC SDK, but this orb has no KVM or attached physical Android device. Microphone capture, remote playback, Bluetooth/wired routing, interruptions, network handoff, lock-screen longevity, OEM battery policies, and a live Cloudflare multi-party call are **not device-verified**. The foreground service and notification implement the Android mechanism needed for an ongoing locked-screen call; manifest declarations alone are not treated as proof. Native WebRTC callbacks and teardown still need physical-device stress testing before voice can be enabled by default.

There is no incoming-call push, ringing, or invitation UI because the server has no push/incoming-call contract. Camera and screen sharing are not working web features and are not exposed. Android exposes a communication route rather than independent browser-style input/output device IDs. Global input gain, the prejoin/in-call natural-versus-enhanced mic test, and its 0–100 processing-strength pipeline remain parity gaps requiring a deliberate native capture/processing path. Detailed audio diagnostics remain unavailable; the implemented connection panel is limited to non-sensitive bitrate, loss, jitter, RTT, and direct-versus-relay statistics. None of the gaps are represented by inert controls.

## Explicit fixture and emulator smoke

Fixture behavior is available only in a separately built debug APK. Production-origin builds never silently inject fixture state, release tasks reject fixture mode, and fixture URLs must use an exact loopback host.

```bash
# Terminal 1, from repository root:
node scripts/native-parity-fixture.mjs

# Terminal 2, after an API 36 emulator is booted:
./apps/native/android/fixture-build.sh
adb wait-for-device
adb reverse tcp:3001 tcp:3001
python3 apps/native/android/smoke.py
```

Only port 3001 is needed by Android; it serves both API requests and `/api/chat/events` WebSockets. The smoke run resets the labeled local test fixture, installs `dist/Caper-android-fixture-debug.apk`, exercises guest conversation, login, one-shot actionable 503, verification, signed-in space selection, private-channel management, desktop and narrow Browse states, and writes screenshots plus UI hierarchies under `dist/ui/`. The fixture credentials are `fixture@example.test` / `ABC234`. This emulator workflow is structural visual and interaction evidence, not physical-device or live-media validation.

## Security notes

- Account credentials use `Authorization`; chat capability uses its dedicated header and media capability stays in the media subscription/body. No credential is placed in a URL or log.
- Both HTTP and WebSocket clients reject redirects, preventing cross-origin credential forwarding.
- Cleartext traffic is disabled in production-origin builds (enabled only by the explicit loopback fixture build); Android backup is disabled.
- `third_party/NOTICE-webrtc-sdk.txt` records the WebRTC SDK's BSD 3-Clause notice.
