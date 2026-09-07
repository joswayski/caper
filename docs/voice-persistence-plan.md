# Deployment-safe voice MVP

Status: revised proposal, September 7, 2026. Planning only, not implemented.

## End goal

**Join voice, deploy the API, and stay in the call.** The new pod knows who is
connected, and joining, leaving, mute, and deafen continue to work. Replacing a
server must not disconnect healthy audio or make people manually rejoin.

The initial migration may disconnect calls; that is accepted. No state-export
bridge or special migration experience is needed while nobody is using the app.

This targets normal deployments, not a guarantee against every network or
Cloudflare outage. Prove it with a live call during a rollout before calling it done.

## Why deployments interrupt voice today

Postgres connectivity already exists, but the API keeps callers and their
Cloudflare connection information in memory. A new pod cannot recognize those
callers. The old pod also explicitly closes their media connections on shutdown.

Audio already travels directly between clients and Cloudflare. It does not need
to move between API pods; the replacement only needs the information required to
manage the existing call.

## Build only what is needed

1. **Share active call state in Postgres.** Store participants, their anonymous
   session authorization, Cloudflare connection references, mute/deafen, and who
   is sending/receiving each media stream. Do not store audio, raw authorization
   tokens, or connection-setup payloads (SDP).
2. **Let either pod manage the call safely.** Serialize changes to each media
   connection so old and new pods cannot make conflicting requests. Preserve
   enough in-progress work to recover after a pod exits; do not blindly repeat
   a Cloudflare request when its result is unknown.
3. **Reconnect status updates, not healthy audio.** Clients reconnect their
   existing event stream to the new pod and refresh the participant list using
   the same session. Use database-backed change detection for cross-pod updates.
   A temporarily unavailable API must not look like an empty room or an invalid
   participant.
4. **Separate pod shutdown from people leaving.** Drain requests on deployment
   without closing healthy Cloudflare connections. When someone actually leaves
   or disappears, release their media resources; unfinished release work must
   survive a pod restart. Keep checking that clients are still present so closed
   tabs do not remain in the call forever.
5. **Wire up and test replacement.** Require the database for shared voice state,
   make readiness reflect whether a pod can serve it, and start the replacement
   before retiring the old pod. Keep schema changes compatible during overlap.

Keep the current single voice destination. No space/channel management UI,
configuration system, admin CLI, Redis, or messaging broker in this MVP. Exact
table layout is an implementation detail, not a new product feature list.

## Limits: requested changes

The following describe existing code, not new requirements or live verification:

- The current **12-session cap** limits connected sessions, even when muted.
  Mic test consumes additional private sessions. Remove this application cap;
  do not replace it with configurable capacity or another arbitrary session cap.
- **One microphone is per person**, not per call. Everyone can talk simultaneously.
  A subscription means receiving another person's stream. Remove the fixed
  11-subscription ceiling along with the participant cap. Keep normal simultaneous
  conversation; do not introduce speaker slots.
- Remove the **one-hour call cutoff**. Provider relay credentials still have an
  expiry; implement and test renewal/transport handling for ongoing calls rather
  than making users hang up and rejoin. Do not assume deleting a constant solves it.
- Remove the application's **30 joins/minute** and **120 media changes/minute**
  limits. No replacement rate-policy system now; the owner will add limits later.

No application cap does not mean infinite provider or hardware capacity. Report
actual Cloudflare failures honestly; do not invent unlimited-capacity guarantees.

Internal reliability mechanics are not user quotas: periodic presence checks
detect abandoned tabs, retry delays let requests recover, and request timeouts
prevent stuck connections. Keep only the mechanics needed for correctness and
deployment continuity, with no configurable timing-policy project.

The old “512 cleanup jobs” value describes a memory queue of failed requests to
close media streams. Persist unfinished release work rather than losing it on
restart or dropping it at that count. This is not message or conversation deletion.

The existing 256 KiB HTTP body ceiling is for client-to-API control JSON, including
WebRTC connection setup—not audio, which goes to Cloudflare. Leave this technical
request-size guard alone in this work. The existing `MEDIA_ENABLED` environment
variable enables/disables voice; no new “emergency switch” feature is proposed.

## Leave room for the next features, without building them

- Use **space**, not workspace. The future relationship is **space → channels**.
  Keep a stable room/channel identifier in call state, using the current General
  destination by default. Do not build multi-space management to fix deployments.
- A call participant/session is not a user account. Later it can reference a
  stable user ID without changing how media connections survive deployments.
- Identity requirements for upcoming accounts: global username and global display
  name, each at most **32 characters**. Usernames must be globally unique. For
  planning, interpret “names should be unique” as display names also being globally
  unique; confirm that distinction when implementing identity. Per-space names
  come later. Do not build account registration or global name reservation into
  this deployment-continuity change.
- Keep media-stream records capable of distinguishing microphone audio from
  future screen video/system audio. Avoid making “one microphone” a permanent
  database constraint against other stream types. Screen sharing itself is later.
- Future chat messages belong to a channel and can have threads/replies. Their
  lifetime must not depend on an active media session. No message/thread tables
  or chat transport now.

## Done means

- Two people can speak and hear each other simultaneously before, during, and
  after a normal API rollout, without recreating their healthy audio connections.
- Either pod recognizes the same participant session; mute/deafen and people
  joining/leaving synchronize during overlap and after the old pod exits.
- A deployment never triggers participant removal or closes healthy media streams.
  Real leave/abandoned-tab handling still releases those resources after a restart.
- Concurrent requests and interrupted provider work cannot corrupt another call.
- No application participant/subscription quota, join/change rate quota, or
  one-hour forced hangup remains. Test past the old thresholds and test relay
  credential renewal, including forced TURN (relayed audio).
- Run real-Postgres multi-pod tests plus live audio rollout checks. Mocks alone
  cannot demonstrate uninterrupted calls. Document browser and native desktop
  coverage separately; one does not prove the other.

No deployment, database migration, runtime limit removal, or infrastructure change
has been performed by this planning PR. Implementation and its infrastructure
changes will be reviewed separately, with exact rollout commands at that point.

## Existing implementation references

- [API state and lifecycle](../apps/api/src/lib.rs)
- [Pod shutdown](../apps/api/src/main.rs)
- [Database connection](../apps/api/src/db.rs)
- [Client recovery](../apps/web/src/media/client.ts)
- [Current behavior and runbook](media.md)
