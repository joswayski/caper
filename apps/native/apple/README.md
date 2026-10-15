# Caper for Apple platforms

Native SwiftUI clients backed by AppKit on macOS and UIKit on iPhone. They contain no Electron, Tauri, WebView, or browser content. This follows the Captures native direction, but is not a direct port of Captures' custom AppKit views.

## Current implementation

- Your own name, avatar or resolved mention in a conversation opens a profile card with **Edit profile**, which opens the existing picture, username and display-name editor. Closing or saving returns to the conversation. The shared view also supports macOS; this entry point still needs Apple UI/device validation.
- Signed-out first launch opens passwordless email sign-in, then onboarding/profile, with no Guest/General fallback. Logout returns to sign-in; valid saved sessions restore through the origin-namespaced Keychain.
- Packaged macOS releases offer **Check for updates** in the account settings menu, Settings and the Caper application menu. Automatic checks reveal a persistent **Update available** button above the fixed-height profile/audio row, with no hiding animation. Clicking it or finding an update manually opens a centered modal with **What’s new expanded**, scrolling notes and fixed restart/Later/installer actions. Later, Escape and the backdrop close it without forgetting the update; restart still warns about leaving calls. Current-version/errors retain native alerts. Development builds and fixtures never run the updater. iPhone updates remain managed by the App Store/TestFlight.
- The update button has a thin rainbow edge and an occasional light sweep. Its label and hit target stay still; Reduce Motion and background windows stop the sweep.
- The caper.chat shell at desktop and narrow widths: space rail, channel sidebar, toggleable responsive member panel, Browse navigation, conversation stage, Satoshi typography, and the web color/spacing tokens.
- Space/channel/member owner workflows: create/delete/rename spaces and channels, add/remove existing accounts by username, private-channel toggle and grants.
- Invite links: in Manage space, owners create links (`<API origin>/invite/<code>`) with an expiry, a use limit and an optional public landing channel, then copy or revoke live ones; a new link is copied automatically. Anyone signed in can paste a link, `caper.chat/invite/<code>` or a bare code into **Join with invite link** (in Create a space, reached from the rail's +, or on the first-space screen), preview the space and join it.
- Opening invite links from other apps: iPhone opens `https://caper.chat/invite/<code>` links directly (universal links, from the `applinks:caper.chat` Associated Domains entitlement), and both apps register the `caper://invite/<code>` scheme that the web invite page's "Open in the Caper app" uses. The Mac app ships as a Developer ID DMG without a provisioning profile, so it opens only `caper://` links. An opened link shows **Join with invite link** with its preview already loading; when signed out or before the profile is finished, it waits (in memory only) until sign-in completes, and a newer link replaces it. Universal links need the Associated Domains capability on the `chat.caper.ios` App ID: automatic signing with `-allowProvisioningUpdates` (`upload-testflight.sh`) normally enables it, but if a TestFlight upload fails on the provisioning profile, enable it for that App ID in the Apple Developer portal. caper.chat must also serve `/.well-known/apple-app-site-association`; signed-device acceptance of both link kinds is still required.
- HTTP chat history and pagination, idempotent sends, ordered gateway delivery/replay, reconnect/error state, typing, and paginated member presence.
- Account-global one-to-one direct messages, including exact-username creation, unread/read cursors, space-independent navigation, and the existing ordered chat/gateway pipeline.
- Username suggestions in the space invite, private-channel grant and New direct message fields: people the app already has (known people minus members, space members minus grantees, known people), ranked like `@` suggestions; pending invitees stay listed as "Already invited". Rows show the avatar and display name with `@username` beneath it, as `@` popup rows now do; the list shows four rows and scrolls to six. Choosing one fills the exact username without submitting; Up/Down, Return and Escape use SwiftUI key handling that is not yet verified on hardware keyboards.
- File attachments (see `docs/media.md`, "Uploads and attachments"): images, video (AVKit), audio and file cards under message text, tolerant decoding, and signed-URL refresh for long-open windows. Images and videos open in an in-app viewer for the message's media (full screen on iPhone and iPad, over the window on the Mac): swipe, Previous/Next or the arrow keys between files, pinch or double-tap zoom, animated GIF/WebP/APNG playback, native video playback, Save or share (share sheet), Open in browser, and swipe down, Escape or Close to leave. The attach control stays in place while disabled; Photos picker, file importer, and drag and drop (macOS, iPad and iPhone, with the drop overlay in `docs/media.md`) become available only when `GET /api/assets/usage` succeeds in a writable conversation. Files are compressed on device with the server's `compression` settings before a direct upload to storage: lossless sources (PNG/BMP/TIFF/lossless WebP) become the smaller of an exact indexed PNG (when the colours fit) and lossless WebP (libwebp), verified pixel for pixel, or stay as they are; photos (JPEG/HEIC/lossy WebP) become AVIF at `avifQuality` (libavif 1.4.2 with aom 3.15.1, speed 6 all-intra, `tune=ssim` so the quality scale matches `avifenc -q`) when the server asks for AVIF, else or on any AVIF error WebP or JPEG at `imageQuality`, kept when at least 10% smaller (HEIC always converts); videos are exported to SDR H.264/AAC MP4 only when too large, not H.264, HDR or over the bitrate target, and the original is kept whenever the result is not verifiably SDR H.264 with its audio; previews and video posters are uploaded alongside. Files uploaded as originals lose their location metadata losslessly (JPEG APPn segments except JFIF/ICC/Adobe with orientation kept, PNG text/eXIf chunks, MP4/QuickTime `udta`/`meta` boxes renamed to `free`). The parked server pipeline's `status`/`animated` fields and `message.attachments`/`attachment.progress` events are still rendered (processing placeholder, failed card, silent looping animations) if an API sends them.
- Channel and thread composers both accept file-only messages through Files, Photos, native paste, and drag and drop. Each conversation/thread owns its own ten-file tray, progress, retry/remove actions and picker target. Unsent files remain in memory through thread closing/reopening and conversation switches; late picker results return to their original composer. Unknown-outcome retries keep the same command/files, definitively rejected messages restore files when edited, and expired assets reupload from the retained local bytes. Sign-out and access revocation discard affected files and invalidate outstanding pickers. These model paths have regression coverage; Apple SDK compilation, picker/paste/drop interaction and rendered iPhone/macOS states still require Apple validation.
- Custom profile pictures and space icons: Edit profile and (for space owners) Manage space show the picture in a frame with an edit badge on its corner; the frame opens a menu of Photos or Files and, while a custom one is set, Remove. Without one, your mascot holds up a "change me!" sign (`caper-change-me`). GIFs (by magic bytes) upload unchanged up to 5 MB and 1024 × 1024, and bigger ones are shrunk with ImageIO (`GIFShrink`) to a square that fits, frames and timing kept; other images are oriented, centre-cropped and scaled to at most 512 px, then re-encoded as lossy WebP (bundled libwebp, PNG/JPEG fallback) up to 1 MB, so HEIC/AVIF and location metadata never upload. Pictures replace the saved mascot in every avatar (members, authors, replies, threads, DMs, reactions, voice, profile cards, invites, feedback, account) and icons fill rail and invite tiles; the mascot or initial shows while loading or after a failure. Custom pictures are rounded squares (corners 30% of the side, with the speaking ring following); saved mascots stay circular. A `profiles` gateway subscription applies other people's `profile.updated` (names, usernames and pictures in members, invitations, DM peers, suggestions, authors including replies, pins, threads and forwards, reactors and profile cards) and `space.updated` (rail, invitations, open space) live; your own account updates when changed on another device, and an older gateway without the feed is ignored. List refreshes (members, invitations, people, DM peers, blocks) record each person's current profile too, so a missed event is repaired by the next refresh and older loaded messages show the latest known name and picture. They decode at display size through the inline GIF frame loader, cached by asset id within a bounded cache, and GIFs animate except under Reduce Motion. Notification images still use the mascot.
- `@mention` autocomplete in the shared `:` emoji popup (loaded space members except you, then `@everyone`/`@here`; in DMs, including notes, everyone from `GET /api/people`, falling back to the other participant until it loads), pills for names the server resolved in `content.mentions`, and an 8% terracotta row tint with a 2pt leading edge on messages that mention you. Tapping a person's pill opens a card (popover on macOS, sheet on iPhone) with **Message**, which opens or creates the DM; on macOS, messages containing person pills are not text-selectable. SwiftUI `Text` cannot pad or round an inline run, so pills are square-cornered background spans without the 2pt padding/4pt corners. Mentions notify only through iPhone push (see Notifications).
- Message requests, blocking and DM privacy. DMs from people you share no space with arrive as requests under a "Message requests" row with a count. The row expands in the macOS sidebar, and on iPhone the list is pushed in Browse. Requests open read-only, with a request bar for Accept, Decline and Block in place of the composer. They never count as unread and never play sounds. Outgoing requests show a "Waiting for @x to accept" notice.
- Blocking is available from the DM header and from message actions, after a confirmation. A blocked DM replaces the composer with Unblock. Blocked authors' messages collapse into "⊘ N blocked messages — Show" runs in channels, DMs and threads, including the thread root. Show and Hide are kept in memory only. Blocked authors never play sounds or show as typing.
- Account settings ▸ "Privacy and blocked accounts" holds "Who can start a DM with you" (Anyone, People in my spaces, No one new) and the blocked-accounts list. The privacy choice saves at once and reverts if the save fails. The `dm_not_accepted` and `dm_blocked` errors use the client's own wording. Pins from blocked authors still show normally in the pins list.
- Any Caper team name in a conversation (`@feedback`, `@caper`, `@support`, `@admin`, `@mod` and the rest of the server's reserved list) asks the Caper team; the message stays where it was written. Composers everywhere (channels, threads, DMs, notes) offer one "Notify the Caper team" suggestion: a bare `@` leads with `@feedback`, and a typed query offers the first team name it starts after people and `@everyone`/`@here`, inserted as is (`@ad` → `@admin`). When the message carries the `feedback` mention, every team name renders as a pill that opens no card, with a muted "Caper team has been notified 🫡" line under the message. Messages from feedback team members (`author.admin`) show a gold glowing ADMIN badge after the name.
- New message with a team name (`caper`, `feedback`, `admin`, …) opens the accepted Caper DM (`peer.team` from `GET /api/dms`); the field never blocks those names, since the server resolves them. Each top-level message there is a feedback report with the "Caper team has been notified 🫡" line, and the team replies in its thread. An empty Caper DM reads "Messages here go to the Caper team."
- Feedback team accounts (`feedbackTeam` from `GET /api/account/me`) get a **Feedback** row at the top of Direct messages: `GET /api/feedback` reports, newest first, with the reporter, "#channel · Space" or "DM · names", a text preview or "Sent N files", a relative time and Answered (the thread's newest message is from the team) or Waiting (terracotta). Each row has Archive ("Archive feedback from <name>"), a hover button on macOS and a context menu (press and hold on iPhone); it archives the whole thread for the team, leaves the list at once and refreshes (a failure shows an error and reloads). A quiet "Show archived" at the bottom lists archived threads ("Archived feedback", no status or waiting count, "Nothing archived." when empty) with "Restore to the inbox", and "Back to feedback" returns. It expands in the sidebar on macOS and wide layouts and is pushed in Browse on iPhone (pull to refresh). It refreshes when opened, after you send in a visit and when a `feedback` notification arrives, with Load older feedback for later pages. A row opens a visit: the conversation around the report with its thread open (the reported message highlighted), titled "#plans · Studio" or "Alice & Bob". Team members take part like members: sending, thread replies, files, reactions, edits, typing, pins and forwarding in. A visited space channel's voice (Join voice, "N in voice" with who is in it in a popover) sits in the conversation header; DMs have no voice. There is no member list, join/leave, channel settings, notification menu or read marker. `@` suggestions offer the reporter and people in the loaded messages. Close (wide layouts) returns to the previous conversation; Back on iPhone returns to the inbox. A conversation you already belong to opens the normal way.
- Generation fences for account/channel transitions, immediate revoked-data clearing, separate in-memory chat/media capabilities, and same-origin-only credential redirects.

The native UI and platform projects still require exact-head Apple builds/tests before they are considered build-verified. Linux Swift compiler parsing and SDK-free model harnesses do not validate Apple SDK types, platform media processing or rendered SwiftUI states; this orb has no Xcode or Apple SDKs.

## Notifications

- **Controls (iOS and macOS).** The space menu and each channel's options menu
  have **Notifications** (`Default (…)`, naming what it inherits, then
  `All messages`, `Only @mentions`, `Nothing`) and **Mute space/channel** (15
  minutes, 1 hour, 8 hours, 24 hours, or until turned back on). A muted item
  reads **Unmute …** with "Muted until 5:00 PM" or "Muted"; a channel in a muted
  space also shows "Muted with the space". DM rows (not personal notes) have a
  context menu, plus a "…" on hover on macOS, with **Turn off/on notifications**
  and **Mute/Unmute conversation**. Muted spaces, channels and DMs are dimmed with
  a bell-slash, and a muted DM shows no unread dot.
- **Account settings ▸ Notifications** holds "Notify me about". On an iPhone whose
  build's APNs environment the server advertises in `GET /api/push/config`, it
  also shows "Send to this phone" and the per-device opt-in switch.
- Settings load after sign-in, on returning to the app, and when Notifications
  opens (menus refresh at most every 30 seconds). Every change applies at once
  and reverts with a short inline error if the save fails.
- **iOS push.** After opt-in the app registers its APNs token with `appId` (the
  bundle ID) for the current sign-in session and shows the server's alert. A tap
  opens the channel (`spaceId` + `channelId`) or DM (`conversationId`), waiting
  for the space list on a cold launch. Kind `feedback` opens a feedback visit at
  `messageId` in its thread (`threadRootId`, else the message's own), and macOS
  desktop notification clicks do the same. The notification service marks
  `feedback` like a direct mention. In the foreground no banner shows for the
  conversation already open. `Configuration/iOS-Debug.entitlements` selects the
  APNs sandbox (`apnsSandbox`) and `iOS-Release.entitlements` production (`apns`).
  Unsigned simulator builds cannot receive APNs tokens, so the opt-in stays hidden
  there.
- **Invitations.** Space/private-channel invitations are regular alerts with
  independent grouping. Tapping refreshes pending invitations and presents
  Accept/Decline review, never automatic acceptance, private history or voice.
- **Message requests.** Each new incoming DM request sends one regular alert,
  without message content or a communication intent. Tapping opens the existing
  read-only request review; pending messages stay quiet until acceptance.
- macOS receives local notifications from the account gateway while running;
  it has no APNs delivery after quit.

Physical signed devices must still validate opt-in, permission denial and
re-enable, token rotation, logout and account switching, sandbox and production
delivery, foreground suppression, and taps from a terminated app.

## Fonts and licenses

`prepare.sh` runs the repository's verified `scripts/native_fonts.py`, then bundles the original, unmodified Satoshi Regular/Medium/Bold/Black OTF files and unchanged Fontshare license. Generated font inputs are ignored. Both app archives also contain `WebRTC-LICENSE.txt`, `libwebp-LICENSE.txt` (libwebp 1.6.0 via `SDWebImage/libwebp-Xcode`, BSD-3-Clause plus the WebM patent grant), `libavif-LICENSE.txt` (libavif 1.4.2, BSD-2-Clause, including its bundled libyuv scaling code), `libaom-LICENSE.txt` (aom 3.15.1, BSD-2-Clause, plus its compiled-in fastfeat, vector and x86inc code) and `libaom-PATENTS.txt` (AOMedia Patent License 1.0), from `Resources/`. `prepare.sh` downloads libavif and the aom AV1 encoder as one static `.build/libavif-1.4.2/libavif.xcframework` (module `libavif`; macOS arm64/x86_64, iOS arm64, simulator arm64/x86_64) from a SHA-256-pinned [joswayski/libavif-apple](https://github.com/joswayski/libavif-apple) release, which builds it from pinned upstream sources and smoke-tests every slice, and checks that these licence files are the ones that release ships.

## Build

Requires the Xcode 16 toolchain on GitHub `macos-15` / `macos-15-intel`. `prepare.sh` downloads the libavif/aom encoder XCFramework from a pinned, SHA-256-checked [joswayski/libavif-apple](https://github.com/joswayski/libavif-apple) release (built there from pinned upstream sources with Neon on arm64 and SSE/AVX on x86_64); upgrading it means a new release there and a new tag and checksum here. XcodeGen is built automatically from pinned commit `21ac9944b0ab546a07422dbed86f33dd2ebd76f8`; WebRTC is pinned to `stasel/WebRTC` 153.0.0.

```sh
./apps/native/apple/build.sh macos
./apps/native/apple/build.sh ios
```

On Apple silicon, `CAPER_MACOS_ARCH=x86_64 ./apps/native/apple/build.sh macos` builds the Intel app and runs its tests under Rosetta; CI does this for `macos-x64` on the self-hosted Mac. Rosetta UI tests use the same window-size-based desktop/medium capture layout as ARM; only a real Intel Mac requires the 1,440×900 desktop window.

Each command first runs the macOS XCTest suite. Outputs are ignored and credential-free:

- `dist/Caper-macos-arm64.zip` or `dist/Caper-macos-x64.zip` with an ad-hoc-signed `.app`
- `dist/Caper-ios-simulator-arm64.zip` (unsigned simulator app; not a device package)

The script verifies app/framework architectures, embedded WebRTC, runpaths/signature on macOS, and embedded font/WebRTC license resources. Bundle IDs default to `chat.caper.macos` and `chat.caper.ios` and may be overridden with `CAPER_MACOS_BUNDLE_ID` / `CAPER_IOS_BUNDLE_ID`.

Signed Mac releases additionally use `dmgbuild==1.6.7` (locked in the `dmg`
dependency group) to create the standard app → Applications Finder window without GUI
automation. `sign-and-notarize.sh` notarizes/staples the app first, preserves the
ZIP self-update payload, then signs, notarizes, staples and assesses the DMG.
From the repository root, run it through
`uv run --locked --only-group dmg bash apps/native/apple/sign-and-notarize.sh <label>`
with the existing signing environment; this supplies dmgbuild without a global pip install.
No additional signing identity or production configuration is required.
Both Mac development jobs also build/mount a disposable fixture DMG using
`uv run --locked --only-group dmg python -m unittest discover -s tests -p test_apple_release_signing.py -v`;
the test skips the real disk-image check on non-Mac hosts. XCTest covers release
installation eligibility and the existing parity suite covers development startup.
Actual Finder layout, downloaded-app Gatekeeper behavior, the install prompt,
and update/relaunch from `/Applications` and `~/Applications` still require Mac
acceptance; Linux checks are not evidence for those states.

macOS AppIcon PNGs use the same main `apps/web/public/caper-face.svg` artwork
as the Windows and Linux installed icons. From the repository root, run
`node scripts/generate-favicons.mjs` with ImageMagick 7 and librsvg installed to
regenerate the 16–1024px transparent macOS exports, or add `--check` to detect
drift without writes. The opaque iOS AppIcon is not regenerated by this command.
The running macOS Dock icon uses the same original green mascot as the installed
app bundle. Wordmarks also stay on the original green Caper; daily characters
rotate only in file-drop illustrations. Finder/Dock cache
and bundle compilation need macOS validation; Linux asset renders do not establish
that behavior.

## Deterministic parity captures

The scripts launch only the explicit loopback fixture; fixture data can never be selected in a production launch.

```sh
./apps/native/apple/parity-screenshots.sh macos
./apps/native/apple/parity-screenshots.sh ios
```

The macOS suite captures populated, members-hidden, login, actionable login error, Manage space, and private Channel overview states. The iOS suite adds narrow conversation and Browse-open states on an `iPhone 16` simulator. The `reaction-chips` fixture also captures who reacted: the hover card on macOS and the press-and-hold sheet on iOS. Its chips have local-only authors, so the app serves those reactor lists itself instead of calling the fixture server. Screenshots are exported from the XCTest result into ignored `apps/native/apple/parity-artifacts/`. Tests also assert fixture content and required controls before capture.

## Voice in development builds

Join and audio preferences are available in normal builds, with no environment flag. Joining requests microphone permission before capture; publication stays locally silent until the remote answer, transport readiness, and generation checks complete. The implementation supports gateway snapshots, lease snapshots, roster subscribe/close, immediate local mute/deafen/leave, bounded rejoin, TURN credential renewal with restart/ack, and iOS `RTCAudioSession` interruption/route handling. The iOS app declares the audio background mode so an active audio session can continue while locked.

Like the existing web client, native signaling sends pending local SDP immediately and gates audio on transport readiness, not completion of every STUN/TURN probe. The previous combination of continuous gathering and waiting for `.complete` made every initial join time out. The native transport regression creates two real peers, gives only the server-side stand-in gathered candidates, connects DTLS/SCTP, and repeats negotiation with a fresh ICE generation. This tests transport, not microphone capture or Cloudflare audio. Separate tests exercise delayed join/stop/replacement and preserve media error codes so provider retry is not mistaken for membership revocation.

Browsing another space or text channel preserves the active call and its displayed `space / channel` context. Opening that context navigates back to it; an explicit Join in another channel synchronously tears down the old transport before starting the replacement. Logout, active-context deletion, and active-channel access loss also tear down locally before any network cleanup.

This is **development calling, not accepted physical-device support**. The UI fixture deliberately returns media 503 and cannot validate Cloudflare. Required acceptance remains: exact-head Apple compilation/tests, two-party Cloudflare calls, TURN-only/network handoff, route/interruption recovery, and sustained locked-iPhone capture/playback on physical hardware. iOS uses Apple's communication-route UI. On macOS the pinned WebRTC binary's custom audio factory and AUHAL route each call to the chosen input/output without changing system defaults; selected devices still require physical acceptance. Output gain is persisted and per-participant local mute/gain is implemented. There is no incoming-call PushKit/CallKit behavior because the server has no incoming-call signaling.

macOS audio preferences include input gain, a live voice contour, and an explicitly started local microphone comparison (up to 30 seconds, also available in-call). The microphone callback enqueues gain-adjusted PCM to a bounded worker that resamples the actual hardware rate to 48 kHz, runs the pinned DPDFNet-8 HR model or native RNNoise fallback, then resamples to the hardware rate. The callback never runs inference and sends silence before processed output is ready or after both engines fail. Natural replay is post-gain/post-denoise; enhanced replay adds the same contour sent live. During in-call recording and replay, publication stays suspended until the sheet closes; dismissal or joining another call removes the temporary recording. The sheet also shows aggregate connection statistics without addresses, candidate IDs, or credentials. The model, native runtime, and fallback are bundled offline with licenses; `prepare.sh` verifies source hashes. Linux synthetic speech exercises the native model and 44.1/48 kHz worker paths, but actual recording, quality, route timing, and Cloudflare calling still require Apple CI and physical-device acceptance.

iPhone now has the same local gain (0–200%), on-device DPDFNet-8/RNNoise worker and live contour strength (0–100%) through an M153 custom audio device backed by iOS VoiceProcessingIO. It follows the current `AVAudioSession` route rather than selecting desktop-style HAL devices. Its explicitly started natural/enhanced microphone comparison records up to 30 seconds locally and keeps publication suspended through replay; gain zero is exact silence. Debug-enabled accounts can view local numeric denoiser counters, while aggregate connection statistics remain available in the sheet. The official pinned ONNX Runtime 1.23.0 static iOS XCFramework supplies arm64 device and arm64/x86_64 simulator slices; macOS keeps its pinned 1.23.2 runtime. iPhone plays the web's interaction sounds mixed with other audio (ambient session) and never changes a call's or microphone test's audio session. Synthetic peer and UI fixture tests do not establish physical iPhone microphone, speaker, Bluetooth route, interruption, background, or live SFU acceptance.
