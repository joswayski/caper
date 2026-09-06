# Public voice-channel MVP

## Scope and architecture

One public **General voice channel**, always available to join while the service
is enabled. This is not a dial/invite/call flow. No accounts, text chat,
camera, screen sharing, channel creation, or server-side recording. Mic test offers
an explicit, tab-memory-only recording of up to ten seconds of received audio. Faker generates
an adjective/animal nickname once per explicit join in the browser; the Rust
registry stores it and distributes the same name to every participant. Automatic
reconnect keeps the nickname; explicit leave/join generates another. The lobby
does not persist presence. Names can collide or be impersonated; participant
IDs, not names, distinguish people. Up to 12 people can join with microphone permission,
mute, deafen, choose devices, and leave. Other visitors may record audio.

Browser → same-origin `/api/media/*` → single Rust Axum service → Cloudflare
control API. Browser ↔ Cloudflare Realtime SFU/TURN for WebRTC audio. No media
relays through AWS, Workers, Durable Objects, RealtimeKit or PlanetScale.
Three-second HTTP snapshots provide presence/heartbeat; no WebSocket is needed.

The website has two AWS k3s replicas. A separate **single-replica** Rust service
owns the in-memory participant registry, with one temporary extra pod during
rolling updates. Overlap is intentionally accepted during development, but the
two registries do not share sessions: requests can hit a pod that does not know
the caller, and shutdown still ends the old pod's calls. Do not increase steady-state
replicas or use sticky sessions as a substitute for shared coordination.
Restarting the service clears presence and clients rejoin. Each tab receives an unguessable short-lived device
capability, not an account/login. Clients use Caper track IDs, not arbitrary SFU
session IDs. Cloudflare terminates transport encryption; this is **not E2EE**.

## Provisioned resources and configuration

On September 6, 2026, the owner authorized Cloudflare provisioning. Created in
the connected Cloudflare account:

- Realtime SFU app `caper-voice-mvp`: `fe12588b1edc7a4c05691fc5beaa7ad2`.
- TURN key `caper-voice-mvp`: `d487cddbbd5b033ceb9a601155e0d6d8`.

Credentials are server-only, outside Git. The orb preview uses a private mode-0600
environment file. They have **not** been installed into AWS production. No DNS,
Worker, database, or production deployment was created by this provisioning.
Keep this temporary test separate from any future production app/key.

| Variable | Meaning |
| --- | --- |
| `MEDIA_ENABLED` | `true` enables voice; absent/false disables it |
| `CF_SFU_APP_ID` | SFU app ID, not account ID |
| `CF_SFU_APP_SECRET` | SFU secret, server-only |
| `CF_TURN_KEY_ID` | TURN key ID |
| `CF_TURN_API_TOKEN` | TURN key secret for generating short-lived credentials |
| `MEDIA_BIND` | Default `0.0.0.0:3001` |
| `RUST_LOG` | Suggested `caper_api=info` |
| `MEDIA_API_URL` | Web-process-only local/orb adapter target |
| `DATABASE_URL` | Optional PlanetScale Postgres URL. When set, the API connects and applies `apps/api/migrations` on startup. Use a direct primary URL on port `5432` (`sslmode=verify-full`). Leave empty to boot without a database. |

Use `.env.example`; Rust does not auto-load dotenv files. Export a private env
file before `cargo run -p caper-api`. Run the web process with
`MEDIA_API_URL=http://127.0.0.1:3001 npm run dev:web`. Remote browsers require HTTPS.
In an orb use supervised services and portal URLs, not direct sandbox host URLs.

Image CI builds `apps/api/Dockerfile` and publishes
`production/caper:api-<full-40-character-sha>` after merge to main via `api-image.yml`,
using the existing `production-caper-ecr-publisher` IAM role.
The Docker target and service name are `api` and `caper-api`, respectively.
This is the application control API, not a self-hosted audio relay. Publishing
does not roll it out. The image notification includes a green **Deploy Caper API**
button for that exact SHA and an **Open GitHub** fallback to
`deploy-caper-api.yml` in `joswayski/infrastructure`.
Deployment/Service/ExternalSecret are named `caper-api`; the container is `api`.
AWS Secrets Manager `production/apps/caper-api` supplies Kubernetes Secret
`caper-api-cloudflare`. Keep one desired API replica with `RollingUpdate`,
`maxSurge: 1`, `maxUnavailable: 0`, and a 60-second termination grace, port 3001,
`/api/media` routing, and existing `MEDIA_*` / `CF_*` configuration names.
The active web Deployment/Service remains `caper` with two replicas: renaming it
to `caper-web` requires a separate reviewed rollout and SSM deployment-target update.

Web and API images publish independently. `api-image.yml` runs only when
`apps/api`, workspace Cargo files, the unused desktop crate manifest, or that
workflow change. The API Dockerfile stubs Tauri sources so a desktop-only
refactor cannot skip API image CI and then break the next API build. `aws-image.yml`
runs only when the website Docker context, `apps/web`, `shared`, npm workspace
manifests, or that workflow change. An API-only merge does not publish a website image or
send a **Deploy Caper web** notification, and a website-only merge does not
publish an API image. Future workers or other services should get their own
workflow and path allowlist instead of riding these two.

Outbound HTTPS to `rtc.live.cloudflare.com` is required for the Rust API. AWS
needs no public media UDP ports. Clients use SFU plus TURN UDP and TCP/TLS
fallback; alternate port 53 is filtered from both STUN and TURN URLs. `/api/media/status` reports
the feature flag; web `/api/health` is independent of provider availability.

## Deployment behavior

- Web-only deployments do not reload already-open tabs; production media control
  requests route straight to Rust. Keep API changes compatible with old tabs and
  future desktop releases, which will not all update at deployment time.
- API rolling updates start a replacement and wait for its readiness probe before
  terminating the old pod. This removes the deliberate stop-before-start gap,
  **not** today's call interruptions from separate in-memory registries.
- On SIGTERM, Rust stops accepting connections and gives in-flight HTTP requests
  up to 30 seconds to finish, then spends up to 20 seconds on existing provider
  cleanup. If HTTP draining exceeds its deadline, the process exits without
  provider cleanup rather than racing cleanup against unfinished mutations.
  As with a crash, provider cleanup is not guaranteed and TURN expiry still applies.
- A database connection alone does not make calls survive deployments. Future
  work must persist capabilities, SFU sessions/tracks/subscriptions and leases;
  coordinate participant mutations and cleanup across pods; and replace
  process-exit call teardown with session handoff and expiry-based cleanup.
  Database migrations must remain compatible with both overlapping versions.
- Deploy the infrastructure grace-period change before the new API image. The
  infrastructure repository's `docs/operations.md` describes Flux reconciliation;
  image deployment commands are below. No live rolling-call validation has been
  performed for these changes; client recovery tests use mocked media/API responses.

## Limits and lifecycle

- 12 registered sessions total, one microphone publication each, 11 subscriptions.
  A running Mic test uses two additional private sessions within that same cap.
- 30 joins/minute globally; 120 provider mutations/minute per participant;
  256 KiB request bodies. These are **not a spending cap or DDoS defense**.
- One-hour session/TURN lifetime; 45-second presence lease, sweep every five
  seconds. Background browser throttling can force a reconnect.
- Leave stops local capture immediately, invalidates the capability, force-closes
  SFU tracks/subscriptions, and revokes TURN credentials. Failed track cleanup has
  a bounded retry backlog; crashes/outages can prevent immediate revocation.
  Credentials remain bounded by expiry. No durable cleanup queue exists.
- Serialized negotiations and per-participant operation guards. Ambiguous provider
  creation is not blindly retried; the session is invalidated and cleaned up.
- Transient heartbeat failures (network/timeout, HTTP 408/429/5xx) retry on the
  normal three-second polling cadence without tearing down media. Snapshots have
  a five-second deadline including response-body reads; failures lasting at least
  30 seconds trigger recovery on the next failed poll. Successful snapshots reset
  that window. Invalid sessions trigger recovery immediately. Failed mute/deafen
  state sync is retried with the latest local state after a successful heartbeat;
  ambiguous SFU mutations are not blindly replayed.
- A transient WebRTC `disconnected` state gets ten seconds to recover in place;
  `failed` or a sustained disconnect triggers up to three rejoins retaining mute/deafen and device
  choice. Permission/device failures are visible. All microphone subscriptions
  are automatic. Deafen mutes playback, not forwarding/bandwidth.
- Mute disables the local track and detaches it from the sender. Opus is preferred;
  browser echo cancellation and gain control are off for headphones. DTX is not guaranteed.
- Speaking indicators and diagnostics use browser stats where available, not
  billing records. Microphone/output selectors are available in-channel. Output
  selection requires `setSinkId`; otherwise use OS settings. Joining requires microphone permission.

## Join startup and preparation

The enabled `/live` screen can download and compile DeepFilterNet or RNNoise
before Join when selected by the client. The current UI defaults to DPDFNet-2
and exposes DPDFNet-2/8 only, so it does not download unused DeepFilter assets.
DPDFNet's worker-owned ONNX initialization still happens during Join; this WASM
cache does not prewarm it. The marketing page does not download these assets.
A call-client-owned cache retains compiled DeepFilter/RNNoise code and model
bytes across joins, reconnects and device changes on that screen.
The browser HTTP cache can reuse asset bytes after a full navigation, but the
application does not persist compiled modules across page loads.

Preparation never requests microphone permission, captures audio, creates an
AudioContext, or publishes a track. It shifts up to about 24 MB of asset loading
and compilation earlier, trading memory/bandwidth on the voice screen for less
work after Join. WASM compilation can run concurrently with the model download.
Join still opens the microphone and initializes a dedicated worklet/model
instance, then waits for its ready acknowledgement before publication. A cold
join waits for unfinished preparation; it does not temporarily publish raw audio.
Existing honest browser-suppression fallback on actual processor failure remains.
Downloads have a 30-second timeout; failures are evicted so a later join can retry.
Cancelling a join stops capture immediately without cancelling shared preparation.

September 6, 2026 investigation (not a post-deployment performance guarantee):

- Jose's reported diagnostics: 5,956 ms total; 406 ms microphone + session;
  5,222 ms ICE + signaling; 328 ms transport; 7 ms connected RTT.
- Real Chromium using the existing `localDescription()` helper and Cloudflare
  public STUN: 5,085–5,098 ms with ports 3478 + 53, versus 251–280 ms with only
  port 3478 (three samples each). With port 53, gathering hit the five-second cap.
  These tests did not provision TURN credentials or connect to a live SFU session.
- The API now excludes port 53 from both STUN and TURN, including string/list
  forms, while preserving the supported UDP/TCP/TLS relay routes and credentials.
  The bounded gathering wait remains: do not remove it without TURN-only testing.
- Chromium UI check with generated microphone, real DeepFilterNet and local
  WebRTC signaling fixture: before Join, zero captures and one WASM compilation;
  after join and rejoin, two captures but still only two asset requests and one
  compilation. Both publications used a processed track. Muted rejoin kept the
  sender detached; leave ended the raw track. This is not live SFU, physical
  speech-quality, remote first-decoded-audio, or native desktop validation.

Three-second roster/track polling is still present and can delay an existing
listener's subscription. Planned follow-up, not implemented here: bearer-authenticated
SSE notifications from the same Rust registry, with initial/recovery snapshots,
coalesced invalidation, reconnect backoff, and periodic heartbeat/snapshot fallback.
HTTP retains mutations; SSE only tells clients to reconcile immediately. Do not
put capability tokens in event-stream URLs. The development proxy must stream
without its normal 25-second request deadline; production ingress must not buffer
events. Validate publish/close/leave/expiry notifications, private monitor isolation,
auth revocation, stream disconnects, and simultaneous joins before replacing polling.

Acceptance remains actual received audio, not a faster connected label. Collect
cold/warm join percentiles and first decoded remote audio on two devices/networks,
including forced TURN/TLS, before claiming the startup goal is met.

## Received-audio microphone test

Mic test no longer plays the microphone track directly. It creates a private
sender and receiver, sending the selected processed microphone through WebRTC/Opus
to Cloudflare SFU and receiving/decoding it on a separate PeerConnection. It uses
the same codec preference and SFU/TURN provisioning as channel participants.
Only the received track is played. This exercises the call path, **not the exact
network conditions, headphones or volume of every other participant**.

Pressing **Mic test** establishes the private return and starts recording as soon
as received audio is ready. A timer, received-audio meter and **Stop & play back**
button make the active state visible. Stopping plays the recording automatically;
recording also stops after ten seconds. The browser records the timestamped Opus
return rather than rebuilding a WAV from manually counted PCM frames, preserving
the received stream's real-time playback cadence. No recording is uploaded or
persisted. Testing again, ending the test, leaving, reconnecting or unmounting
discards the recording and cancels capture.

Starting detaches the public sender before enabling private test audio, saves
mute/deafen and marks both true. Stopping restores those choices. Switching a
filter/device replaces the private sender track without replacing the received
stream. Reconnecting recreates the private return. Failure never falls back to
direct local playback and leaves the channel muted until explicit Stop.

Use headphones: delayed self-playback through speakers can feed back and provoke
echo cancellation. This is not a useful way to judge speakerphone double-talk.
Test normal speakerphone conversations with a second participant instead.

Private sessions are tied to the authenticated channel capability. They are
hidden from the roster, have sender/receiver roles, cannot subscribe to public
tracks or another person's test, and are cleaned up on Stop/leave/expiry. They
share global join/capacity limits and add normal SFU/TURN bandwidth charges.
They use the existing routes: `join` accepts `monitor: "sender" | "receiver"`
with the parent's bearer capability; `publish` returns the opaque `trackId`.
The receiver can subscribe only to its sibling sender's publication. No provider
session identifier or arbitrary provider operation is accepted from the browser.

**Rollout:** deploy the updated API before the web image. Older APIs reject the
private join field; the new client will show a failed test and stay muted safely.
No database migration, new secret, or Cloudflare configuration is required.
After merge and both image builds, the operator can deploy the exact merge SHA:

```sh
gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure -f git_sha=<merge-sha>
# Wait for the API rollout to finish successfully, then:
gh workflow run deploy-caper.yml --repo joswayski/infrastructure -f git_sha=<merge-sha>
```

These are operator instructions, not commands automatically run by this change.
The later default-preset/snippet change is web-only and requires no further API
deployment when private received tests are already deployed. After its web image
build, use only the `deploy-caper.yml` command above with that merge SHA.

### Audio setup and speech consistency

Headphones natural input is now the default, with browser echo cancellation and
automatic gain control **off**. Use headphones: speakerphone echo protection is
not enabled, and no selector remains to enable it. This preserves the preset Jose
preferred rather than stacking processing. OS-level processing may still apply.
It does not repair hardware-clipped input or guarantee clean speech.

## On-device noise suppression

DPDFNet-2 48 kHz HR is the default microphone mode. Capture (browser AEC, AGC and
noise suppression off) → 48 kHz mono DPDFNet → MediaStream output track → existing
WebRTC Opus sender → Cloudflare SFU. The model and DSP settings are unchanged from
the experimental option Jose preferred; runtime performance limitations remain.
The new private test changes the control API as described above, not SFU configuration.
No LiveKit dependency, external denoising API,
license server, per-minute inference fee, or raw-audio upload is introduced.

The default model and runtime (~23 MB combined) are vendored and loaded from Caper's
`/audio/dpdfnet2-v1/` path when joining. They are
versioned/cacheable and included by the existing web build/Docker COPY stages.
License notices, source provenance, and checksums are in the adjacent README.
No new environment variables or infrastructure configuration are required.

### Retained engine implementations (no user-facing selectors)

| Mode | Purpose |
| --- | --- |
| DeepFilterNet balanced | 20 dB attenuation limit, retaining about 10% original spectral amplitude. |
| DeepFilterNet gentle | 12 dB limit, retaining about 25% original amplitude; more voice **and noise** return. |
| DeepFilterNet strong | Original 40 dB limit, retaining about 1% original amplitude. |
| RNNoise | Independent lightweight 48 kHz neural model, 3.6 MB same-origin download. No VAD gating. |
| DPDFNet-2 HR (default) | 48 kHz model; approximately 23 MB model/runtime download. Worker-based ONNX inference; substantially heavier than RNNoise. |
| Browser suppression | Built-in baseline; implementation/support varies by browser/device. |
| Off | No requested noise suppression; Audio setup independently controls AEC and automatic gain. |

DeepFilter presets blend the enhanced and time-aligned original spectrum; they do
not retrain the model or change its speech decisions. Post-filter beta is explicitly
zero. Lower limits can soften artifacts but cannot guarantee recovery of clean speech
from loud AC. Neither model fixes clipping already introduced by microphone hardware,
gain control, or echo cancellation. Compare Off too before attributing all artifacts
to denoising. Use headphones for local monitoring: speaker feedback is not a fair
suppression test. Keep microphone position and gain fixed when comparing modes.

As of September 2026, [DeepFilterNet3](https://github.com/Rikorose/DeepFilterNet)
and [RNNoise](https://github.com/xiph/rnnoise) are practical free full-band options;
neither is universally best or demonstrated here to outperform Krisp. Newer research
such as [GTCRN](https://github.com/Xiaobin-Rong/gtcrn) is worth tracking, but its
16 kHz reference path and extra streaming integration are not an automatic upgrade
for natural full-band voice. These are specialist neural audio models, not LLMs.
RNNoise provenance/reproduction is in `apps/web/public/audio/rnnoise-v1/README.md`.
The shared adapter lives at `/audio/noise-v1/`; previously published immutable
DeepFilter assets are unchanged. Normal web deployment includes all new assets;
no operator configuration commands are required.

An in-channel noise-suppression selector offers DPDFNet-2 HR (default) and
DPDFNet-8 HR (experimental) for comparison. Speakers/Headphones stays fixed to
natural headphone input; no mode selector is shown.
Microphone/output selectors remain; output selection also applies to live and
recorded mic-test playback. New visitors use DPDFNet with natural headphone input.
Changing a filter/device clears the old recording immediately and disables
recording during initialization. Runtime fallback also discards any old recording.
Run a fresh mic test after the chosen model reports active. Model 8
adds a lazy 14.9 MB model download and reuses model 2's vendored runtime/DSP.
Use `node scripts/vendor-dpdfnet.mjs 8` to reproduce its model/metadata/licenses.
The active status appears only after the worklet
acknowledges initialization. If loading/initialization fails, capture falls back
to browser suppression when supported, otherwise unsuppressed audio, with an
explicit status. A runtime processor error bypasses the worklet without replacing
or unmuting the outgoing track. This is failure recovery, not an assurance that
all CPU overload or audio artifacts can be detected automatically.

Device/mode changes replace the outgoing track and release the previous hardware
track and AudioContext. Reconnect retains the selected mode. Cancel/leave aborts
downloads and closes both capture and processed tracks; a late permission grant
is released. Mode is in memory for the page lifetime, not persisted to storage.
DeepFilter is not an echo canceller, voice gate, or guaranteed primary-speaker
isolation. It can affect laughter, music, whispers, and natural voice timbre.
The RNNoise/DeepFilter adapter adds 10 ms buffering **in addition to** model and
system latency. DPDFNet uses a 20 ms analysis window, 10 ms hops and three output
hops of startup buffering, plus scheduling/device/network latency. Its Worker
warms up and resets state before readiness. An eight-hop backlog or output
underrun causes explicit browser/raw fallback, not unbounded delay or intermittent
zero-filled output. Do not judge DPDFNet quality when the status says fallback.

DPDFNet model/runtime provenance, checksums, full licenses and reproduction are
in `apps/web/public/audio/dpdfnet2-v1/README.md`. CEVA code/weights are Apache-2.0;
ONNX Runtime is MIT with third-party notices. All assets load from Caper, lazily.
No inference runs inside the AudioWorklet callback and no raw PCM goes to a
denoising service. It is the default based on owner listening feedback, not a
proven universal Krisp replacement.

Quality guidance checked against [upstream DPDFNet](https://github.com/ceva-ip/DPDFNet):
keep the current 960-point unnormalized FFT, 480-sample hop, Vorbis window and
metadata-initialized recurrent normalization. Do not normalize again outside the
model or add an extra gate/AGC. Upstream lists 7.17G MACs for DPDFNet-8 HR versus
2.42G for DPDFNet-2 HR. This is a published operation count, not a measured Caper
CPU/latency result. Both pinned models now pass real stateful inference tests;
model 8 also reaches ready/output in Chromium against the built app. This is not
a physical listening comparison or sustained performance benchmark. Its quality
advantage is unknown; it is offered for owner A/B testing, not promoted to default.
Keep input gain below hardware clipping, use a consistent close mic position,
and disable duplicate OS/vendor voice filters when comparing quality. Check the
active/fallback status before attributing a sound to DPDFNet.

Default-preset / snippet validation, September 6, 2026:

- DPDFNet comparison update: `npm test --workspace @caper/web`: 54 passed;
  `npm run check`: passed. Both model hashes, metadata/stateful inference,
  worker selection and track-preserving fallback checked.
- UI fixture for the previous five-second recorder: switching from model 2 to 8
  cleared the existing recording and retained device selectors. Desktop/mobile inspected.
- Review correction: live-mode exit reads **Stop live listening**, not Back to
  snippet, since entering live mode discards the snippet.
- Chromium local WebRTC receive fixture: five-second, 48 kHz WAV, 480,044 bytes,
  nonzero RMS; silent until play, playback advances, live mode uses the received
  stream, old blob URLs revoked, cancellation leaves borrowed tracks live.
- Entry, recording, ready and live layouts inspected, including 390px mobile;
  device selectors restored after correcting the removal scope. Signaling/channel state mocked and generated tone used. No new
  live Cloudflare, physical speech-quality or native desktop acceptance claimed.
- Recording keeps a muted media element attached to the received stream while
  capturing PCM: Chromium otherwise left its WebRTC jitter buffer undrained and
  produced silent recordings in this test.

Received-test / DPDFNet validation, September 6, 2026:

- All 48 web tests, `npm run check`, Rust formatting, all 12 API tests and
  workspace Clippy passed. API release build passed; Docker daemon unavailable.
- Built-browser desktop and 390px mobile layouts inspected, including received
  playback, headphones warning and DPDFNet fallback. Stop restored the public
  sender. These UI interactions used the local relay fixture described below.
- Real DPDFNet ONNX inference produces finite, nonzero audio. Identity-inference
  FFT/window/OLA reconstruction has unity gain and a 480-sample delay. Adapter
  tests verify startup, frame order with response jitter, overload and underrun.
- This orb's CPU was slower than the 10 ms/hop real-time budget (roughly 25 ms/hop
  in Node); generated-microphone Chromium triggered explicit fallback. Quality
  has **not** been compared with Krisp or on physical microphones.
- Rust private-session tests cover capability scope, hidden roster, sibling-only
  subscription, duplicate/in-flight reservations and parent teardown/expiry.
- Chromium with a local WebRTC relay fixture received actual Opus, nonzero decoded
  audio and a distinct receive track while the public sender stayed detached.
  This fixture re-encodes at its relay; it is **not a live Cloudflare SFU result**.
  Provider credentials were not available for a fresh live SFU acceptance test.
- Headphones capture requested and Chromium reported echo cancellation and AGC
  off; Speakers keeps both requested on. Physical speakerphone double-talk,
  sustained low-end-device performance, other browsers and native desktop remain
  unverified. No promise of artifact-free speech is made.

Alternative-engine validation, September 6, 2026:

- `npm test --workspace @caper/web`: 38 passing tests after rebasing onto the
  merged microphone-monitor PR #24, including real pinned
  RNNoise inference (>6 dB stationary-noise attenuation), engine/preset selection,
  and shared worklet frame ordering. `npm run check` passed.
- Microphone-monitor integration and web typecheck passed; no monitor API changes
  were needed. The monitor feature is provided by PR #24, not duplicated here.
- Generated-microphone Chromium check: balanced, gentle, strong and RNNoise
  reached active status; browser baseline honestly reported unavailable in this
  environment; Off worked. Switching during monitoring retained live/enabled local
  playback and ended the old track. Desktop and 390px layouts inspected. Room
  signaling/PeerConnection were mocked, not the audio processing.
- Built Nitro RNNoise WASM response: HTTP 200, `application/wasm`, immutable cache
  header and matching vendored SHA-256. Docker daemon unavailable.
- No physical AC/voice comparison, new live-SFU test, cross-platform desktop
  capture test, or sustained performance benchmark was performed for these modes.

Original DeepFilter integration validation, September 6, 2026:

- Actual pinned WASM/model inference in Node: finite, nonzero output and >6 dB
  attenuation on deterministic stationary noise (not a speech-quality benchmark).
- Mocked lifecycle tests: readiness, fallback, runtime bypass preserving mute,
  abort/late permission, disposal, and same-origin-only asset loading.
- Chromium with a generated microphone: real AudioContext/AudioWorklet/model;
  processed (not raw) track publication, enhanced/off switching while muted,
  and old capture/context disposal checked. Room signaling was mocked for UI
  verification; this is **not** a new live Cloudflare SFU acceptance result.
- Separate real Chromium WebRTC loopback: 9,646 bytes received, 183,360 decoded
  samples, nonzero audio energy, zero concealed samples in a four-second check;
  replacing the enhanced track with Off succeeded. This tests local Opus transport,
  not physical speech quality, sustained performance, or the Cloudflare network.
- Built Nitro app: real worklet reached active status; model HTTP download hash
  matched the vendored bytes with no Content-Encoding transformation. Docker
  daemon was unavailable; web build stages and built-server asset serving were
  validated directly, not via a container run.
- Desktop/mobile browser layouts inspected; physical microphones, headset and
  speakerphone echo, speech preservation, sustained CPU/gaming load, Firefox,
  Safari, and all Tauri webviews/native capture remain unvalidated.

Desktop direction: the existing Tauri shell has no working voice implementation.
This browser adapter is not a native desktop audio pipeline. The same model can
be used with upstream Rust `libDF`; native capture, echo cancellation, and feeding
processed PCM into the chosen native WebRTC sender still need implementation.
Do not transport continuous PCM through ordinary Tauri command/event IPC.

## Cost and acceptance

[Cloudflare SFU/TURN pricing](https://developers.cloudflare.com/realtime/sfu/pricing/)
was checked September 6, 2026: **outbound GB × $0.05**, ignoring credits/allowances.
The combined SFU/TURN path is not double-charged. AWS application costs are separate.
This is an unmoderated temporary test; anyone can consume capacity/bandwidth.

Automated tests cover ownership, expiry, limits, non-voice rejection, TURN
cleanup, provider errors, negotiations, permission/leave races, mute, deafen,
and the fixed-path adapter. These tests alone do not prove live media works.

Live Chromium test, September 6, 2026: two browser participants exchanged Opus
audio through the actual SFU (over 180 KB sent and received each, nonzero decoded
audio energy). One participant forced relay; selected ICE stats confirmed a
successful **TURN/TCP** connection. Mute detached the sender; deafen muted the
audio element; leave closed the PeerConnection, removed playback, and cleared
the participant from the other browser's roster. Input was a generated tone,
not a physical microphone. No SFU/signaling responses were mocked. This test
caught and fixed session creation incorrectly sending `{}` instead of no body.

Physical microphone/speaker quality, multiple networks, prolonged sessions,
mobile background behavior, Firefox/Safari, and Tauri remain separate checks.

Before wider testing, use physical browsers on two networks; verify audible
speech, device unplug/change, denial, repeated join/leave, restart, mute/deafen,
and a long session. Force relay in a test-only PeerConnection override and inspect
selected ICE stats, then test a restrictive network including TCP/TLS fallback.
Do not infer TURN success from ordinary Wi-Fi. Compare muted/speaking RTP deltas.

## Runbook

- Unavailable: inspect feature flag, `/health`, ingress routing, secret names,
  and outbound HTTPS. Never print credentials, SDP or raw media in logs.
- Silent: check microphone permission, mute/deafen, output selection and Play
  audio fallback, then incoming/outgoing RTP and selected ICE candidate stats.
- Stale roster: verify exactly one Rust API instance, snapshots and lease sweep.
- Provider cleanup failure: use private operator credentials to inspect/close
  tracks, never a public admin proxy. Disable new joins during provider outages.
- Rotation: create replacement app/key, update the private deployment secret,
  restart via the approved deployment path, verify two clients, then revoke old
  resources. Never put these secrets in browser bundles or desktop builds.
- Take down: disable `MEDIA_ENABLED` and restart the API (GitOps in production;
  stop the supervised media service for the orb). Revoke the temporary SFU app
  and TURN key after testing. The website can remain up.
