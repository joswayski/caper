# Caper for Android

Native Android client (Kotlin and platform Jetpack Compose; no WebView or cross-platform UI). The provisional application ID is `chat.caper.android`; debug builds install as `chat.caper.android.debug` and are labeled **Caper (Development)**.

## Reproducible build

Prerequisites:

- JDK 17
- Android SDK Platform 36
- Android SDK Build Tools 36.0.0
- Android platform-tools
- Android NDK 27.2.12479018 and CMake 3.22.1 (for the native microphone DSP and AVIF encoder)
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

The checked-in wrapper pins Gradle 9.1.0 and verifies the official distribution with `distributionSha256Sum`. Android Gradle Plugin 9.0.1 uses built-in Kotlin with the compiler, Compose compiler plugin and serialization plugin pinned to Kotlin 2.2.20. Compose BOM 2025.09.01, OkHttp 5.1.0, Coil 3.3.0, Media3 1.8.0 and `io.github.webrtc-sdk:android:150.7871.01` remain pinned. JDK 17, SDK 36 and the explicit NDK version are unchanged.

```bash
./apps/native/android/build.sh
```

The build first invokes `scripts/native_fonts.py`, which downloads and verifies the official, unmodified Satoshi Regular/Medium/Bold/Black OTF files and Fontshare license. It also validates the checked-in DPDFNet-8 HR model and downloads SHA-256-pinned ONNX Runtime 1.23.2 Android and RNNoise source/model archives before compiling native microphone processing, and `prepare-avif.sh` builds the AVIF photo encoder from pinned libaom 3.15.1 (SHA-256) and libavif 1.4.2 (commit) sources (BSD-2-Clause; notices in `third_party/`). Neither model nor native code is fetched at runtime. Generated inputs remain ignored. It then runs JVM unit tests, Android lint, and `assembleDebug` with bounded native compilation, asserts that the Satoshi and WebRTC notices are packaged, and writes `apps/native/android/dist/Caper-android-debug.apk`. The APK is installable and debug-signed by the Android toolchain for development only. It is not a production release.

### Release optimization and Play Console metrics

Release builds use R8 full mode, code shrinking, optimized resource shrinking and class repackaging. AGP 9 enables the integrated code/resource shrinking pipeline; release resource shrinking still needs `isShrinkResources = true`. Repackaging shortens package names, but the WebRTC/JNI keep rules must remain: native libraries look up those classes by name. `build.sh` and the signed release workflow validate those names against R8's mapping and removal reports. Debug builds stay unminified.

Play Console's optimization, obfuscation and shrinking percentages describe code subject to those transformations, not runtime speed gains or the percentage of bytes removed. Total uncompressed DEX size covers compiled Java/Kotlin code, excluding native libraries and assets; it is not the user's download size. Inspect a locally built release bundle with:

```bash
unzip -l app/build/outputs/bundle/release/app-release.aab | grep -E '\.dex$' | awk '{sum+=$1} END {print sum " uncompressed DEX bytes"}'
unzip -p app/build/outputs/bundle/release/app-release.aab BUNDLE-METADATA/com.android.tools/r8.json
```

The bundle requires the release-signing variables described below. Before publishing the next release, test the actual minified, signed build on physical devices: sign-in/session restoration, text and push, voice/WebRTC and native microphone processing, attachment decoding/encoding and media playback. APK/ELF alignment checks do not establish runtime support on a 16 KB-page device. A smaller DEX does not by itself prove faster startup or rendering.

The API defaults to `https://caper.chat`. Override it at build time with `-PcaperApiBaseUrl=https://host.example` (HTTPS is required by the manifest).

Launcher icons always use the original green mascot. Legacy avatar aliases retain
their component names for upgrade/shortcut compatibility, but also use that icon.
The app no longer switches launcher components: fresh installs use the default,
and upgrades retain the previously enabled entry. Wordmarks also stay on the
original green Caper; daily characters rotate only in file-drop illustrations. Verify declarations with
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
  conversation already on screen. Tapping opens the DM or channel; a `feedback` report
  (shown in the `mentions` channel) opens the visit view at the message with its thread
  (`threadRootId`, or the message's own) open.
- **Invitations.** Space/private-channel invitations use the `invitations` OS
  channel with their own IDs and text/event style. Tapping refreshes pending
  invitations and opens Accept/Decline review, not private history or voice.
- **Message requests.** Each new incoming DM request sends one text/event alert
  in `message_requests`, without message content. Tapping opens the existing
  read-only request review; pending messages stay quiet until acceptance.
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

- Your own name, avatar or resolved mention in a conversation opens a profile card with **Edit profile**, which opens the existing picture, username and display-name editor. Closing or saving returns to the conversation. This entry point still needs Android UI/device validation.
- Signed-out first launch opens the web-equivalent account entry, verification (with "Resend code" after a 60-second wait, at most twice per email), and profile onboarding, with no Guest/General fallback. Session restoration failures stay on sign-in with an error; valid saved sessions open account spaces, and logout returns to sign-in.
- Account bearer token encrypted with AES-GCM; the non-exportable AES-256 key lives in Android Keystore. Chat and media capabilities stay in process memory.
- Adaptive space rail, channel navigation, toggleable member panel, owner space/channel/member/private-grant management (removing a member asks for confirmation), paginated presence, typing, grouped messages (same author within five minutes), `http(s)://` and `www.` links in message text that open in the browser (shared cases in `shared/messages/link-cases.json`), history pagination, and narrow-layout Browse navigation. Colors, dimensions, and bundled Satoshi typography follow the working web client.
- Invite links. Owners create links in Manage space (expiry, max uses and an optional public landing channel; the new link is copied at once), then copy or revoke the live ones. Anyone signed in can use "Join with invite link" (the space rail and the first-space screen): paste a link on any host, `caper.chat/invite/<code>` or a bare code, preview the space, then join and open it. Invite links opened from other apps (`https://caper.chat/invite/<code>` App Links and `caper://invite/<code>`, which the web invite page's "Open in the Caper app" uses) open "Join with invite link" filled in and load the preview; a link opened while signed out or before the profile is finished waits in memory until sign-in completes, and a newer link replaces it. Verified App Links need the Play app-signing certificate's SHA-256 listed for `chat.caper.android` in the web's `/.well-known/assetlinks.json` (debug builds, `chat.caper.android.debug`, verify only if that package and its signing certificate are listed too); until then Play installs open `https://caper.chat/invite/…` links in the browser, whose invite page has "Open in the Caper app". Link handling is not device-verified.
- Account-global one-to-one DMs in the permanent channel sidebar, including exact-username start, unread/read cursors, foreground and 15-second refresh, no-space accounts, and reuse of the channel history/send/typing/gateway pipeline.
- Username suggestions in the space invite, private-channel grant and New DM fields: people the app already has (known people minus members, space members minus grantees, known people), ranked like `@` suggestions; pending invitees stay listed as "Already invited". Rows show the avatar and display name with `@username` beneath it, as `@` popup rows now do. Choosing one fills the exact username without submitting; hardware Up/Down, Enter and Escape move, pick and close the list.
- File attachments when the server enables uploads (`GET /api/assets/usage` succeeds): a composer attach button and drag and drop from other apps with an overlay (system photo picker for photos/videos, document picker for any file, up to 10 per message) with draft chips showing thumbnail, name, `X MB → Y KB`, compression/upload progress and remove. The device compresses with the server's `compression` settings (docs/media.md "Client compression and previews"): lossless stills (PNG, BMP, lossless WebP) stay lossless — exact indexed PNG within `paletteColors`, else lossless WebP on Android 11+ when smaller, else the original; photos (JPEG, HEIC/HEIF, lossy WebP) become AVIF at `avifQuality` (libavif quality, 4:2:0 8-bit, speed 8, off the UI thread) when the server sends `imageFormat: "avif"`, else, or if AVIF encoding fails, lossy WebP at `imageQuality`, within `imageMaxEdge` and only when at least 10% smaller (HEIC always converts; EXIF orientation applied, EXIF/GPS dropped; a missing `imageFormat` means WebP); videos are transcoded with Media3 Transformer to H.264/AAC MP4 only when the short edge exceeds `videoMaxHeight`, the codec is not H.264, or the bitrate is over 1.25× the pixel-scaled target (size-only transcodes must save 10%, audio is never dropped, HDR is tone mapped to SDR with OpenGL on Android 10+ or uploaded unchanged, any failure keeps the original). Files uploaded as originals lose location metadata losslessly (JPEG Exif/XMP with orientation kept, PNG text/Exif chunks, zero-filled MP4/MOV `udta`/`meta` boxes). GIF, SVG, AVIF, audio and documents upload unchanged; images and video posters get previews. The client reserves, PUTs preview and file straight to storage with only the presigned headers (no app credentials, no redirects), retries `/complete` on 409, and sends `attachmentIds`. Messages render by `status` (dormant server-processing support; absent means ready): `processing` shows a placeholder sized from width/height with the `previewUrl` (or this device's own picked image), a spinner and the `attachment.progress` percent; `failed` shows "Couldn’t process this file"; ready files show images (reserved aspect ratio), videos, `animated` videos inline muted and looping without controls, audio (in-app Media3 playback), file cards (opened in the browser, which handles `Content-Encoding: gzip`), and "File removed" placeholders. `message.attachments` events replace a message's files with the same sequencing as reactions (`attachmentsSeq`); malformed attachment entries are skipped. AVIF decodes natively on Android 12+; Android 8–11 use the bundled AOMedia libavif decoder (about 0.9 MB per ABI), with the WebP preview as the full-size fallback if decoding fails. Signed URLs are refreshed through `POST /api/assets/urls` before they expire and once after a 403/404, with images cached by attachment ID (Coil 3, Media3 1.8.0). Tapping a ready image or video opens a full-screen media viewer (`MediaViewer.kt`) on that message's images and videos in message order: swipe, the arrow buttons or keyboard Left/Right move between them with an `n / m` counter; images show the preview until the original loads at full resolution, zoom with pinch or a double-tap (2×) and pan while zoomed, and GIFs/animated WebP play (`coil-gif`, viewer only); videos autoplay with Media3 controls (poster until the first frame) and pause when paged away; Download saves through DownloadManager (shared Downloads on Android 10+, the app's own Downloads folder on 8–9), Open in browser opens the signed URL, and Close, back or a swipe down closes. Load errors use the same once-per-file URL refresh, and a file removed while open shows "File removed".
- Custom profile pictures and space icons. Edit profile and (for owners) Manage space show the picture in a frame with an edit badge on its corner; the frame opens the system photo picker (images only), or with a custom picture set a menu to choose another or remove it. Without one, your mascot holds up a "change me!" sign (`caper_change_me`). GIFs (by magic bytes) upload unchanged within 5 MB and 1024 × 1024, and bigger ones are shrunk by `GifShrink` (the web's algorithm, pure Kotlin) to a square that fits, frames and timing kept; anything else is oriented (ImageDecoder on Android 9+, BitmapFactory with EXIF below), center-cropped to a square of at most 512 px and encoded as lossy WebP (quality 90) within 1 MB. Uploads use the attachment presigned PUT (`purpose` `avatar`/`spaceIcon`), and the server applies the picture on `/complete`; the returned account goes through the own-profile refresh, so members, authors, pins, threads, replies, DMs, reactions, profile cards and your voice row update at once, and the returned space updates its rail tile. Every avatar shows `avatarUrl` aspect-filled in a rounded square (`RoundedCornerShape(percent = 30)`, web's `--radius-avatar`) over the saved mascot, which keeps its circle while the picture loads or if it fails; speaking rings and voice-stack outlines follow the shape that shows. Rail, invitation and invite-preview tiles show `iconUrl` over the initial. Coil caches by `avatarAsset`/`iconAsset` (never the re-signed URL), decodes near display size, animates GIFs and animated WebP everywhere, and shows a still first frame under Remove animations. While signed in, the conversation's gateway socket also subscribes to `profiles` (again after each reconnect): `profile.updated` updates that person by account ID in members, invitations, DM peers and titles, suggestions, blocked accounts, message authors (pins, threads, replies, forwards), cached conversations, typing, reactor lists and open profile cards, and your own on another device also updates the account (and its chat session after a rename); `space.updated` renames and re-icons that space in the rail, invitations and the open space. An older gateway's `error` for the kind turns it off for that socket's client without touching anything else. The feed has no replay, so refreshed lists (space members, DM peers, people, blocks) also repair a missed event: any loaded or cached message, or other list, still showing an older name or picture takes the fetched profile, unless an event for that person arrived while the request was in flight. Not yet built or checked on a device or emulator.
- Message requests, blocking and DM privacy. The DM section lists accepted and outgoing conversations; incoming requests sit behind a "Message requests" row with a count and never add unread dots, message sounds or DM notifications. Opening a request reads its history by id without accepting it, and the composer is replaced by Accept / Decline / Block. Outgoing requests show a waiting notice above the composer. Block / Unblock is in the DM header (the narrow channel menu) and in message actions, with a confirmation before blocking; a blocked DM replaces the composer with Unblock. Blocked authors' consecutive messages collapse into "N blocked messages — Show" rows behind a Ban icon in channel, DM and thread timelines (revealing is in memory only), and they never chime or show as typing. User settings → Privacy and blocked accounts holds "Who can start a DM with you" (saved immediately, reverted on failure) and the blocked list. `dm_not_accepted` / `dm_blocked` refusals keep the conversation and show their own copy. Pinned-message lists still show blocked authors' pins.
- Caper team mentions in any conversation: `@feedback`, `@caper`, `@admin`, `@mod`, `@support` and the server's other reserved team names. A bare `@` lists `@feedback` ("Notify the Caper team") first; a typed prefix lists people first and then the first matching team name, inserted as named (`@ad` → `@admin`). With the server's `feedback` mention every team name renders as a pill that opens no profile, and the message shows "Caper team has been notified 🫡". Messages from feedback team members (`author.admin`) carry a gold ADMIN badge. New message accepts team names (the server opens the Caper DM, whose peer has `team: true`); its empty state reads "Messages here go to the Caper team." Accounts with `feedbackTeam` get a Feedback row above the DMs that opens the inbox (`GET /api/feedback`, newest first, pull to refresh, older pages): reporter, "#channel · Space" or "DM · names", a preview or "Sent N files", a relative time and Answered/Waiting (Answered while the thread's newest message is the team's). A long press (or, with a mouse, a hover button, or the accessibility action "Archive feedback from <name>") archives the whole thread for the team (`PUT /api/feedback/archive`): it leaves the list at once and the list reloads, keeping any error on show. "Show archived" at the bottom switches to "Archived feedback" (`?archived=true`) with "Restore to the inbox" instead and no status; "Back to feedback" returns. A report opens its conversation by ID as a visit (`visitor: true`), where the team member takes part like a member: timeline, threads, replies (with "Also send to"), files, reactions, edits, pins, typing, forwarding, live over the gateway, and for a space channel the same voice as any channel (while people are in its voice, a room card above the channels with Join or Switch, since a visited channel has no row, and on phones the strip under the header; DMs have no voice). There is no member list, joining, channel settings, notification menu or read marker. It loads around the report; top-level reports stay in the conversation, and only reported replies open their existing thread. Back returns to the inbox. Conversations the team member is in open the usual way. Mention suggestions in a visit are the reporter and people the loaded messages name. Not yet verified on a device or emulator.
- Idempotent HTTP sends and multiplexed gateway chat with heartbeat watchdog, durable cursor replay, reconnect backoff, fresh-history resync, and visible connection/error state.
- Access revocation and channel/logout generation guards clear prior messages and reject late results. HTTP send confirmation does not advance the gateway replay cursor. Unknown send outcomes retain their UUID and text; a matching gateway event confirms them, while definitive rejection unlocks a new operation.
- Native WebRTC voice is available from the default Android UI after explicit microphone permission. Its controls follow "Live now" (design H, revised) in the channel list, with no "Live now" heading. Your call's channel row grows in place into your call card: the row (name, unread dot, mute bell, ⋯ menu, selected state) ending in the timer (amber Connecting…/Reconnecting… until connected), then up to two rows of faces (you first, then join order; speaking rings and mute/deafen badges; a "+N" tile past ten), a muted participant count ("3 people"), then Mute, Deafen and Leave (Cancel while joining). A tap anywhere on it but those controls and the menu opens the chat; a long press on a face or "+N", or its accessibility action, opens the full list with per-person volume and Mute for me (a bottom sheet on phones, a popover beside the sidebar on wider layouts), already open for that face. Another live room keeps its row (waveform and count, quick-join rules unchanged) and gains a body with three faces, one line of names, the room's timer and Join or Switch; a tap anywhere else on it opens its chat. Rows keep the list's order, and collapsing Channels hides quiet rows only. Voice with no row in the list (your call in another space or a visited channel, a feedback visit's live room) sits in a card above Channels. Nothing names who is speaking; the faces' green rings show it. Quiet rows keep a headphones join, and on phones a strip under the chat header shows your call's channel, the card's faces in its order (up to five, otherwise four and "+N"), Connecting…/Reconnecting… until connected, the timer and Mute, and opens Browse; the conversation header has no voice. The revised layout's Compose UI tests and `smoke.py` assertions have not yet run on an emulator or device. It implements Cloudflare SFU join/publish/subscribe, roster snapshots, lease renewal, mute/deafen with prior intent restored, Android communication-route selection, input/output gain, per-remote local mute/gain, bounded DPDFNet-8 HR with RNNoise fallback and voice shaping, prejoin/in-call local mic comparison, sanitized debug-gated processing diagnostics, connection statistics, TURN renewal, restart-ICE/ack recovery, call replacement, and an ongoing microphone foreground service. These mechanisms are not a claim of live or device-verified parity.

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
