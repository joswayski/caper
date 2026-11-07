# Caper for Android

Native Android client (Kotlin and platform Jetpack Compose; no WebView or cross-platform UI, apart from the isolated WebView that shows KLIPY ads). The provisional application ID is `chat.caper.android`; debug builds install as `chat.caper.android.debug` and are labeled **Caper (Development)**.

## Reproducible build

Prerequisites:

- JDK 17
- Android SDK Platform 36
- Android SDK Build Tools 36.0.0
- Android platform-tools
- Android NDK 27.2.12479018 and CMake 3.22.1 (for the native microphone DSP and AVIF encoder)
- Python 3 and network access for the pinned Fontshare acquisition step
- Rust (rustup; `rust-toolchain.toml` pins the version) with the `aarch64-linux-android`, `armv7-linux-androideabi`, `i686-linux-android` and `x86_64-linux-android` targets, and `cargo install cargo-ndk --locked --version 4.1.2`, for the shared Rust library

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

The checked-in wrapper pins Gradle 9.1.0 and verifies the official distribution with `distributionSha256Sum`. Android Gradle Plugin 9.0.1 uses built-in Kotlin with the compiler, Compose compiler plugin and serialization plugin pinned to Kotlin 2.2.20. Compose BOM 2025.09.01, OkHttp 5.1.0, Coil 3.3.0, Media3 1.8.0 and `io.github.webrtc-sdk:android:150.7871.01` are pinned.

```bash
./apps/native/android/build.sh
```

The build:

1. runs `scripts/native_fonts.py`, which downloads and verifies the official, unmodified Satoshi Regular/Medium/Bold/Black OTF files and Fontshare license;
2. validates the checked-in DPDFNet-8 HR model and downloads SHA-256-pinned ONNX Runtime 1.23.2 Android and RNNoise source/model archives for native microphone processing;
3. runs `prepare-avif.sh`, which builds the AVIF photo encoder from pinned libaom 3.15.1 (SHA-256) and libavif 1.4.2 (commit) sources (BSD-2-Clause; notices in `third_party/`);
4. builds `libcaper_mobile.so`, the shared Rust logic in [`../mobile`](../mobile/README.md), with cargo-ndk for every ABI (and once for this machine, for the unit tests);
5. runs JVM unit tests, Android lint, and `assembleDebug`, asserts that the Satoshi and WebRTC notices are packaged, and writes `apps/native/android/dist/Caper-android-debug.apk` (debug-signed, for development only).

Neither model nor native code is fetched at runtime. Generated inputs are ignored.

The API defaults to `https://caper.chat`. Override it at build time with `-PcaperApiBaseUrl=https://host.example` (HTTPS is required by the manifest).

### Release signing and optimization

Release tasks fail when signing is absent instead of producing an unsigned or debug-signed release. Set all four variables: `CAPER_ANDROID_KEYSTORE`, `CAPER_ANDROID_KEYSTORE_PASSWORD`, `CAPER_ANDROID_KEY_ALIAS`, and `CAPER_ANDROID_KEY_PASSWORD`. Never commit those values.

Release builds use R8 full mode, code shrinking, resource shrinking (`isShrinkResources = true`) and class repackaging; debug builds stay unminified. The WebRTC/JNI keep rules must remain because native libraries look up those classes by name; `build.sh` and the release workflow validate those names against R8's mapping and removal reports.

Play Console's optimization/shrinking percentages describe code subject to those transformations, not speed or download size. Inspect a locally built bundle with:

```bash
unzip -l app/build/outputs/bundle/release/app-release.aab | grep -E '\.dex$' | awk '{sum+=$1} END {print sum " uncompressed DEX bytes"}'
unzip -p app/build/outputs/bundle/release/app-release.aab BUNDLE-METADATA/com.android.tools/r8.json
```

### Launcher icons

Launcher icons use the original green mascot. Legacy avatar aliases keep their component names for upgrade/shortcut compatibility but use the same icon; fresh installs use the default entry and upgrades keep the previously enabled one. Verify declarations with `node scripts/generate-android-launcher-aliases.mjs --check` from the repository root.

### Push notifications (FCM)

The server sends data-only FCM messages for DMs and channel messages (`docs/notifications.md`). Firebase is compiled in but enabled only when `app/google-services.json` exists; otherwise `FIREBASE_ENABLED` is false. CI writes that file from the base64 repository secret `GOOGLE_SERVICES_JSON` in the native and release workflows. It must list both `chat.caper.android` and `chat.caper.android.debug`. Never commit it or any service credentials.

- **Turning it on.** User settings → Notifications shows "Send to this phone" and the "Notifications on this phone" switch only when the build has Firebase and `GET /api/push/config` lists `fcm`. The switch asks for the Android 13+ notification permission, then registers the token with `POST /api/push/devices` (application ID as `appId`) for the current sign-in session. Logout unregisters it.
- **Showing a push.** `push/CaperMessagingService.kt` drops messages unless this session turned push on, then builds one `MessagingStyle` notification per DM or channel in the `direct_messages`, `channel_messages` or `mentions` channel, skipping the conversation on screen. Taps open the DM or channel; a `feedback` report opens the visit at the message with its thread (`threadRootId`, or the message's own).
- **Invitations** use the `invitations` channel and **message requests** the `message_requests` channel (without message content); taps open Accept/Decline review.
- **Controls.** The space menu, each channel's menu and the DM row menu set levels and mutes through `/api/notifications/settings` and the override routes. Changes apply at once and revert with an inline error when a save fails.

## Features

- Email sign-in (with "Resend code" after 60 seconds, at most twice per email) and profile onboarding; no Guest/General fallback.
- Account bearer token encrypted with AES-GCM; the non-exportable AES-256 key lives in Android Keystore. Chat and media capabilities stay in process memory.
- Adaptive space rail, channel navigation, member panel, owner space/channel/member/private-grant management, presence, typing, grouped messages, links (shared cases in `shared/messages/link-cases.json`), history pagination and narrow Browse navigation.
- Invite links: owners create, copy and revoke them in Manage space; anyone can join from "Join with invite link". `https://caper.chat/invite/<code>` App Links and `caper://invite/<code>` open the join screen; a link opened while signed out waits in memory until sign-in completes. Verified App Links need the Play app-signing certificate's SHA-256 listed for `chat.caper.android` in the web's `/.well-known/assetlinks.json` (debug builds verify only if `chat.caper.android.debug` and its certificate are listed too); otherwise links open the browser's invite page.
- Direct messages, message requests (Accept/Decline/Block), blocking with collapsed "N blocked messages" rows, and Privacy and blocked accounts settings.
- Username suggestions in invite, grant and New DM fields; `:` emoji and `@` people suggestions; your own profile card offers **Edit profile**.
- Caper team names notify the team; `feedbackTeam` accounts get a Feedback inbox with archive/restore and visits (`visitor: true`).
- Attachments when `GET /api/assets/usage` succeeds: the attach drawer (`AttachDrawer.kt`; see docs/media.md "Attach drawer"), document picker and drag and drop, up to 10 files. The device compresses with the server's `compression` settings (docs/media.md "Client compression and previews") and strips location metadata from originals. AVIF decodes natively on Android 12+ and through the bundled AOMedia libavif decoder on Android 8–11. Media viewer (`MediaViewer.kt`) with zoom, GIF/animated WebP, Media3 video, and DownloadManager saves.
- Attachment editor (`AttachmentEditor.kt`, `data/AttachmentEdits.kt`, geometry shared with `shared/messages/edit-cases.json`): spoiler, crop, rotate, draw (with eraser) and text. Videos transcode with Media3 (HDR tone mapped on Android 10+).
- KLIPY GIFs and stickers (`GifViews.kt`, `data/Klipy.kt`, `model/Klipy.kt`; docs/media.md "GIFs and stickers (KLIPY)") when `GET /api/klipy` answers. Ads render in a WebView with scripts but no cookies, storage, file access, popups or app bridge.
- Voice messages (`VoiceMessageViews.kt`, `data/VoiceMessages.kt`, `voice/VoiceNoteCapture.kt`): a tap on the mic dictates (Done sends the recording to `POST /api/dictation` and inserts the words at the caret; "Voice note" keeps it as a voice message; a failure keeps the audio for Retry or "Keep as voice note"), and press-and-hold records a voice message up to 10:00 through the call's microphone processing (AudioRecord `VOICE_COMMUNICATION` → input gain → DPDFNet-8 HR/RNNoise → voice DSP), encoded with MediaCodec as AAC-LC MPEG-4 (mono, 48 kHz, 64 kbps), with a MediaRecorder fallback (44.1 kHz). Recordings stage as a chip or, with "Send voice messages as soon as I stop recording" (Audio settings), send at once. Before Android 10 the microphone is unavailable during a call.
- Custom profile pictures and space icons (GIFs up to 5 MB/1024 px unchanged, larger ones shrunk by `GifShrink`; others 512 px WebP within 1 MB). The `profiles` gateway feed applies `profile.updated` and `space.updated` live.
- Idempotent sends and gateway chat with heartbeat watchdog, cursor replay, reconnect backoff and resync. Access revocation and logout guards clear prior messages and reject late results.
- Native WebRTC voice after microphone permission: Cloudflare SFU join/publish/subscribe, roster and lease snapshots, mute/deafen, Android communication-route selection, input/output gain, per-person local mute/gain, DPDFNet-8 HR with RNNoise fallback and voice shaping, local mic comparison, connection statistics (processing diagnostics only for `debugEnabled` accounts), TURN renewal, ICE restart, and an ongoing-call microphone foreground service. Voice in the channel list follows the shared layout in [apps/native](../README.md#layout-rules).

Android exposes a communication route rather than browser-style input/output device IDs, and has no interaction sounds setting (web suppresses them on mobile). There is no incoming-call ringing, camera, or screen sharing.

## Fixture and emulator smoke

Fixture behavior exists only in a separately built debug APK. Release tasks reject fixture mode, and fixture URLs must use an exact loopback host.

```bash
# Terminal 1, from repository root:
node scripts/native-parity-fixture.mjs

# Terminal 2, after an API 36 emulator is booted:
./apps/native/android/fixture-build.sh
adb wait-for-device
adb reverse tcp:3001 tcp:3001
python3 apps/native/android/smoke.py
```

Android needs only port 3001 (API and `/api/chat/events`). The smoke run resets the fixture, installs `dist/Caper-android-fixture-debug.apk`, walks sign-in, error, restoration, space selection, private-channel management and Browse states, and writes screenshots and UI hierarchies under `dist/ui/`. Fixture credentials are `fixture@example.test` / `ABC234`.

Extra fixture controls: `POST /__fixture/control {"messageRequest":{}}` adds an incoming request; `PUT /api/blocks/member000001` blocks Maya; `POST /__fixture/control {"pushPlatforms":["fcm"]}` advertises FCM and `GET /__fixture/push-devices` reads back registrations (the switch needs a fixture APK built with `app/google-services.json`).

## Performance: Baseline Profile and macrobenchmarks

`:benchmark` holds Macrobenchmarks for cold and warm start, opening the 3,000-message `#general`, scrolling back through its history, typing, and typing into a long formatted draft with links, each run without ahead-of-time compilation (`None`) and with the Baseline Profile (`Partial`). Its `BaselineProfileGenerator` walks the same journey to produce the app's Baseline Profile; `profileinstaller` installs it (and Compose's own profiles) on sideloaded APKs. Cold start reports both first frame and "fully drawn" (`ReportDrawnWhen`: past the brand screen and the first history page).

They run on a physical phone or an arm64 emulator (API 28+; API 33+ to generate the profile without root) against the app's `benchmarkRelease` build: release code and R8, debug-signed, installed beside the shipped app as `chat.caper.android.benchmark`, talking to `http://localhost:3001`.

```bash
# Terminal 1, from the repository root:
CAPER_BENCH_MESSAGES=3000 node scripts/native-parity-fixture.mjs

# Terminal 2, with the phone connected (prepare-fonts.sh and prepare-audio.sh first, as for build.sh):
adb reverse tcp:3001 tcp:3001
cd apps/native/android
./gradlew :benchmark:connectedBenchmarkReleaseAndroidTest   # results in benchmark/build/outputs/connected_android_test_additional_output/
./gradlew :app:generateBaselineProfile                       # writes app/src/release/generated/baselineProfiles/; commit it
```

On an emulator, add `-Pandroid.testInstrumentationRunnerArguments.androidx.benchmark.suppressErrors=EMULATOR`; its numbers compare runs on the same emulator, not phones.

Builds never regenerate the profile; regenerate and commit it after large UI changes.

## Security notes

- Account credentials use `Authorization`; chat capability uses its dedicated header and media capability stays in the media subscription/body. No credential is placed in a URL or log.
- Both HTTP and WebSocket clients reject redirects, preventing cross-origin credential forwarding.
- Cleartext traffic is disabled in production-origin builds (enabled only by the explicit loopback fixture build); Android backup is disabled.
- `third_party/NOTICE-webrtc-sdk.txt` records the WebRTC SDK's BSD 3-Clause notice.
- `third_party/NOTICE-Lucide.txt` records the ISC notice for the Lucide icon vectors in `app/src/main/res/drawable/lucide_*.xml`.
