# Workspace and voice persistence plan

Status: proposed, September 7, 2026. Planning only; none of the behavior below
is implemented or live-validated. Current operations remain in [media.md](media.md).

## Outcome and scope

Use **workspace** for the container and **channel** for a conversation space.
Start with one seeded Caper workspace and its General voice channel. Model multiple
workspaces/channels now, without requiring a workspace-management UI, accounts,
membership, invitations, permissions, or text chat. Anonymous call capabilities
remain authorization for a single voice session, not persistent user identities.
Workspace IDs are not access-control secrets; these spaces are public in this phase.

The acceptance goal is that replacing an API pod does not end an established
voice call. Its replacement can authenticate the same capability, serve the same
roster, synchronize mute/deafen and joins/leaves, and operate existing SFU sessions.
SSE connections necessarily reconnect; healthy WebRTC connections should not.
This is not a promise of uninterrupted media through every database, provider,
network, or client failure.

## Findings in the current source

- `apps/api/src/db.rs` already connects to PlanetScale Postgres through SQLx and
  supports embedded migrations, but the connection is optional and voice does
  not read or write it. There are no application schema migrations yet.
- `apps/api/src/lib.rs`: `Registry` owns capabilities, participants, SFU session
  IDs, tracks, subscriptions, leases, join reservations, operation guards, rate
  limits, private mic-test sessions, and cleanup jobs in process memory.
- `events` uses process-local watch channels for SSE. `snapshot` renews presence;
  a database-backed roster alone would not make either cross-pod safe.
- `apps/api/src/main.rs` drains HTTP on SIGTERM, then `shutdown_cleanup` removes
  participants, closes provider tracks, and revokes TURN credentials.
- `apps/web/src/media/client.ts` already tolerates some transient control failures,
  but invalid capabilities or exhausted recovery lead to `rejoin`, tearing down
  and rebuilding audio. A new empty registry produces precisely that failure.
- Infrastructure source declares one desired API pod, `maxSurge: 1`,
  `maxUnavailable: 0`, and 60 seconds termination grace. Replacement starts first,
  but both Ready pods can receive requests against different registries. `/health`
  currently serves all probes and does not check database readiness.
- Infrastructure source does not yet map `DATABASE_URL` into the API's
  ExternalSecret. This is a source observation, not a check of live secret values.

Thus the reported interruption is consistent with the implementation, rather
than simply waiting for a pod to boot. No production rollout was observed here.

## Recommended architecture

Keep Rust/Axum, SQLx, Postgres, and Cloudflare SFU/TURN. Do not add Redis, a broker,
sticky sessions, or a permanent per-workspace pod owner for this first version.
Any Ready API pod handles any request using shared database state. Media continues
between the client and Cloudflare; audio and SDP do not belong in the database.

### Persistent model

Use relational columns and constraints for core state, not a serialized Registry blob.

| Table / responsibility | Essential data |
| --- | --- |
| `workspaces` | Stable UUID, slug, name, timestamps |
| `channels` | UUID, workspace FK, name, kind (initially voice), ordering, roster revision; unique slug within workspace |
| Workspace/channel voice settings | Validated, versioned policy; workspace defaults and nullable channel overrides |
| `voice_sessions` | Channel FK, anonymous display name, capability hash, SFU session ID, muted/deafened, lifecycle state, lease expiry, absolute call expiry, policy version, private monitor parent/role |
| `voice_tracks` | Session FK, Caper track UUID, provider track name and MID, kind, lifecycle state |
| `voice_subscriptions` | Subscriber session FK, source track FK, subscriber MID and lifecycle state |
| `voice_operations` | Client operation ID, session, kind, generation, status, claim deadline, non-SDP reconciliation metadata |
| `voice_cleanup_jobs` | Track close / TURN revoke target, deduplication key, next attempt, attempts, claim deadline, completion state |
| Shared admission/rate state | Expiring join reservations and scoped rate counters; atomic with admission decisions |

TURN revocation usernames must survive crashes; do not retain TURN passwords,
raw bearer capabilities, SDP, IP addresses, or media. Keep the existing optional
country code only for the active session. Use database time and `timestamptz`,
not Rust `Instant`, for cross-pod expiry. Terminal sessions stop appearing in
rosters immediately; purge their metadata after dependent cleanup is complete.
Proposed initial retention: completed operation/job metadata 24 hours, with
unresolved cleanup retained and alerted rather than silently discarded.

Distinguish public people from provider sessions: mic test currently consumes two
extra private sessions. Retain parent authorization, one monitor per role, and
cascading termination, without exposing monitors in public rosters. Keep a separate
provider-session budget so raising the people limit does not hide provider cost.

All lookups and subscriptions must enforce channel scope. Capability possession
must never authorize another channel's tracks or arbitrary SFU IDs. Maintain old
unscoped join requests as General-channel joins while old clients remain supported.

### Concurrency and recovery contracts

1. **Atomic admission:** lock the channel admission row in a short transaction,
   enforce shared capacity/rate policy, and create an expiring reservation before
   provider work. Commit the participant only if the reservation/parent is valid.
2. **Serialized provider operations:** replace local operation booleans with a
   durable per-session operation state machine. Claim in a short transaction,
   call Cloudflare outside the transaction, then conditionally commit results for
   the claimed generation. A pending SDP negotiation remains exclusive until its
   answer or explicit recovery, not merely until one HTTP request finishes.
3. **Ambiguous provider results:** record intent before calling Cloudflare. A
   timeout or pod crash does not mean the call failed. Never blindly replay
   session creation, track creation, or negotiation. Reconcile known sessions
   through provider inspection where possible; otherwise recover only the affected
   participant. A database generation prevents stale DB commits but cannot fence
   requests already executing at Cloudflare. Expired claims must enter recovery,
   not immediately permit a conflicting provider mutation.
4. **Idempotency:** deduplicate repeated client commands by operation ID. Store
   safe result references/status, not SDP responses. A lost negotiation response
   may require participant recovery rather than exact replay. Verify Cloudflare's
   inspection/idempotency guarantees in a bounded implementation spike before
   finalizing operation recovery. Unknown session-creation outcomes remain an
   explicit provider limitation, not an exactly-once claim.
5. **Durable cleanup:** mark a session leaving/expired and enqueue its own and
   dependent cleanup in the same transaction. Workers claim jobs with short leases
   (`FOR UPDATE SKIP LOCKED`), retry with backoff, and resume after worker death.
   Serialize dependent track closes with mutations of the receiving session.
   Confirm already-closed/revoked provider semantics before treating retries as safe.
6. **Presence races:** expiry transitions must atomically recheck the lease;
   a renewed participant must not be removed from an earlier sweep's result.
   Explicit leave revokes authorization immediately and stops capture locally,
   even if asynchronous provider cleanup takes longer.

### Cross-pod updates and client continuity

Increment a per-channel roster revision in the same transaction as every visible
join, leave, track, or mute/deafen change. Return that revision with snapshots.
For the first small deployment, each pod polls revisions for locally watched
channels once per second and emits existing SSE `changed` invalidations. This
avoids requiring a durable message bus or session-pinned LISTEN connections.
Snapshots remain authoritative, including on SSE reconnect; missed invalidations
cannot lose state. Coalesce reads per channel, not per connected browser.

Keep lease renewal at 15 seconds initially; propose a 90-second presence lease and
a 60-second transient control recovery budget, validated against expected deployment
times and stale-presence tolerance. Return negotiated timing policy to clients
instead of maintaining conflicting server/client literals. Old clients keep their
shorter behavior until refreshed. SSE disconnect alone must not close the existing
PeerConnection. Retry control against any pod using the same capability; preserve
local mute/deafen and reconcile the latest desired state after recovery.

Database unavailability returns a retryable unavailable response, not unauthorized
or an empty roster. Do not delete subscriptions based on failed reads. Stop new
joins and provider mutations without authoritative state; keep established audio
for the bounded recovery window. Cleanup resumes after database recovery. Genuine
revocation/expiry and broken WebRTC still trigger scoped recovery or termination.

Do not persist high-frequency speaking levels or diagnostics. They remain client
media observations; mute/deafen and membership are shared control state.

## Configuration policy

Expose only safe effective settings through status/join responses. Initially manage
settings with an operator CLI or reviewed administrative script using validated
transactions; no unauthenticated public settings-write API and no account system.
Record policy version and operator change metadata. Resolve defaults then overrides;
reject invalid combinations and values above server safety ceilings.

| Current value | Proposed ownership |
| --- | --- |
| 12 registered sessions, including monitors | DB policy: public participant capacity plus separate provider-session budget; preserve current budget initially |
| 1 microphone, 11 subscriptions | One microphone remains a product invariant; derive subscription ceiling from capacity within tested provider ceilings |
| 1-hour call and TURN lifetime | DB call policy for new joins, constrained by provider credential lifetime; increasing duration needs credential/transport renewal work |
| 30 joins/minute globally, 120 provider mutations/minute per participant | Shared DB-enforced service/session limits; optional workspace/channel overrides cannot bypass global protection |
| 45-second lease, 5-second sweep, 15-second client renewal | Versioned service timing policy, not arbitrary workspace overrides |
| 30-second client control recovery, 3 rejoins, 10-second RTC grace | Versioned client protocol policy with bounded retry/backoff |
| 512 in-memory cleanup jobs | Durable jobs, monitored backlog and admission backpressure, no drop-oldest cleanup loss |
| 256 KiB HTTP body, 40-character names, provider/request/SSE timeouts | Code/deployment safety limits unless an actual product requirement warrants runtime policy |
| Provider secrets, database URL, bind address, logs, emergency media switch | Environment / secret manager, never public workspace settings |

Capacity reductions block new joins rather than evicting active callers. Call
lifetime changes apply to new sessions; active sessions retain issued deadlines.
Treat workspace disablement and forced disconnect as explicit administrative
actions, not accidental side effects of refreshing a settings cache. User-local
device and audio-processing preferences stay client-side.

## Delivery sequence and rollout

1. **Schema and policy foundation:** seed workspace/General, migrations, scoped
   identifiers, validated configuration reads/operator writes, and contract tests.
   This does not yet make calls restart-safe.
2. **Shared voice authority:** persist every Registry responsibility; implement
   atomic reservations, capabilities, operation recovery, durable cleanup, and
   cross-pod revisions. Require Postgres for this mode; no automatic fallback to
   isolated in-memory state on database failure. Test two API processes against
   one real Postgres database with a controlled provider.
3. **Client and shutdown handoff:** reconnect control without rebuilding healthy
   audio; introduce readiness separate from process liveness. On SIGTERM mark
   unready, reject new work, close SSE, drain accepted requests, and relinquish
   recoverable worker claims. Never close healthy shared sessions merely because
   a pod exits. Never release a provider operation as safe while it is still running.
4. **Infrastructure and live qualification:** wire `DATABASE_URL` securely; verify
   TLS, connection budget, migration rights, and recovery. Keep migrations additive
   and compatible with both overlapping versions. Readiness checks schema/DB
   availability and drain state; liveness must not restart pods just because DB is
   down. Keep replacement-first rollout; consider two steady replicas and an API
   disruption budget only after multi-pod tests pass and capacity is approved.

The first migration from in-memory authority cannot transparently recover existing
calls unless an explicit old-pod state-export bridge is built. Prefer announcing a
one-time reconnect at cutover instead of building that temporary subsystem. The
rollout safety goal applies after all serving pods use shared authority. Never mix
legacy and shared-authority pods as if compatible. Deploy backward-compatible web
behavior first, then coordinate the API cutover. A rollback must use a compatible
shared-state API version; reverting to the old registry means another call reset.

These are separate focused implementation PRs. This plan changes no infrastructure,
schema, credentials, or production data. Future infrastructure PRs must include
exact approved configuration/reconciliation commands. The existing image rollout
entry point (not executed for this plan) is:

```bash
gh workflow run deploy-caper-api.yml \
  --repo joswayski/infrastructure --ref main \
  -f git_sha=<full-40-character-caper-sha>
```

## Acceptance gates

- Two pods, one database: join on A, authenticate/snapshot/mute on B, receive
  cross-pod roster changes within a proposed two-second target under normal load.
- Concurrent last-slot joins, duplicate requests, leave versus heartbeat/sweep,
  subscribe versus source leave, monitor creation/parent expiry, and cross-channel
  access cannot violate capacity, authorization, or provider serialization.
- Kill workers before/after provider calls and database commits; no stale worker
  commits, blindly repeated creation, or lost cleanup. A genuinely ambiguous
  negotiation may recover its participant, not reset the entire channel.
- Existing calls retain participant IDs, capabilities, SFU sessions, and browser
  PeerConnections through rolling SIGTERM and abrupt idle-pod termination. Confirm
  no shutdown-driven track closes/TURN revocations and continued audio delivery.
- Join/leave/mute/deafen still work during overlap, and after the old pod exits.
  Exercise lost SSE notifications, database interruption/recovery, expiry, cleanup
  retry, configuration changes, and mixed compatible API/client versions.
- Measure control-recovery time, audio gaps/RTP progression, unexpected rejoins,
  DB contention, and cleanup age. Do not call the feature seamless based only on
  mocks or a Kubernetes rollout success message.
- Run live multi-client audio across separate networks and forced TURN, including
  sustained calls and physical devices. Browser success does not certify Tauri;
  report macOS/Windows/Linux webview validation separately in `media.md`.
- Run repository JS checks/tests and Rust format/tests/clippy for implementation;
  add real-Postgres integration tests and image-stage validation. This planning
  PR requires documentation/diff checks only.

## Later, not in this implementation

Messages will reference a stable channel ID. A thread can reference its channel
and root message, with replies referencing the thread. Decide thread depth,
retention, anonymous authorship, and permissions when chat is actually scoped;
do not create unused messages/users/thread tables now. Voice sessions are not
future message authors or permanent membership records.

## Source references

- [API state and lifecycle](../apps/api/src/lib.rs), [shutdown](../apps/api/src/main.rs),
  [database bootstrap](../apps/api/src/db.rs), [browser recovery](../apps/web/src/media/client.ts).
- Infrastructure desired state, inspected September 7, 2026 (not live verification):
  [deployment](https://github.com/joswayski/infrastructure/blob/main/clusters/production/apps/caper/api-deployment.yaml),
  [secret mapping](https://github.com/joswayski/infrastructure/blob/main/clusters/production/apps/caper/api-secret.yaml),
  [routing](https://github.com/joswayski/infrastructure/blob/main/clusters/production/apps/caper/api-ingress-patch.yaml),
  [operations](https://github.com/joswayski/infrastructure/blob/main/docs/operations.md).
