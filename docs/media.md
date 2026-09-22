# Channels, messaging, and voice

## Scope and architecture

The **Public demo / General** channel remains available to guests while the
service is enabled. Account-owned spaces contain unified text/voice channels;
their history, live messages, presence and calls require membership. See
[spaces and channel access](#spaces-and-channel-access). Signed-in participants
use their account display name; guests receive a random name. This is not an
outgoing-call flow. No camera, screen sharing, or server-side voice recording.
The independently enabled [public text demo](#public-text-demo) shares `/live`;
demo reads and sends do not require an account or joining voice.
Mic test offers an explicit, tab-memory-only recording of up to 30 seconds of
received Natural audio and an on-device Enhanced comparison from the same take.
Input/output device lists open directly beside the profile's microphone/headphone
controls; input gain lives in Settings. Output volume in the headphone menu scales
received audio alongside each person's volume and also controls mic-test playback.
Settings opens mic test and connection details
in dialogs without moving the message list. Close or Escape cancels a test,
releases pre-join capture, or restores the active call's previous mute/deafen state.
`node scripts/test-voice-controls.mjs http://localhost:5174` checks these flows,
typing animation/reduced motion, and desktop/narrow layout against a local Vite
server with synthetic audio and mocked HTTP/WebSocket/WebRTC. It is not live SFU
or physical-device validation. Client tests also cover transport returning to
`connecting` during startup subscription negotiation: Join waits for it to be
connected again, with the existing timeout/cancellation and audio-readiness gates.
The Rust API replaces a signed-in participant's submitted name
with the account display name. Guest names can collide and are not verified or
reserved; participant IDs, not names, distinguish people. With `VALKEY_URL` configured,
live call state is shared in Valkey; without it, development uses process memory.
Up to 12 people can join with microphone permission,
mute, deafen, choose devices, and leave. Other visitors may record audio.
Cloudflare's IP Geolocation setting adds an approximate country code at ingress;
the registry keeps that code for the call and shares it in roster snapshots. Caper
does not retain the visitor IP itself. Unknown and Tor locations are omitted.
Visitors see the public roster before joining through the unauthenticated
`/api/media/presence/events` SSE stream, without joining voice. That projection
includes each participant's session ID, name, country code when available, mute,
and deafen state, but never media track IDs, session tokens, or audio.
The JSON `/api/media/presence` endpoint remains available for inspection and older
clients; the current browser does not poll it. Spectators never renew call leases.

Browser → same-origin `/api/media/*` → Rust Axum API pods → Cloudflare
control API. Browser ↔ Cloudflare Realtime SFU/TURN for WebRTC audio. No media
relays through AWS, Workers, Durable Objects, RealtimeKit or PlanetScale.
Authenticated SSE snapshots provide immediate in-call roster/track discovery.
Fifteen-second HTTP snapshots renew presence leases and repair missed state; HTTP
also carries commands. Audio still uses WebRTC, not SSE or WebSockets.

Production activation is staged: keep the API at one replica until every pod uses
the same Valkey endpoint and schema. The first switch from process memory requires
an empty-channel maintenance window; the old binary cannot hand off its state.
After activation, API pods can be replaced without closing healthy Cloudflare
media sessions. Their HTTP/SSE connections still reconnect; TCP connections cannot
move between pods. See [shared call state](#shared-call-state-and-rolling-deployments)
for the rollout and remaining validation. Each tab receives an
unguessable short-lived call capability. Clients use Caper track IDs, not
arbitrary SFU session IDs. Cloudflare terminates transport encryption; this is
**not E2EE**.

### Public text demo

Implemented behind `CHAT_ENABLED=true`; production activation is separate. The
temporary **Public demo** space contains one **General** unified channel. Account
spaces are separate from this guest demo. Rich content, message edits/deletion,
moderation rules, and message notifications are not implemented. Typing indicators are
best-effort ephemeral presence, not saved messages. This demo is not a
permanent public space when the product launches. Voice remains SSE/WebRTC.

The same Rust image has two independently deployable roles:

- `caper-api`: HTTP commands/history plus transactional-outbox publishing (3001).
- `caper-api --gateway`: WebSocket delivery (3002, `CHAT_GATEWAY_BIND`). It uses
  `DATABASE_URL` only, never `MIGRATION_DATABASE_URL` or media provider credentials.
  `/health` is liveness; `/readyz` requires DB/broker readiness and no drain.

Both need Postgres, the **same** authenticated TLS `VALKEY_URL`, and
`CHAT_ENABLED=true`. The flag defaults off outside Compose. Compose defaults it
on when no override exists; change the example `.env`'s false value to true to
enable it. Secrets Manager overrides process environment settings. API startup
runs the additive migration and runtime grants; startup seeding allocates stable
random IDs under a transaction lock. Start the API before the gateway. The
gateway retries startup in Compose if migrations are not ready yet.

IDs follow the existing random alphanumeric convention: **12 characters** for
spaces/channels/guest identities, **15** for messages, with unique constraints.
Internal joins use bigint. IDs contain no timestamp and do not order messages.
Per-channel decimal-string `seq` values are reconnect cursors; browser code uses
BigInt, never floating-point numbers. The channel row lock allocates positions
in commit order. There is no cross-channel ordering or sharding in this demo.

HTTP contract (same origin, no cache):

- `GET /api/chat/general`: space/channel IDs, latest 50 messages oldest-first,
  `cursor`, and `hasMore`.
- `POST /api/chat/session {name}`: opaque sender token and public author. Login
  is optional; a valid account supplies the authoritative name. Guest names are
  not verified. Tokens are stored hashed in Postgres and expire after 30 days.
  Account-linked chat tokens also stop working after parent logout/expiry.
- `POST /api/chat/channels/{id}/messages {clientMessageId,text}` with
  `X-Caper-Chat-Token`: returns the committed message. The browser retries the
  same UUID and original text after ambiguous errors, using Enter or Retry send.
  A reused key with different text or a different sender returns conflict.
- `GET /api/chat/channels/{id}/messages?before={seq}`: earlier history, up to
  50 messages. Only the demo's General channel is accessible through these APIs.
- `POST /api/chat/channels/{id}/typing {typing:boolean}` with
  `X-Caper-Chat-Token`: checks the same sender/channel permissions as sending;
  returns 204 after best-effort publication, 429 when throttled. No draft text is
  accepted, saved, or broadcast. There are no new database migrations.

Messages preserve literal Unicode text (up to 4,000 code points, nonblank;
control characters other than newline/tab are rejected). Content is versioned
`{version:1,type:"text",text}`. `prepare_text` is the pre-publication boundary for
future replacement rules such as BO2 → Wardogs: transform once before persistence
and broadcast, not by deleting and reposting. No example replacement is enabled.
Message bodies, guest capabilities, and account credentials are never logged.
Messages and author snapshots are saved in Postgres and visible to everyone;
there is no automatic retention purge in this demo. Browser tokens use local
storage; drafts/pending sends survive reconnects but not closing/reloading a tab.
Guests reuse their saved identity; signed-in startup obtains a fresh capability
from the current account session rather than identifying an account by its name.

History opens at the latest 50 messages. Scrolling to the oldest loaded rows
automatically requests the next 50-message page; the history button remains a
keyboard-accessible fallback and becomes Retry after a failure. React Virtuoso
measures variable-height rows and renders only the visible region plus overscan.
Prepending history preserves the reading position; live arrivals follow the
bottom only when the reader is already there. Loaded message data stays in tab
memory; virtualization bounds mounted DOM rows, not the history cache. Reloading
starts again at the latest page.

The browser immediately shows an optimistic message (without a `Sending…` label) and clears the
composer. It keeps one outstanding command; a new draft can be typed while it
waits. HTTP, ordered WebSocket replay, or a history resync replaces that row by
matching its client UUID and sender, using the server's text, timestamp, and
sequence. Provisional rows never advance the replay cursor. Ambiguous failures
retain the exact command for retry; definitive rejections retain the text for
Edit/Dismiss. A late HTTP failure cannot undo WebSocket confirmation.

Typing uses a separate `caper:chat:v1:typing` Pub/Sub topic and a bounded
64-event gateway buffer; it never enters the transactional outbox, history, or
message sequence. Gateways deliver `typing.updated {channelId,author,typing,revision}`
only to sockets that request `&typing=true`. This protects already-open older
browser tabs and older gateways during rollout. Deploy API, gateway, then web;
no configuration or infrastructure change is needed for typing.

Typing publications are limited across API replicas to four per author/channel
per second and 120 globally per second. The browser pulses at most every 500ms
while editing, stops on clear/blur/send or 500ms of inactivity,
and expires peer indicators after six seconds without a newer signal. Broker
microsecond `revision` strings deduplicate overlapping socket events independently
of message cursors; a brief stop tombstone prevents delayed starts from reviving
an indicator. The browser caps tracked authors at 64 and never shows its own
identity. Disconnect/resync clears presence; dropped signals are not replayed.
Typing errors never block sending. This is approximate presence, not an online
member list or a guarantee that every keystroke/start/stop is delivered.

Delivery and recovery:

1. One transaction writes message, channel position, and full event payload.
   HTTP success means **durably accepted**, not delivered to every browser.
2. The API wakes its publisher immediately; a one-second pending-row scan repairs
   a lost wakeup. A transaction advisory lock serializes demo publishers across
   API replicas without locking the sending channel row during broker I/O.
3. Publisher sends the full event on Valkey Pub/Sub, then marks it published.
   The event remains in Postgres for replay. A crash in between may republish it;
   gateway/client sequence deduplication prevents a second visible message.
4. `GET /api/chat/events?channelId={id}&after={cursor}` upgrades to a WebSocket.
   It needs no login/token for this public demo. Each socket subscribes/buffers
   before reading the committed high-water mark, replays through it, emits
   `ready {cursor}`, then merges live events in order. Sends still use HTTP.
5. One DB head check per gateway every two seconds repairs missed events even if
   the final Pub/Sub event was lost and no later message arrives. Broker outage
   does not erase accepted messages. Normal delivery uses full broker payloads,
   not a per-recipient database fetch.

Limits: 30 new messages/guest/minute, 120/channel/minute, and 60 new sender
sessions/minute globally for this demo. Each gateway admits 128 sockets; its
broadcast ring holds 256 events. A lagging receiver replays from Postgres.
Replay reads batches of 16; sockets use 4 KiB read buffers and at most 64 KiB of
queued writes. Writes time out after three seconds; initial/recovery replay is bounded to ten
seconds and 2,000 events. Older/impossible cursors or missing retained events get
`resync_required`, and the UI reloads recent history with older-page access.
Heartbeat ping/pong runs every ten seconds, with a 30-second inactivity deadline.
These are pre-launch demo limits, not an internet-scale abuse-prevention system.

**Planned deployment handoff:** keep two ready gateway replicas, maxUnavailable 0,
maxSurge 1, five-second preStop, and 65-second termination grace. SIGTERM rejects
new upgrades/readiness and sends `migrating`, but old sockets continue delivering
for 20 seconds. The runtime remains alive for at least 21 seconds. The browser
opens a replacement while retaining the old socket and closes the old only after
the replacement is ready and caught up to the current applied cursor. A stale or
failed candidate cannot discard a healthy old stream. Unexpected failures use
reconnect/backoff and replay. TCP sockets themselves do not move between pods.

Production manifests, independent image activation, exact operator commands,
and the one-time deployment target are in infrastructure
[PR #117](https://github.com/joswayski/infrastructure/pull/117). Apply that reviewed
infrastructure plan, then deploy the new **API image first**, **gateway second**,
and **web third**. Do not start `--gateway` using an older pre-gateway image.
Infrastructure merging may trigger Flux reconciliation of existing workloads;
the new gateway is staged at zero until explicit activation. No production
migration, infrastructure apply, or deployment was executed during development.

Local development without Compose (point URLs at **disposable local data**):

```bash
# Shell 1: set DATABASE_URL, MIGRATION_DATABASE_URL, VALKEY_URL, CHAT_ENABLED=true.
cargo run -p caper-api
# Shell 2: same runtime DB/broker and feature flag; no migration credentials needed.
cargo run -p caper-api -- --gateway
# Shell 3: Vite proxies HTTP to 3001 and /api/chat/events upgrades to 3002.
npm run dev:web
```

Validation matrix for this slice:

| Check | Evidence / limitation |
| --- | --- |
| Transaction rollback, concurrent retry/order, replay, duplicate publish, lost last event, overlap | Automated integration test with disposable Postgres and real broker/WebSockets; also runs in CI with Postgres 17 + Valkey 8.1 |
| Typing auth/rate limits, cross-gateway fanout, legacy opt-out, unchanged durable cursor | Automated disposable Postgres/broker integration; browser unit tests cover throttling, expiry, overlap and nonblocking sends |
| Real guest browser send/receive | Two isolated Chromium sessions, real local HTTP/DB/broker/gateway; literal HTML-like text stays text |
| Real process SIGTERM with replacement | Readiness-aware local test proxy; 12 messages received once, zero offline transitions; old socket closed after replacement ready |
| Lost acknowledgment after commit | Injected browser fetch failure after real HTTP commit; Enter retry returned the same message, one visible copy |
| Appearance | Desktop and 390px Chromium, empty/populated/offline/error states inspected; not physical-device/Safari evidence |
| Production rolling deployment, sustained load, regional p95/p99, DB failover | Not measured; must be verified after operator activation |

Run the integration test with `CHAT_TEST_DATABASE_URL` pointing at disposable
loopback Postgres (test role can create/drop test databases) and
`CHAT_TEST_VALKEY_URL` pointing at a disposable broker:

```bash
cargo test -p caper-api chat::tests::durable_guest_delivery_replay_and_handoff -- --ignored
```

CI runs this against cluster-mode Valkey 8.1 with one primary owning all hash
slots. This enforces the multi-key Lua restrictions that standalone mode misses:
the typing rate counters share the `{caper:chat:v1:typing}` hash tag. It tests
cluster command compatibility, not multi-node failover or managed-service TLS.

With the web dev server running and `agent-browser` installed, run
`node scripts/test-chat-history.mjs` for the explicitly mocked 3,000-message
browser regression. It checks scroll-triggered pages, variable-height prepend
anchors with concurrent live delivery, bounded DOM rows, retry and end-of-history,
narrow layout, and optimistic reconciliation. `CHAT_TEST_WEB_URL` may select a
different loopback preview; `CHAT_TEST_ARTIFACTS` optionally saves screenshots.
This is browser rendering/interaction coverage, not production delivery evidence.

The proposed healthy same-region target remains p95 below 250ms and p99 below one
second commit-to-client, including routine rolls. Orb handoff timing is **not** a
measurement of that production SLO. Database durability depends on the provider's
storage/failover guarantees; this protocol does not claim exactly-once transport.

## Spaces and channel access

Accounts with completed profiles can create spaces and unified text/voice
channels. The creator owns the space and manages its name, channels and members.
New spaces start with `general`. Public channels are visible to all members of
that space, not to anonymous visitors or unrelated accounts. Private channels
are visible to their explicitly selected space members and the owner. The public
demo cannot be managed through these APIs.

Owners add existing accounts by exact username and can remove them. This is a
direct membership change, not an invitation awaiting acceptance. There are no
invite links, custom roles, ownership transfers or public space discovery yet.
Non-owner members can leave a space themselves. Removing a space member also
removes their private-channel grants. The owner cannot be removed.
Counts include active resources only: 20 owned spaces per
account, 100 total memberships (including owned spaces), and 100 channels per
space. Quota checks and creation run under database locks to prevent racing
requests exceeding those limits.

Space and channel external IDs use the existing 12-character cryptographically
random alphanumeric generator with database uniqueness constraints; internal
joins use bigint IDs. Names are not authorization tokens. Space names contain
1–80 Unicode characters after trimming and no control characters. New channel
names contain 1–80 characters matching `[a-z]+(-[a-z]+)*`: lowercase ASCII
letters with single dashes between segments. Channel names are unique among
active channels of one space. Renaming does not change IDs. Deletion is a
soft delete: it removes access and frees the quota, but retained message rows
are not physically purged by this feature. There is no restore UI.

The account APIs live at `/api/spaces`, `/api/spaces/{space}`, and their
`/channels`, `/members`, and `/channels/{channel}/members` subresources. Browser
cookies or account bearer authentication are required. Only owners manage these
resources, except a member removing their own membership. Text history and
commands use `/api/chat/channels/{channel}/...`;
WebSocket subscriptions still use `/api/chat/events`. Both command and gateway
paths check membership and channel visibility, including replay and live delivery.

Account voice uses `/api/channels/{channel}/media/*`, with the same operation
names as the guest `/api/media/*` endpoints. Every request needs a valid account
session and channel access; in-call commands also need the room's
`x-caper-media-token`. That capability is bound to its original account session.
Each room has separate participants, tokens, sequences, join limits, tracks,
monitor sessions and cleanup work. Existing 12-participant voice limits apply
per channel. Account channels cannot access the demo's voice registry or vice
versa. SSE checks access again on each emission. The expiry worker also checks
active account sessions/membership and queues SFU track closure and TURN
revocation after access is removed. Provider cleanup is asynchronous and may
retry; access removal is not a claim of instantaneous media disconnection.

In shared mode, the demo keeps `caper:{general}:v1:state`; account rooms use
`:channel:<external-id>` suffixes. An `:active` set in the same Redis hash slot is
updated atomically with room writes so another pod can recover cleanup without
a new visitor opening that channel. Idle rooms leave that set but retain their
small metadata record so revisions never reset under connected spectators.
One bounded connection pool and notification subscription serve the rooms;
notifications contain no channel
payloads. This first implementation shares one Redis hash slot and is not a
claim of horizontally sharded voice state.

### Local data, CI, and production rollout

Standard local Postgres and Valkey are sufficient. No paid hosted staging
database, TIN, or Lead extension is required. Exact usernames use the existing
unique B-tree index. TIN is a possible future full-text message-search tool;
Lead provides its SQL/search compatibility in local/CI environments but not its
performance. Neither is part of this feature. Cloudflare SFU/TURN and AWS email
integration checks remain separate from database tests.

The ignored database tests create/drop fresh databases on a disposable loopback
server. CI also exercises them with Postgres 17 and cluster-mode Valkey:

```bash
# All URLs below must point to disposable local services, never production.
CHAT_TEST_DATABASE_URL="$DATABASE_URL" \
CHAT_TEST_VALKEY_URL=redis://127.0.0.1:6379 \
TEST_VALKEY_URL=redis://127.0.0.1:6379 \
cargo test --locked -p caper-api -- --ignored --skip tests::shared
```

Channel voice tests use a mocked SFU with real SQL and, for shared-mode tests,
real Valkey. They cover isolated rosters/tokens/tracks, account binding, privacy
changes, membership/session revocation, and fresh-pod cleanup discovery. They do
not validate live Cloudflare calls, sustained audio, TURN across networks, or
physical devices. Those still need an operator smoke test with two accounts in
two different channels and a private channel, including removal during a call.

September 22, 2026 local verification: 85 default Rust tests, 17 explicitly
enabled integration tests on disposable Postgres 15 and cluster-mode Redis, and
222 web tests passed. The integration tests include concurrent quota creation
and a send blocked behind private-grant removal (authorization uses a fresh
snapshot after acquiring the space lock). Real local API/gateway/browser checks
covered creation, username membership, private visibility, cross-account live
messages, channel-switch isolation, rename/delete, duplicate-name errors,
self-leave, and desktop/390px narrow layouts. Browser voice was disabled; media
tests used a mock provider. This is not physical-mobile or live SFU validation.

Deploy the **API first** (startup applies the migration and runtime
grants), **gateway second**, **web last**. Do not expose the new UI while old
gateways are still serving. No new secrets are needed; use the existing database,
auth, `CHAT_ENABLED`, `MEDIA_ENABLED`, and shared `VALKEY_URL` settings. Commands
for an operator after merge and image publication:

```bash
MERGED_SHA=<full-merged-caper-commit>
gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
# Wait for that workflow to succeed before proceeding.
kubectl -n default rollout status deployment/caper-api --timeout=15m
gh workflow run deploy-caper-gateway.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
# Wait for that workflow to succeed and every gateway pod to use the new image.
kubectl -n default rollout status deployment/caper-gateway --timeout=15m
gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
kubectl -n default rollout status deployment/caper-web --timeout=15m
```

These are instructions, not deployments performed during development. Existing
guest URLs and their room key are retained. Rollbacks must keep a channel-aware
API/gateway pair once account spaces are in use; older APIs do not maintain or
revoke the new channel rooms. The migration replaces the channel-name unique
constraint with an active-channel partial index. Old API/gateway startup seed
queries are incompatible with that index, so do not restart or roll back to a
pre-spaces image after migration; complete the forward rollout instead.

## Shared call state and rolling deployments

The original shared-state rollout below covers the public General room. Account
channels use the same protocol with the isolated room keys described above, each
capped at 12 participants. Membership and history live in Postgres; active voice
state lives in Valkey. Typing indicators are transient Pub/Sub events.

- Valkey stores the call capability hash, Caper-to-Cloudflare session mapping,
  track/subscription metadata, mute/deafen state, leases, operation ownership,
  temporary join reservations, cleanup jobs, and the latest ephemeral TURN
  credentials for retry-safe renewal. Treat this store as secret-bearing: TURN
  passwords are not provider API keys, but do grant relay access until expiry.
  It never stores audio or SDP (only an offer hash for renewal replay validation).
  A successful join is not returned until its mapping is committed.
- `caper:{general}:v1:state` is a hash with one field per participant plus bounded
  channel metadata. Short WATCH/MULTI/EXEC transactions arbitrate concurrent pods;
  only changed fields are written. Cloudflare calls never run inside a retried
  transaction. An uncertain EXEC or provider mutation is not blindly replayed.
  [ElastiCache Serverless supports WATCH within one hash slot](https://docs.aws.amazon.com/AmazonElastiCache/latest/dg/ServerlessWatch.html);
  the state key and notification topic share the `{general}` hash tag.
- The same transaction publishes a change notification. Each API has **one**
  Pub/Sub connection and one Tokio listener; `tokio::sync::watch` coalesces local
  notifications and wakes its SSE clients. There is no fanout worker, Stream
  consumer group, or blocked Valkey connection per channel or browser. Each SSE
  client reads current shared state before sending its projection.
- Pub/Sub is deliberately not a history. A reconnect sends the latest snapshot,
  not a playback of missed mute/unmute events. Subscription loss makes the API
  unready; reconnection wakes streams to resynchronize. Read/write failure returns
  503, never an independent in-memory fallback. `/health` remains liveness;
  configure Kubernetes readiness to `/readyz` after the compatible API is deployed.
- SIGTERM rejects new media commands and allows in-flight HTTP requests to finish.
  Streams opting into `handoff=1` receive `migrating` and keep forwarding shared
  state during a ten-second handoff window. Older streams receive `draining` and
  close. Shared-mode shutdown does **not** remove participants or close provider
  tracks. Healthy media stays browser ↔ Cloudflare while SSE changes pods with the
  same capability. No sticky sessions are needed.
- Explicit leave publishes immediately. A vanished browser is different: its
  existing 45-second lease expires, with a five-second sweep (up to about 50 seconds
  after the last renewal). SSE disconnect alone is not leave, because deployments
  and brief network changes also disconnect SSE. Active calls have no API age cap;
  see the separate TURN credential lifetime below.
  Provider cleanup is asynchronous, with shared 30-second claims and safe retries.
  Abandoned joins and uncertain in-flight operations expire after 30 seconds;
  uncertain operations invalidate only the affected call and discover its tracks.
  ICE renewal is an exception: its replayable offer has a replaceable 30-second
  claim, not a deadline that removes the participant.

Normal in-call and spectator updates have no polling interval: commit → Pub/Sub → API → SSE.
Production end-to-end latency is **not measured**. The 15-second request is a lease
renewal, not the notification path. Speaking indicators remain browser-side audio
analysis, with no per-frame Valkey traffic. Spectator streams send an initial roster
and push changes, including an empty roster after leave. Ten-second SSE heartbeats
detect broken connections; they are not the roster update interval. Spectators
reconnect with backoff from 250 ms to five seconds after failures and after 50 ms
on legacy draining, then replace the roster with a fresh snapshot. The sidebar
marks last-known data with “Updating live roster…” until a snapshot arrives.

During a rolling restart the existing SSE connection still belongs to the old
pod; replica availability does not transfer that connection. The infrastructure
companion adds a five-second `preStop` serving window before SIGTERM so endpoint
withdrawal can propagate while existing requests and SSE still work. Its 65-second
termination grace includes that window and preserves the API's 60-second budget.
The first deployment of the hook terminates old pods that do not yet have it.

For opt-in streams, `migrating` opens a replacement **without closing the old
stream**. The browser continues rendering old-stream pushes until the replacement
delivers a valid snapshot at least as recent as its last received revision. `ready`
alone is insufficient. Only then does it cancel the old stream. Replacement
registration changes the shared connection ID, but a retiring stream temporarily
ignores that supersession while still authenticating the capability on each read;
leave/revocation still ends it. Non-retiring streams retain single-connection
replacement semantics. No participant schema change is required.

At most two streams are open per logical subscription. Replacement failure retries
while the old stream keeps serving; a candidate without a current snapshot times
out after five seconds. Only explicit `503/api_draining` admission rejections use
up to ten fast 50ms retries; other failures back off. If the old stream expires and
no replacement succeeds, existing outage recovery takes over without tearing down
healthy voice. The handoff fits inside the existing 30-second HTTP drain budget.
Process crashes, network loss, blocked intermediaries, or a replacement unavailable
beyond the serving window still cannot provide uninterrupted updates.

Validation uses real local HTTP servers and disposable Redis to test Axum graceful
shutdown, overlapping authenticated/spectator delivery after replacement
registration, capability revocation, and completion after cancellation. Cloudflare
is mocked. Browser tests cover delayed/stale snapshots, retry/timeout/cancellation,
and repeated handoffs. The real Call component fixture holds replacement snapshots
while pushing 30 status changes through old streams and verifies each rendered
state and unchanged voice peer (mocked API/WebRTC). These do not measure production
cluster routing or physical-device latency.

Deploy **all API pods first, then web**, and refresh both clients. Keep the merged
infrastructure preStop hook. The protocol is opt-in: old browsers still get the
legacy close/reconnect behavior, and new browsers talking to an older API fall
back to its `draining` event. The first API rollout replaces binaries without the
overlap behavior, so retest after both deployments finish. After both image builds:

```sh
gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure -f git_sha=<merge-sha>
# Wait for that workflow and every API pod to run the new image, then:
kubectl -n default rollout status deployment/caper-api --timeout=15m
gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure -f git_sha=<merge-sha>
```

Deploy the API containing `/api/media/presence/events` before the updated web image.
Existing clients remain compatible with the JSON endpoint. This change requires
no Valkey migration, provider configuration, or infrastructure change.

The browser regression fixture deliberately lets the idle sidebar read the old
roster before leave commits. The previous polling page retains that row; the SSE
page clears it on the pushed update. Run against a local Vite server with
`node scripts/test-public-presence.mjs http://localhost:5174`. It exercises the real
Call component/client with mocked API/WebRTC, including repeated join/leave and
stream loss/reconnect; it does not validate live audio. Shared-store tests cover
the server's cross-pod push independently, with provider cleanup still pending.

The browser keeps lease renewal, mute/deafen synchronization, and SDP negotiation
independent. Rapid mute/deafen changes coalesce to the latest local intent rather
than replaying queued toggles. State writes time out after five seconds and retry
transient failures after 250 ms while connected; local controls remain usable and
each write carries an increasing per-client sequence. The API ignores older or
duplicate sequences, preventing a timed-out write on another pod from replacing
newer intent. A newer self snapshot that disagrees with local intent also triggers
repair. Pushed rosters render without waiting for state writes or SDP negotiation.

SSE heartbeats use a fixed ten-second interval: unrelated connection notifications
cannot postpone them indefinitely and trip the browser's 25-second watchdog. If a
heartbeat observes a new roster revision before its Pub/Sub notification arrives,
it sends the snapshot instead of silently acknowledging that revision. Normal
updates remain push-driven. The browser limits each SSE frame, not an arbitrary
network chunk that may contain many valid frames. Connected stream recovery no
longer inserts a paragraph that shifts the call layout; actual call errors remain.

For these synchronization fixes, deploy **all API pods before web**, then refresh
clients. Old APIs reject the new `sequence` request field. Existing participant
records default `state_sequence` to zero; there is no manual Valkey migration.
Older browsers can still send unversioned state writes. After merge and both image
builds, run the following from an operator environment (not performed by tests):

```sh
gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure -f git_sha=<merge-sha>
# Wait for that workflow to succeed and every API pod to run the new image.
kubectl -n default rollout status deployment/caper-api --timeout=15m
gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure -f git_sha=<merge-sha>
```

Regression tests cover heartbeat starvation, a delayed notification, and stale
state ordering across two API instances sharing disposable Redis. The browser
fixture holds an authenticated SSE reconnect, verifies unchanged layout/peer
identity, then pushes mute/deafen changes after recovery. These checks use mocked
API/WebRTC or mocked Cloudflare; they do not establish production delivery latency.

Join includes the browser's current `muted` and `deafened` values. The API commits
them with the participant, so the first public/in-call snapshot is correct rather
than briefly advertising false defaults until publication finishes. Omitted fields
remain false for older clients. A follow-up state write reconciles changes during
capture/join concurrently with publication, rather than waiting for SDP/transport.
Deploy all API pods with this request shape **before** the web update; the previous
API rejects unknown join fields. No shared-state schema or secret changes are needed.

Rollout regression tests reproduce two failures in the previous browser: obsolete
queued mute values and lease renewal blocked behind a pending state write. Tests
also cover 503/504 state responses during SSE draining, slow subscription setup,
late commits, and cancellation across leave/rejoin. Disposable Redis tests check
alternating mute updates across API instances before and after replacement with
unchanged capabilities/provider sessions and no provider track closures. These use
mocked Cloudflare, not real media. Desktop and narrow Chromium UI checks use labeled
mock calls; they are not physical iOS or live rollout verification. The reported
production 504's originating request/intermediary and iOS audio-output warning
remain unconfirmed.

The follow-up disconnect audit also covers these interactions:

| Trigger | Recovery behavior |
| --- | --- |
| Join/publish/subscribe/negotiate/close reaches a draining API | Explicit `503` + `code: api_draining` proves admission rejection before mutation. Retry up to three times after 250/500/1000 ms within the original request deadline. Generic 5xx/timeouts do not authorize replay. |
| A speaker leaves before subscription starts | `404` + `code: track_gone` skips only that track; continue subscribing to other participants. Authentication errors still invalidate the session. |
| A speaker leaves while a successful subscription is being created | Preserve the listener, finish any returned SDP offer, then close the unwanted MID from the latest roster. Do not discard the listener's own microphone/session. |
| Cloudflare explicitly rejects a departed track during subscription | A successful HTTP response with one `not_found_track_error`/`track_error`, no MID/SDP, and explicit `requiresImmediateRenegotiation: false` becomes `track_gone`; release the operation lock and preserve the listener. HTTP errors, partial offers, allocated MIDs, and session errors remain ambiguous and do not take this path. |
| Closing a departed track fails or times out | Remove local playback immediately; retry transient HTTP cleanup failures on subsequent reconciliation/lease heartbeat without rejoining. API close commits track removal and a cleanup job atomically. Provider cleanup failure never revokes the listener or its other tracks. |
| SSE fails while authenticated lease renewals work | Retry SSE independently and keep voice/layout stable. Snapshots still repair state, but normal real-time delivery requires SSE recovery. |
| A previous call still has a pending device request | New signaling/media queues do not wait on old work. Old rollback cannot restart the new call. |
| Several separate successful recoveries over time | Reset the consecutive retry budget after each successful rejoin; no lifetime three-recovery quota. |

Track close uses Cloudflare's `force: true`, documented as stopping data flow
[without WebRTC renegotiation](https://developers.cloudflare.com/realtime/static/realtime-api-2024-05-21.yaml).
It is cleanup, not evidence that the remaining participant's connection failed.
The browser fixture checks repeated remote joins/leaves with failed close requests:
the connected heading, connection details, peer identity, and microphone remain
unchanged. API tests exercise the actual provider HTTP adapter against a local
stub for rejected pulls and verify the boundary with ambiguous responses. These
are fault-injection checks, not live SFU or physical-device verification.

Failure-injection coverage includes these cases and cancellation during retry
backoff. Deploy the **API first, then web**, and refresh both clients before
repeating the two-device rollout test. Error codes are additive; no Valkey schema,
secret, or infrastructure change was needed for those interaction fixes. The TURN
renewal extension below adds participant fields and requires API-first deployment.

Remaining disconnect conditions are not solved by adding retries: unknown outcomes
of provider SDP mutations, invalid/expired sessions, sustained network loss, and
failed local capture/transport recovery.
In particular, a Cloudflare HTTP 200 body containing `internal_error` is a failed
provider operation; Caper reports 502. Without evidence that no MID/SDP mutation
occurred, retrying the same subscription is unsafe. The September 21 production
logs establish that error mapping, not the underlying provider cause or whether
the session changed upstream. No unconditional subscription retry was added.
Initial SSE setup still fails Join if it cannot establish a valid handshake. The
private mic test has separate failure handling; losing it does not leave General.

**Call duration:** the API no longer expires public or private monitor sessions
because of their age. Live sessions retain their IDs, tracks, and subscriptions
while heartbeats renew the 45-second lease. TURN credentials for new joins use
Cloudflare's maximum **48-hour lifetime**, not the previous one hour. Leave and
lease expiry still enqueue credential revocation. If revocation cannot complete,
credentials may now remain usable for up to 48 hours instead of one hour.

**Automatic TURN renewal:** public calls and both private mic-test peers request
fresh credentials halfway through the lifetime (24 hours), then restart ICE on
the **same** peer and SFU session. Track IDs, subscriptions, capture, mute, and
deafen state are retained. This removes the credential-age cutoff; it is not a
guarantee against network outages, browser suspension, or a brief audio gap while
the selected relay changes. `setConfiguration()` alone does not update an existing
TURN allocation. The browser must gather and negotiate fresh ICE credentials.

- `POST /api/media/turn` caches one current credential generation. A lost response
  returns that same generation across pods, rather than minting another password.
  Issuance is limited to two attempts/minute; expired passwords are pruned and
  still-valid usernames retained for leave-time revocation. At most three known
  credential generations may overlap.
- `POST /api/media/restart-ice` accepts only an audio ICE-restart offer: no novel
  sending MID, video, data channel, duplicate MID, or sendrecv direction. It uses
  Cloudflare's `POST /sessions/{id}/tracks/new` with `autoDiscover:true` and requires
  an answer with zero newly created tracks. `PUT /renegotiate` rejected restart
  offers in the live probe; do not substitute it.
- The credential generation, sequence, offer hash, and expiring ownership claim
  live in Valkey. Retries replay the **exact** offer. No SDP or provider secret is
  persisted. The browser holds its signaling queue until it applies the answer
  and receives an idempotent `POST /api/media/restart-ice-ack`. Heartbeats,
  mute/deafen writes, and leave do not wait behind that queue.
- Transient network/HTTP failures retry with 1/2/4/8/15/30-second capped backoff.
  Permanent signaling rejection uses existing bounded session recovery instead
  of inventing a different offer while the old operation is pending. A stale
  completion cannot clear a newer claim or restore a departed participant.
- Deploy **all API replicas first**, then web. Old API writers do not preserve
  the added fields. Refresh existing tabs and leave/rejoin once to install the
  new browser renewal lifecycle; already-issued credentials cannot be extended.
  No new infrastructure, secrets, or Postgres migration is required. Do not roll
  the API back to an old writer underneath active renewal-enabled calls.

References:
[Cloudflare credential lifetime](https://developers.cloudflare.com/realtime/turn/generate-credentials/),
[expiry behavior](https://developers.cloudflare.com/realtime/turn/faq/), and
[WebRTC configuration semantics](https://w3c.github.io/webrtc-pc/#set-the-configuration).
Regression tests simulate call ages around one hour and at 49 hours, including
two API instances sharing disposable Valkey. These verify state retention and
lease cleanup. Separate live Chromium probes used an isolated Cloudflare app/key,
synthetic 440 Hz audio, and TURN/UDP forced on both peers. Same-session renewal
preserved decoded audio beyond short-lived credentials' expiry; replaying a
discarded answer kept the same remote ICE credentials and created zero tracks.
No-renewal and configuration-only controls stopped receiving audio after expiry
plus relay grace. The integrated probe runs the actual Rust routes against two
API instances with shared disposable Redis and the actual `TurnRenewal` helper,
with successful credential/answer/ACK responses deliberately discarded.
With 90-second credentials, that probe completed eight renewals per peer over
378 seconds, with zero recovery callbacks and decoded audio still present on
the original tracks. The isolated Cloudflare app and TURN key were deleted afterward.
These are accelerated expiry tests, not a 48-hour soak, physical-device testing,
or proof of gapless audio. Safari/Firefox and restrictive TCP/TLS networks still
need their own renewal acceptance checks.

The hash is a bounded channel unit, not a global blob for every future channel.
Account channels now use separate keys with a shared notification subscription;
see [spaces and channel access](#spaces-and-channel-access).
Do not remove the 12-person limit and call this a thousand-speaker media system:
all-to-all audio needs separate active-speaker/subscription limits and load tests.
Changing Valkey endpoints or losing its data loses live calls; API replacement
does not. A crash after Cloudflare creates a resource but before its response is
recorded can still leave a resource until provider expiry. There is no distributed
transaction with Cloudflare.

### Activation and verification

The infrastructure companion provisions an inactive private ElastiCache Serverless
Valkey endpoint (100 MB automatic billing minimum, not a storage cap). No AWS apply,
production secret projection, replica increase, or deployment is performed by this
application change. Follow the infrastructure `docs/shared-valkey-runbook.md`:
provision first, deploy compatible API/web images while still in single-process
mode, then perform the **one-time empty-channel cutover** before enabling two pods.
Never overlap local-mode and shared-mode callers or roll back to a local-only image.
Subsequent same-schema deployments use RollingUpdate with `maxUnavailable: 0`,
`maxSurge: 1`, `/readyz`, and a 65-second termination grace including the five-second
preStop serving window from the infrastructure companion.

Real disposable Redis integration tests (Redis-compatible protocol) cover independent
API instances, cross-pod notifications and capability replacement, full replacement
with unchanged Cloudflare mappings, capacity/operation races, lease/claim recovery,
and connection loss/recovery without replay. Cloudflare is mocked in these tests.
CI runs them against Valkey 8.1. Run locally using a disposable loopback server:

```bash
# In an orb; do not point TEST_VALKEY_URL at a shared/production service.
amp orb service start caper-test-redis --command 'redis-server --bind 127.0.0.1 --port 6389 --save "" --appendonly no'
TEST_VALKEY_URL=redis://127.0.0.1:6389 cargo test --locked -p caper-api tests::shared -- --ignored
```

Before claiming production zero-audio-interruption: run two real browsers against
different pods, confirm shared join/mute/leave, replace one API then all old API
pods, and verify unchanged browser peer/session IDs and continuous bidirectional
audio. Repeat with private mic test, a killed pod, and a short Valkey interruption.
Record actual notification/reconnect latency, multi-network/TURN, sustained voice,
and physical-device results separately. None of those live-media checks is proven
by the shared-store tests or by HTTP readiness.

### Local Compose shared-state testing

`npm run dev` starts Valkey 8.1 alongside the API, web, gateway and Postgres.
The API waits for Valkey's health check and defaults to `redis://valkey:6379`.
Only Compose sets the process-only `VALKEY_ALLOW_INSECURE=true` opt-in, which
allows plaintext to the exact service hostname `valkey`, not arbitrary hosts.
Production still requires authenticated TLS; this flag never disables certificate
verification. No Valkey port is published to the host and no AWS cache is needed.
Leave `VALKEY_URL` absent from `staging/apps/caper` and unset/empty in `.env` to use
local Valkey; a Secrets Manager URL still takes precedence over Compose's default.

From the repository root, with Docker running and the staging AWS profile logged in:

```bash
npm run dev -- --scale api=2
```

In another terminal:

```bash
docker compose -f compose.staging.yaml exec valkey valkey-cli ping
docker compose -f compose.staging.yaml ps
docker compose -f compose.staging.yaml restart api
```

Expect `PONG` and two API containers. The last command replaces API processes
while keeping Valkey alive; it is not a Kubernetes rolling rollout. Voice testing
still requires `MEDIA_ENABLED=true` and staging Cloudflare credentials. Check
join/mute/leave in two browsers, then verify sessions survive the API restart.
Two replicas alone do not prove that two particular clients hit different replicas.

Valkey is deliberately disposable: disk snapshots and append-only persistence are
disabled. Restarting Valkey clears active calls; restarting only the API does not.
Stop the stack with Ctrl-C. `docker compose -f compose.staging.yaml down` removes
containers; do not add `--volumes` unless you also want to delete local Postgres data.

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
| `APP_SECRET_ID` | Optional AWS Secrets Manager JSON record loaded by the API before other configuration. Application settings in this record take precedence over process environment fallback. AWS credentials, region, and this secret ID remain bootstrap settings outside the record. |
| `MEDIA_ENABLED` | `true` enables voice; absent/false disables it |
| `VALKEY_URL` | API-only shared live call state. Empty uses single-process development mode outside Compose; Compose defaults to local Valkey. Hosted endpoints require `rediss://username:password@host:6379` with certificate verification. Plain `redis://` is allowed on loopback, or the Compose hostname with the explicit opt-in below. Never project this into the web container. |
| `VALKEY_ALLOW_INSECURE` | Process-only local-development opt-in (`true` or `1`); allows the exact hostname `valkey` without TLS/password. Set by staging Compose, absent in production. Does not relax requirements for other remote hosts or disable TLS verification. |
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
| `DATABASE_URL` | API-only runtime URL: pooled port `6432`, database `/caperchat`, restricted app role, verified TLS. Loopback development may explicitly use `sslmode=disable`. Missing configuration fails account access closed. |
| `MIGRATION_DATABASE_URL` | API-only startup migration URL: direct port `5432`, database `/caperchat`, separate schema-changing role, verified TLS. Required when `DATABASE_URL` is set; never falls back to it. Startup grants the parsed runtime role access to migrated application tables. Neither DB secret belongs in WEB. |
| `DATABASE_ALLOW_INSECURE` | Local development only. Set by Compose so the API may connect without TLS to the private `postgres` service. Hosted databases still default to verified TLS. |
| `AUTH_SECRET` | API-only random secret of at least 32 bytes. Enables account login and HMAC-protects low-entropy codes/IP rate-limit keys. Keep stable across replicas and rotations deliberate. |
| `AUTH_CODE_ATTEMPTS` | Attempts per code; default `3`, allowed `1`–`10` |
| `AUTH_EMAIL_15M_LIMIT` | Code requests accepted per email in 15 minutes; default `3` |
| `AUTH_EMAIL_DAILY_LIMIT` | Code requests accepted per email in 24 hours; default `5` |
| `AUTH_IP_HOURLY_LIMIT` | Code requests accepted per source-IP hash in one hour; default `10` |
| `AUTH_GLOBAL_HOURLY_LIMIT` | Code requests accepted across the service in one hour; default `500` |
| `NOTIFICATIONS_WEBHOOK_URL` | Optional server-only HTTPS URL for best-effort application notifications. The current event is `user.created`; URLs with credentials, query strings, or fragments are rejected and leave notifications disabled. |
| `AWS_REGION` | SES region; production and staging use `us-east-1` |
| `SES_FROM_ADDRESS` | Verified Caper sender, including the friendly name |
| `SES_CONFIGURATION_SET` | Required SES transactional configuration set |

Use `.env.example`; Rust does not auto-load dotenv files. Export a private env
file before `cargo run -p caper-api`. Run the independent web process with
`npm run dev:web`. Remote browsers require HTTPS.
In an orb use supervised services and portal URLs, not direct sandbox host URLs.

`npm run secrets:check` reads `staging/apps/caper` with the `staging` AWS profile
and reports key names as `Secrets Manager`, `in sync`, `differs`, `fallback only`,
or `missing`; it never prints values. Pass `--environment production --profile
production --cluster` to also report each production ExternalSecret's Ready
condition from the current kubectl context.

Image CI builds `apps/api/Dockerfile` and publishes
`production/caper:api-<full-40-character-sha>` after merge to main via `api-image.yml`,
using the existing `production-caper-ecr-publisher` IAM role.
The Docker target and service name are `api` and `caper-api`, respectively.
This is the application control API, not a self-hosted audio relay. Publishing
does not roll it out. The image notification includes a green **Deploy Caper API**
button for that exact SHA and an **Open GitHub** fallback to
`deploy-caper-api.yml` in `joswayski/infrastructure`.
Deployment/Service/ExternalSecret are named `caper-api`; the container is `api`.
The application-wide AWS Secrets Manager record is `production/apps/caper`.
All configured consumers use this record; legacy-secret retirement is managed
through the infrastructure repository's OpenTofu cleanup, not console deletion.
Consumers receive only their required fields:
the API's Cloudflare projection remains Kubernetes Secret `caper-api-cloudflare`.
Keep one desired API replica with `RollingUpdate`,
`maxSurge: 1`, `maxUnavailable: 0`, and a 65-second termination grace, port 3001,
`/api/media` routing, and existing `MEDIA_*` / `CF_*` configuration names.
The web Deployment, Service, Ingress, PDB, container, and deployment target are
named `caper-web`; it runs with two replicas.

Web and API images publish independently. `api-image.yml` runs only when
`apps/api`, workspace Cargo files, or that workflow change. `aws-image.yml`
runs only when the website Docker context, `apps/web`, `shared`, npm workspace
manifests, or that workflow change. An API-only merge does not publish a website image or
send a **Deploy Caper web** notification, and a website-only merge does not
publish an API image. Future workers or other services should get their own
workflow and path allowlist instead of riding these two.

Outbound HTTPS to `rtc.live.cloudflare.com` is required for the Rust API. AWS
needs no public media UDP ports. Clients use SFU plus TURN UDP and TCP/TLS
fallback; alternate port 53 is filtered from both STUN and TURN URLs. `/api/media/status` reports
the feature flag; web `/health` is independent of provider availability.

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
same privacy rules - this is not an arbitrary-string redaction engine. The richer
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
record `production/apps/caper`, preserving **every existing property**. The
infrastructure operations runbook includes secure private-file upload commands;
the Secrets Manager console's key/value editor is also suitable. No IAM or
Terraform apply is needed for adding these logging properties to the active record;
legacy-secret cleanup has its own infrastructure steps.
Wait for the new ExternalSecret to be Ready, then use
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
  future native clients, which will not all update at deployment time.
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
- No fixed call-age limit; 48-hour TURN credentials renewed at 24 hours;
  45-second presence lease, sweep every five seconds. Background browser
  throttling can force a reconnect.
- Leave stops local capture immediately, invalidates the capability, force-closes
  SFU tracks/subscriptions, and revokes TURN credentials. Failed track cleanup has
  a bounded retry backlog; crashes/outages can prevent immediate revocation.
  Credentials remain bounded by expiry. Shared mode persists cleanup in Valkey;
  memory-only development loses its backlog on process exit.
- Serialized negotiations and per-participant operation guards. Ambiguous provider
  creation is not blindly retried; the session is invalidated and cleaned up.
- Transient heartbeat failures (network/timeout, HTTP 408/429/5xx) retry after
  three seconds without tearing down media (healthy heartbeat cadence remains 15 seconds). Snapshots have
  a five-second deadline including response-body reads; failures lasting at least
  30 seconds trigger recovery on the next failed poll. Successful snapshots reset
  that window. Invalid sessions trigger recovery immediately. Failed mute/deafen
  state sync retries the latest local state independently of heartbeats;
  ambiguous SFU mutations are not blindly replayed.
- A transient WebRTC `disconnected` state gets ten seconds to recover in place;
  `failed` or a sustained disconnect triggers up to three consecutive failed rejoin
  attempts retaining mute/deafen and device choice; success resets that budget.
  Permission/device failures are visible. All microphone subscriptions
  are automatic. Deafen mutes playback, not forwarding/bandwidth.
- Mute disables the local track and detaches it from the sender. Opus is preferred;
  browser echo cancellation and gain control are off for headphones. Codec-managed
  Opus DTX is requested for outgoing audio (see below); savings are not guaranteed.
- Speaking indicators and diagnostics use browser stats where available, not
  billing records. Microphone/output selectors are available in-channel. Output
  selection requires `setSinkId`; otherwise use OS settings. Joining requires microphone permission.

### Codec-managed silence suppression (Opus DTX)

The retained web voice engine requests `usedtx=1` for Opus in remote publication
answers and subsequent subscription offers, including the private mic-test sender.
[RFC 7587](https://www.rfc-editor.org/rfc/rfc7587.html#section-7) defines this as a
receiver preference: the **remote** description controls our outgoing encoder.
Setting it only in our local offer would request the opposite direction.
The preference is reapplied during renegotiation and is retained when replacing
the microphone track. Non-Opus codecs are left alone.

There is no application volume threshold, noise gate, speech-triggered track
detachment, or added buffering. DPDFNet processing, bitrate preferences, packet
duration and FEC are unchanged. Opus decides when to reduce transmission during
silence; it does not replace noise removal. Browser WebRTC exposes no DTX
sensitivity or hangover tuning. DTX is not lossless silence removal: RFC 7587
notes a possible quality tradeoff, and this change cannot guarantee preservation
of every whisper, consonant, laugh or word ending. Prefer continuous transmission
if physical listening tests reveal degradation; do not add an aggressive gate.

September 15, 2026 verification: all 119 web tests and `npm run check` passed.
Tests cover remote answer/offer handling, independent media payload IDs, existing
or absent format parameters, unchanged FEC/bitrate/packet duration, idempotence,
publication, private monitoring, renegotiation and existing mute/device behavior.
Two local Chromium Opus loopback runs with generated speech measured 14 silence
packets / 231 RTP payload bytes versus 150–151 packets / 4,800–4,832 bytes over
three seconds without DTX. These are sender RTP measurements, not billed egress.
After silence, synthetic speech at 30 dB below the preceding phrase resumed at
the same observed onset (within 1 ms between the two receivers); sampled decoded
energy was 81–87% of the control. This is a coarse regression check, not a
speech-quality score or proof that individual words are intact. The test bypasses
DPDFNet and uses no hardware microphone, SFU, TURN, or production credentials.

Reproduce with local Vite, installed `agent-browser` and `espeak-ng`:

```sh
espeak-ng -s 145 -v en-us -w /tmp/caper-dtx-speech.wav 'Please keep the first quiet words, soft endings, and short pauses.'
node scripts/test-opus-dtx.mjs http://localhost:<vite-port> /tmp/caper-dtx-speech.wav
```

Before accepting audio quality, compare DTX on/off with real quiet speech,
whispers, initial consonants after several seconds of silence, short pauses,
word endings and laughter, with DPDFNet active and under device load. Verify live
SFU forwarding, forced TURN, loss/jitter, joining another participant, and device
replacement separately. Firefox, Safari, and future native clients remain unverified.
Account identity does not change media transport. Authenticated display names in
live SFU voice still require staging validation after deployment.

## Leaving voice

Explicit Leave/Cancel and page unload share synchronous local teardown: stop
capture and processed tracks, close the PeerConnection and event stream, clear
playback/roster state, and invalidate the old call generation. Explicit Leave
then shows an enabled Join button without awaiting the provider cleanup response.
The old capability is captured before reset and sent to `/leave` with Fetch
`keepalive`; completion or failure never changes the next call's state. Late
peer events and microphone replacement completions are also session-guarded.
Automatic reconnect and join-error teardown also stop local media synchronously
and send old-token cleanup in the background. A failed Join reports its original
error without waiting for Leave; late cleanup never changes a newer generation.

The four-to-five-second leave delay was separate from ICE gathering. The Rust
endpoint removes registry membership/tokens and notifies SSE listeners before
queueing TURN revocation and track/dependent cleanup. It returns without waiting
for Cloudflare cleanup. The previous client kept its `leaving` phase until that
entire HTTP request returned. If Leave cannot reach the API, remote presence may
persist until the existing 45-second lease expires. Local audio stays stopped
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

Those measurements describe the initial web-only Leave fix. The reliability
changes below also require an API rollout. Do not treat background request
duration as local leave latency.

## Provider failures and cleanup

Caper HTTP 502 does **not** prove Cloudflare returned 502. Provider HTTP errors,
timeouts, network failures, response-body failures, invalid JSON, top-level or
per-track error envelopes, and locally rejected response shapes keep the existing
`502 {"error":"media provider unavailable"}` contract. They now carry an
`x-caper-error-id` response header matching the Rust warning's `id`. This header
also passes through the development adapter. Do not confuse it with the outer
Cloudflare edge's `cf-ray` response header.

`Cloudflare operation failed` logs include the operation (session creation, TURN
issue/revoke, publish, subscribe, negotiate, close, or session read), failure kind,
upstream HTTP status when available, upstream `cf-ray`, bounded error code, and
elapsed milliseconds for that provider request. `media request failed` links the
returned reference to those details. Local shape-validation failures have no
invented upstream status or timing. Successful calls log duration at debug level.
No request/response bodies, SDP, raw media, credentials, full URLs, session IDs,
MIDs, or free-text upstream error descriptions are logged. Response bodies are
bounded to 256 KiB; once a non-success HTTP status is received, collecting its
optional error code has only a 250 ms budget before returning the known failure.

Subscription failure preserves the original provider error instead of replacing
it with a missing-MID rejection. An ambiguous mutation still invalidates that
participant; continuing with potentially inconsistent SDP is not safe. Unknown
subscription MIDs are discovered via a background session read and then closed.
Registry removal, token revocation, dependent-subscription removal, and roster
invalidation do not wait for provider cleanup.

Cleanup uses an in-memory queue of at most 512 jobs and a single worker with four
concurrent slots. Local queue changes wake the worker immediately, retry deadlines
wake it at their scheduled time, and a five-second reconciliation poll recovers
shared work left by another replica. A separate five-second expiry sweep cannot
be held up by provider IO. Each job has a 12-second bound; Cloudflare HTTP requests
retain their 10-second timeout. Only session reads, `force:true` track closure (no
SDP renegotiation), and TURN credential revocation are eligible for cleanup
retries. Session creation, TURN issuance, tracks/new, and renegotiation are
**never automatically replayed by the HTTP transport**. The existing client's
bounded whole-session recovery is separate from replaying a mutation against the
old session.

Cleanup retries only network/timeouts and HTTP 408/429/500/502/503/504, at most five
attempts per queued job, with 2/4/8/16-second delays plus up to 1.02 seconds jitter.
Other HTTP rejections and invalid responses stop that job rather than looping.
A full queue drops new jobs with a warning/error instead of growing memory;
exhausted jobs are logged as abandoned, not reported as successful cleanup.
Shutdown stops the background loops, waits for an active local expiry pass, and
spends up to 20 seconds draining cleanup batches after HTTP requests drain.
Crashes, exhausted retries, queue overflow, and shutdown deadlines can leave
provider resources behind. TURN expiry remains the 48-hour backstop; this is
not a durable cleanup system or automatic provider failover.

After merging, deploy **both** merged images using the normal **Deploy Caper API**
and **Deploy Caper web** buttons. No migration, secrets, or Cloudflare configuration
changes are required. Optional equivalent operator commands (replace the SHA):

```sh
MERGED_SHA=<full-merge-commit-sha>
gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
```

For a new failure, copy its `x-caper-error-id` from Network → failed request →
Response Headers, plus UTC timestamp and endpoint (not SDP or bearer tokens).
From an authorized Kubernetes operator shell, correlate it with:

```sh
kubectl -n default logs -l app.kubernetes.io/name=caper-api -c api --since=30m --prefix --max-log-requests=5 | rg '<error-id>'
```

If a 502 lacks that header, check whether the current API image is deployed and
whether ingress generated the response; absence alone does not identify the
failed layer. The September 7 ~5.5–6-second trace lines alone do not establish
Cloudflare as the source. There is currently no application-log CloudWatch group
available through the orb's read-only AWS role, so those historical failures
remain unconfirmed. These changes improve diagnosis and recovery behavior; they
do not establish that the original upstream issue is fixed.

Verification: injected provider HTTP errors, per-track errors, invalid JSON,
header/body timeouts, stalled cleanup, bounded retry/backoff, capability expiry,
and shutdown draining are covered by Rust tests. Browser DOM checks on desktop
and a 390px viewport used real DPDFNet-8 with synthetic silence and a mocked
publication 502 plus unresolved Leave: the error was visible, Join enabled, raw
tracks ended, and peers closed while cleanup remained pending (4 ms error-to-UI
on the desktop probe). This is controlled failure testing, not live SFU outage,
physical microphone, multi-network/TURN, or sustained-voice validation.

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

`GET /api/media/events?snapshots=1` uses `x-caper-media-token`, not a capability
in a query string or cookie. The browser consumes SSE with streaming Fetch.
The server emits `ready`, an initial `snapshot`, coalesced current snapshots on
public roster mutations, and `heartbeat` after ten seconds without another event.
Snapshots contain `participants` and a monotonic channel `revision`; other events
contain `{}`. Older clients without `snapshots=1` still receive `changed` and fetch
the roster. No private monitor session is exposed or authorized
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
reopens only the event stream with the same capability, backing off from 250 ms
to three seconds while keeping healthy audio. Opt-in `migrating` events use the
overlapping snapshot-gated handoff described above. Legacy `draining` reconnects start
after 50 ms; explicit `503/api_draining` rejections share the bounded fast retry
budget described above. An SSE-only outage does not force a new voice session while
authenticated snapshot renewals succeed. Connected recovery keeps the layout stable;
invalid sessions or sustained heartbeat failure still trigger session recovery.
Each restored stream receives current state, not a replay of missed mute/unmute
transitions. Initial Join readiness remains strict.
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
while older clients remain compatible with the additive API. Keep one API replica
until the shared-state activation described above is complete.

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
The private receiver attaches a muted sink as soon as its track arrives so the
browser drains the WebRTC jitter buffer continuously. Waiting until Record is
pressed can make Chromium consume queued startup audio at catch-up speed.

Pressing **Mic test** establishes the private return from the denoised Natural tap.
**Mic Test** starts recording explicitly. A timer, received-audio meter
and **Stop Testing** button make the active state visible. Recording stops after
30 seconds. The browser records one timestamped Opus return, preserving the
received stream's real-time playback cadence, then renders a WAV comparison from
that decoded take with the current voice-processing strength. Natural automatically
plays first, followed by the Enhanced comparison. Natural therefore
exercises the real private SFU return; the Enhanced comparison uses the same Web
Audio settings but does not make a second SFU trip. No recording is uploaded or
persisted. Testing again, ending the test, leaving, reconnecting or unmounting
discards both versions and cancels capture.
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
gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure -f git_sha=<merge-sha>
```

These are operator instructions, not commands automatically run by this change.
The later default-preset/snippet change is web-only and requires no further API
deployment when private received tests are already deployed. After its web image
build, use only the `deploy-caper-web.yml` command above with that merge SHA.

### Audio setup and speech consistency

Headphones natural input is now the default, with browser echo cancellation and
automatic gain control **off**. Use headphones: speakerphone echo protection is
not enabled, and no selector remains to enable it. This preserves the preset Jose
preferred rather than stacking automatic gain control. OS-level processing may still apply.
It does not repair hardware-clipped input or guarantee clean speech.

Voice processing defaults to 25% after noise suppression and is adjustable from
0–100%. At 100%, it uses Web Audio nodes on-device: a 75 Hz high-pass, restrained
warmth and presence EQ, 3:1 compression, 1.35× makeup gain and final peak limiting.
High-pass frequency, EQ gain, compression ratio and logarithmic makeup gain scale
with the selected strength; peak limiting remains fixed protection. Zero bypasses
the entire post-processing chain, while DPDFNet noise cleanup remains active at
every setting. This is fixed dynamics processing, not AGC: it does not continually
raise gain during silence. The microphone test records one returned Natural take
and creates the selected Enhanced comparison from that same recording. The setting
remains active after leaving the test for the page lifetime. Processing cannot repair
clipping that occurred before browser capture.

The voice-level control remains 0–200%; 100% is unity input gain. Device selectors
show the browser's current default hardware by name, omit the synthetic “System
default” row, and collapse duplicate default/device labels.

## On-device noise suppression

DPDFNet-8 48 kHz HR is the default microphone mode: capture (browser AEC, AGC and
noise suppression off) → input gain → 48 kHz mono DPDFNet → adjustable voice
processing → MediaStream output track → existing WebRTC Opus sender → Cloudflare
SFU. Runtime performance limitations remain.
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
- The private return keeps a muted media element attached from track arrival
  through teardown, and recording retains its own sink as a safeguard: Chromium
  otherwise leaves its WebRTC jitter buffer undrained, which can produce empty
  audio or accelerated catch-up at the start of a recording.

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
  and Safari remain unvalidated.

Future native clients are separate implementations, not wrappers around this
browser adapter. Native capture, echo cancellation, and feeding processed PCM
into a native WebRTC sender remain roadmap work; no native client currently ships
from this repository.

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
mobile background behavior, Firefox/Safari, and future native clients remain
separate checks.

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

## Accounts

The `/live` demo remains public. For guests, the browser uses
`unique-names-generator` to assign a readable color-and-animal name for each visit
and keeps it through reconnects. No account, profile, or database is needed for
guest voice. The API accepts guest names of 1–64 Unicode characters after trimming,
with no control characters. Guest names are unverified, nonunique, and not reserved.
When a valid account bearer token or browser session cookie accompanies a join, the
API uses the stored display name instead of the submitted name. Country flags use
the API-provided Cloudflare country code; no flag is invented when location is
unavailable. The web login page sends email codes and then requires a unique
username and display name. Account login is enabled when `AUTH_SECRET`, the database
URLs, and SES settings are configured. Desktop/native account screens remain future work.
Each code is six uppercase characters from `ABCDEFGHJKMNPQRSTWXYZ23456789`,
excluding visually ambiguous `0/O`, `1/I/L`, and `U/V`. Codes expire after 10
minutes and have three attempts by default. A replacement code consumes the prior
active code for that email. Request limits default to 3/email/15 minutes,
5/email/day, 10/IP/hour, and 500 globally/hour; all are configurable through the
application secret.

### Notifications webhook

Set the server-only `NOTIFICATIONS_WEBHOOK_URL` property in the existing AWS
Secrets Manager JSON record (`production/apps/caper`; staging uses
`staging/apps/caper`). The existing `caper-api-account` ExternalSecret extracts
that record into the API Pod, so no new Kubernetes Secret or manifest is needed.
Preserve every existing JSON property when updating the record, wait for
`caper-api-account` to become Ready, then deploy the API image (or restart the
API after its normal five-minute ExternalSecret refresh). Never put this URL in
web configuration or a `VITE_` variable.

The notification API accepts typed events so future application notifications can
use the same endpoint and delivery policy. After the database transaction creates
a user, Caper asynchronously POSTs this JSON body once:
`{"event":"user.created","user":{"id":"<external id>","email":"<verified email>"}}`.
Existing users who sign in again do not emit another event. The request has a
three-second deadline; invalid URLs, connection errors, timeouts, and non-2xx
responses are recorded only as bounded event/status diagnostics. No webhook URL
or payload is logged, and delivery failure cannot delay, roll back, or otherwise
prevent registration. Delivery is best effort: it is neither retried nor durable.

`MEDIA_ENABLED=true` and all four Cloudflare credentials remain required. Existing
12-participant capacity, global join and per-participant operation limits,
45-second leases, private monitor isolation, and
leave/expiry cleanup still apply.
This is an anonymous public test channel, not an abuse-resistant public launch.
Anyone can consume its limited capacity. Keep provider usage under observation
and disable media after testing if needed.

For local testing, export the server-only variables from `.env.example`, leave
database URLs unset, and run `cargo run -p caper-api` with `npm run dev:web`.
Vite proxies `/api` to the Rust service on port 3001. Production keeps its existing
Traefik routing; deploy the web and API changes together through GitOps, with
`MEDIA_ENABLED=true` in the API deployment configuration. No migration is added.
After the approved deployment, verify:

```bash
kubectl -n default rollout status deployment/caper-api --timeout=15m
kubectl -n default rollout status deployment/caper-web --timeout=15m
curl --fail --silent --show-error https://caper.chat/api/media/status
# Expected: enabled is true. Then open /live in two browsers and test audio/leave.
```

Guest restoration validation: Rust provider mocks cover unauthenticated joins,
name validation, capability enforcement and revocation; browser client tests cover
name retention on reconnect and media/SSE headers. These do not prove live SFU,
multi-network/TURN, sustained voice, physical devices, or future native support.

`apps/api/migrations/202609070001_accounts.sql` creates a provider-neutral `users`
table: bigint identity PK, unique random public ID, nullable unique email, nullable
verification/deletion timestamps, and profile fields. Verification has no default.
Usernames are lowercase
ASCII letters/digits/underscore, 3–32 characters and globally unique. Display names
are global, nonunique, 1–64 Unicode characters. Both start NULL and are completed
together. Profile responses expose neither bigint IDs nor email.

The table keeps only primary-key, required-field and uniqueness constraints.
Format, length, normalization and onboarding validation live in the application,
not SQL `CHECK` expressions.

`202609190001_email_auth.sql` adds single-use email challenges and revocable
30-day sessions. Codes expire after 10 minutes, allow five attempts, and are stored
only as HMAC-SHA-256 values. Session tokens contain 256 random bits and only their
SHA-256 hashes are stored. Request limits are enforced in PostgreSQL across API
replicas: three sends per address per 15 minutes, ten per address per day, twenty
per keyed IP hash per hour, and a 500-email global hourly budget. Throttled requests
return an indistinguishable synthetic challenge ID and do not call SES.

External-provider middleware, callbacks, token verification, key fetching, session
hooks, and browser forwarding remain removed. Server functions retain CSRF middleware.
The production web service owns no `/api` routes; Traefik sends same-origin
`/api/*` requests directly to Rust.

### Public API: web and future native clients

The public browser API is **`https://caper.chat/api/*`**, routed by Traefik
directly to the existing Rust `caper-api` service. `https://caper.chat/*` serves
the website through `caper-web`. `https://api.caper.chat` remains a direct API
alias. No second Rust backend or native-specific gateway exists.

Media status and guest join are public;
all subsequent media requests require the issued `x-caper-media-token` capability
header (including SSE and private monitor joins). Tokens stay in browser memory,
not URLs, cookies, or persistent storage. Possession authorizes that call session.

| Method | Public endpoint | Purpose |
| --- | --- | --- |
| GET | `/health`, `/api/health` | Unauthenticated health checks |
| POST | `/api/auth/email/request` | Request a 6-digit, 10-minute email code; returns a challenge ID |
| POST | `/api/auth/email/verify` | Consume a challenge and create a 30-day session |
| POST | `/api/auth/logout` | Revoke the current session |
| GET | `/api/account/me` | Read the authenticated public account |
| POST | `/api/account/profile` | Set a unique username and display name |
| GET | `/api/media/status` | Public availability flag |
| POST | `/api/media/join` | Public guest join with `{ "name": "Guest name" }`; issues a call capability |
| GET | `/api/media/events` | Capability-protected roster events |
| POST | `/api/media/snapshot`, `/publish`, `/subscribe`, `/negotiate`, `/close`, `/state`, `/leave` | Capability-protected operations; all paths under `/api/media` |

Web verification defaults to an `HttpOnly`, `Secure`, `SameSite=Lax` cookie.
Future native desktop and mobile clients will request `tokenTransport: "bearer"`
and store the returned opaque token in the OS credential vault (Keychain/Keystore),
never plain preferences or web storage. Protected routes accept either transport. Logout
revokes the same database session for every client type. Guest media capabilities
are not account credentials.

Native account screens and secure-vault integration are not implemented yet. The
web account flow uses the secure same-origin cookie transport.
Arbitrary third-party browser CORS access, API keys and developer OAuth consent
are not implemented; native/server HTTP clients do not require CORS. Browser WebRTC
does not prove native audio support.

Bigint account IDs stay internal and are never exposed in profile responses.
Random 12-character alphanumeric IDs provide permanent public references without
revealing signup order or the internal sequence. Stored as `external_id` for
integrations and external references, the API returns this value as `id`, alongside
`username` and `displayName`. Usernames are globally unique, changeable handles;
changing a username or email does not change either account ID.

### Removed account lifecycle integration

The provider lifecycle endpoint, signing secret, event receipt table, and delivery
handling have been removed. There is no lifecycle endpoint to register or test.

Lifecycle state is now limited to the generic nullable `deleted_at` column in the
initial users migration. There is no event receipt table or follow-on migration.

### Deployment and verification

No production database changes are performed by tests. When `DATABASE_URL` is set,
normal API startup first applies embedded migrations through `MIGRATION_DATABASE_URL`,
closes that connection, then opens the runtime pool and starts HTTP/media. Missing
or failed migrations prevent startup. No Job or manual migration step is required.
Both secrets must be available to the API pod; neither belongs in WEB. This means
the long-running API environment holds schema-changing credentials, an accepted
early-stage tradeoff. SQLx advisory locks serialize concurrent startup migrations.
Use direct port 5432: those locks require session affinity; pooled port 6432 is
rejected for migrations with no runtime-URL fallback. Both URLs must target the same
existing `/caperchat` database. Startup does not create the database or roles. It
connects with `DATABASE_URL` to identify the actual runtime role, then the migration
connection grants that role only the application table and sequence access it needs.
The direct migration connection explicitly sets `search_path=public`, so a schema
named for the migration role or a database-level custom search path cannot redirect
new tables or SQLx's ledger. This startup override is not applied to the runtime
pool because PlanetScale PgBouncer rejects search-path startup overrides. The
profile update explicitly targets `public.users`. Manual `SELECT * FROM users`
works when `public` is on the search path with no other `users` table ahead of it:
PostgreSQL's common `"$user", public` default lets a same-named user schema shadow
unqualified objects. Pinning migrations does not discover or move tables or
`_sqlx_migrations` ledgers that an earlier run created elsewhere.
If the provider-free migration is already applied in `public`, this search-path
fix requires no reset, new migration, or grant changes. The reset below is only
for the original provider removal; do not repeat it for this fix.

The initial migration was rewritten for the approved empty installation. Existing
SQLx checksums will fail until the unused account schema is reset. This is not a
forward migration: stop the old API and suspend application reconciliation first,
then reset only the dedicated `caperchat` database's `public` schema, which contains
the unused users, obsolete event receipts, and migration ledger. Do not run this
against a schema with unrelated objects or user data.

From the infrastructure checkout with the production kube context selected:

```bash
flux suspend kustomization production-apps -n flux-system
kubectl -n default scale deployment/caper-api --replicas=0
kubectl -n default wait --for=delete pod -l app.kubernetes.io/name=caper-api --timeout=5m
```

From this checkout, securely export `DATABASE_URL` (runtime role) and
`MIGRATION_DATABASE_URL` (schema owner, direct port 5432) for `caperchat`:

```bash
psql "$MIGRATION_DATABASE_URL" -v ON_ERROR_STOP=1 <<'SQL'
DO $$ BEGIN
  IF current_database() <> 'caperchat' THEN
    RAISE EXCEPTION 'Expected caperchat database';
  END IF;
END $$;
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
SQL
cargo run --locked --release -p caper-api -- --migrate
```

Do not recreate the database, other schemas, roles, or cluster. Verify the new
API image pin in infrastructure `main` before resuming reconciliation; never restart the old image against
the reset schema. Coordinate with Captures' separate schema reset so reconciliation
is resumed only after both APIs are ready to use the new schema. From infrastructure:

```bash
flux resume kustomization production-apps -n flux-system
flux reconcile kustomization production-apps -n flux-system --with-source
kubectl -n default rollout status deployment/caper-api --timeout=15m
kubectl -n default exec deployment/caper-web -- node -e \
  'fetch("http://caper-api:3001/api/account/me").then(r=>{console.log(r.status);if(r.status!==401)process.exit(1)})'
```

The unauthenticated account request must return 401. These are operator instructions
only; this change performs no production writes or SES sends.

An optional operator command remains available to migrate and apply runtime grants
without starting HTTP. With `DATABASE_URL` set it identifies and grants the runtime
role; without that variable it applies only migrations:

```bash
cargo run --locked --release -p caper-api -- --migrate
# Or with the built binary / API container entrypoint:
caper-api --migrate
```

The API Docker build copies migrations. Fresh startup creates the users table;
existing installations need the one-time reset above before the new image starts.
Old images are not compatible with the rewritten schema; do not roll them back
without an explicitly reviewed schema recovery. No production reset or deployment
is performed by this PR.

Do not activate account login merely by merging this application code. Populate the
account, database, SES, and auth-limit keys in `production/apps/caper`, project them
into the API through a reviewed infrastructure change, verify SES workload identity,
then deploy and test the API.

Database tests are explicitly ignored in the ordinary no-DB Rust suite. The CI
`Account database (Postgres)` job runs them against disposable Postgres 17. To run
locally, supply a **disposable local** Postgres URL whose role can create databases:

```bash
DATABASE_URL='postgres://user@127.0.0.1:55432/postgres' \
  cargo test --locked -p caper-api --test accounts -- --ignored
```

SQLx creates a fresh migrated database per test. Never use a production URL.
Orb setup installs Postgres binaries; an isolated UTF-8 cluster can be initialized
with `/usr/lib/postgresql/15/bin/initdb -D /tmp/caper-test-pg -A trust -E UTF8` and
run with `amp orb service start account-test-db --command '/usr/lib/postgresql/15/bin/postgres -D /tmp/caper-test-pg -h 127.0.0.1 -p 55432 -k /tmp'`.
Local trust authentication is for the disposable loopback-only test server, not
deployment. Database tests cover code issuance, resend throttling, session creation,
authentication, and revocation without calling SES.
Provider-specific staging results are obsolete. PlanetScale connectivity, physical
desktop/mobile account access, and live authenticated SFU voice remain untested.
Earlier media results above predate account-backed participant display names and do
not validate that integration.
