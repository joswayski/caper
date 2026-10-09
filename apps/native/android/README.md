# Caper for Android

Native Android client (Kotlin and platform Jetpack Compose; no WebView or cross-platform UI). The provisional application ID is `chat.caper.android`; debug builds install as `chat.caper.android.debug` and are labeled **Caper (Development)**.

## Reproducible build

Prerequisites:

- JDK 17
- Android SDK Platform 36
- Android SDK Build Tools 36.0.0
- Android platform-tools
- Android NDK 27.2.12479018 and CMake 3.22.1 (for the native microphone DSP)
- Python 3 and network access for the pinned Fontshare acquisition step

One reproducible command-line SDK setup is:

```bash
export ANDROID_HOME="$HOME/android-sdk"
mkdir -p "$ANDROID_HOME/cmdline-tools"
# Install Google's command-line tools under $ANDROID_HOME/cmdline-tools/latest first.
yes | "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --licenses >/dev/null
"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" \
  "platform-tools" "platforms;android-36" "build-tools;36.0.0" \
  "ndk;27.2.12479018" "cmake;3.22.1"
export PATH="$ANDROID_HOME/platform-tools:$PATH"
```

The checked-in wrapper pins Gradle 8.13 and verifies the official distribution with `distributionSha256Sum`. Android Gradle Plugin 8.13.0, Kotlin 2.2.20, Compose BOM 2025.09.01, OkHttp 5.1.0 and `io.github.webrtc-sdk:android:150.7871.01` are pinned.

```bash
./apps/native/android/build.sh
```

The build first invokes `scripts/native_fonts.py`, which downloads and verifies the official, unmodified Satoshi Regular/Medium/Bold/Black OTF files and Fontshare license. It also validates the checked-in DPDFNet-8 HR model and downloads SHA-256-pinned ONNX Runtime 1.23.2 Android and RNNoise source/model archives before compiling native microphone processing. Neither model nor native code is fetched at runtime. Generated inputs remain ignored. It then runs JVM unit tests, Android lint, and `assembleDebug` with bounded native compilation, asserts that the Satoshi and WebRTC notices are packaged, and writes `apps/native/android/dist/Caper-android-debug.apk`. The APK is installable and debug-signed by the Android toolchain for development only. It is not a production release.

The API defaults to `https://caper.chat`. Override it at build time with `-PcaperApiBaseUrl=https://host.example` (HTTPS is required by the manifest).

Launcher icons always use the original green mascot. Legacy avatar aliases retain
their component names for upgrade/shortcut compatibility, but also use that icon.
The app no longer switches launcher components: fresh installs use the default,
and upgrades retain the previously enabled entry. Daily characters rotate only
in in-app wordmarks. Verify declarations with
`node scripts/generate-android-launcher-aliases.mjs --check` from the repository root;
OEM launcher caches and signed upgrades still require real-device validation.

### Push notifications (FCM)

The server sends data-only FCM messages for DMs and channel messages (phase 1 of
`docs/notifications.md`). Firebase is compiled in but only enabled when
`app/google-services.json` exists; without it `FIREBASE_ENABLED` is false and the
build works as before. CI writes that file from the base64 repository secret
`GOOGLE_SERVICES_JSON` in the native and release workflows. It must list both
`chat.caper.android` and `chat.caper.android.debug`. Never commit it or any service
credentials.

- **Turning it on.** User settings → Notifications shows "Send to this phone" and the
  "Notifications on this phone" switch only when this build has Firebase and
  `GET /api/push/config` lists `fcm`. The switch asks for the Android 13+ notification
  permission, then registers the FCM token with `POST /api/push/devices` (with the
  application ID as `appId`) for the current sign-in session. Logout unregisters it.
- **Showing a push.** `push/CaperMessagingService.kt` drops messages unless this sign-in
  session turned push on, then builds a `MessagingStyle` notification from the data:
  one per DM or channel (tagged with its ID), in the `direct_messages`,
  `channel_messages` or `mentions` notification channel. Nothing shows for the
  conversation already on screen. Tapping opens the DM or channel.
- **Controls.** The space menu, each channel's menu and a new DM row menu (⋯ or long
  press) set notification levels and mutes with `/api/notifications/settings` and the
  override routes. Changes show at once and revert with an inline error when a save
  fails. Muted spaces, channels and DMs are dimmed with a bell-slash; a muted DM has no
  unread dot.

Real-device FCM delivery, the permission prompt and denial, token rotation, taps from
the background and from a stopped app, and notification grouping have not been
validated on a device. JVM tests cover payload parsing, channel and style choice, mute
labels, settings decoding and optimistic reverts; `NotificationMenuUiTest` needs an
emulator.

Release tasks fail when signing is absent instead of producing an unsigned or debug-signed release. To sign a release, set all four variables: `CAPER_ANDROID_KEYSTORE`, `CAPER_ANDROID_KEYSTORE_PASSWORD`, `CAPER_ANDROID_KEY_ALIAS`, and `CAPER_ANDROID_KEY_PASSWORD`. Never commit those values.

## Implemented

- Signed-out first launch opens the web-equivalent account entry, verification (with "Resend code" after a 60-second wait, at most twice per email), and profile onboarding, with no Guest/General fallback. Session restoration failures stay on sign-in with an error; valid saved sessions open account spaces, and logout returns to sign-in.
- Account bearer token encrypted with AES-GCM; the non-exportable AES-256 key lives in Android Keystore. Chat and media capabilities stay in process memory.
- Adaptive space rail, channel navigation, toggleable member panel, owner space/channel/member/private-grant management (removing a member asks for confirmation), paginated presence, typing, grouped messages (same author within five minutes), `http(s)://` and `www.` links in message text that open in the browser (shared cases in `shared/messages/link-cases.json`), history pagination, and narrow-layout Browse navigation. Colors, dimensions, and bundled Satoshi typography follow the working web client.
- Account-global one-to-one DMs in the permanent channel sidebar, including exact-username start, unread/read cursors, foreground and 15-second refresh, no-space accounts, and reuse of the channel history/send/typing/gateway pipeline.
- Message requests, blocking and DM privacy. The DM section lists accepted and outgoing conversations; incoming requests sit behind a "Message requests" row with a count and never add unread dots, message sounds or DM notifications. Opening a request reads its history by id without accepting it, and the composer is replaced by Accept / Decline / Block. Outgoing requests show a waiting notice above the composer. Block / Unblock is in the DM header (the narrow channel menu) and in message actions, with a confirmation before blocking; a blocked DM replaces the composer with Unblock. Blocked authors' consecutive messages collapse into "⊘ N blocked messages — Show" rows in channel, DM and thread timelines (revealing is in memory only), and they never chime or show as typing. User settings → Privacy and blocked accounts holds "Who can start a DM with you" (saved immediately, reverted on failure) and the blocked list. `dm_not_accepted` / `dm_blocked` refusals keep the conversation and show their own copy. Pinned-message lists still show blocked authors' pins.
- Idempotent HTTP sends and multiplexed gateway chat with heartbeat watchdog, durable cursor replay, reconnect backoff, fresh-history resync, and visible connection/error state.
- Access revocation and channel/logout generation guards clear prior messages and reject late results. HTTP send confirmation does not advance the gateway replay cursor. Unknown send outcomes retain their UUID and text; a matching gateway event confirms them, while definitive rejection unlocks a new operation.
- Native WebRTC voice is available from the default Android UI after explicit microphone permission. It implements Cloudflare SFU join/publish/subscribe, roster snapshots, lease renewal, mute/deafen with prior intent restored, Android communication-route selection, input/output gain, per-remote local mute/gain, bounded DPDFNet-8 HR with RNNoise fallback and voice shaping, prejoin/in-call local mic comparison, sanitized debug-gated processing diagnostics, connection statistics, TURN renewal, restart-ICE/ack recovery, call replacement, and an ongoing microphone foreground service. These mechanisms are not a claim of live or device-verified parity.

## Known voice gaps and validation boundary

The voice implementation targets the pinned native WebRTC SDK, but this orb has no KVM or attached physical Android device. Microphone capture, remote playback, Bluetooth/wired routing, interruptions, network handoff, lock-screen longevity, OEM battery policies, and a live Cloudflare multi-party call are **not device-verified**. The foreground service and notification implement the Android mechanism needed for an ongoing locked-screen call; manifest declarations alone are not treated as proof. Native WebRTC callbacks and teardown still need physical-device stress testing before voice can be considered production-accepted.

There is no incoming-call push, ringing, or invitation UI because the server has no incoming-call contract. Camera and screen sharing are not working web features and are not exposed. Android exposes a communication route rather than independent browser-style input/output device IDs. Web deliberately suppresses interaction sounds on coarse-pointer/mobile devices, so Android does not add a separate sounds setting. Connection diagnostics are non-sensitive bitrate, loss, jitter, RTT, and direct-versus-relay statistics; detailed microphone processing timing/mode is visible only for accounts with `debugEnabled`. Physical capture, processing fallback, actual route changes, and local comparison playback still need device acceptance.

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

Only port 3001 is needed by Android; it serves both API requests and `/api/chat/events` WebSockets. The smoke run resets the labeled local test fixture, installs `dist/Caper-android-fixture-debug.apk`, exercises fresh-install login, one-shot actionable 503, verification, failed/successful session restoration, signed-in space selection, private-channel management, desktop and narrow Browse states, and writes screenshots plus UI hierarchies under `dist/ui/`. The fixture credentials are `fixture@example.test` / `ABC234`. This emulator workflow is structural visual and interaction evidence, not physical-device or live-media validation. The default fixture has no message request; `POST /__fixture/control {"messageRequest":{}}` adds Jordan's incoming request, and `PUT /api/blocks/member000001` collapses Maya's two seeded #general messages. `smoke.py` does not yet exercise requests or blocking. `POST /__fixture/control {"pushPlatforms":["fcm"]}` advertises FCM and `GET /__fixture/push-devices` reads back registrations; the switch only appears in a fixture APK built with `app/google-services.json`. Notification settings and overrides are kept in the fixture's memory. `smoke.py` does not exercise them yet.

## Security notes

- Account credentials use `Authorization`; chat capability uses its dedicated header and media capability stays in the media subscription/body. No credential is placed in a URL or log.
- Both HTTP and WebSocket clients reject redirects, preventing cross-origin credential forwarding.
- Cleartext traffic is disabled in production-origin builds (enabled only by the explicit loopback fixture build); Android backup is disabled.
- `third_party/NOTICE-webrtc-sdk.txt` records the WebRTC SDK's BSD 3-Clause notice.
- `third_party/NOTICE-Lucide.txt` records the ISC notice for the Lucide icon vectors in `app/src/main/res/drawable/lucide_*.xml`.
