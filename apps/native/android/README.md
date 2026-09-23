# Caper for Android

Native Android client (Kotlin and platform Jetpack Compose; no WebView or cross-platform UI). The provisional application ID is `chat.caper.android`; debug builds install as `chat.caper.android.debug` and are labeled **Caper (Development)**.

## Reproducible build

Prerequisites:

- JDK 17
- Android SDK Platform 36
- Android SDK Build Tools 36.0.0
- Android platform-tools

The checked-in wrapper pins Gradle 8.13 and verifies the official distribution with `distributionSha256Sum`. Android Gradle Plugin 8.13.0, Kotlin 2.2.20, Compose BOM 2025.09.01, OkHttp 5.1.0 and `io.github.webrtc-sdk:android:150.7871.01` are pinned.

```bash
./apps/native/android/build.sh
```

This runs JVM unit tests, Android lint, and `assembleDebug`, then writes `apps/native/android/dist/Caper-android-debug.apk`. The APK is installable and debug-signed by the Android toolchain for development only. It is not a production release.

The API defaults to `https://caper.chat`. Override it at build time with `-PcaperApiBaseUrl=https://host.example` (HTTPS is required by the manifest).

Release tasks fail when signing is absent instead of producing an unsigned or debug-signed release. To sign a release, set all four variables: `CAPER_ANDROID_KEYSTORE`, `CAPER_ANDROID_KEYSTORE_PASSWORD`, `CAPER_ANDROID_KEY_ALIAS`, and `CAPER_ANDROID_KEY_PASSWORD`. Never commit those values.

## Implemented

- Email-code login using bearer token transport, profile onboarding, session restore, and logout.
- Account bearer token encrypted with AES-GCM; the non-exportable AES-256 key lives in Android Keystore. Chat and media capabilities stay in process memory.
- Membership space/channel navigation, message history, idempotent HTTP sends, and multiplexed gateway chat with heartbeat watchdog, durable cursor replay, reconnect backoff, fresh-history resync, and visible connection/error state.
- Access revocation and channel/logout generation guards clear prior messages and reject late results. HTTP send confirmation does not advance the gateway replay cursor. Unknown send outcomes retain their UUID and text; a matching gateway event confirms them, while definitive rejection unlocks a new operation.
- An experimental native WebRTC implementation covers Cloudflare's SFU join/publish/subscribe contract, roster lease renewal, mute/deafen state, TURN credential renewal, and an ongoing microphone foreground service. It is compiled but **disabled and absent from the default UI** because its device lifecycle has not been verified. Maintainers can build an explicitly experimental APK with `-PcaperEnableNativeVoice=true`; this is not a parity or production-readiness claim.

## Known voice gaps and validation boundary

The voice implementation compiles against the pinned native WebRTC SDK, but this orb has no KVM, Android emulator, or attached physical Android device. Microphone capture, remote playback, Bluetooth/wired routing, network handoff, lock-screen longevity, OEM battery policies, and a live Cloudflare multi-party call are **not device-verified**. The foreground service and notification implement the Android mechanism needed for an ongoing locked-screen call; manifest declarations alone are not treated as proof. Remaining native WebRTC callback/teardown races must be exercised and resolved on real devices before enabling voice by default.

ICE/TURN credentials renew for an established call, but an ICE failure currently ends the call safely instead of running the server's `restart-ice`/ack recovery sequence. The user must explicitly join again. There is no incoming-call push, ringing, or invitation UI because the server has no push/incoming-call contract. Camera, screen sharing, call switching, and output-device selection are not implemented. These are parity gaps, not working-looking controls.

## Security notes

- Account credentials use `Authorization`; chat and media capabilities use their dedicated headers. No credential is placed in a URL or log.
- Both HTTP and WebSocket clients reject redirects, preventing cross-origin credential forwarding.
- Cleartext traffic and Android backup are disabled.
- `third_party/NOTICE-webrtc-sdk.txt` records the WebRTC SDK's BSD 3-Clause notice.
