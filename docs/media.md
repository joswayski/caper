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
Cloudflare's IP Geolocation setting adds an approximate country code at ingress;
the registry keeps that code for the call and shares it in roster snapshots. Caper
does not retain the visitor IP itself. Unknown and Tor locations are omitted.

Browser → same-origin `/api/media/*` → single Rust Axum service → Cloudflare
control API. Browser ↔ Cloudflare Realtime SFU/TURN for WebRTC audio. No media
relays through AWS, Workers, Durable Objects, RealtimeKit or PlanetScale.
Authenticated SSE invalidations provide immediate public roster/track discovery.
Fifteen-second HTTP snapshots renew presence leases and repair missed state; HTTP
also carries commands. Audio still uses WebRTC, not SSE or WebSockets.

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
| `RUST_LOG` | Default `caper_api=info,tower_http=info` |
| `LOG_FORMAT` | `json` for structured stdout; otherwise human-readable output |
| `ENVIRONMENT` | Log resource environment; default `development` |
| `AXIOM_TOKEN` | Optional server-only ingest API token; absent/empty disables export |
| `AXIOM_DATASET` | Defaults to the existing `caper` dataset |
| `AXIOM_ENDPOINT` | Required when a token is set: the dataset's actual HTTPS Axiom edge URL ending in `/v1/logs`; no region is assumed |
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

## Axiom API logging

The Rust API exports structured **log events**, including events outside request
spans, through OpenTelemetry OTLP/HTTP to the existing `caper` Events dataset.
This follows the Godis logging convention rather than installing a cluster-wide
collector. Stdout remains available; browser and native-client telemetry are not
collected. Axiom setup documentation: https://axiom.co/docs/send-data/opentelemetry.

`AXIOM_TOKEN` must be an ingest-only API token authorized for `caper`. Do not
commit it, prefix it with `VITE_`, put it in command-line arguments, or paste it
into a PR/chat. `AXIOM_ENDPOINT` is the dataset's actual Axiom edge URL plus
`/v1/logs` (for example `https://<your-edge>.axiom.co/v1/logs`). The application
requires HTTPS, an Axiom hostname, and no embedded credentials/query/fragment.
It deliberately does not default to Godis's region. Do not mix this configuration
with `OTEL_EXPORTER_OTLP_HEADERS` or `OTEL_EXPORTER_OTLP_LOGS_HEADERS`: those can
override the selected authorization/dataset and therefore disable this exporter.
Missing tokens and invalid Axiom configuration leave voice available with local
logging; configuration errors are reported without echoing supplied values.

Request logs contain `event_name=http_response`, status, numeric `duration_ms`,
router-owned `http_route`, method, and a generated request ID. Durations measure
time to response headers, not an SSE stream's lifetime. Log resources identify
`service.name=caper-api`, package version, environment, and pod `HOSTNAME`.
Only Caper events and Tower's HTTP failure events are exported; span attributes
are limited to method, matched route, and request ID. No headers, query strings,
SDP, credentials, raw media, provider session IDs/MIDs, arbitrary unmatched URLs,
or dependency debug output are exported. Future log statements must follow the
same privacy rules—this is not an arbitrary-string redaction engine. The richer
provider error references from reliability PR #66 are exported when that work is
also deployed; enabling Axiom alone does not add those diagnostic fields. Legacy
cleanup events with session/MID fields remain stdout-only; PR #66 replaces those
with exportable diagnostics.

The SDK's dedicated worker batches up to 128 records, flushing every second,
with a 2,048-record queue and a two-second export timeout. Producers use
non-blocking enqueue; full queues drop records rather than delaying voice.
Failed batches are not automatically retried or durably buffered. Axiom export
failures print a static stderr warning; SDK internal logging is disabled because
its debug errors can contain upstream bodies/URLs. Queue overflow can lose logs
without an individual warning. Shutdown allows at most three seconds for the
caller to flush after HTTP draining and media cleanup; a timed-out worker can
continue until process exit. Stdout is the fallback, not a replay queue.

### Enable it after merge

Production needs the companion infrastructure manifest change: separate optional
`caper-api-axiom` ExternalSecret/envFrom, leaving mandatory Cloudflare credentials
unchanged. Add `AXIOM_TOKEN` and `AXIOM_ENDPOINT` to the existing AWS Secrets Manager
record `production/apps/caper-api`, preserving **every existing property**. The
infrastructure operations runbook includes secure private-file upload commands;
the Secrets Manager console's key/value editor is also suitable. No IAM or
Terraform apply is needed. Wait for the new ExternalSecret to be Ready, then use
**Deploy Caper API** for the logging-capable merged image. No web deployment is
required for logging. Optional equivalent deployment command:

```sh
CAPER_API_SHA=REPLACE_WITH_FULL_MERGED_LOGGING_COMMIT_SHA
gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$CAPER_API_SHA"
```

If that image is already running, secret updates alone do not change process
environment. After ESO sync, an authorized operator must restart the API using
the infrastructure runbook; restarting currently interrupts calls. To disable
export, set the existing remote `AXIOM_TOKEN` property to empty, sync, and restart.
Deleting its properties can leave ESO's last successfully retained token intact.

With `LOG_FORMAT=json`, inspect only the safe startup marker:

```sh
kubectl -n default logs deployment/caper-api -c api --since=5m \
  | jq -c 'select(.event_name == "telemetry_initialized") | {event_name, axiom_enabled}'
```

`axiom_enabled: true` means configured, not delivery verified. Confirm recent
`service_starting` and `http_response` records in Axiom's `caper` dataset. Tests
cover standalone logs, span allowlisting, route/query privacy, OTLP headers and
payloads against a mock collector, queue saturation, and bounded shutdown during
an exporter outage. No token or endpoint was supplied during implementation, so
live ingestion into the user's dataset has not been verified.

## Deployment behavior

- Web-only deployments do not reload already-open tabs; production media control
  requests route straight to Rust. Keep API changes compatible with old tabs and
  future desktop releases, which will not all update at deployment time.
- API rolling updates start a replacement and wait for its readiness probe before
  terminating the old pod. This removes the deliberate stop-before-start gap,
  **not** today's call interruptions from separate in-memory registries.
- On SIGTERM, Rust ends SSE streams, stops accepting connections and gives in-flight HTTP requests
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
- Transient heartbeat failures (network/timeout, HTTP 408/429/5xx) retry after
  three seconds without tearing down media (healthy heartbeat cadence remains 15 seconds). Snapshots have
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

## Leaving voice

Explicit Leave/Cancel and page unload share synchronous local teardown: stop
capture and processed tracks, close the PeerConnection and event stream, clear
playback/roster state, and invalidate the old call generation. Explicit Leave
then shows an enabled Join button without awaiting the provider cleanup response.
The old capability is captured before reset and sent to `/leave` with Fetch
`keepalive`; completion or failure never changes the next call's state. Late
peer events and microphone replacement completions are also session-guarded.
Automatic reconnect and join-error teardown still await server cleanup.

The four-to-five-second leave delay was separate from ICE gathering. The Rust
endpoint already removes registry membership/tokens and notifies SSE listeners
before awaiting TURN revocation and track/dependent cleanup. The previous client
kept its `leaving` phase until that entire HTTP request returned. Server cleanup
still runs; it is no longer a UI gate. If the request cannot reach the API,
remote presence may persist until the existing 45-second lease expires; provider
cleanup retries and credential expiry remain unchanged. Local audio stays stopped
even on cleanup failure. Immediate rejoin still obeys the server's capacity and
join-rate limits.

Verification with the real browser UI, DPDFNet processing of synthetic silence,
and the production API through the development adapter: desktop Join became
enabled 8 ms after Leave while all captured tracks were ended and peers closed;
the real `/leave` response took 5,600 ms. Holding that response in the browser
allowed a successful next Join before releasing it, without interrupting the new
call. At a 390px mobile viewport the same UI transition took 11 ms. These are
single local browser observations, not latency percentiles or physical-device
coverage. An initial cold join failed before the leave test and was retried;
these results do not resolve the earlier cold-start limitation.

This is a web-only rollout. Deploy its merged web image through the existing
`deploy-caper.yml` workflow; no API image, migration, or provider changes are
required. Check that Leave immediately stops local audio and restores Join while
an old `/leave` request is pending, including repeated leave/rejoin and a failed
cleanup response. Do not treat background request duration as local leave latency.

## Join startup and preparation

The enabled `/live` screen starts one DPDFNet-8 worker before Join. It downloads
the model/runtime, initializes ONNX inference, and runs the existing synthetic
warm-up before acknowledging readiness. Its live recurrent and overlap-add state
start fresh after warm-up. The marketing page does not start this work, and the
voice page does not download unused DeepFilter assets.

Preparation never requests microphone permission, captures audio, creates an
AudioContext, provisions an SFU session, or publishes a track. It shifts roughly
27 MB of model/runtime loading and initialization earlier, trading bandwidth and
one idle worker's memory on the voice screen for less work after Join. This is
preparation for the next capture, not a pool of spare workers during a call.
The browser HTTP cache can reuse bytes after navigation; the application does
not persist initialized workers across page loads.

Join still opens the microphone and initializes a dedicated worklet, but takes
exclusive ownership of the prepared worker rather than starting another model
instance. It waits for any unfinished initialization, SSE readiness, transport,
and initial roster/state synchronization before enabling outgoing audio. A cold
join does not temporarily publish raw audio. Filter initialization failure fails
Join instead of downgrading. DPDFNet runtime overload or underrun switches the
existing processed track to a bypass, preserving the call, and attempts to enable
the browser's microphone noise-suppression constraint. The reported track setting
determines whether the UI says browser suppression is active or unavailable.
An unrecoverable processor error stops and unpublishes the microphone without
restarting an otherwise healthy connection.

Preparation has a 60-second readiness timeout. Failed workers are terminated and
evicted so a later join can retry. Cancellation terminates a worker already handed
to capture. Page exit also disposes unused preparation. Explicit Leave stops the
used worker immediately and starts fresh preparation for the next Join; a worker
that has processed microphone samples is never reused by another capture. Device
changes and automatic recovery still create fresh workers without holding a spare
throughout the call. The legacy DeepFilter/RNNoise client modes retain their
separate shared code/bytes cache and 30-second download timeout.

The displayed join duration measures click-to-locally-ready-to-talk, not the first
word heard on a remote device. Remote subscription, receiver jitter buffering and
playback still affect join-to-heard latency. Keep two-device listening checks
separate from local preparation benchmarks.

September 7, 2026 verification (Chromium in an orb, synthetic silence, not physical
microphone or remote-listening measurements):

- Three unprepared captures with cached asset bytes took 403–464 ms to become
  filter-ready; three prewarmed captures took 6–7 ms. Their 395–447 ms preparation
  ran before capture, without microphone access or AudioContext creation. All six
  kept DPDFNet-8 active and produced worker output during the 300 ms smoke check.
- The actual voice UI joined through the production API/SFU in 1,781 ms from the
  orb. It reused one worker, showed a live disabled sender while connecting, and
  enabled it only with the connected UI. Leave ended capture/closed the peer and
  prepared a fresh worker. A separate client probe joined in 2,056 ms with no
  observed enabled sender before connected; prewarm made zero media API calls.
  These network timings are not comparable to Jose's earlier 707 ms measurement.
- A CPU-contended run on the animated landing page hit processor failure, including
  without prewarming. Do not treat preparation as a fix for sustained inference
  overload or relax the fail-closed guard to manufacture better timing. Repeat
  physical speech and constrained-device checks separately.
- PR #53's relocated v1 browser runtime failed `WebAssembly.validate`; the deployed
  copy was also invalid when inspected. v2 restores the original pinned runtime
  with a binary-safe copy and changes the URL to avoid cached corrupt responses.
  A new test checks the shipped runtime's size, SHA-256 and WASM validity, in
  addition to the model inference test (which loads the runtime from node_modules).

Deploy the web image after merging, then refresh the app. No API rollout or new
provider configuration is needed. The new worker request uses `/audio/dpdfnet8-v2/`.

September 6, 2026 investigation (not a post-deployment performance guarantee):

- Jose's reported diagnostics: 5,956 ms total; 406 ms microphone + session;
  5,222 ms ICE + signaling; 328 ms transport; 7 ms connected RTT.
- Real Chromium using the existing `localDescription()` helper and Cloudflare
  public STUN: 5,085–5,098 ms with ports 3478 + 53, versus 251–280 ms with only
  port 3478 (three samples each). With port 53, gathering hit the five-second cap.
  These tests did not provision TURN credentials or connect to a live SFU session.
- The API now excludes port 53 from both STUN and TURN, including string/list
  forms, while preserving the supported UDP/TCP/TLS relay routes and credentials.
  This alone did not eliminate Jose's later five-second stall; see the follow-up below.
- Chromium UI check with generated microphone, real DeepFilterNet and local
  WebRTC signaling fixture: before Join, zero captures and one WASM compilation;
  after join and rejoin, two captures but still only two asset requests and one
  compilation. Both publications used a processed track. Muted rejoin kept the
  sender detached; leave ended the raw track. This is not live SFU, physical
  speech-quality, remote first-decoded-audio, or native desktop validation.

### ICE-gathering follow-up: signal before every probe completes

After the port-53 fix deployed, Jose measured 5,631 ms total, with 246 ms in
microphone/session setup and 5,226 ms in ICE/signaling. A fresh Chromium probe
using production-issued ICE settings confirmed no port-53 URLs remained. With
all configured servers, direct and relay candidates appeared within 126 ms, yet
gathering still had not completed at 6,500 ms. STUN-only port 3478 and TURN/UDP
also remained pending; TLS-only gathering completed in 129 ms. These observations
identify the application-level wait for global gathering completion, not the
particular unanswered network probe or Jose's device/network topology.

`localDescription()` now returns the current SDP immediately after
`setLocalDescription()` rather than waiting up to five seconds. This follows
[Cloudflare's browser echo example](https://github.com/cloudflare/realtime-examples/blob/main/echo/index.html),
which posts the offer before gathering completes. It adds no undocumented
candidate endpoint, drops no TURN URLs, and retains the same WebRTC connection
and microphone readiness checks. ICE gathering can continue in the browser;
SDP availability alone never enables public microphone audio.

Live SFU verification used private sender/receiver monitor sessions, a synthetic
oscillator (no hardware microphone or public audio publication), and an audio
sink with volume zero. Both the initial offer and receiver answer contained zero
candidate lines when submitted. Chromium received decoded samples with normal
routing and forced relay-only UDP, TCP, and TLS-on-443 configurations:

| Route configuration | Publication HTTP + offer setup | Both transports connected | First nonzero received-sample observation |
| --- | ---: | ---: | ---: |
| Normal (selected server-reflexive UDP) | 244 ms | 1,840 ms | 2,144 ms |
| Relay-only TURN/UDP | 289 ms | 1,864 ms | 2,066 ms |
| Relay-only TURN/TCP | 214 ms | 1,816 ms | 2,018 ms |
| Relay-only TURN/TLS, port 443 | 273 ms | 2,881 ms | 2,982 ms |

These are single samples from one Linux orb's Chromium/network, measured from
sender offer creation, **not** full UI join times, model initialization costs,
physical speech-quality tests, latency percentiles, or native webview validation.
TCP/TLS stats confirmed relay candidate selection on both peers; the first UDP
sample returned no nominated-pair rows, although relay-only policy was enforced
and decoded samples arrived. All temporary sessions were released. Receiver
subscription was requested after sender transport connected. An initial probe
with a suspended AudioContext returned subscription HTTP 502; enabling the test
context resolved that fixture failure. Safari/Firefox, real restrictive networks,
sustained voice, and Jose's own join-to-heard timing still need acceptance checks.

The actual modified `PublicCallClient`, served by local Vite and using the
production API through the same-origin adapter, also completed a warm join with
real DPDFNet processing of synthetic silence: 1,576 ms total; microphone/session
468 ms, live updates 107 ms, ICE/signaling 447 ms, transport 345 ms, roster/state
208 ms. The selected filter reported active; no observed pre-connected state had
an enabled sender. An earlier cold attempt failed at 23,580 ms with a live-update
disconnect; its underlying cause was not isolated. This is not a cold-start or
all-networks success guarantee. Additional private probes subscribing immediately
after the publisher's answer also received decoded samples (normal and TLS-only).
Two intervening probes failed during Join with HTTP 502 around an API rollout;
later probes succeeded, but the errors were not conclusively attributed.

Rollout: this follow-up changes web only. First deploy the API from the merged
SSE/readiness PR #45 (or a later API image containing it), then deploy the web
image from this fix's merge commit. A web-only merge does not publish an API image.

### Required live-update and audio readiness

`GET /api/media/events` uses the existing Bearer capability in an Authorization
header, not a query string or cookie. The browser consumes SSE with streaming
Fetch so the capability never enters the URL. The server emits `ready` immediately,
coalesced `changed` invalidations after public roster mutations, and `heartbeat`
after ten seconds without another event; each event has `{}` data. Clients fetch
the authoritative snapshot on a change and retain a dirty flag for changes during
an in-flight reconciliation. No private monitor session is exposed or authorized
to receive the public stream. One stream per participant is retained; a new one
replaces the previous stream. Auth/expiry is rechecked for every event, and SSE
alone never renews the lease. There is no durable event log or second registry.

Join/rejoin waits for the selected audio processor before publication. The SSE
handshake and publication run concurrently; the published track stays disabled
(silence). After both finish, mute/deafen state synchronization overlaps the
transport handshake. Snapshot/subscription negotiation still waits for transport
and the initial state acknowledgement. A newer dirty state is repaired before
completion, and SSE invalidations during roster synchronization are drained.
Only after actual SSE readiness, transport connection, initial roster/subscription
negotiation, state synchronization and a final live-stream/track check does the client enable audio
and show Connected, respecting mute/monitor state. This gates the joining client's
setup; it does not wait for an acknowledgement from every remote speaker device or
guarantee another listener's autoplay, deafen, network or playout state.

Diagnostics now report elapsed groups: `microphone + session`, `signaling + live
updates`, `transport + state`, and `roster`. Parallel work is not added twice;
these groups are not directly comparable to the earlier individual-stage labels.
There is no pre-Join SFU connection, provider change, API schema change or audio
quality change. A web deployment and page refresh activate the new ordering.

September 7 concurrency validation: `npm run check` passed; web tests passed,
including both handshake completion orders, failure/cancellation of concurrent
startup, dirty-state repair and the existing subscription/readiness gates. Real
Chromium UI with synthetic silence and the production API/SFU issued `events`
and `publish` five milliseconds apart, then started `state` while transport was
still `new`. With another participant present, that orb run took 2,649 ms,
including 1,424 ms of roster/subscription work (one `subscribe` plus `negotiate`).
This is not a before/after speed claim or comparable to Jose's 537–741 ms runs.
A second real-SFU check deliberately held delivery of the SSE handshake: transport
connected and publication completed, but the UI stayed Connecting and the sender
stayed disabled. Releasing the handshake allowed the remaining gates to finish
and enabled the DPDFNet-8-processed track. Physical join-to-heard remains unmeasured.

Further latency work should measure provider-request duration separately from
API/client time and evaluate batched subscriptions for populated channels. Merely
replacing SSE with WebSockets does not remove media transport establishment or
Cloudflare control calls. Standby media transports would require separating
channel membership from connection lifetime, including authentication, capacity,
expiry and idle-resource policy; they are not implemented here.

No handshake within ten seconds or no valid event within 25 seconds fails the
stream. An SSE failure during startup fails Join; during an established call it
reopens only the event stream with the same capability, retrying after three
seconds while keeping healthy audio. A 30-second recovery deadline after loss
triggers the bounded full-session reconnect if live updates cannot be restored.
Each restored stream triggers a snapshot to reconcile missed invalidations;
there is no event replay requirement. Initial Join readiness remains strict.
Cancel/leave aborts the stream, startup event waits, and delayed retries. Fifteen-second snapshots
remain a lease heartbeat/recovery mechanism, not the normal track-discovery delay.

The development adapter forwards the streaming body and cancellation and limits
only the wait for SSE response headers, not the stream's lifetime. The API and
adapter send `X-Accel-Buffering: no`; production intermediaries must also stream
without buffering. The infrastructure repository routes `/api/media` to the API
through Traefik with no path/method restriction or explicit buffering middleware.
Dashboard-managed Cloudflare Tunnel settings and running cluster settings were
not verified; test the public stream through the deployed hostname after rollout.
Deploy the new API before the new web image: new clients require the endpoint,
while older clients remain compatible with the additive API. Keep one API replica.

Readiness validation uses provider/router mocks, real streaming response bodies,
client-controlled handshake/transport/negotiation/state delays, cancellation,
stream watchdogs, coalesced changes, and enhanced-filter failure tests. These are
not new live SFU/TURN or remote first-decoded-audio acceptance results.

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

Pressing **Mic test** establishes the private return. **Record microphone** starts
recording explicitly. A timer, received-audio meter and **Stop & play back**
button make the active state visible. Stopping plays the recording automatically;
recording also stops after ten seconds. The browser records the timestamped Opus
return rather than rebuilding a WAV from manually counted PCM frames, preserving
the received stream's real-time playback cadence. No recording is uploaded or
persisted. Testing again, ending the test, leaving, reconnecting or unmounting
discards the recording and cancels capture.
Reconnects and device/filter changes return to the Record button, never start
recording automatically. Status updates and output changes preserve playback.
A decoded recording with no samples above 0.001 amplitude produces a no-signal
warning; a nonempty Opus container or connected peer alone is not proof of sound.
This is a silence check, not a speech-quality score. Blocked autoplay leaves
native Play controls available; output-device denial is reported separately.

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

DPDFNet-8 48 kHz HR is the default microphone mode: capture (browser AEC, AGC and
noise suppression off) → 48 kHz mono DPDFNet → MediaStream output track → existing
WebRTC Opus sender → Cloudflare SFU. Runtime performance limitations remain.
The new private test changes the control API as described above, not SFU configuration.
No LiveKit dependency, external denoising API,
license server, per-minute inference fee, or raw-audio upload is introduced.

The default model and runtime (~27 MB combined) are vendored and loaded from Caper's
`/audio/dpdfnet8-v2/` path during voice-page preparation or an unprepared join. They are
versioned/cacheable and included by the existing web build/Docker COPY stages.
License notices, source provenance, and checksums are in the adjacent README.
No new environment variables or infrastructure configuration are required.

### Retained engine implementations (no user-facing selector)

| Mode | Purpose |
| --- | --- |
| DeepFilterNet balanced | 20 dB attenuation limit, retaining about 10% original spectral amplitude. |
| DeepFilterNet gentle | 12 dB limit, retaining about 25% original amplitude; more voice **and noise** return. |
| DeepFilterNet strong | Original 40 dB limit, retaining about 1% original amplitude. |
| RNNoise | Independent lightweight 48 kHz neural model, 3.6 MB same-origin download. No VAD gating. |
| DPDFNet-8 HR (default) | 48 kHz model; approximately 27 MB model/runtime download. Worker-based ONNX inference; substantially heavier than RNNoise. |
| Browser suppression | Built-in baseline retained internally; implementation/support varies by browser/device. |
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
The shared adapter lives at `/audio/noise-v1/`. Normal web deployment includes
all new assets; no operator configuration commands are required.

DPDFNet-8 HR is the fixed in-channel noise-suppression filter; no engine selector
is shown. Speakers/Headphones stays fixed to natural headphone input.
Microphone/output selectors remain; output selection also applies to live and
recorded mic-test playback. Changing a device clears the old recording immediately
and disables recording during initialization. Runtime failure also discards any old
recording. Run a fresh mic test after DPDFNet reports active. Its 14.9 MB model and
runtime load lazily from the same versioned asset directory.
Use `node scripts/vendor-dpdfnet.mjs 8` to reproduce its model/metadata/licenses.
The active status appears only after the processor acknowledges initialization.
Loading/initialization failure rejects capture and stops its tracks. Runtime overload
or underrun keeps the existing microphone track live, attempts browser suppression
and reports whether it is active; unsupported or rejected browser suppression leaves
unprocessed audio. This avoids a call-wide reconnect loop.
An unrecoverable AudioWorklet processor error still stops the microphone. This does
not ensure that all CPU overload or audio artifacts can be detected.

Device/mode changes replace the outgoing track and release the previous hardware
track and AudioContext. Reconnect retains the selected mode. Cancel/leave stops
capture and processed tracks; bounded shared asset preparation can finish for reuse.
A late permission grant is released. Mode is in memory for the page lifetime, not persisted to storage.
DeepFilter is not an echo canceller, voice gate, or guaranteed primary-speaker
isolation. It can affect laughter, music, whispers, and natural voice timbre.
The RNNoise/DeepFilter adapter adds 10 ms buffering **in addition to** model and
system latency. DPDFNet uses a 20 ms analysis window, 10 ms hops and three output
hops of startup buffering, plus scheduling/device/network latency. Its Worker
warms up and resets state before readiness. An eight-hop backlog or output
underrun terminates inference and switches the existing worklet output to raw input,
rather than ending the track or building unbounded delay.

DPDFNet model/runtime provenance, checksums, full licenses and reproduction are
in `apps/web/public/audio/dpdfnet8-v2/README.md`. CEVA code/weights are Apache-2.0;
ONNX Runtime is MIT with third-party notices. All assets load from Caper, lazily.
No inference runs inside the AudioWorklet callback and no raw PCM goes to a
denoising service. It is the default based on owner listening feedback, not a
proven universal Krisp replacement.

Quality guidance checked against [upstream DPDFNet](https://github.com/ceva-ip/DPDFNet):
keep the current 960-point unnormalized FFT, 480-sample hop, Vorbis window and
metadata-initialized recurrent normalization. Do not normalize again outside the
model or add an extra gate/AGC. Upstream lists 7.17G MACs for DPDFNet-8 HR. This is
a published operation count, not a measured Caper CPU/latency result. The pinned
model passes real stateful inference tests and reaches ready/output in Chromium
against the built app. This is not a physical listening comparison or sustained
performance benchmark.
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
  MediaRecorder is active: Chromium otherwise leaves its WebRTC jitter buffer
  undrained and can produce an empty recording.

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

Microphone-playback verification, September 7, 2026 (UTC):

- Reproduced DPDFNet stopping the capture during join before the current runtime
  bypass behavior. In this CPU-only orb,
  its first five processed hops averaged 15.0 ms (18.6 ms maximum) against a
  10 ms/hop budget. Its former fail-closed path could stop audio, trigger reconnect,
  and remount an automatically recording test. DPDFNet remains the fixed product
  choice; the current path bypasses it when that CPU-performance risk occurs.
- The real private SFU return produced a 1.98-second decoded recording with
  peak amplitude 0.204 from synthetic 440 Hz input. Switching the UI microphone
  selector requested the exact second device ID and changed the returned
  recording to 880 Hz (measured over its middle second), RMS 0.142.
- Forced TURN on both private peers: selected candidates confirmed relay/UDP;
  106,608 outbound and 102,951 inbound RTP bytes were observed. The returned
  1.98-second recording had peak amplitude 0.204. Signaling/SFU were real;
  capture and device enumeration were synthetic. This does not test physical
  browser noise suppression; the synthetic track honestly reported it unavailable.
- Stop/replay and subsequent stats updates preserved the recording; device
  replacement cleared it without starting a new one. Silent returned audio
  showed the warning. Ending/restarting the test left zero active recorders;
  Leave closed all peers and removed playback. Desktop and 390px mobile
  ready/playback/warning layouts were inspected.
- Repeatable component regression: start Vite, then run
  `node scripts/test-mic-playback.mjs http://localhost:5174` (adjust the port;
  requires `agent-browser`). It uses real Chromium recording/decoding with
  synthetic local input, not SFU mocks presented as live validation. It covers
  explicit start, stop/replay, parent updates, cancellation, stream replacement,
  URL cleanup, silence, the ten-second limit, output denial/recovery, autoplay
  blocking, and unmount cleanup.

This is a web-only change; no API deployment or provider configuration is required.

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
- Stale roster: verify exactly one Rust API instance, an authenticated `/events`
  stream with immediate `ready` and ten-second heartbeats, event flushing through
  intermediaries, snapshots and lease sweep. Do not paste auth-bearing HAR files.
- Startup failure after a rollout: new web clients require the SSE endpoint;
  deploy the API first. Filter failures deliberately stop rather than downgrade.
- Provider cleanup failure: use private operator credentials to inspect/close
  tracks, never a public admin proxy. Disable new joins during provider outages.
- Rotation: create replacement app/key, update the private deployment secret,
  restart via the approved deployment path, verify two clients, then revoke old
  resources. Never put these secrets in browser bundles or desktop builds.
- Take down: disable `MEDIA_ENABLED` and restart the API (GitOps in production;
  stop the supervised media service for the orb). Revoke the temporary SFU app
  and TURN key after testing. The website can remain up.
