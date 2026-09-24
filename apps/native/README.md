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

The clients are being aligned with the current website: guest General, account
sign-in and text conversations, owner space/channel/member management, private
grants, presence, typing, and the responsive rail/sidebar/conversation layout.
Implementation checkpoints are not acceptance: check each platform's README and
the exact revision's CI results before relying on a feature. Windows/Linux do not
implement calling yet. Apple and Android expose native voice in default builds,
but neither has a verified locked-phone call. Visual
matching, audio controls, platform lifecycle behavior, and physical-device
acceptance remain in progress.

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
Guest General works without an account. The fixture supports profile edits,
space/channel/member management, private grants, message history/pagination,
idempotent sends, gateway replay, typing, and deterministic presence.

`POST /__fixture/control` accepts `{ "reset": true }`, `{ "disconnect": true }`,
or a one-shot failure such as
`{ "failure": { "path": "/api/auth/email/request", "method": "POST", "status": 503 } }`.
It can inject another participant's typing with
`{ "typing": { "channelId": "chan00000001", "active": true } }`.
Media operations deliberately return 503. Authentication, authorization and
presence are simplified; this fixture is not a substitute for real API tests.

Compare signed-out/error, populated conversation, guest, narrow Browse-open, and
owner-management states. Check actual clicks and keyboard input as well as
screenshots. Desktop reference size is 1440×900, narrow reference 390×844; browser
viewport emulation does not prove native mobile or touch behavior. Each platform
README describes its own preview controls and remaining validation gaps.

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
  privacy disclosures, rollback and a supported update path. No auto-updater is
  installed by the development workflow.

Native voice and background-call acceptance are required work, not optional
product cuts. Do not advertise these development apps as supporting them until
implementation **and physical-device checks** pass. Caper currently has channel
voice, not incoming direct calls; CallKit/Telecom/push ringing would also require
a defined incoming-call backend protocol rather than invented client behavior.

See [native distribution and acceptance](../../docs/media.md#native-distribution-and-acceptance)
for signing, download commands, and the platform validation record.
