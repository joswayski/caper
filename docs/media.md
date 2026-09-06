# Public voice-channel MVP

## Scope and architecture

One public **General voice channel**, always available to join while the service
is enabled. This is not a dial/invite/call flow. No accounts, text chat, database,
camera, screen sharing, channel creation, or recording by Caper. Faker generates
an adjective/animal nickname once per explicit join in the browser; the Rust
registry stores it and distributes the same name to every participant. Automatic
reconnect keeps the nickname; explicit leave/join generates another. No database
or browser storage is needed. Names can collide or be impersonated; participant
IDs, not names, distinguish people. Up to 12 people can join with microphone permission,
mute, deafen, choose devices, and leave. Other visitors may record audio.

Browser → same-origin `/api/media/*` → single Rust Axum service → Cloudflare
control API. Browser ↔ Cloudflare Realtime SFU/TURN for WebRTC audio. No media
relays through AWS, Workers, Durable Objects, RealtimeKit or PlanetScale.
Three-second HTTP snapshots provide presence/heartbeat; no WebSocket is needed.

The website has two AWS k3s replicas. A separate **single-replica** Rust service
owns the in-memory participant registry. Do not scale that registry or use sticky
sessions as a substitute for shared coordination. Restarting the service clears
presence and clients rejoin. Each tab receives an unguessable short-lived device
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

Use `.env.example`; Rust does not auto-load dotenv files. Export a private env
file before `cargo run -p caper-api`. Run the web process with
`MEDIA_API_URL=http://127.0.0.1:3001 npm run dev:web`. Remote browsers require HTTPS.
In an orb use supervised services and portal URLs, not direct sandbox host URLs.

Image CI builds `apps/api/Dockerfile` and publishes
`production/caper:api-<full-40-character-sha>` after merge to main via `api-image.yml`,
using the existing `production-caper-ecr-publisher` IAM role.
The Docker target and service name are `api` and `caper-api`, respectively.
This is the application control API, not a self-hosted audio relay. It does not deploy itself.
The corrective infrastructure PR stages `api-deployment.yaml`, `api-service.yaml`,
`api-secret.yaml`, and `api-ingress-patch.yaml` under `clusters/production/apps/caper/`.
Deployment/Service/ExternalSecret are named `caper-api`; the container is `api`.
AWS Secrets Manager `production/apps/caper-api` supplies Kubernetes Secret
`caper-api-cloudflare`. Keep one API replica with `Recreate`, port 3001,
`/api/media` routing, and existing `MEDIA_*` / `CF_*` configuration names.
Activation requires a verified image digest and credentials; follow the Caper API
activation runbook in infrastructure `docs/operations.md`.
The active web Deployment/Service remains `caper` with two replicas: renaming it
to `caper-web` requires a separate reviewed rollout and SSM deployment-target update.
The companion IAM trust fix from `master` to `main` requires a reviewed OpenTofu
apply before image publishing. No production apply was performed here.

Outbound HTTPS to `rtc.live.cloudflare.com` is required for the Rust API. AWS
needs no public media UDP ports. Clients use SFU plus TURN UDP and TCP/TLS
fallback; browser-blocked TURN port 53 is filtered. `/api/media/status` reports
the feature flag; web `/api/health` is independent of provider availability.

## Limits and lifecycle

- 12 simultaneous people, one microphone publication each, 11 subscriptions.
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
- Network failure triggers up to three rejoins retaining mute/deafen and device
  choice. Permission/device failures are visible. All microphone subscriptions
  are automatic. Deafen mutes playback, not forwarding/bandwidth.
- Mute disables the local track and detaches it from the sender. Opus is preferred;
  echo cancellation/noise suppression requested. DTX is not guaranteed.
- Speaking indicators and diagnostics use browser stats where available, not
  billing records. Output selection requires `HTMLMediaElement.setSinkId`;
  otherwise use OS settings. Joining requires microphone permission.

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
