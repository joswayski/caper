# Anonymous browser media MVP

## Scope and architecture

This adapts the September 6 infrastructure handoff to the owner's newer request:
**no accounts, no login, one public lobby**, browser-first while desktop work
continues separately. There was no existing Rust API, authentication, channel
model, WebSocket service, or database integration to preserve. The website has
two AWS k3s replicas. A **separate single-replica Rust service** owns the lobby;
the website's replicas remain unchanged. Do not run independent copies of this
registry or scale it using sticky sessions. Future API replication requires a
shared authoritative store/leases and recovery protocol (for example Valkey).

Browser → same-origin `/api/media/*` → Rust Axum → Cloudflare control API.
Browser ↔ Cloudflare SFU/TURN for WebRTC media. No AWS media relay, Workers,
Durable Objects, RealtimeKit, custom SFU, or new database. Three-second HTTP
snapshots provide presence and heartbeat; no WebSocket upgrade is required in
this MVP. PlanetScale becomes relevant when durable product data exists.

Every join generates an unguessable in-memory device capability. This is not
an account/login, but prevents one anonymous visitor from changing another
visitor's SFU session. Operations take Caper track IDs, never arbitrary provider
session IDs. Every browser tab is a separate anonymous person; there is no
cross-device user identity, moderation, kick, or ban system in this public test.

## Operator configuration (visitors need none)

Create **Realtime SFU**, not RealtimeKit, applications named `caper-development`
and `caper-production`, and separate TURN keys. As inspected September 6, 2026,
the connected Cloudflare account had no SFU applications. No credentials or
provider resources were created by this change.

Keep these values exclusively in the media service's secret environment:

| Variable | Meaning |
| --- | --- |
| `MEDIA_ENABLED` | `true` enables calls; absent/false keeps calls off |
| `CF_SFU_APP_ID` | SFU app ID, not Cloudflare account ID |
| `CF_SFU_APP_SECRET` | SFU app secret, not a browser token |
| `CF_TURN_KEY_ID` | TURN key ID |
| `CF_TURN_API_TOKEN` | TURN credential-generation token |
| `MEDIA_BIND` | Default `0.0.0.0:3001` |
| `RUST_LOG` | Suggested `caper_api=info` |
| `MEDIA_API_URL` | Web-process-only dev adapter target; production ingress bypasses it |

Use `.env.example` as a template; Rust doesn't auto-load dotenv files. In a
trusted local shell, export a private env file before `cargo run -p caper-api`.
Run the web app with `MEDIA_API_URL=http://127.0.0.1:3001 npm run dev:web`.
Use HTTPS for remote browser access; localhost is the browser's local exception.
In an Amp orb use supervised services/portal URLs, not raw sandbox host URLs.

AWS image CI builds the media container separately and, after merge to main,
publishes `production/caper:media-<git-sha>`. It does **not** deploy that image.
The companion infrastructure PR stages a single-replica `Recreate` Deployment,
Service, and `/api/media` ingress patch. They are deliberately excluded from
the active Kustomization until a verified media image digest and credentials
exist. Follow **Activate Caper media (one-time)** in the infrastructure repo's
`docs/operations.md` for exact secret, GitOps activation, rollout, and verification
commands. The manifests need no new AWS resources. The companion PR also fixes
Caper's stale `master` IAM trust to match its actual `main` default branch;
apply its reviewed OpenTofu role-trust plan before expecting image publishing.

Outbound HTTPS to `rtc.live.cloudflare.com` is required from the API. AWS needs
no public media UDP ports. Clients need Cloudflare WebRTC connectivity; issued
TURN URLs include UDP and TCP/TLS fallback. Browser-blocked TURN port 53 is
filtered. HTTPS status `/api/media/status` returns `{ "enabled": false }` until
enabled; web `/api/health` remains independent of provider availability.

## Lifecycle, limits, and privacy

- 12 simultaneous visitors, four publications each (microphone, camera,
  screen, optional screen audio), 44 incoming subscriptions per visitor.
- Global 30 joins/minute; 120 provider mutations/minute per visitor; 256 KiB
  request bodies. These are guardrails, **not a hard spending cap or DDoS defense**.
  A public visitor can consume capacity and relay bandwidth. Shut the test down
  if abused; don't expose this as a permanent unmoderated service.
- One-hour session/TURN credential lifetime; 45-second heartbeat lease, expiry
  sweep every five seconds. Background timer throttling can force a reconnect.
- Capability invalidation and provider force-close on leave/expiry, including
  downstream subscriptions. TURN credentials are revoked on cleanup; provider
  failure leaves credentials bounded by expiry. Failed track cleanup has a
  bounded in-memory retry backlog. A crash loses that backlog and registry;
  immediate revocation during a provider outage/crash is not guaranteed.
- Serial client negotiations, per-participant in-flight server guard. No blind
  retries of ambiguous session/track creation. Ambiguous publication invalidates
  the session and closes its MID; ambiguous subscription attempts a provider
  snapshot to discover allocated MIDs before invalidation.
- Client leaves release devices immediately. Network failure triggers up to
  three full rejoins, retaining mute/deafen and microphone choice. Camera/screen
  capture is stopped; screen sharing must be explicitly restarted. Browser/OS
  permission and device errors surface in the call UI.
- Microphone subscriptions are automatic. Watch opts into a person's camera,
  screen and optional screen audio; Stop watching closes SFU subscriptions.
  Deafen mutes playback but does not reduce forwarding/bandwidth. Microphone mute
  detaches the sender track and disables the local track, not just its icon.
- Screen capture requests at most 1920×1080, 30fps and a 3 Mbps sender target.
  Browsers may vary; this is not an exact billing guarantee. System audio only
  exists when the browser/OS/source chooser supplies a separate audio track.
- Opus is preferred where supported; echo cancellation/noise suppression are
  requested. DTX/silence suppression is **not measured or guaranteed**.
- Speaking indicators use browser audio-level stats when available. Diagnostics
  show connection bytes, loss, jitter, RTT, and observed TURN candidate usage.
  Counters reset on reconnect; they are local estimates, not billing records.
- Display names can be impersonated. Capabilities live only in memory, not
  cookies/storage. No recording, persistence, raw media logs, or E2EE claims.
  Cloudflare terminates the WebRTC transport. Other visitors can record content.

## Pricing

[Cloudflare's published SFU/TURN rate](https://developers.cloudflare.com/realtime/sfu/pricing/),
checked September 6, 2026, is **$0.05 per outbound GB**. Ignore allowances and
credits: outbound GB × $0.05 is the media cost model. SFU/TURN traffic in this
combined path is not double-charged. A steady 3 Mbps share watched by three
people for an hour is about 4.05 GB, or $0.2025, before protocol overhead and
other tracks. AWS application costs are separate. Use provider billing/usage
as authority, not send+receive client totals.

## Verification and platform matrix

Automated checks cover anonymous ownership, capacity, expiry, provider error
envelopes, TURN cleanup, negotiation, client permission/leave races, sender mute,
204 handling, and the fixed-path adapter. These are mocks, **not live media proof**.

| Platform | Status |
| --- | --- |
| Chromium browser in Linux orb | UI and mocked control/lifecycle checks; no live Cloudflare media |
| Chrome/Edge on macOS/Windows/Linux | Intended browser target; physical devices and multiple networks untested |
| Firefox/Safari | Untested; screen audio, output selection and stats vary |
| Mobile browsers | Responsive UI; capture capabilities and background behavior untested |
| Tauri macOS / Windows / Linux | Not implemented or validated here; independent desktop work |

Output selection is offered only with `HTMLMediaElement.setSinkId`; otherwise
use OS audio settings. Joining currently requires microphone access (no
permission-free listen-only mode). No long gaming session, 15-minute share,
restrictive-network relay test, CPU measurement, or DTX measurement has been
completed without credentials and physical/multi-network testers.

Before enabling for public testing:

1. Configure the development app; join with two browsers and verify actual
   incoming/outgoing RTP bytes and audible speech, then screen/camera watch/stop.
2. Use four visitors across two networks; share for 15 minutes and run a long
   voice session. Record CPU, RTP byte deltas, jitter/loss/RTT and user feedback.
3. Test microphone denial, device unplug/change, repeated join/leave, tab close,
   service restart, and interrupted negotiation; ensure no persistent roster or
   provider track ghosts. Stop screen sharing using the OS control too.
4. Force `iceTransportPolicy: "relay"` in a test-only browser PeerConnection
   override, verify selected relay stats, then test a real restrictive network
   including TCP/TLS fallback. Do not infer TURN works from ordinary Wi-Fi.
5. Compare muted/silent/speaking byte deltas and negotiated codec. Do not claim
   silence suppression from a requested audio constraint alone.
6. Test modified/uncooperative clients against expired capabilities and other
   participants' MIDs. Validate server cleanup through provider session snapshots.

## Runbook

- Unavailable lobby: check feature flag, media `/health`, ingress path routing,
  then required secret names. Never print secret values while debugging.
- Join/provider errors: inspect sanitized API logs for upstream status, SFU app
  versus account ID confusion, expired/revoked secret, outbound HTTPS, and quota.
- Connected but silent: check permission, mute/deafen, speaker output and the
  browser's Play audio fallback; inspect RTP stats and selected ICE candidate.
- Stale roster: verify a single API replica, snapshot requests, and lease sweep.
  Restart causes rejoin, not room persistence. Camera/screens must be restarted.
- Cleanup warnings: inspect Cloudflare session tracks; retry/close with the
  owner's credentials, never expose a generic admin proxy. During provider
  outages track closure cannot be guaranteed; turn off new public joins.
- Rotate secrets: create replacement SFU app/TURN key, update the private k3s
  secret, restart the media workload via approved operator path, verify two
  clients, then revoke old provider resources. Never reuse production secrets
  in frontend tests or desktop bundles.
- Take down: change `MEDIA_ENABLED` to `false` in the GitOps deployment and
  reconcile, which restarts/cleans up the service. For urgent shutdown the
  operator may scale media to zero and commit that change so Flux preserves it.
  Revoke the temporary SFU app and TURN key after testing; leave the website up.
