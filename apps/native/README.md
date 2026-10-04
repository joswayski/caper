# Native Caper clients

These are **development clients, not feature-parity replacements for caper.chat**.
There is no embedded browser, JavaScript runtime, local HTTP server, Tauri, or
Electron. The website remains available and is not replaced by these builds.

| Platform | Presentation / networking | Project |
| --- | --- | --- |
| macOS | Swift / SwiftUI with AppKit / URLSession | [Apple](apple/README.md) |
| iPhone | Swift / SwiftUI with UIKit / URLSession | [Apple](apple/README.md) |
| Windows, Linux | Rust / egui + wgpu / native HTTP and WebSocket | [Desktop](desktop/README.md) |
| Android | Kotlin / Android Compose / native HTTP and WebSocket | [Android](android/README.md) |

The desktop choices follow Captures' native direction. egui/wgpu is a compiled,
shared renderer for Windows/Linux, not a WebView and not Windows/Linux system
widgets. Apple and Android have separate native UI implementations. All clients
talk directly to the same Caper API. No server or Cloudflare credentials belong
in an app.

The clients are being aligned with the current website: account sign-in and
text conversations, owner space/channel/member management, private
grants, presence, typing, and the responsive rail/sidebar/conversation layout.
Implementation checkpoints are not acceptance: check each platform's README and
the exact revision's CI results before relying on a feature. All clients expose
native voice in default builds, but physical audio and locked-phone calls remain
unverified. Visual
matching, audio controls, platform lifecycle behavior, and physical-device
acceptance remain in progress.

All native clients provide searchable emoji reactions, counted chips, and
own-reaction highlighting. Channel previews display reactions without allowing
mutations until the user joins.

First launch without a valid session opens email sign-in, not a Guest workspace.
New accounts finish their profile, then name their first space. Existing accounts
restore their saved session and open their spaces; logging out returns to sign-in.

## Build and download

The **Native development builds** GitHub Actions workflow uses macOS, Windows, and
Linux runners on relevant pull requests and pushes to `main`, or manual dispatch.
It runs independently from the website/API image pipelines and does not deploy
services, publish a GitHub release, or upload to app stores.

Open a successful workflow run's **Artifacts** section, then select your platform.
Each artifact contains the package, `SHA256SUMS`, and `BUILD.json` recording the
exact checked-out Git revision and development-only distribution status. Artifact
names on PRs refer to the tested merge commit, not necessarily the PR branch tip.
Artifacts expire after 14 days and downloading requires repository access.

Apple and Android jobs also exercise an explicit loopback-only fixture with native
UI automation. They check populated conversations, authentication/error states,
owner management, and narrow Browse layouts, and upload separate `caper-ui-*`
artifacts. Android uses a separately built fixture APK; that APK is not the normal
download package. These checks do not contact the production API or exercise
microphones, Cloudflare audio, or locked-phone calling. A failed UI job does not
establish parity even if its preceding package build passed; inspect the captures
as well as the assertions.

| Target | Package | Installation limits |
| --- | --- | --- |
| Windows x64 | `Caper-windows-x64.zip` | Portable, unsigned; Windows security policy can block it |
| Linux x64 | `Caper-linux-x64.deb`, `Caper-linux-x64.tar.gz` | Ubuntu 24.04 build; native runtime libraries required |
| macOS Apple Silicon | `Caper-macos-arm64.zip` | Development `.app`, not Developer ID signed/notarized |
| macOS Intel | `Caper-macos-x64.zip` | Development `.app`, not Developer ID signed/notarized |
| iOS Simulator Apple Silicon | `Caper-ios-simulator-arm64.zip` | **Cannot install on an iPhone** |
| Android | `Caper-android-debug.apk` | Debug-signed test APK, not a Play/store release |

On a matching development machine, run from the repository root:

```sh
bash apps/native/desktop/build.sh        # Linux
bash apps/native/apple/build.sh macos   # Mac
bash apps/native/apple/build.sh ios     # Mac, iOS Simulator
bash apps/native/android/build.sh       # JDK 17 + Android SDK 36
```

For Windows, run `./apps/native/desktop/build.ps1` in PowerShell. Each project's
README lists its prerequisites, checks, supported behavior, and remaining gaps.

The root Rust workspace remains the API. The desktop client's separate Cargo
workspace/lockfile does not add GUI dependencies to server images. Native builds
do not require `npm install`; the website build is not an app-bundling step.

## Signed builds for testers

The **Native release** workflow (`.github/workflows/release.yml`) builds every
app from one `main` commit:
- a Developer ID-signed, notarized Mac app for Apple Silicon and Intel,
- a release-signed Android APK,
- Windows and Linux desktop packages (not code-signed yet).

It replaces the `native-latest` pre-release, so these links always point at the
newest build:

- Mac (Apple Silicon): https://github.com/joswayski/caper/releases/download/native-latest/Caper-macOS-Apple-Silicon.zip
- Mac (Intel): https://github.com/joswayski/caper/releases/download/native-latest/Caper-macOS-Intel.zip
- Android: https://github.com/joswayski/caper/releases/download/native-latest/Caper-Android.apk
- Windows: https://github.com/joswayski/caper/releases/download/native-latest/Caper-Windows-x64-Setup.exe
- Linux: https://github.com/joswayski/caper/releases/download/native-latest/Caper-Linux-x64.deb

Windows users open Setup once; it installs immediately and opens Caper, with no
Welcome/Next/Finish clicks. Later launches use Start or the desktop shortcut.
There is no extraction or console window. It installs per-user without admin
access and appears in Installed apps for removal. Close any old portable copy
first; the installer does not remove downloads from previous versions.
Linux users on Ubuntu 24.04 or compatible Debian-based systems open the `.deb`
in their software installer, then launch Caper from the application menu without
a terminal. This is not a universal Linux package. ZIP/tar archives remain for
self-updates and advanced portable use, not the default install experience.

**Releasing from Discord.** When **Native development builds** passes on `main`,
`native-ready.yml` posts a "Caper apps build is ready" notification with a
**Deploy Caper apps** button to the production deploys channel. This is the same
channel and webhook the web app, API and gateway notifications use. Godis
dispatches `release.yml` for that exact commit, and the run edits the same
message with the result (`scripts/update-discord-release.sh`). Deploy any server
changes the apps depend on first. The workflow can also be run by hand from the
Actions tab; a blank `git_sha` releases the newest `main` commit.

Signing material is read from AWS Secrets Manager through GitHub OIDC; see
`docs/release-signing.md` in joswayski/infrastructure. Android testers allow
installs from their browser once; updates install over the previous APK because
every release uses the same upload key.

The same run archives the iPhone app (`chat.caper.ios`) and uploads it to
TestFlight with `apps/native/apple/upload-testflight.sh`. Signing is cloud-managed
through the App Store Connect API key, so no distribution certificate or profile is
stored. Each run's build number is `<run number>.<attempt>`. Testers who join the
TestFlight public link get new builds automatically.

**Desktop self-updates.** Release builds of the Mac, Windows and Linux apps carry
`caper-updater` (`apps/native/updater`) and the run number as their build number.
About 20 seconds after launch and every six hours they read `latest.json` from
`native-latest`. That file is signed with the Ed25519 key in
the `caper_update` section of `production/signing/release`, and each app only trusts the public key compiled
into it. When a newer build exists, the Mac app shows an alert (and has
**Caper › Check for Updates…**), and the Windows and Linux app shows a banner.
**Restart to update** hands off to the updater, which verifies the download's
SHA-256 (and, on Mac, its Developer ID team), waits for Caper to quit, swaps in
the new copy and reopens it. A copy it cannot replace, such as the Linux `.deb`
or a Mac app outside a writable folder, gets a **Download** link instead.
That link downloads the matching Mac architecture's ZIP, Windows Setup, or Linux
`.deb` directly; it does not open the GitHub release page. These downloads still
need manual installation.
Releases made before the key is stored skip `latest.json`, and apps from before
this change need one manual reinstall. Android APKs do not self-update yet.

## Comparing native screens with the web

Use the existing web UI as the reference, not platform-default widgets with
roughly similar colors. Both use Satoshi; acquire the official, unmodified native
fonts with `python3 scripts/native_fonts.py`. See [font licensing and integrity
checks](../../shared/fonts/README.md). Do not commit or separately publish fonts.

For disposable visual and interaction checks, Node 24 can run the shared fixture:

```sh
node --test tests/native-parity-fixture.test.mjs
node scripts/native-parity-fixture.mjs
```

It listens only on loopback: HTTP on port 3001 and the unified WebSocket on 3002
(also accepted on 3001). These match the web development proxy defaults. It never
sends email, calls a production API, or connects to an SFU. Sign in with any
syntactically valid test email and the code `ABC234`. The deterministic owner
account opens **Fixture Studio**, containing visibly labeled sample conversations.
The fixture supports profile edits,
space/channel/member management, private grants, message history/pagination,
idempotent sends, gateway replay, typing, and deterministic presence.

`POST /__fixture/control` accepts `{ "reset": true }`, `{ "disconnect": true }`,
or a one-shot failure such as
`{ "failure": { "path": "/api/auth/email/request", "method": "POST", "status": 503 } }`.
It can inject another participant's typing with
`{ "typing": { "channelId": "chan00000001", "active": true } }`.
Read-only voice rosters use `{ "media": { "channelId": "chan00000002", "participants": [{ "id": "fixture-voice", "name": "TEST FIXTURE Voice", "muted": false, "deafened": false }] } }`.
`{ "mediaAccessDenied": { "channelId": "chan00000002" } }` evicts that spectator
subscription; add `"denied": false` to permit future subscriptions again.
Media join and participant-token operations deliberately return 503. Authentication, authorization and
presence are simplified; this fixture is not a substitute for real API tests.

Compare fresh-install sign-in, restoration errors, populated conversation, narrow Browse-open, and
owner-management states. Check actual clicks and keyboard input as well as
screenshots. Desktop reference size is 1440×900, narrow reference 390×844; browser
viewport emulation does not prove native mobile or touch behavior. Each platform
README describes its own preview controls and remaining validation gaps.

### Layout regression checks

Check these states on every native release, not only a wide populated screenshot:

- A fresh install, expired session, or logout shows email sign-in with no Guest
  account, conversation composer, or retired public General fallback. A valid
  saved account still opens its spaces.
- Join shares the channel row when space permits. Crowded voice groups may wrap,
  as in the browser; neither Join nor channel settings may clip. Check the minimum
  desktop sidebar width, long names, and populated voice rosters.
- Members stay on the right: a column when chat has enough room, an overlay below
  the chat header otherwise. The toggle must stay accessible. Never stack members
  underneath the composer. Check 840px desktop windows as well as 1440px and mobile.
- Desktop sidebar drag, arrow keys, bounds, double-click reset and restored width
  must agree with the actual conversation edge. All bottom account controls must
  remain reachable at the minimum width.
- Apple workspace/profile/audio dialogs dismiss on the backdrop or Escape on Mac;
  underlying controls cannot activate through the dialog. Destructive confirmation
  sheets still use protected native presentation and cannot dismiss during a write.
- Mac window controls occupy the system title bar, not the space rail. Apple audio
  sliders retain keyboard/VoiceOver adjustment; iPhone retains the system audio-route
  picker. Sending a message does not introduce a new keyboard-dismissal policy.
- Navigation does not show a global loading bar or play a voice-toggle sound for
  the Members button. Genuine pending-send and disconnected/reconnecting states
  stay visible; hiding them is not a connection fix.

Linux fixture rendering and egui geometry tests cover the shared Windows/Linux
implementation, not Windows rendering. Apple UI tests cover resize, inline Join,
profile backdrop dismissal, audio controls, and narrow layouts but require Xcode.
Android requires its SDK and emulator/device checks. Do not mark either mobile
platform visually accepted based on Linux or browser screenshots.

## Release gates

Passing a build or mock protocol test does **not** establish any of these gates:

- Authenticated end-to-end login, onboarding, logout/revocation, and secure
  credential restoration on each OS.
- Channel authorization, private-channel grants, space/channel management,
  durable chat replay, unknown-outcome retries, typing and member presence parity.
- Native microphone capture/playback through Cloudflare SFU, forced TURN and
  multi-network calls, mute/deafen, device changes and sustained audio.
- iPhone/Android ongoing calls while locked or backgrounded, interruption and
  audio-route handling, Wi-Fi/cellular transitions, and immediate local hangup.
- Desktop/narrow layouts, accessibility, IME, keyboard navigation, high DPI,
  native resource use and battery measurements. Compiled UI alone proves no
  performance improvement over the web implementation.
- Signed package installation/upgrade, platform trust checks, app-store policy,
  privacy disclosures and rollback. Development builds carry no build number, so
  they never self-update; release self-updates are described above.

Native voice and background-call acceptance are required work, not optional
product cuts. Do not advertise these development apps as supporting them until
implementation **and physical-device checks** pass. Caper currently has channel
voice, not incoming direct calls; CallKit/Telecom/push ringing would also require
a defined incoming-call backend protocol rather than invented client behavior.

See [native distribution and acceptance](../../docs/media.md#native-distribution-and-acceptance)
for signing, download commands, and the platform validation record.
