# Native Caper clients

Native clients for caper.chat. There is no embedded browser, JavaScript runtime,
local HTTP server, Tauri, or Electron; the website remains available.

| Platform | Presentation / networking | Project |
| --- | --- | --- |
| macOS | Swift / AppKit / URLSession | [Apple](apple/README.md) |
| iPhone | Swift / UIKit / URLSession | [Apple](apple/README.md) |
| Windows | Rust / Win32, DirectComposition, Direct2D, DirectWrite, UI Automation / native HTTP and WebSocket | [Windows](windows/README.md) |
| Linux | Rust / GTK 4 / native HTTP and WebSocket | `linux/` (below) |
| Android | Kotlin / Android Compose / native HTTP and WebSocket | [Android](android/README.md) |

Windows packages carry the Win32 client (`windows/`, package `caper-windows`)
and Linux packages carry the GTK 4 client (`linux/`). `desktop/build.ps1` and
`desktop/build.sh` ship them as `Caper.exe` and `caper-desktop` in the earlier
desktop client's install layout, so existing installs, saved sessions, launch at
login, `caper://` links and self-updates carry over. Apple and Android have
separate native UI implementations. All clients talk directly to the same Caper
API. No server or Cloudflare credentials belong in an app.

`linux/` (package `caper-linux`) is the Linux client, in GTK 4 on the shared
`client/` core. `desktop/build.sh` builds it into the Linux release packages
under the earlier client's name and layout (`caper-desktop` beside
`caper-updater`, FFmpeg, ONNX Runtime and `caper-web-player`), so existing
installs, saved sessions, launch at login, `caper://` links and self-updates
carry over. Run it with `cargo run -p caper-linux` from `desktop/`
(`CAPER_API_URL` or `--api-url` picks the server). It needs GTK 4.8 or later.
The nightly Linux GTK job also packages a tester tarball with
`linux/package.sh` (artifact `caper-linux-x64-<sha>`, kept 14 days; `install.sh`
installs it for your user in `~/.local`, `--uninstall` removes it), installs it
and runs `linux/smoke.py`, which drives the app through AT-SPI under Xvfb
against the parity fixture (`scripts/native-parity-fixture.mjs`).

## Shared behavior

- First launch without a valid session opens email sign-in (no Guest workspace).
  New accounts finish their profile, then name their first space; saved sessions
  restore; logout returns to sign-in.
- Text conversations, owner space/channel/member management, private grants,
  invite links, direct messages, presence, typing, and the responsive
  rail/sidebar/conversation layout, following the website.
- Searchable emoji reactions with counted chips and own-reaction highlighting.
  Channel previews show reactions but cannot change them or pins until joined.
- Message pins: Pin/Unpin, "Pinned by …" attribution, and a Pins button in the
  conversation header (Apple uses a sheet; Android and Rust desktop switch the
  conversation view). Lists include pins outside loaded history.
- Editing an attached photo or video before sending (crop, rotate, draw, text,
  colour and size, undo/redo/reset), re-uploading the original with the edits.
  Rust desktop cannot draw on videos because its bundled FFmpeg lacks the
  `overlay` filter. See [editing attachments](../../docs/media.md#editing-attachments-before-sending).
- Sharing into Caper from other apps' share sheets: a recent-first, searchable
  conversation picker, then that composer with the files staged and the text in
  place. Android has a share target with Sharing Shortcuts; macOS a Share
  extension (iPhone's is built but off until its App ID is registered). See
  [sharing into Caper](../../docs/media.md#sharing-into-caper).
- Live forwarding: a channel/DM picker with an optional note, a live original
  card, and a read-only shared conversation. See
  [live message forwarding](../../docs/media.md#live-message-forwarding).
- Composer suggestions after `:` (Emoji 15 catalog; `:tomato`, `:+1`) and `@`
  (the open space's loaded members except you, then `@everyone`/`@here`; in DMs,
  people from `GET /api/people`). Selecting inserts the token without sending.
  Resolved `content.mentions` render as terracotta pills (square-cornered on
  native, rounded on web); messages mentioning you get a terracotta tint and
  leading edge. A person's pill opens a profile card with **Message**. Mentions
  only notify through push. See
  [docs/media.md](../../docs/media.md#scope-and-architecture).
- Space invite, private-channel grant and **New direct message** username
  fields suggest known people; lookups stay exact-username with no directory.
  See [spaces and channel access](../../docs/media.md#spaces-and-channel-access).
- One Settings screen from the account gear, with the same sections on every
  client: Profile, Appearance, Notifications (Do Not Disturb first), Voice &
  audio, Privacy & blocked accounts, About (version, What’s new) and Log out
  (desktop adds Startup after Voice & audio). Phones push it full-screen, and
  Back returns to it from its sub-pages. Do Not Disturb's quick toggle is a
  moon button beside the gear on phones and macOS, your avatar on Linux, and
  your name's account menu (after Edit profile) on web and Windows; see [notifications](../../docs/notifications.md#do-not-disturb).
- Each client reopens the last space and channel (per account and device, IDs
  only, joined channels only), falling back to the first joined channel.
- Unsent conversation drafts and thread replies survive quitting, relaunching
  and updates, kept on the device per account and deleted at logout. See
  [docs/media.md](../../docs/media.md#scope-and-architecture).
- Native channel voice in default builds. There are no incoming direct calls,
  so no CallKit/Telecom/push ringing.

## Keyboard navigation

Every client, including iPad and Android with a hardware keyboard, has the same
keys (Cmd on Apple, Ctrl elsewhere):

- **Ctrl/Cmd+K** quick switcher: channels in every space (other spaces load
  when it opens), DMs, spaces and people, unread first; type to filter,
  Up/Down and Enter to open.
- **Alt+Up/Down** previous or next conversation in sidebar order (channels,
  then DMs), wrapping; with **Shift**, only unread ones.
- **Esc** marks the open conversation read up to its latest message, once
  suggestions, threads, menus and dialogs have had the key.
- **Ctrl/Cmd+Shift+M / D** mute or deafen, only while connected to a call.
- **Ctrl/Cmd+/** lists the shortcuts.

Matching, ranking and stepping follow `shared/shortcuts/switcher-cases.json`,
which each client's tests run. macOS lists the keys under **Go** and **Help**;
iPadOS shows them when ⌘ is held and in the menu bar.

## Launch at login (desktop only)

**Account gear → Settings → Startup → Launch at login** (off by default) opens
the normal window at sign-in. macOS also exposes **Caper → Settings…** (⌘,)
while signed out. Web, iOS and Android have no startup option.

- macOS uses `SMAppService.mainApp`; if approval is required, Settings links to
  Login Items in System Settings.
- Windows registers the current executable in the current user's `Run` registry
  key; the uninstaller removes it.
- Linux writes `chat.caper.desktop` under `$XDG_CONFIG_HOME/autostart`
  (normally `~/.config/autostart`); it needs an XDG-compatible session. Remove
  that file before uninstalling a package or deleting a portable copy.

Switching off removes the registration immediately. Portable copies must stay
at the registered path (toggle off/on after moving). Startup does not keep
development command-line overrides, and fixture previews never touch real
registrations.

## Build and download

The **Native development builds** workflow (`.github/workflows/native.yml`)
runs nightly and on manual dispatch. Apple jobs run only on manual `main`
dispatches, using the owner's existing Mac VMs; their admission hook does not
allow scheduled jobs. No Mac job falls back to cloud hardware. It does not deploy, publish a
release, or upload to stores. Each artifact (kept 14 days) contains the
package, `SHA256SUMS`, and `BUILD.json` with the exact Git revision. Apple and
Android jobs also run native UI automation against the loopback fixture and
upload `caper-ui-*` artifacts; Android uses a separately built fixture APK.

| Target | Package | Installation limits |
| --- | --- | --- |
| Windows x64 | `Caper-windows-x64.zip` | Portable, unsigned; Windows security policy can block it |
| Linux x64 | `Caper-linux-x64.deb`, `Caper-linux-x64.tar.gz` | Ubuntu 24.04 build; native runtime libraries required |
| macOS Apple Silicon | `Caper-macos-arm64.zip` | Development `.app`, not Developer ID signed/notarized |
| macOS Intel | `Caper-macos-x64.zip` | Development `.app`, not Developer ID signed/notarized |
| iOS Simulator Apple Silicon | `Caper-ios-simulator-arm64.zip` | **Cannot install on an iPhone** |
| Android | `Caper-android-debug.apk` | Debug-signed test APK, not a store release |

Local builds, from the repository root:

```sh
bash apps/native/desktop/build.sh        # Linux
bash apps/native/apple/build.sh macos   # Mac
bash apps/native/apple/build.sh ios     # Mac, iOS Simulator
bash apps/native/android/build.sh       # JDK 17 + Android SDK 36
```

On Windows, run `./apps/native/desktop/build.ps1` in PowerShell. Each project's
README lists its prerequisites.

The root Rust workspace is the API. The desktop client has its own Cargo
workspace and lockfile, so GUI dependencies stay out of server images. Native
builds do not need `npm install`.

## Signed builds for testers

The **Native release** workflow (`.github/workflows/release.yml`) builds every
app from one `main` commit:

- a Developer ID-signed, notarized Mac app for Apple Silicon and Intel,
- a release-signed Android APK and Play bundle,
- an iPhone build for TestFlight,
- Windows and Linux desktop packages (not code-signed).

Mac and iPhone release jobs use the owner's existing `josemac-ui` and
`josemac-sim` VMs. Intel packages are cross-built and tested under Rosetta on
the ARM64 Mac, not on physical Intel hardware. Offline Mac runners keep those
jobs queued rather than starting a cloud Mac.

It replaces the `native-latest` pre-release:

- Mac (Apple Silicon): https://github.com/joswayski/caper/releases/download/native-latest/Caper-macOS-Apple-Silicon.dmg
- Mac (Intel): https://github.com/joswayski/caper/releases/download/native-latest/Caper-macOS-Intel.dmg
- Android (phones without Google Play only): https://github.com/joswayski/caper/releases/download/native-latest/Caper-Android.apk
- Windows: https://github.com/joswayski/caper/releases/download/native-latest/Caper-Windows-x64-Setup.exe
- Linux: https://github.com/joswayski/caper/releases/download/native-latest/Caper-Linux-x64.deb

The Intel Mac build never holds up the release: the previous Intel files stay
until it finishes, then it is uploaded and added to the self-update manifest.

**Installing.** Mac users drag Caper from the disk image onto Applications (or
`~/Applications` if `/Applications` is not writable). Signed release copies run
from anywhere else show install instructions and quit; development builds do
not. The ZIPs remain for self-updaters. Windows Setup installs per-user without
admin access or wizard pages, opens Caper, and appears in Installed apps; it
does not remove old portable copies. Linux users on Ubuntu 24.04 or compatible
Debian-based systems open the `.deb` in their software installer. The ZIP and
tar archives are self-update payloads and optional portable copies.

**Releasing.** Every push to `main` that changes the native apps starts
`release.yml` for that commit (20-40 minutes, one release at a time; the newest
waiting push replaces older waiting ones). Builds start at once. Handing the
iPhone build to testers, the Play rollout, and publishing the desktop
self-update wait for the servers they need (`.github/actions/wait-for-servers`):
the newest API image at or before the commit must have published, and the Ship
for caper that followed it must have rolled out the API and gateway. If either
failed, the release fails without reaching anyone; fix forward. The workflow can
also be run by hand (a blank `git_sha` releases the newest `main` commit).

Signing material is read from AWS Secrets Manager through GitHub OIDC; see
`docs/release-signing.md` in joswayski/infrastructure.

**Versions.** Each run is one release, named by its New York date and a daily
revision: version `2026.11.19.3`, which every client shows and copies and Google
Play names the release. Where a platform takes only three numbers, the app
version packs the day and revision as `YEAR.MONTH.DAY×100+REVISION`
(`2026.11.1903`, Apple's CFBundleShortVersionString; the apps read the shown
version from their `CaperVersion` key). The build spreads it into one integer,
`YYYYMMDDNN` (`2026111903`): Apple's CFBundleVersion, Android's versionCode and
the self-updater's `build`. `scripts/native_release_version.py` numbers the run
in its Resolve job: the revision is one more than the earlier release runs
created that day whose first attempt started (runs cancelled while waiting
never built anything). A re-run keeps its version; a run cancelled before it
started cannot be re-run, so start a new one. At most 99 releases a day.
Builds before CalVer were `0.1.<run number>` (build `<run number>`); every
CalVer build is larger, and clients read both formats in update history.

**Phone betas.** The homepage's phone buttons are the supported installs; their
links live in `apps/web/src/downloads.ts` and stay hidden until set.

- Android: each release goes to Play's internal track and its closed testing
  track (`alpha`, reviewed by Google first) via `scripts/play-upload.py`
  (`PLAY_TRACK`). Testers join an open Google Group, opt in at
  https://play.google.com/apps/testing/chat.caper.android, and install from
  Google Play. Play re-signs bundles with Google's app signing key, so the
  GitHub APK (upload key) cannot update to or from a Play install and never
  updates itself.
- iPhone: the run archives `chat.caper.ios` and uploads it with
  `apps/native/apple/upload-testflight.sh`. Signing is cloud-managed through the
  App Store Connect API key; no certificate or profile is stored. Build numbers
  are `<build>.<attempt>` (`2026111903.1`), unique across re-runs. `scripts/testflight-distribute.py` adds each
  build to every tester group and submits it for Beta App Review when an
  external group needs it.
- Store reviewers sign in with the API's review account (`AUTH_REVIEW_EMAIL`,
  `AUTH_REVIEW_CODE`; see `docs/media.md`).

**Desktop self-updates.** Release builds of the Mac, Windows and Linux apps carry
`caper-updater` (`apps/native/updater`) and compare release builds (see Versions).
About 20 seconds after launch and every minute they check
`https://caper.chat/api/updates/native`; the API caches GitHub's
`native-latest/latest.json` and its signature for 60 seconds per process, and
the updater falls back to GitHub if the site fails or its response does not
verify. Downloads come from GitHub. The manifest is signed with the Ed25519 key
in the `caper_update` section of `production/signing/release`; each app trusts
only the public key compiled into it.

- Mac shows an alert and **Caper › Check for Updates…**; Windows/Linux show a
  banner and **User Settings › Settings… › Updates › Check for updates**.
  Manual checks restore an offer dismissed with **Later**.
- The review shows the download size beside the version, which links to its
  commit. When the release lists a patch from the installed build it is a few
  MB; otherwise it is the full archive. See
  [patch updates](../../docs/media.md#desktop-patch-updates).
- **Restart to update** hands off to an independent updater that waits for
  Caper to quit, verifies the download's SHA-256 (and, on Mac, its Developer ID
  team), swaps in the new copy and reopens it. On failure it reopens the
  existing copy and logs to `caper-updater.log` (Linux: `$XDG_STATE_HOME/caper`
  or `~/.local/state/caper`; elsewhere the temporary directory).
- Copies it cannot replace (the Linux `.deb`, or a Mac app in a folder that
  became read-only) get a **Download** link to the matching DMG, Setup or
  `.deb` for manual installation.
- Development builds and fixtures never check. Android updates come from
  Google Play.

## Comparing native screens with the web

The web UI is the visual reference. Every client uses the committed Inrond 40
fonts in [shared/fonts](../../shared/fonts/README.md).

The parity fixture is a disposable, loopback-only fake API. After `npm ci`, run
with Node 24:

```sh
npm run test:native
node scripts/native-parity-fixture.mjs
```

It serves HTTP on port 3001 and the unified WebSocket on 3002 (also accepted on
3001), matching the web development proxy. It never sends email, calls a
production API, or connects to an SFU. Sign in with any valid test email and
code `ABC234`; the owner account opens **Fixture Studio** with labelled sample
conversations. It supports profile edits, space/channel/member management,
private grants, history/pagination, idempotent sends, gateway replay, typing
and deterministic presence. Authentication and authorization are simplified.
Sent links unfurl into local cards: a YouTube link into a player card whose `play`
page the fixture serves for debug clients' `CAPER_DEV_PLAY_ORIGIN`, and a link
ending in `.mp4` into a direct video card playing the clip named by
`CAPER_FIXTURE_EMBED_VIDEO` (the fixture never encodes video itself).

For screenshot and performance gates, `CAPER_BENCH_MESSAGES=3000 node scripts/native-parity-fixture.mjs`
adds 45 members to Fixture Studio and replaces `#general` with 3,000 generated messages (fixed seed, so
every run and client gets the same text, mentions, links, emoji and reactions). `CAPER_FIXTURE_PORT` and
`CAPER_FIXTURE_GATEWAY_PORT` move it off 3001/3002.

The Linux client runs the Windows client's scripted scenarios too
(`caper_client::bench`): with `BENCH` set to `startup`, `idle`, `scroll` or `typing`, a signed-in client
waits until its first frame showing the conversation's messages, settles 2 s, runs the scenario and quits,
appending the Windows client's JSON line (plus `content_ms`, process start to that frame) to `BENCH_OUT`.
CPU and memory (RSS and PSS) come from `/proc/self`.

The owner's notifications inbox (`GET /api/notifications/inbox`, `POST /api/notifications/inbox/read`) holds two
unread items: a mention from Alex and a 🎉 reaction from Maya in `#general`.

`POST /__fixture/control` accepts:

- `{ "reset": true }`, `{ "disconnect": true }`;
- one-shot failures: `{ "failure": { "path": "/api/auth/email/request", "method": "POST", "status": 503 } }`;
- typing: `{ "typing": { "channelId": "chan00000001", "active": true } }`;
- read-only voice rosters: `{ "media": { "channelId": "chan00000002", "participants": [{ "id": "fixture-voice", "name": "TEST FIXTURE Voice", "muted": false, "deafened": false }] } }`;
- `{ "mediaAccessDenied": { "channelId": "chan00000002" } }` (add `"denied": false` to allow again).
- uploads: `{ "uploadsEnabled": true }`. Reservations that send `"multipart": true` for files over one part
  (64 MiB, or `{ "multipartPartBytes": 4 }` for small test files) get a part plan, `POST /api/assets/{id}/parts`
  and part PUTs like the API, and `complete` answers `upload_incomplete` (409) while parts are missing;
  `{ "failPart": 2 }` fails part 2's next PUT once with 500. `GET /__fixture/assets/{id}` reports an upload's
  size, part plan, state and, once assembled, its SHA-256.

Media join and participant-token operations return 503.

Reference sizes are 1440×900 (desktop) and 390×844 (narrow). Compare sign-in,
restoration errors, populated conversation, narrow Browse-open, and owner
management, using real clicks and keyboard input.

### Layout rules

- Signed-out, expired or logged-out states show email sign-in only, with no
  Guest account, composer, or public General fallback.
- Voice (design H): channels are text and voice together, and the conversation
  header has no voice controls. A live channel's row grows in place into a card
  (no "Live now" heading); pressing the card outside its controls opens that
  chat. Your call's card shows "You're in · N" or Connecting…, a timer, faces
  (you first, at most two rows, "+N" past ten), and Mute, Deafen and Leave (or
  Cancel). Right-click, long press or the accessibility action on a face opens
  the full list with per-person volume (beside the sidebar when wide, a sheet on
  phones). Other live rows add faces, names, timer and Join/Switch. Only voice
  with no row in the listed space (your call elsewhere, a visited feedback
  channel) sits above Channels; collapsing Channels keeps live cards. Quiet rows
  keep a headphones join (hover-revealed on Mac and Rust desktop; dimmed with a
  44-point target on touch and in Rust desktop windows 760px or narrower).
  Phones add a 56-point strip under the chat header: your call's faces (up to
  five, or four and "+N") and Mute, or who is talking there with Join.
- Members stay on the right: a column when there is room, an overlay below the
  chat header otherwise; never under the composer.
- Desktop sidebar drag, arrow keys, bounds, double-click reset and restored
  width match the conversation edge; bottom account controls stay reachable at
  the minimum width.
- Apple workspace/profile/audio dialogs dismiss on the backdrop or Escape on
  Mac; destructive confirmations cannot dismiss during a write. Mac window
  controls sit in the system title bar, not the space rail.
- No global loading bar and no voice-toggle sound for the Members button.
  Pending-send and disconnected/reconnecting states stay visible.
