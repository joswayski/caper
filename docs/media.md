# Channels, messaging, and voice

## Scope and architecture

The homepage shows a labelled, client-only simulated conversation. Its join
button opens email-code sign-in/sign-up; it does not connect to a real channel.
The former **Public demo / General** is retired. Account-owned spaces contain
unified text/voice channels; history, live messages, presence and calls require
membership. See [spaces and channel access](#spaces-and-channel-access).
Participants use their account display name and saved Caper avatar. This is not
an outgoing-call flow. No camera calls, screen sharing, or server-side voice
recording.

### iPhone Duo layout

Apple's [preparation guide](https://developer.apple.com/iphone-duo/prepare/)
requires Xcode 27.1 / the iOS 27.1 SDK for the edge-to-edge Duo experience; the
TestFlight job runs on the owner's existing `josemac-sim` VM and selects
Xcode 27.1 with a job-local `DEVELOPER_DIR`. The
display sizes in Apple's specifications are physical pixels, not app viewports:
folding, rotation, Split View, keyboard and asymmetric safe areas change the
space available. Never derive breakpoints from them or identify the phone by its
dimensions.

The Apple client keeps Browse and the conversation in one stable `AnyLayout`
hierarchy instead of recreating the conversation when switching between compact,
sidebar and member-panel arrangements. The 760-point compact threshold is a
content breakpoint, not a Duo profile. Members opens automatically only when it
fits beside the conversation. Only backgrounds ignore container safe areas.

iPhone Duo reserves a trailing column for its island and status that the leading
edge does not mirror. The surface beside the column (conversation, thread or
tiled member list) and its header rules run under it; text stays out of it. The
conversation's Pins, Members and (1:1 DM) options sit in the column as a
vertical capsule level with the composer. With Browse open, the conversation and
its column controls slide away, leaving the column blackout. Regular iPhones in
landscape inset both sides equally. This is Apple-only because only Apple's
hardware reserves such a column. Duo support is not yet accepted on hardware.

### Shared client behavior

Channels and direct messages occupy independently scrolling sidebar panes on web,
Android, iOS/macOS and Rust desktop. Drag their horizontal divider to redistribute
the height; both remain bounded above the account controls. The split defaults
to half, retains its saved proportion across launches, and reserves a useful
minimum for each pane in short windows. Mouse pointers use the platform's
vertical-resize cursor. Up/Down adjusts the focused divider, Home/End reaches its
bounds, and double-click/tap resets it. Touch resize gestures do not start a
Browse swipe. The labelled homepage simulation keeps its unsplit layout.

Web's outer room edges keep resize cursors and keyboard controls without a
floating icon or tooltip. macOS attaches resize pointers directly to the SwiftUI
dividers (with a cursor-stack fallback on macOS 14). The wide Apple member list
has a draggable leading edge with a saved 220–440-point width, keyboard bounds
and reset; window constraints keep room for the conversation.

Mobile channel, DM and thread timelines dismiss the software keyboard when a
vertical scroll drag starts. The unsent draft stays in its composer; tapping it
reopens the keyboard. Automatic following/paging and scrolling within a multiline
draft do not dismiss it. Web waits past touch slop and leaves horizontal
navigation gestures to Browse/Members; iOS uses native immediate scroll
dismissal and Android uses the list's drag interactions.

The narrow iOS workspace sizes its sliding layers from the outer, keyboard-aware
`GeometryReader`; those layers ignore additional keyboard safe-area adjustments.
Apple Browse navigation records its horizontal motion in `DrawerPose` without
also applying a SwiftUI animation to the workspace, so the stationary sidebar's
scroll position and divider do not drift during slides.

#### Emoji and mention autocomplete

Composers offer `:` emoji autocomplete on every client, using the same bundled
Emoji 15 catalog. A colon at the start of a draft, after whitespace or after an
opening bracket activates suggestions; URLs, times, selected text and active IME
composition do not. A single character after the colon (`:D`, `:P`, `:3`) is an
emoticon: it opens no suggestions, so Enter sends it as typed. The bare `:`
offers the default emoji. Selection inserts Unicode at the caret and respects
the 4,000-scalar limit; emoji presentation selectors are preserved separately
from normalized artwork IDs. Enter/Tab accepts, arrows move, Escape closes;
Shift+Enter keeps newline behavior. iPhone users select a row and send with
Send; Android's IME Send accepts an active suggestion first. Custom emoji uploads
are not implemented.

Composers suggest people after `@`. An `@` at the start of a draft, after
whitespace or after an opening bracket starts a name of up to 32 ASCII letters,
digits or underscores; `bob@maya.com` and `x/@maya` do not. Space channels
suggest the open space's loaded members except you, then `@everyone` ("Everyone
in this channel") and `@here` ("Everyone online in this channel"). DMs suggest
everyone you share a space or DM with, from authenticated `GET /api/people`
(`{"people":[{"id","username","displayName","avatarId"}]}`, excluding you, at
most 500), fetched when a DM opens; until it loads the DM peer is offered. Rows
show avatar, display name and `@username`. Selecting inserts `@username `
without sending; keys and limits match emoji. Anyone who can post may use
`@everyone`/`@here`. Mention suggestions span the composer width on every client.

On send, `chat::persist` resolves names inside the message transaction (grammar
in `apps/api/src/mentions.rs`) to any non-deleted account with that username, in
channels and DMs alike. `content.mentions` lists resolved users
(`{"type":"user","id","username"}`) and, in space channels only,
`{"type":"everyone"}` / `{"type":"here"}`, in first-appearance order with at most
20 distinct names. Unresolved names stay plain text; the field is omitted when
empty. `content.version` stays `1`; history, live events and replay carry the
same payload. Clients find tokens with the same grammar (native clients:
`shared/messages/mentions.rs`; every client runs `shared/messages/mention-cases.json`),
ignore unknown entry types, render resolved tokens as terracotta pills, and tint messages that mention the reader (their id, or
`@everyone`/`@here` from someone else). `everyone` and `here` are reserved
usernames. Tagging is not access: notifications go only to people who can read
the conversation. Mention ids are the 12-character account `external_id`, never
the internal sequence key. Mentions drive phone push kinds (`mention.user`,
`mention.everyone`); see [notifications.md](notifications.md).

Channel, DM, thread and profile quick-message composers preview complete known
`@username` tags and the Caper team's aliases (including `@feedback`) with the
same terracotta background before sending; `@everyone`/`@here` preview only in
space channels. This is a local preview from the composer's loaded directory;
the server still resolves mentions on send and drafts stay plain text.

#### Profile cards

Clicking or tapping a person's pill opens a small profile card (`@everyone`/`@here`
pills stay inert): avatar, display name, `@username`, and an inline DM composer
with text, emoji and file attachments. Typing stays local; choosing or pasting
files resolves the DM and starts uploading immediately. **Send** waits for
remaining uploads and opens the conversation only after confirmed delivery.
Unsent uploads keep the normal 24-hour cleanup; stale uploads are renewed before
a new send without changing an uncertain retry's attachment IDs. **Open
conversation** opens a DM without sending the draft; incoming requests still
require explicit acceptance. Message-author avatars/names (channels, DMs,
threads, Pins) and member-list rows open the same card; guest authors stay inert.
Your own card says "You" and has no composer. Card data comes only from what the
client already loaded: space members, then `GET /api/people`, then DM peers.
Authors absent from that directory keep their message name and avatar without a
guessed username or messaging action. Wide layouts show a popover by the clicked
identity; web narrow/touch, Android and iPhone show a bottom sheet. DM errors stay
inline in the card.

#### Composer sizing

Apple composer frames use their measured content height rather than filling the
174-point maximum: channel/DM drafts start at 42 points; thread replies keep a
72-point minimum. Longer drafts grow and then scroll. iOS and macOS keep a
12-point bottom margin below composer controls, matching the other clients.
Conversation and thread backgrounds—not their controls—ignore the safe area,
filling the rounded keyboard corners with the chat color. Apple pending sends use
the confirmed row's full-width, leading-aligned layout and scroll into view
without yielding another task first.

#### Voice in the sidebar

Voice lives in the sidebar's channel list on every client. Channels stay text and
voice together: opening a channel never joins or switches voice, and joining
never changes the conversation you are reading. A live channel's own row grows
into a card in its place in the list, so no channel appears twice. Pressing a
card anywhere except its controls and `⋯` menu opens that channel's chat.

- Your call's card is green: the row line (name, unread, muted, `⋯`, selected
  tint) ends with "You're in · N" (Connecting… or Reconnecting… in amber) and
  the shared session timer. Faces follow, you first and then in join order
  (speaking never reorders them), with a green speaking ring and mute/deafen
  badges, in at most two rows: up to ten faces, or nine and a "+N" tile. Then
  Mute, Deafen and Leave (Cancel while joining). The card never changes size
  with speech. Right-click, long press or the accessibility action on the faces
  opens the full list ("In #channel · N people") with per-person volume and Mute
  for me, beside the sidebar on wide layouts and as a bottom sheet on phones; a
  face opens with that person's controls expanded. On web the context-menu key
  opens it too. Escape, an outside press and Close dismiss it.
- Another live room keeps its row (waveform and count) and gains a body: up to
  three faces, a one-line summary of names, the session timer, and Join
  (eggshell) or Switch (neutral) when you are in a call elsewhere.
- Voice without a row in the listed space sits above the Channels heading: your
  call in another space (its card names the space) or in a channel opened from
  the feedback inbox, and such a visited channel's live room.
- Collapsing Channels hides quiet rows only; live cards stay listed.

Quiet channel rows are one line. Every row except your call's keeps a one-click
headphones join ("Join voice in #x" / "Switch voice to #x"). Fine pointers reveal
it on hover or keyboard focus; touch clients and Rust desktop at 760px or
narrower show it dimmed with a 44px target on quiet rows and hide it on live
rows. The conversation header carries no voice; "N members · M in voice" stays
beneath channel names where shown. On phones, every conversation adds a 56px
strip under its header: your call (channel, up to five faces or four and "+N",
connection state, timer and a 44px Mute; the rest opens Browse), or, out of a
call, the open channel's live room with Join. Mic, deafen and device settings
stay in the account bar, including before joining. Speaking rings appear only for
your own call. Browsing other chats, spaces or DMs does not leave voice; voice
ends when its own channel is left, deleted or revoked. Leave channel lives in the
channel menu and requires confirmation.

`scripts/test-voice-controls.mjs` runs the mocked web flow (`VOICE_TEST_*=1`
modes focus on parts of it); `VOICE_TEST_PREVIEW=1` serves a simulated UI on
port 5180. It is not live SFU validation.

#### Dialogs

Browser modals use viewport-bounded, stable-height shells with scrollable
overflow; errors, pending states and loading do not resize them. Profile,
channel and space settings and new-DM dialogs dismiss on outside click/tap;
nested delete confirmation leaves settings open. Web dialogs handle Escape on
key-down; child-owned Escape (suggestions, native select) closes that layer first,
and IME composition Escape leaves the editor open. Apple's dialog close button
has the standard cancel shortcut. Desktop leave/delete-space handling clears the
departed conversation and fences late results before selecting another space.

#### Mic test and devices

Mic test offers an explicit, tab-memory-only recording of up to 30 seconds of
received Natural audio and an on-device Enhanced comparison from the same take.
Input/output device lists open beside the profile's microphone/headphone
controls; input gain lives in Settings. Output volume scales received audio
alongside each person's volume and also controls mic-test playback. Close or
Escape cancels a test, releases pre-join capture, or restores the call's previous
mute/deafen state.

#### Join and roster

Join carries the initial mute/deafen state; the browser sends a follow-up state
write only if that intent changed while Join or capture was pending. The initial
snapshot starts as soon as live updates are ready; subscription reconciliation
waits for publication, and the microphone stays silent until roster, state and
transport readiness have all succeeded. If transport returns to `connecting`
during startup negotiation, Join waits for it to reconnect within the existing
timeout.

The API replaces a signed-in participant's submitted name with the account
display name. Participant IDs, not names, distinguish people. With `VALKEY_URL`
configured, live call state is shared in Valkey; without it, development uses
process memory. Up to 12 people can join. Caper does not collect country codes
and no client renders flags; old Valkey participant records with country fields
still decode and drop them when rewritten.

Visitors see the roster before joining through a `media` subscription on the
application WebSocket. That projection includes each participant's session ID,
name, mute and deafen state, never track IDs, session tokens or audio. The
HTTP/SSE endpoints remain for older clients and inspection; current clients use
neither SSE nor polling. Spectators never renew call leases.

#### Voice session timer

Channel names show a shared voice-session timer to everyone authorized to view
the roster. It starts from the first accepted participant's **Join click**,
survives that participant leaving while others remain, and clears when the last
participant leaves or their lease expires. Mic-test monitors and
preparation/warm sessions never start timers. Snapshots carry nullable
`sessionStartedAt` (UTC Unix ms); clients render `MM:SS`, then `H:MM:SS`. Clients
show a local provisional timer during a pending join, keep an occupied channel's
existing start, and adopt the shared timestamp on admission; cancelled or failed
joins clear only provisional time. Join requests carry optional `joinStartedAt`
(UTC Unix ms); the API bounds it to the 30 seconds before receipt and clamps
future values to receipt time. Valkey stores the start with room metadata.
Elapsed time uses device clocks, so skew can offset the display; future starts
clamp to `00:00`. Server roles that predate `joinStartedAt` reject unknown join
fields, so keep server support while clients send it.

#### Media control path

Browser → same-origin `/api/chat/events` WebSocket → Rust gateway → authorized
media handlers → Cloudflare control API. Fifteen-second `media.snapshot`
commands renew call leases; pushed snapshots discover roster/track changes.
Clients ↔ Cloudflare Realtime SFU/TURN carries audio. No audio relays through the
gateway, AWS, Workers, Durable Objects, RealtimeKit or PlanetScale.

Keep the API at one replica until every pod uses the same Valkey endpoint and
schema; see [shared call state](#shared-call-state-and-rolling-deployments). Each
tab receives an unguessable short-lived call capability. Clients use Caper track
IDs, not arbitrary SFU session IDs. Cloudflare terminates transport encryption;
this is **not E2EE**.

### Channel navigation

Clicking the displayed channel is a no-op; clicking it while another channel
opens cancels the pending transition. Repeated clicks on one pending destination
do not duplicate navigation. Hover/focus on desktop web, macOS and Rust desktop
speculates read-only history, never a sending session, read receipt, request
acceptance or voice join. Visited timelines resume from their retained replay
cursor; Apple and Rust desktop clicks recheck space access before using
speculative history. Channel switches keep the displayed title and messages
until the target is ready; only a first load without a ready conversation shows a
skeleton. Loaded empty channels count as ready.

Android keeps up to 20 channel/DM snapshots in account-scoped memory, rejects
incomplete context windows, and clears removed/revoked conversations and
sign-out data. Apple displays prepared history while its sending session opens and
keeps the space's roster/presence subscription. Rust desktop reconnects its
combined chat/presence gateway on channel changes.

Web DMs use page-scoped prepared histories like channels. The cache keeps at most
20 visited DMs and four five-second speculative reads, and disappears when the
Spaces page unmounts. DM-only switches do not reload the selected space. Switching
away aborts a pending DM history read and the previous channel's
voice-availability request. Loading failures stay visible with Retry;
cancellation is not a UI error. Native clients keep DM snapshots and replay
cursors in account-scoped navigation state, and superseded results cannot replace
a newer selection. Rail icons keep the same corners when hovered or selected.

### Application gateway and account presence

One WebSocket per tab multiplexes authorized chat, voice-roster, and member-status
subscriptions. HTTP keeps durable messages/history and space/account management.
Temporary typing and media commands use the socket but invoke the same Rust
handlers and permissions as HTTP.

- `hello {idleTimeoutSeconds,serverTime}` establishes the session. Heartbeats carry
  `activityAgeMs`; input activity is throttled. Focus, visibility, incoming audio
  and heartbeat traffic do **not** reset activity. Native clients report their own
  input activity. Sending audio packets is not treated as speech.
- `subscribe {id,kind,channelId?,after?,token?,spaceId?,userIds?,allMembers?}`
  opens one logical stream. `event {id,event}` routes updates to it;
  `subscribed {id}` follows its initial snapshot/replay. `unsubscribe {id}`
  releases interest. At most 32 subscriptions per socket and 100 explicit IDs per
  presence subscription. A presence subscription with `allMembers: true` and no
  `userIds` derives the roster from the authenticated space membership, using one
  slot. Credentials never appear in the URL. Sessions and memberships are checked
  again during use.
- Account presence is **online** if any live session has recent input, **idle**
  otherwise, and **offline** only after every session lease expires.
  `PRESENCE_IDLE_TIMEOUT_SECONDS` defaults to 600 (valid 1–86400). Each socket
  renews its own 40-second lease. The per-user hash expires within 80 seconds of
  its last renewal and has at most 32 socket records; no status history
  accumulates. A suspended phone whose heartbeats stop loses its lease.
- The space member list watches the complete space roster, with Online (including
  idle), Offline and Status unavailable sections. Private-channel readers stay
  separate from other space members. There is no UI pagination. The search icon
  opens a local name/username filter; X or Escape clears it without changing the
  subscription. Section counts describe the filtered roster; the Members count is
  the space total. Initial member metadata comes from the space detail endpoint.
  Membership changes are repaired within the ten-second authorization interval;
  removed requesters fail closed. Gateways batch expiration checks once per
  watched user per pod every two seconds and push only changed snapshots. Failed
  Valkey or Postgres checks keep the last known statuses and retry every two
  seconds; only after 30 seconds of failures does the gateway end the
  subscription with a 503 (logged as `presence_check_failed`,
  `presence_subscription_retrying`/`_failed` and `presence_broker_failed` with
  the broker's reason). Denials (401/403/404) end it immediately. Every client
  subscribes again with backoff (5 s doubling to 60 s) after a non-denial error
  or when no first snapshot arrives within 10 seconds. During an outage the UI
  shows unknown/updating rather than inventing offline status.
- Chat/typing and room invalidations use channel-specific broker topics. Each pod
  subscribes only where it has local interest. Committed chat cursors repair
  missed messages; versioned room snapshots repair mute/deafen/roster state.
  Typing expires and is not replayed.

#### Member and channel list performance

Android member rows are keyed `LazyColumn` items, Apple uses lazy stacks, Rust
desktop skips offscreen member contents, and web uses `content-visibility` with a
44px row estimate (all row DOM nodes are retained; not full virtualization).
Search does not fetch, subscribe or reconnect. Full-roster presence is
proportional to space membership, with complete snapshots on changes.

Before increasing the 100-channel-per-space quota:

1. Benchmark channel switching, fast scrolling, filtering and presence churn with
   100 channels and 100/1,000/10,000-member fixtures, including a low-end Android
   device. Record p95 input-to-paint, slow/blank frames, peak memory, rendered
   row count, and gateway bytes/queries.
2. Keep stable IDs and explicit row-height variants; virtualize web rows and use
   native lazy/recycled rows only after measurements show the bottleneck.
   Preserve keyboard focus, profile anchors and scroll position during presence
   moves.
3. If wire/DB work dominates, add server-maintained presence counts and deltas
   plus an internally windowed roster/search API. Counts stay whole-space, search
   covers unloaded members, and windows stay invisible to users.

#### Typing and media commands

Typing is best-effort: sent only over an already connected socket, never
queued/replayed after reconnect, and never stored in the command ledger. The
receiving UI expires typing after six seconds. Authorization and one-second
per-author/per-channel rate counters apply.

Media commands include a UUID and fixed `issuedAt`. A shared Valkey ledger binds
the request to its identity and body and retains the result for 150 seconds;
requests older than 120 seconds are rejected rather than executed again.
Reconnects retry the **same** command. Accepted commands continue after socket
loss. Mute/deafen also keep a monotonic intent sequence. This is bounded
retry/deduplication, **not exactly-once across a Valkey data loss or a crash
during a Cloudflare operation**; an unknown outcome is surfaced for current-state
recovery. Use a non-evicting, monitored shared broker; losing it clears temporary
leases and command receipts, not Postgres messages.

The retired one-channel-per-socket protocol (`/api/chat/events?channelId=…`)
gets `410 Gone` before any database work. Old SSE endpoints remain only for
compatibility. Compose, CI and orbs pin Valkey 9.1.2; the AWS-managed cache runs
Valkey major version 9. Staging has no hosted app cluster: it uses local Compose
(`compose.staging.yaml`) with staging AWS secrets and SES. Production gateway
rollout is described in the
[infrastructure runbook](https://github.com/joswayski/infrastructure/blob/main/docs/caper-chat-gateway.md#merge-and-production-rollout-order).

### Homepage simulation

The public demo was retired. `LiveWindow.tsx` renders a local scripted
conversation with fictional participants, typing, visual speaking indicators and
a bundled AI-generated capybara meme (`public/images/demo-tiny-hat.webp`). It has
an accessible simulation label. Its profile and mic/headphone/settings icons are
passive illustration. It plays no audio and requests no chat/media capability or
gateway subscription. Reduced-motion visitors get a static illustration. The
centered join CTA (to `/spaces`; signed-out visitors reach email-code sign-in) is
its only action; channels, roster, composer and reactions are read-only. Seven of
the 24 scripted posts receive one to three delayed reactions. Messages stay in
local history for one earlier loop; older loops are dropped and the clock pauses
off screen or in a hidden tab. Avatars (`public/images/demo-avatars.webp`) were
generated from `caper-face.svg`; the bundled Twemoji 15.0 SVGs carry their CC BY
4.0 attribution in `images/demo-emoji`. Contact actions link to X and copy the
email address, showing the address when clipboard access is denied.

The API does not seed General or expose `/api/chat/general` or unscoped
`/api/media/*` (both 404). Chat session creation requires an account. Shared
authorization rejects every demo-space channel for history, sends, typing and
subscriptions, even with an explicit membership. Demo data is retained.
`CHAT_ENABLED` and `MEDIA_ENABLED` control account features. `/spaces` sends
signed-out visitors to `/login`.

### Server roles

The same Rust image has two independently deployable roles:

- `caper-api`: HTTP commands/history plus transactional-outbox publishing (3001).
- `caper-api --gateway`: application WebSocket sessions and media/typing commands
  (3002, `CHAT_GATEWAY_BIND`). It uses the runtime `DATABASE_URL`, never
  `MIGRATION_DATABASE_URL` or email-login signing credentials. It requires
  `MEDIA_ENABLED`, the four Cloudflare media settings and the same `VALKEY_URL`
  as the API. `/health` is liveness; `/readyz` requires DB/broker readiness and
  no drain.

Both need Postgres, the **same** authenticated TLS `VALKEY_URL`, and
`CHAT_ENABLED=true`. The flag defaults off outside Compose; Compose defaults it on
when no override exists. Secrets Manager overrides process environment. API
startup runs migrations and runtime grants. Start the API before the gateway; the
gateway retries startup in Compose if migrations are not ready.

IDs are random alphanumeric: **12 characters** for spaces/channels/guest
identities, **15** for messages, with unique constraints. Internal joins use
bigint. IDs contain no timestamp. Per-channel decimal-string `seq` values are
reconnect cursors; browser code uses BigInt. The channel row lock allocates
positions in commit order. There is no cross-channel ordering.

### Message HTTP contract

Same origin, no cache:

- `POST /api/chat/session {name}`: opaque sender token and public author. Login
  and a completed profile are required (403 `complete profile required`); the
  account's display name is always the author name and `name` is ignored. Tokens
  are stored hashed and expire after 30 days, and stop working after parent
  logout/expiry or if the account has no username and display name.
- `POST /api/chat/channels/{id}/messages {clientMessageId,text}` with
  `X-Caper-Chat-Token`: returns the committed message. Clients retry the same
  UUID and original text after ambiguous errors. A reused key with different
  text or sender returns conflict.
- `GET /api/chat/channels/{id}/messages?before={seq}`: earlier history, up to
  50 messages. Requires membership and any private-channel grant.
- `POST /api/chat/channels/{id}/typing {typing:boolean}` with
  `X-Caper-Chat-Token`: same permissions as sending; 204 after best-effort
  publication, 429 when throttled. No draft text is accepted or broadcast.

Messages preserve literal Unicode text (up to 4,000 code points, nonblank;
control characters other than newline/tab are rejected). Content is versioned
`{version:1,type:"text",text}`. `prepare_text` is the pre-publication boundary for
any future replacement rules: transform once before persistence and broadcast.
None is enabled; such a feature must keep the submitted original in restricted
audit storage. Stable message IDs and channel sequences support same-ID edits
(see [message editing](#message-editing)). Message bodies, chat capabilities and
account credentials are never logged. Messages and author snapshots stay in
Postgres with no automatic retention purge.

Browser tokens use local storage. Each channel and DM keeps its own unsent draft
(text, staged files and thread replies) in memory while the app stays open, on
every client: switching away and back restores it, sending clears it, signing out
discards every draft, and restoring one sends no typing indicator. Drafts are
never sent to the API and do not survive a reload. Signed-in startup obtains a
fresh capability from the current account session.

History opens at the latest 50 messages. Scrolling to the oldest loaded rows
requests the next 50; the history button stays as a keyboard-accessible fallback
and becomes Retry after a failure. Web uses React Virtuoso for variable-height
rows; Rust desktop lays out only rows within 600 points of the viewport
(`src/virtual_list.rs`), caches measured heights by message and layout revision,
and keeps the first visible row's place on prepends. Prepending history preserves
the reading position; live arrivals follow the bottom only when the reader is
there. Reloading starts again at the latest page.

Clients show an optimistic message (no `Sending…` label) and clear the composer,
keeping one outstanding command. HTTP, ordered WebSocket replay, or a history
resync replaces that row by matching client UUID and sender. Provisional rows
never advance the replay cursor. Ambiguous failures keep the exact command for
retry; definitive rejections keep the text for Edit/Dismiss. A late HTTP failure
cannot undo WebSocket confirmation, and confirmation aborts the redundant POST.

Typing uses channel-scoped `caper:chat:v1:typing:{channelId}` Pub/Sub topics and
never enters the outbox, history or sequence. Each chat subscription rechecks the
sign-in session and channel access before every durable event and every ten
seconds while idle; typing and upload progress reuse a check from the last
second. Losing access ends the subscription with an `error` frame. Typing is
limited across replicas to four per author/channel per second and 120 per
channel per second. Clients pulse at most every 500ms while editing, stop on
clear/blur/send or 500ms of inactivity, and expire peer indicators after six
seconds. Broker microsecond `revision` strings deduplicate overlapping events; a
brief stop tombstone prevents delayed starts from reviving an indicator. The
browser tracks at most 64 authors and never shows its own identity. Typing errors
never block sending.

### Delivery and recovery

1. One transaction writes message, channel position and full event payload.
   HTTP success means **durably accepted**, not delivered.
2. The API wakes its publisher immediately; a one-second pending-row scan repairs
   a lost wakeup. `FOR UPDATE SKIP LOCKED` claims disjoint outbox batches across
   replicas. Committed per-channel sequences are the ordering authority.
3. The publisher sends the full event on Valkey Pub/Sub, then marks it published.
   A crash in between may republish; sequence deduplication prevents a duplicate.
4. `GET /api/chat/events` upgrades to the application WebSocket. The client sends
   `subscribe {id,kind:"chat",channelId,after}`. Each subscription buffers before
   reading the committed high-water mark, replays through it, emits
   `ready {cursor}`, then merges live events in order. Sends use HTTP.
5. One DB head check per gateway every two seconds repairs missed events even if
   the final Pub/Sub event was lost. A broker outage does not erase messages.

Limits: 30 new messages/account/minute in a channel (shared by that account's
sessions), 120/channel/minute, and 60 new sender sessions/minute per account.
Signed-in clients reuse one sender session per account and mint a new one only
after a 401 or a profile rename; a 403 never replaces it. Anonymous gateway
upgrades get 401 before taking a slot. `GATEWAY_MAX_CONNECTIONS` defaults to 4096
per pod (an admission limit, not measured capacity). Each channel has a 256-event
ring; a lagging receiver replays from Postgres in batches of 128. Sockets use
4 KiB read buffers, a 256-frame outgoing queue and a 256 KiB write-buffer cap;
writes time out after three seconds; replay is capped at 2,000 events. Impossible
cursors or missing retained events get `resync_required`, and the UI reloads
recent history. Heartbeats run every ten seconds with a 30-second deadline.

Gateway handoff: keep two ready gateway replicas, maxUnavailable 0, maxSurge 1,
five-second preStop and 65-second termination grace. SIGTERM rejects new
upgrades/readiness and sends `migrating`; old sockets keep delivering for 20
seconds, and the runtime stays alive at least 36 seconds to finish commands
bounded to 35 seconds. Clients open a replacement and close the old socket only
after the replacement is ready and caught up. TCP sockets do not move between pods.

Local development without Compose (point URLs at **disposable local data**):

```bash
# Shell 1: set DATABASE_URL, MIGRATION_DATABASE_URL, VALKEY_URL, CHAT_ENABLED=true.
cargo run -p caper-api
# Shell 2: same runtime DB/broker and feature flag; no migration credentials needed.
cargo run -p caper-api -- --gateway
# Shell 3: Vite proxies HTTP to 3001 and /api/chat/events upgrades to 3002.
npm run dev:web
```

The durable-delivery integration test needs `CHAT_TEST_DATABASE_URL` (disposable
loopback Postgres whose role can create databases) and `CHAT_TEST_VALKEY_URL`:

```bash
cargo test -p caper-api chat::tests::durable_guest_delivery_replay_and_handoff -- --ignored
```

CI runs it against cluster-mode Valkey, which enforces the multi-key Lua
restrictions: typing rate counters share a per-channel
`{caper:chat:v1:typing:channelId}` hash tag. The target is p95 below 250ms and p99
below one second commit-to-client, including routine rolls; it is not measured.

## Spaces and channel access

Accounts with completed profiles can create spaces and unified text/voice
channels. The creator owns the space and manages its name, channels and members.
New spaces start with `general`. Public channels are visible to all members of
that space, not to anonymous visitors or unrelated accounts. Private channels are
visible to their explicitly selected space members and the owner.

Owners invite existing accounts by exact username and can remove members or
cancel pending invitations. Every client confirms before removing a space member
("Remove {name}?") or a private-channel member ("Remove {name} from #channel?");
cancelling an invitation stays one click. Invitees must accept before becoming
members. Owners can also share [invite links](#invite-links). There are no custom
roles, ownership transfers or public space discovery.

Username fields suggest people the client already has. The space invite field
offers `GET /api/people` (people you share an active space or visible DM with)
minus current members. The private-channel grant field offers the space's
members minus the owner and current grantees. Both also list people with a
pending invitation, their `@username` line ending "· Already invited"; inviting
them again is still refused. **New direct message** offers `GET /api/people`. Each
list is refreshed when its dialog opens and uses the `@` ranking (exact username,
username prefix, display-name word prefix, then username substring; at most six,
nothing until you type). Choosing a row fills in the exact username and never
submits; Up/Down, Enter and Escape work, and Escape closes the list, not the
dialog. In **New direct message**, an exact known recipient enables **Open
conversation**; other valid usernames offer **Find person**. `POST
/api/dms/lookup` with `{"username":"…"}` returns a person without creating a
conversation or accepting an incoming request; the user confirms before opening,
and editing discards that confirmation. Lookup shares the per-account
exact-username rate limit with DM creation; creation still enforces blocks and
privacy.

Non-owner members can leave a space. Removing a space member also removes their
private-channel grants. The owner cannot be removed. Removal is a soft delete:
memberships, private-channel grants and channel joins get `deleted_at`.
Rejoining inserts a new row; it does not restore old private-channel grants.

Counts include active resources only. Defaults are 20 owned spaces per account
(`SPACE_OWNED_LIMIT`), 100 total memberships including owned spaces
(`SPACE_MEMBERSHIP_LIMIT`), and 100 channels per space (`SPACE_CHANNEL_LIMIT`).
All accept positive integers from the API environment or the Secrets Manager
record (which takes precedence); invalid values fail startup. Restart all API
replicas with the same settings after changing them; clients read current limits
from `/api/spaces`. Lowering a limit blocks new creates/additions but removes
nothing. Quota checks and creation run under database locks.

Space and channel external IDs are 12-character random alphanumeric strings with
uniqueness constraints. Names are not authorization tokens. Space names contain
1–80 Unicode characters after trimming (see "Name characters" below). Channel
names contain 1–80 characters matching `[a-z]+(-[a-z]+)*` and are unique among
active channels of a space. Renaming does not change IDs. Deletion is a soft
delete that removes access and frees quota; there is no restore UI.

The account APIs live at `/api/spaces`, `/api/spaces/{space}`, and their
`/channels`, `/members`, `/invitations`, `/invitation`, and
`/channels/{channel}/members` subresources, with browser cookies or account
bearer authentication. Only owners manage resources, except a member removing
their own membership or an invitee accepting/declining. Text history and commands
use `/api/chat/channels/{channel}/...`; subscriptions use `/api/chat/events`. Both
check membership and channel visibility, including replay and live delivery.

#### Name characters

Display names, space names and guest names refuse characters that can make a
name look empty, hide text or reorder it: control characters, the Unicode
`Default_Ignorable_Code_Point` set (bidi controls, LRM/RLM/ALM, zero-width space,
word joiner, invisible operators, BOM, soft hyphen, combining grapheme joiner,
Hangul fillers, tag characters, Mongolian vowel separator) and U+2800 BRAILLE
PATTERN BLANK. Allowed: a single ZWJ/ZWNJ between two visible characters, one
variation selector right after a visible character, and tag characters only
inside a subdivision flag (U+1F3F4 … U+E007F). The rule is
`apps/api/src/names.rs`, mirrored for inline errors by `apps/web/src/names.ts`
and the native clients' `apps/native/client/src/names.rs` (Android and Apple call
it through `apps/native/mobile`). Each runs `shared/names/cases.json`. Space and
channel names follow web's `spaceNameError`, `channelNameError` and
`normalizeChannelName` (`apps/web/src/spaces/client.ts`) on every client through
the same file, with JavaScript's white space for the trim and for dashes; web
generates `shared/names/channel-cases.json` and every client runs it. Only
new writes are checked: a stored display name stays savable while unchanged; a
space rename must use a valid name. Attachment file names have these characters
removed instead of failing (control characters still fail). Message text is not
checked by this rule.

### Invite links

Owners create Discord-style links at `/invite/<code>`. Codes are 10 random
alphanumeric characters (about 59 bits), so the unauthenticated preview cannot be
enumerated. **Invite people** opens an invitation-only dialog on every client;
renaming, icons, member removal and deletion stay in **Manage space**. Opening it
prepares a ready-to-copy link for **100 total joins within 30 days of creation**,
landing in the oldest public channel (no preference for `general`). Under the
space lock, the server reuses the newest link with those settings and that
landing channel only if it is still live, so reopening does not reset its use
count or expiry. With no public channels, space-only invitations remain usable.
No QR code is generated.

**More invite options** sits beside Copy. **Regenerate link** revokes the
displayed code and creates a new one for the same public channel, with 30 days
and 100 joins, in one transaction. If the old destination became private or was
deleted, regeneration and joining fall back to the oldest public channel.
Regeneration and deactivation require confirmation on every client; failures
leave the previous link intact, and simultaneous regenerations mint only one
replacement (the stale request returns 404). Deactivating stops new joins, not
existing memberships. The UI shows remaining joins and remaining time.

**Custom links** creates separate links. Owners type the expiry in minutes, hours
or days (the API accepts `maxAge` from 60 to 31,536,000 seconds) or choose Never,
and optionally max uses (1 to 10,000; empty is unlimited). Forms start at 30 days
and 100 joins; the API treats omitted/null limits as unlimited. A link can land
joiners in a chosen public channel; one later deleted or made private falls back
to the starter channel and is no longer named in previews. Links are kept in
`space_invite_links` (`revoked_at`, never deleted); each join is recorded in
`space_invite_link_uses`.

| Operation | Route | Result |
| --- | --- | --- |
| Owner lists live links | `GET /api/spaces/{space}/invite-links` | `{links: [...]}` |
| Owner prepares/reuses a link | `POST /api/spaces/{space}/invite-links/default?channelId=...` (no body; omit query for oldest public channel) | 200 link |
| Owner regenerates the displayed link | `POST /api/spaces/{space}/invite-links/{code}/regenerate` (no body) | 201 replacement link |
| Owner creates a link | `POST /api/spaces/{space}/invite-links` with `{channelId?, maxAge?, maxUses?}` | 201 link |
| Owner revokes a link | `DELETE /api/spaces/{space}/invite-links/{code}` | 204 |
| Anyone previews | `GET /api/invites/{code}` (optional account session) | space name, landing channel, inviter, member count, expiry, `alreadyMember` |
| Signed-in account joins | `POST /api/invites/{code}/join` | `{space, channelId, joined}` |

Creating links shares the owner's invitation attempt budget below; a space can
have at most 100 live links. Reusing the default link does not consume budget.

Joining checks the membership quota under the same space lock as invitation
acceptance, consumes one use, joins the landing channel, and fulfils any pending
username invitation. Existing members get the space back without consuming a
use. Leaving does not block rejoining by link, but an owner removal or an owner's
block returns 403 until the owner invites the account by username. A removal is
recorded in `space_invitations.removed_at` and holds until an invitation is
accepted: a live re-invitation can be fulfilled through a link, but a cancelled,
declined or expired one leaves the 403. Expired, revoked and exhausted links
return 404 to outsiders; existing members can still open them as shortcuts.
Unknown links and deleted spaces return 404 for everyone. The preview checks
active membership from the session, does not join or consume a use, and is
`no-store`.

The web `/invite/<code>` page previews the link and sends signed-out visitors to
`/login?invite=<code>` (then the profile step) and back. Clients also accept a
pasted link or bare code (web: under **Create a space**), parsed by web's
`inviteCode`; the native clients run the same rule from `shared/invites/code.rs`
and every client replays `shared/invites/invite-cases.json`, generated from web.
Only the ten-character code is taken from a pasted link: previews and joins always
go to the client's configured API origin, never the link's host or scheme.
Existing members see
**You’re already a member** and **Open space**, which uses the idempotent join
endpoint; newcomers choose **Join space**.

Every native app registers the `caper://` scheme, and the invite page links to
`caper://invite/<code>` ("Open in the Caper app"). An opened link fills in the
join dialog and loads its preview, after sign-in and the profile step if needed.

- iPhone: universal links for `https://caper.chat/invite/*` through the
  `applinks:caper.chat` Associated Domains entitlement and
  `/.well-known/apple-app-site-association` (team `NAUU43LATL`, `chat.caper.ios`).
  If a TestFlight export fails on the profile, enable Associated Domains for
  `chat.caper.ios` in the Apple developer portal.
- Mac: `caper://` only; the Developer ID build has no provisioning profile.
- Android: verified App Links for `https://caper.chat/invite/` against
  `/.well-known/assetlinks.json` (`apps/web/src/server/app-links.ts`), which lists
  the upload key that signs the GitHub APK. Play re-signs store installs, so their
  links open the browser until the Play app-signing SHA-256 is added.
- Windows/Linux desktop: `caper://` is registered per user on launch (HKCU
  `Software\Classes\caper`; `~/.local/share/applications/chat.caper.links.desktop`
  plus `xdg-mime`) and by the installer/`.deb`. A link clicked while Caper runs is
  handed to the open window over loopback (a token file in the app data folder).
  Launches and invite activations maximize the window (not fullscreen); window
  geometry is not saved.

**To do (owner): finish automatic app-link opening.**

- [ ] Android Play installs: once Play App Signing exists for `chat.caper.android`,
  copy **App signing key certificate → SHA-256** from Play Console → Test and
  release → App integrity → Play app signing, add it to
  `ANDROID_CERT_FINGERPRINTS` in `apps/web/src/server/app-links.ts`, and deploy web.
- [ ] iPhone: confirm a TestFlight build with `applinks:caper.chat` signs and that
  tapping a `caper.chat/invite/...` link in Messages opens the app.

### Invitation consent and abuse limits

`GET /api/spaces` returns active `spaces`, metadata-only pending `invitations`
(space ID, name, owner ID and `inviter: {username, displayName}` from the owner's
current profile), and `limits`. Pending invitations live in `space_invitations`,
not `space_members`: they grant no space-detail, membership, channel, history,
presence, WebSocket or media access. Acceptance alone inserts membership and
checks the total-membership quota under database locks. No private-channel
grants are implicit.

| Operation | Route | Result |
| --- | --- | --- |
| Owner sends an invitation | `POST /api/spaces/{space}/members` with `{username}` | 201 with invitee member metadata; not an active member |
| Owner lists pending invitees | `GET /api/spaces/{space}/invitations` | `{members: [...]}` |
| Owner cancels an invitation | `DELETE /api/spaces/{space}/invitations/{user}` | 204 |
| Invitee accepts | `POST /api/spaces/{space}/invitation` | 200 with space metadata |
| Invitee declines | `DELETE /api/spaces/{space}/invitation` | 204 |

Usernames are lowercase ASCII letters, digits and underscores, 3–32 characters.
The API trims/lowercases and rejects invalid syntax (400 `invalid username`). An
authorized owner gets 404 `user not found`, or 409 `user already in space` /
`user already invited`. Authorization precedes account lookup. Unauthorized or
missing spaces and expired/cancelled invitations return generic 404s; clients say
“This space is no longer available”.

Invitation attempts use a durable Postgres fixed-window counter shared across
replicas: **20 per owner per 10 minutes across all spaces**, including invalid
usernames, failed lookups and duplicates; further attempts return 429. Invites
expire after **7 days**. Decline, cancel and member removal impose a **24-hour
cooldown** on the space/user pair. At most **50 live pending invites per
recipient** and **100 per space** (409 beyond). One row per pair keeps
state/cooldown. These are fixed limits, not configuration.

The browser opens an inert empty shell behind a blurred consent dialog; it never
downloads private content. Consent shows “You’re invited!”, the space name and
the inviter's display name and `@username`, with the 32px Twemoji
incoming-envelope artwork on every client (canonical PNG and CC BY 4.0
attribution in `apps/web/public/images/invitation`; the web suite checks the
Android/Apple copies byte-for-byte). Decline has initial focus. Owners see
pending invitees separately and can cancel them. Browser space lists refresh on
focus/visibility and every 15 seconds while visible; revocation removes the rail
entry and clears the selected space; an outage is not revocation. Native clients
discover invites at account load and prune spaces when navigation returns 404.

Reaction mutations require an active `channel_joins` row (including cross-channel
tokens and mutations queued behind leave); preview history reads stay available.

### Channel voice rooms

Account voice uses `/api/channels/{channel}/media/*`. Every request needs a valid
account session and channel access; in-call commands also need the room's
`x-caper-media-token`, bound to its original account session. Each room has
separate participants, tokens, sequences, join limits (12 people), tracks,
monitor sessions and cleanup work. SSE checks access again on each emission. The
expiry worker also checks sessions/membership and queues SFU track closure and
TURN revocation after access is removed; provider cleanup is asynchronous and may
retry.

In shared mode, account rooms use `caper:{general}:v1:state:channel:<external-id>`
keys. An `:active` set in the same hash slot is updated atomically with room
writes so another pod can recover cleanup. Idle rooms leave that set but keep
their small metadata record so revisions never reset under connected spectators.
One bounded connection pool and notification subscription serve the rooms;
notifications contain no channel payloads. All rooms share one hash slot; voice
state is not sharded.

### Local data and tests

Standard local Postgres and Valkey are sufficient. Exact usernames use the unique
B-tree index; there is no full-text message search. The ignored database tests
create/drop fresh databases on a disposable loopback server:

```bash
# All URLs below must point to disposable local services, never production.
CHAT_TEST_DATABASE_URL="$DATABASE_URL" \
CHAT_TEST_VALKEY_URL=redis://127.0.0.1:6379 \
TEST_VALKEY_URL=redis://127.0.0.1:6379 \
cargo test --locked -p caper-api -- --ignored --skip tests::shared
```

Channel voice tests use a mocked SFU with real SQL and Valkey. Live Cloudflare
calls, TURN across networks and devices need a separate operator smoke test.

## Uploads and attachments

Signed-in members attach up to 10 files to a message. The API reserves quota and
presigns an exact upload to private R2 staging. On completion it streams those
bytes through temporary disk, removes media metadata without recompression, and
publishes separate server-written objects. The `apps/cdn` Worker serves only
published objects after checking a signature the API issued.

### Data model

One `assets` row per logical file. `purpose` is `attachment`, `avatar` or
`space_icon` (see
[Custom profile pictures and space icons](#custom-profile-pictures-and-space-icons)).
An optional preview is a fixed derived object beside the original, not another
row:

| Object | Key |
| --- | --- |
| Original | `original/{asset id}` |
| Preview (≤ 640 px, ≤ 512 KiB, WebP/JPEG/PNG) | `preview/{asset id}` |
| Private upload sources (never a CDN route) | `staging/original/{asset id}`, `staging/preview/{asset id}` |

`message_id`/`position` link a sent file to one message; a file an edit replaced
stays linked with `retired_at` set (see
[Editing attachments after sending](#editing-attachments-after-sending)). Message
content keeps its version 1 `type:"text"` shape with an optional `attachments`
array (`id, kind, contentType, name, size, width, height, durationMs, preview`).
**Stored payloads never contain URLs.** Rows are never deleted: `deleted_at`
hides a file and `purged_at` records that its R2 objects were removed.

### Upload flow

1. `POST /api/assets` (account cookie/bearer) with channel, name, type, exact byte
   size, optional dimensions/duration and preview size. The API checks channel
   access, locks the user row, enforces quota, 30 reservations/minute and 20
   pending uploads, inserts a pending row, and returns presigned `PUT` URLs
   (15 minutes). The signature covers `content-type`, `content-length` and the
   download name in `content-disposition` (not the body).
2. The client uploads the preview, then the original, directly to R2.
3. `POST /api/assets/{id}/complete`: the API `HEAD`s each object, requires the
   exact reserved size, and for inline types checks magic bytes from a 64-byte
   ranged `GET`. It streams the full source, checks length and signature again,
   strips metadata from original and preview, and PUTs the results to distinct
   CDN keys. A mismatch or unsafe media soft-deletes the reservation (`422`);
   there is no fallback to the source. Quota and returned sizes use the cleaned
   bytes. A row lock serializes completion and cancellation. `/complete`
   responds within ten seconds with the result or a retriable `409`; slow jobs
   continue and clients poll.
4. `POST /api/chat/channels/{id}/messages` accepts `attachmentIds`. Inside the
   send transaction the API locks the sender's ready, unattached uploads for that
   channel, embeds their descriptions and links them. Text may be empty only when
   files are attached. Attachment IDs join the idempotency hash.

Attachment spoilers use optional `spoilerAttachmentIds` on the send request
(distinct IDs from `attachmentIds`). Marked descriptions store `spoiler:true`;
missing means false. The choice is part of the retry hash and survives history,
text edits, forwards and URL refreshes; the author can change it by editing the
message's files. Each composer offers a per-file toggle. Clients conceal images
and video posters with an on-device blur of a still thumbnail (the image original
when no preview exists) without starting playback; files without a visual preview
use a neutral cover. The eye-off badge explains how to reveal (click or tap).
Revealed attachments keep a labelled "Hide spoiler" action; hiding in the viewer
closes it, covers the attachment again and removes its inline player. Reveal is
local UI state; hiding one attachment leaves neighbours unchanged. Push
notifications omit spoiler thumbnails and filenames. This is presentation, not
access control. Text spoilers are not implemented.

`GET /api/assets/usage` returns `{used, limit}` (and `compression`, below).
`POST /api/assets/urls` returns fresh URLs for visible attachments for tabs open
longer than a URL's lifetime; a file an edit replaced is refreshed only for its
owner. Files of a forwarded message, and of its direct replies, are also
refreshed for readers of a channel holding a live forward.

### Limits and quota

- Per person, across every space: `ASSET_QUOTA_BYTES` (default 10 GiB) of stored
  bytes, original plus preview. Space owners are not charged for members' uploads.
- Quota counts what is **stored** (after compression and metadata removal), and
  reservations count immediately so parallel uploads cannot overshoot. Bytes stay
  counted until the purge loop deletes the original and preview; files replaced
  by an edit keep counting. Late storage writes may briefly recreate uncharged
  bytes until the reconciliation sweep removes them.
- Files up to one part (64 MiB) upload with a single `PUT`. Larger files from
  clients that send `"multipart": true` upload in parts: `create` answers
  `multipart: {partBytes, partCount}` instead of `upload`, clients ask
  `POST /api/assets/{id}/parts` (`{"parts": [numbers]}`, up to 100) for presigned
  part `PUT`s valid 15 minutes, send each byte range with no extra headers
  (retrying a part with a fresh URL), then call `complete`. The server checks
  every part size and assembles them; `upload_incomplete` (409) means parts are
  missing. Part size is 64 MiB, growing so no file needs more than 10,000 parts.
- A file can be as large as its owner's remaining storage. Photos, video and
  audio stay within 5 GiB (`media_too_large`) because they are cleaned on the API
  pod's scratch disk; clients that don't upload in parts keep the 5 GiB
  single-`PUT` limit (`file_too_large`, 413). Assembled documents and archives are
  copied inside R2. Cleaning gets 2 minutes plus a second per 20 MB, up to 15
  minutes.
- Asking for part URLs keeps an upload alive: an unfinished upload is abandoned
  an hour after its last request. Cancelling, abandoning or purging aborts the
  multipart upload; R2 also aborts incomplete uploads after seven days.

### Delivery and URL signing

Every response or socket frame that carries a message signs URLs as it leaves:
history, the send response, and the gateway's live and replay paths. URLs look
like
`https://cdn.caper.chat/assets/{id}?exp={unix}&sig={base64url HMAC-SHA256("{key}\n{exp}")}`.
The public `/assets/{id}` path maps to the `original/{id}` R2 key, which the HMAC
signs; previews are at `/preview/{id}`. The Worker also accepts `/original/{id}`
links, and both main-file paths share one edge cache entry. Expiry snaps to UTC
day boundaries for profile pictures and space icons (identical URL for a day,
lives 24–48 hours); message files snap to the hour and live one to two hours. The
Worker rejects anything signed more than three days out, serves allowlisted
image/video/audio types inline, forces every other type to download as
`application/octet-stream`, and always sends `nosniff` plus a sandboxing CSP.
PDFs open in a browser tab (`inline` with their file name and no page CSP); the
API only completes a PDF whose bytes start with `%PDF-`. Range requests are
supported. The edge cache is keyed by object, after signature verification. A
leaked URL works until it expires.

### Client compression and previews

Clients compress before upload, using settings the API serves with
`GET /api/assets/usage` (`compression`), so operators tune them with
configuration and an API restart, not a client release. When a platform cannot
meet a rule it uploads the original instead of a worse file.

- **Screenshots and graphics** stay lossless (identical pixels).
- **Photos** are re-encoded at very high quality (SSIMULACRA2 ≈ 83 on 9–14 MP
  photos); photos larger than `ASSET_IMAGE_MAX_EDGE` are resized.
- **Video** is converted to H.264 so it plays everywhere. 4K and HDR footage
  loses resolution (down to a 1440 short edge), some fine detail and its HDR
  range. The bitrate and frame-rate settings are delivered per request, so they
  could later differ by plan.

The defaults (8 Mbps at 1080p30, 1440 short-edge cap) score VMAF 77–95 on
50/60 fps HEVC test clips and ≈99.9 on Retina screen recordings; VMAF 93+ is
close to indistinguishable for most viewers.

| Setting | Default | Effect |
| --- | --- | --- |
| `ASSET_PALETTE_COLORS` | 256 | Stills with at most this many distinct colours become a **lossless** indexed PNG. 0 disables. |
| `ASSET_IMAGE_FORMAT` | avif | Photo format: `avif`, or `webp`. Clients that cannot encode AVIF use WebP. |
| `ASSET_AVIF_QUALITY` | 85 | AVIF photo quality (libavif scale). Matches WebP q92 on SSIMULACRA2 at 19% fewer bytes. |
| `ASSET_IMAGE_QUALITY` | 92 | WebP photo quality (JPEG where WebP encoding is unavailable). 100 disables lossy photo encoding in either format. |
| `ASSET_IMAGE_MAX_EDGE` | 4096 | Longest still edge; 0 keeps the original size. Never upscales. |
| `ASSET_PREVIEW_EDGE` | 640 | Preview size for images and video posters (≤ 512 KiB). |
| `ASSET_VIDEO_MAX_HEIGHT` | 1440 | Short-edge cap for videos; 0 uploads videos unchanged. |
| `ASSET_VIDEO_BITRATE_KBPS` | 8000 | Target bitrate at 1080p30, scaled by pixel count and frame rate. |
| `ASSET_VIDEO_SOURCE_BITRATE_PERCENT` | 200 | A re-encode never exceeds this percentage of a non-H.264 source's bitrate (H.264 sources: 100%). 0 removes the cap. |
| `ASSET_VIDEO_HIGH_FPS_BOOST_PERCENT` | 150 | Above 30 fps the target grows with the frame rate (fps ÷ 30) up to this percentage. 100 turns it off. |
| `ASSET_AUDIO_BITRATE_KBPS` | 128 | Audio bitrate for transcodes (AAC, or Opus where AAC encoding is unavailable). |

Rules shared by every client:

- **Lossless stays lossless.** PNG, BMP, TIFF and lossless WebP are never encoded
  lossily: the smaller of an indexed PNG (when the colours fit) and libwebp
  lossless WebP (`method 3`, `exact`), else the original. Pixels are identical
  and the ICC profile is kept.
- **Photos** (JPEG, HEIC/HEIF, lossy WebP) are re-encoded as AVIF at
  `ASSET_AVIF_QUALITY` when `imageFormat` is `avif`, else (or when AVIF encoding
  fails, or for HDR/other colour spaces) as lossy WebP at `ASSET_IMAGE_QUALITY`,
  and kept only when at least 10% smaller. HEIC is always converted. Browsers that
  cannot decode HEIC (Chrome, Firefox) use libheif instead, as do the web editor
  and picture upload. Re-encoding applies the EXIF orientation and drops
  EXIF/XMP/GPS. A missing `imageFormat` means WebP.
- **Originals that upload unchanged lose their metadata losslessly:** JPEG APPn
  segments other than JFIF, ICC and Adobe are dropped (orientation kept as a
  minimal Exif segment), PNG `eXIf`/`tEXt`/`zTXt`/`iTXt` chunks are dropped, and
  MP4/MOV `udta`/`meta` boxes under `moov` and each track are renamed `free` and
  zero-filled in place, so sample data is byte-identical.
- **Video** is transcoded to H.264/AAC MP4 only when the short edge is above
  `ASSET_VIDEO_MAX_HEIGHT`, the codec is not H.264, or the bitrate is more than
  1.25× the target. The output bitrate is the target at the output size and frame
  rate, capped at `ASSET_VIDEO_SOURCE_BITRATE_PERCENT` of the source's bitrate,
  with a 1.5 Mbps floor. A size-only transcode is kept only when at least 10%
  smaller; the audio track is never dropped.
- **HDR video** (HLG, HDR10) is tone mapped to SDR BT.709 (Hable, 1000-nit peak)
  where the platform can; otherwise the original uploads unchanged.
- **GIF, AVIF, audio, documents and other files** are not re-encoded locally.

`avifQuality` is libavif's quality scale as of libavif 1.0 (`avifenc -q`).
libavif 1.3+ with aom 3.13+ switches stills to aom `tune=iq`, which shifts that
scale, so Android and Apple pin `tune=ssim`; desktop's rav1e maps quality 85 to
quantizer index 55.

H.264 rather than HEVC/AV1: it plays in every browser and app, and browsers
cannot use a bundled decoder for `<video>`. Web uses WebCodecs via Mediabunny
(loaded on demand); Google Chrome on Linux encodes H.264 but not AAC, so those
uploads carry Opus audio in MP4; Chromium builds without H.264 keep the original.

The server never transcodes, but it always removes embedded media metadata before
publication. Client compression is optional; server metadata removal is not.

### Metadata privacy on completion

- JPEG, PNG, GIF, WebP, AVIF and HEIC/HEIF use ExifTool to remove EXIF/GPS, XMP,
  comments and other writable descriptive metadata. Only orientation and
  colour-rendering metadata are copied back. GIF/WebP frames are not re-encoded.
- MP4/MOV, WebM, AVI and the supported audio containers (MP3, MP4/M4A, AAC, Ogg,
  WAV, FLAC, AIFF) are remuxed by FFmpeg with stream copy. Global and per-stream
  tags, capture dates, chapters, cover art and non-audio/video tracks (GPS,
  timecode, data, subtitles) are omitted. Samples, rotation, colour information
  and timing remain.
- Files that need their own tags to decode — TIFF and camera raws built on it
  (DNG, NEF, CR2, ARW…), RW2, ORF, RAF, CRW, JPEG XL containers, JPEG 2000 and
  PSD — keep their format and other tags; ExifTool removes GPS, XMP, IPTC, serial
  numbers, owner and artist names, host computer and image IDs. The result is
  read back and refused if any remain or ExifTool cannot rewrite the file. A bare
  JPEG XL codestream has no metadata and is shared unchanged.
- Detection uses bytes, not filenames or MIME types; labelling media
  `application/octet-stream` does not bypass cleaning. Declared media nothing can
  clean (including SVG and BMP originals) is refused.
- This is **container metadata removal, not complete anonymisation**: it does not
  redact visible/audible content, data inside codec samples, ICC profile contents,
  or documents/archives and their embedded files.
- Upload URLs target only staging, so a repeated PUT after completion cannot
  replace a cleaned file. Sources are erased after success; the purge sweep
  repeats cleanup after the 15-minute PUT lifetime, including late writes.
- The API image includes FFmpeg, ExifTool and `prlimit`. One job per pod streams
  to a private temporary directory, removed on success/error/cancellation.
  Processes have address-space, CPU, output-size and wall-clock limits, closed
  stdin, no shell interpolation and no network protocols. Neither command output
  nor embedded metadata is logged. The production API needs a disk-backed
  writable `/tmp` (12 GiB for a 5 GiB object plus its result) and 512 MiB memory.
  Failures fail closed; a restart safely restarts from private staging.

Apple's “Location Included” picker badge describes the **selected source**, not
the shared object; verify privacy by inspecting the downloaded result.

### Encoders by platform

| Platform | Photos (AVIF) | Lossless | Video and HDR | Size cost |
| --- | --- | --- | --- | --- |
| Web | libavif 1.0.1 + aom 3.7.0 WASM (`@jsquash/avif` 2.1.1), single-threaded worker, speed 9 | `@jsquash/webp` 1.5.0 worker; indexed PNG | Mediabunny/WebCodecs H.264; HDR tone mapped on WebGL2 | 3.5 MB WASM (1.1 MB gzipped), fetched only when a photo is encoded. HEIC the browser cannot decode: libheif 1.23 + libde265 WASM (`libheif-js` 1.23.5, LGPL-3.0, own worker chunk; joins iPhone tile grids, applies rotation, converts Display P3 to sRGB), 2 MB (0.7 MB gzipped) |
| Android | libavif 1.4.2 + aom 3.15.1 JNI built from pinned sources (`prepare-avif.sh`), speed 8, `tune=ssim`; AOMedia decoder shows AVIF on API 26–30 | Bitmap lossless WebP; indexed PNG | Media3 Transformer H.264; HDR tone mapped with OpenGL | +1.8 MB download per ABI |
| Apple | libavif 1.4.2 + aom 3.15.1 static XCFramework from [joswayski/libavif-apple](https://github.com/joswayski/libavif-apple), speed 6, `tune=ssim`; ImageIO decodes | libwebp 1.6.0 lossless; indexed PNG | AVAssetWriter H.264 at the shared size and bitrate; HDR to BT.709 via AVVideoComposition | ≈4.3 MB per arm64 slice |
| Desktop (Windows, Linux) | rav1e 0.8.1 + avif-serialize, speed 10, quantizer 55 for quality 85; x86 assembly when nasm is present | libwebp 1.6.0 lossless (`webpx`); indexed PNG | Bundled FFmpeg 9.0.2: Media Foundation H.264 on Windows, else x264 `veryfast`; HDR tone mapped with zimg (Hable) | ≈2.4 MB, plus FFmpeg ≈16.6 MB |

Desktop decodes HEIC and AVIF with the bundled FFmpeg (r2 or later, whose
`xstack` joins iPhone tile grids), converts Display P3 to sRGB, then converts
HEIC like web. HEIC uploads unchanged when no FFmpeg is found or the photo
exceeds the 50-megapixel compression bound. Desktop and web read Display P3 from
an nclx box or an ICC profile.

### Server-side processing (parked)

A server pipeline (an arm64 Lambda, `apps/media-worker`, compressing S3 uploads
into R2) was built and deliberately not shipped: server encoding costs roughly
$0.003–0.02 per video minute, while client encoders are free. It remains in the
history of joswayski/caper#246 and joswayski/infrastructure#129. Clients still
accept the optional `status` (`processing|ready|failed`), `animated` and
`attachmentsSeq` fields and the `message.attachments` / `attachment.progress`
events. Today's API sends only `attachmentsSeq`, on edits that change a message's
files; an absent `status` means ready. Possible follow-ups: HEVC/AV1 delivery once
every client can play it, and JPEG XL once Android and native clients can
display it.

### Attachments in other features

A text edit keeps a message's attachments, and a message with files may be edited
to empty text. Its author can replace its photos and videos or change spoilers,
but not add or remove files (see
[Editing attachments after sending](#editing-attachments-after-sending)). A
forward embeds a snapshot of its source message, including attachments, whose
URLs are signed with the forward and refresh when it is reloaded. Pinned messages
and thread replies show their files. Channel, DM, thread-reply and profile-message
composers accept attachments.

Pasting copied files or images into a composer uses the same draft attachments as
the file picker: upload starts immediately, but nothing is sent until Send/Enter.
Existing text stays intact, and ordinary text and links paste at the selection.
Web accepts the files the browser exposes in the paste event (not HTML images or
URLs). Apple accepts native file URLs and file-like data (GIF, PDF, audio, video,
archives), preferring GIF bytes to a static image. Android accepts clipboard
content URIs through Paste or Ctrl+V, and keyboard rich content via
`commitContent` (including Gboard GIF/sticker buttons); it advertises `*/*` and
keeps the temporary URI permission until the attachment job finishes. Rust
desktop accepts native file lists, encoded media/documents and images with
Ctrl+V; native file references come first, and the encoded fallback recognizes
media, PDF, ZIP and generic binary data, preferring GIF over PNG. Rust vendors a
narrow `egui-winit` 0.37.0 patch that delivers paste intent on key-down even
without text; see the vendored `PATCHES.md` when upgrading egui.
`scripts/test-chat-paste.mjs` covers web against `native-parity-fixture.mjs`.

### Editing attachments before sending

A pencil on a draft's tile opens a full-window editor for still images and videos
before they are sent, on every client. Each tile carries two small round buttons
on opposite corners: Remove (X) top right and Edit (pencil) bottom left. Faces
are 22 px (24 on touch) with 44 px/pt touch targets (48dp on Android). While a
file prepares or uploads, a progress ring wraps the X; the figure is in its
accessible label. A failed upload shows one centred eggshell Retry button; a
spoilered tile is blurred with a centred crossed-out eye.

Edit, like a tap on the picture, opens the editor for viewing with no tool
chosen. Its tool row is always **Spoiler** | **Crop** **Draw** **Text** |
**Remove**. Choosing a tool switches it in place (choosing it again returns to
viewing): the top bar, the picture's position and size, and the buttons never
change, and the picture is not decoded again; only the tool's own options fade in
above the row, in reserved space. Spoiler toggles the mark live; Remove removes
the file and closes; Done saves edits (with none it just closes). While marked as
a spoiler and no tool is chosen, the picture shows covered as it will be sent.
Files the editor can't open but can show (web's viewer) get Spoiler and Remove in
their viewer; files with no preview (PDFs, audio) get a menu with Mark as spoiler
and Remove attachment. Long press or context click offers Edit, the spoiler
toggle and Remove attachment. Send can be pressed while files still upload; the
message goes as soon as they finish. Every client opens editable files in this
editor rather than the system viewer. The viewing state zooms like the media
viewer (pinch or Ctrl+scroll; double-tap/click; drag to pan); video doesn't zoom
except desktop's still frame. Desktop's preview also has Open in default app,
which opens what will be sent (the original when unedited, otherwise the prepared
edited file, \"Available when your edit is ready\" while it prepares). On web
phones the editor takes over the preview's history entry, so Back closes it.

- **Crop** with corner and edge handles (drag inside to move; arrow keys nudge on
  web and Android) and **Rotate left** in quarter turns.
- **Draw** freehand with round lines, and **Text** (Satoshi Bold, one line, up to
  200 characters): click or tap to place, click to change, drag to move. With a
  mouse, trackpad, pen or iPad pointer, Draw shows a ring the size and colour of
  the brush in place of the cursor (at least 6 px). Both take one of six colours
  (red, yellow, green, blue, white, black) or any colour from the platform's
  picker, and a size slider (2–40 pt lines, 14–96 pt text, as seen on screen).
  Draw's **Eraser** removes every whole line its ring passes over, as one undo
  step; never the picture or text. Picking a colour or another tool turns it off.
- **Undo**, **Redo** (Ctrl/⌘+Z, Ctrl/⌘+Shift+Z, Ctrl+Y) and **Reset**; Escape
  closes a text field, then cancels.

**When it happens.** Uploads start the moment a file is added, so unedited files
send as quickly as before. Editing uses the local original. Saving an edit
cancels that file's upload (`POST /api/assets/{id}/cancel`) and prepares the
**original** again with the edit applied, uploading the result in the same tile.
The draft keeps the original and a description of the edit, so reopening shows
the earlier edits still editable and edits never stack generation loss. The
profile card composer, which uploads at Send, applies the edit when it prepares
the file. The pencil is hidden while a send carrying the files is pending.

**Never the unedited file.** An edit that cannot be applied (undecodable image, no
H.264 encoder, failed transcode) fails the tile with \"Couldn’t apply your
edits.\" and Retry, rather than uploading the original. Send retries it once and
never sends the message without it.

**The model** (`apps/web/src/chat/edit.ts`, mirrored by each client): quarter
turns of the upright original (EXIF applied), a crop as fractions of the turned
picture, then marks painted in order: strokes (points as fractions, width as a
fraction of the longer edge) and text centred on a point (size as a fraction of
the longer edge, always upright). Every client's tests run
`shared/messages/edit-cases.json` (turns, crop rounding including even video
crops, when a file counts as edited, and which lines an eraser touches).

**What gets uploaded.**

- Images are painted at the original's resolution, then encoded by the source's
  class under the compression rules: photos become AVIF/WebP at the server
  quality and `ASSET_IMAGE_MAX_EDGE`; screenshots/graphics stay lossless. The
  smallest encoding is kept (the original is not an option), with PNG as the last
  resort. Output is upright with no EXIF.
- Videos are always transcoded to H.264. The turn and crop are baked in, the crop
  starts and measures on even pixels, its short edge is capped by
  `ASSET_VIDEO_MAX_HEIGHT` (0 keeps the crop's size), and the bitrate target is
  for the output size. Drawings and text are one still overlay over the whole
  video (the editor shows a frame from just after the start). Where the platform
  cannot tone map HDR, editing fails. Audio is kept.
- Editable: still JPEG, PNG, WebP, AVIF, HEIC/HEIF, BMP and TIFF the platform
  decodes, and videos it can transcode. GIFs, animated WebP/PNG, audio and other
  files show no pencil.

The server sees an ordinary upload.

| Client | Editor | Images | Video |
| --- | --- | --- | --- |
| Web | `AttachmentEditor.tsx` (loaded on demand), full-window dialog | Canvas at full resolution, then the existing encoders | Mediabunny/WebCodecs: rotate, crop and resize, then the overlay; HDR through `hdr-video.ts`. Offered only where the browser encodes H.264 |
| Android | `AttachmentEditor.kt`, full-screen Compose dialog; the custom swatch opens a small hue and saturation/value picker | `EditPainter` (android.graphics) on the decoded crop region, then the existing encoders | Media3: `ScaleAndRotateTransformation`, `Crop`, `Presentation`, then a `BitmapOverlay`; HDR needs Android 10+. AVIF editable from Android 12, HEIC from 9; TIFF not at all |
| Apple | `AttachmentEditor.swift`: full-screen cover on iPhone/iPad, window overlay on Mac (inside the profile popover there); system `ColorPicker` | `AttachmentEditRenderer` (CoreGraphics/CoreText), then the existing encoders | The BT.709 composition read as BGRA, each frame turned, cropped, scaled and overlaid with Core Image, written through a pixel-buffer adaptor |
| Rust desktop | `attachment_editor.rs`, full-window egui modal with egui's colour picker | `attachment_edit.rs`: tiny-skia strokes, Satoshi Bold through ab_glyph, then the existing encoders; HEIC/AVIF decode through the bundled FFmpeg | FFmpeg filter graph: `transpose`/flips, `crop`, then scale or tone map. Marks need `overlay` with `png_pipe` input, which the bundled FFmpeg lacks, so Draw and Text are disabled for videos (with an explanation) until joswayski/ffmpeg-desktop enables them; the app detects them once present. Videos show a frame from 1 s in |

### Editing attachments after sending

**API only:** the server accepts these edits, but no client offers them yet. The
author of a message can replace any of its photos and videos with an edited
render, put back the file first sent in that slot, and turn any file's spoiler on
or off. Files can't be added, removed or reordered after sending.

**Request.** The text edit `PUT /api/chat/channels/{channel}/messages/{message}`
takes an optional `attachments` list: the message's files in their current order,
one entry each.

```json
{
  "text": "Trip",
  "expectedRevision": 3,
  "attachments": [
    { "id": "N1", "replaces": "B", "spoiler": true,
      "edit": { "rotation": 90, "crop": { "x": 0.1, "y": 0, "width": 0.8, "height": 1 }, "marks": [] } },
    { "id": "D", "spoiler": true },
    { "id": "V", "replaces": "N0" }
  ]
}
```

- `{id}` keeps the current file. `spoiler` defaults to false, so every entry
  states it.
- `{id, replaces, edit}` replaces the photo or video `replaces` with a fresh
  upload of the same kind: the author's, completed in this channel within 24
  hours and not sent. `edit` says how it was rendered from the slot's first-sent
  file, in the pre-send editor's model. Its rules and limits (quarter turns, a
  crop inside the picture, at most 500 marks and 100,000 point values, one line
  of 1–200 Unicode scalar values, `#RRGGBB`, 128 KiB of JSON, fractions rounded to
  a millionth) are the `wire` section of `shared/messages/edit-cases.json`.
- `{id, replaces}` where `id` is the slot's first-sent file undoes every edit of
  that file: no upload and no `edit`.
- Without `attachments` the request behaves as a text edit; clients send it only
  when files change (an older API refuses the field with 422). A spoiler toggle
  alone is an edit, with a revision, the edited marker and a version. Text may be
  empty while files remain.
- The response and the `message.edited` event carry the new list, and
  `attachmentsSeq` becomes the edit's `editSeq` whenever the files change; clients
  keep a message's files unless `attachmentsSeq` is newer. Edits never notify.
- Resending the same body after a lost response returns the stored message,
  checked before any upload is linked. A different list at an old revision is
  409.

| Refusal | Response |
| --- | --- |
| Unknown field in an entry (`remove`, …) | 422 |
| More than 10 entries, or an ID or `replaces` twice | 400 `edit up to 10 different files` |
| Not the current files in order (length, order, `replaces` not the slot's file) | 400 `list the message's files in their current order` |
| `edit` on a kept file / missing on an upload / on a restored file | 400 `only a replacement file has an edit` / `a replacement file needs its edit` / `a restored file has no edit` |
| Replacing audio or another file | 400 `only photos and videos can be replaced` |
| A video for a photo, or the reverse | 400 `a replacement must be the same kind of file` |
| A description breaking the rules / too many marks, points or bytes | 400 `invalid attachment edit` / 400 `attachment edit is too large`, code `edit_too_large` |
| The upload is missing, cancelled, someone else's, another channel's, older than 24 hours or already sent | 409 `Failed to upload.`, code `attachment_unavailable`: upload again once, then retry |
| The replaced or first-sent file was deleted | 409 `this file can no longer be edited` |
| Not the author / no access, or a forward | 403 / 404 |

**Who sees what.** Replaced files stay stored and linked to their message, marked
`retired_at`; a render records its first-sent file (`edit_source_id`) and the
description (`edit`).

- Everyone sees the current files wherever the message appears: history,
  threads, pins, single messages, forwards and notifications.
- Only the author can still open replaced files: `POST /api/assets/urls` refreshes
  them only for their owner, and `GET …/versions` signs every version's files for
  the author.
- Others get versions without URLs, in which files no longer on the message are
  placeholders: `{id, kind, contentType, name: "Replaced file", size: 0,
  unavailable: true}`. Both views mark a revision whose `(id, spoiler)` list
  differs from the one before with `filesChanged: true`.
- Stored events (`message.created`, `message.edited`, pins, forward snapshots)
  still name earlier files, so gateway replay, the outbox publisher and
  notification content turn replaced files into the same placeholder for every
  viewer.
- The edit description appears only in the author's `attachment-sources`.
- URLs already issued work until they expire (one to two hours), as do downloaded
  copies.
- Replaced files are never purged, still count toward `ASSET_QUOTA_BYTES`, and
  cancelling one returns 409.

**Re-editing** starts from the first-sent file, so edits never compound.
`GET /api/chat/channels/{channel}/messages/{message}/attachment-sources` (author
only: 403 for others, 404 without access or for a forward) lists each current
photo and video with the first-sent file to edit from, freshly signed, and the
current file's edit (`null` when unedited):

```json
{
  "messageId": "m",
  "revision": 3,
  "sources": [
    { "attachmentId": "N1",
      "source": { "id": "B", "kind": "image", "contentType": "image/avif", "name": "IMG_1.avif",
                  "size": 812345, "width": 4032, "height": 3024, "preview": {},
                  "url": "https://…/assets/B?…", "previewUrl": "https://…/preview/B?…" },
      "edit": { "rotation": 90, "crop": { "x": 0.1, "y": 0, "width": 0.8, "height": 1 }, "marks": [] } },
    { "attachmentId": "V", "source": { "id": "V", "kind": "video", "…": "…" }, "edit": null }
  ]
}
```

The first-sent file is the compressed upload, with any pre-send edit already in
it, and keeps its EXIF orientation, so editors decode it upright.

### Inline GIF playback

GIF uploads stay ordinary `image/gif` originals. Upload preparation may also
produce a **still** preview for placeholders and notifications; ready GIF tiles
load the original on every client, while still photos use their previews. Android
uses Coil's GIF decoder, Apple its ImageIO frame loader and timeline, and Rust
advances composited frames on the tile's texture (local first-frame thumbnails
cannot overwrite a playing GIF). Click/tap opens the viewer. The legacy
`animated` flag is not needed for ordinary GIFs.

Reduced-motion readers see a still inline (web draws a bounded canvas still).
Apple and Rust inline decoding is limited to a 720px long edge, 600 frames and
32 MiB of retained frame pixels per GIF; when frames would exceed that budget,
playback resolution shrinks while every frame and delay is kept. Files beyond 600
frames fall back to one still. Rust also applies decoder allocation/dimension
limits and a 40 MiB network limit. Apple releases frames on disappearance and Rust
releases inactive animation cache entries. Animated WebP/APNG inline timelines
are not implemented.

### In-app viewer

Clicking a sent image or video opens it almost full screen inside the app on every
client (`MediaViewer.tsx`, `MediaViewer.kt`, the `AttachmentViews.swift` viewer,
`viewer.rs`). The viewer holds that message's ready images and videos in order,
starting at the one clicked: arrows, Left/Right or a swipe move between them (no
wrap) with an \"n / m\" counter. Images show the preview until the original loads,
zoom with double-click/tap, pinch or Ctrl+scroll, and pan while zoomed; animated
GIF/WebP play. Videos autoplay with sound and pause when you move away or close;
`animated` videos loop muted. Each viewer has Save/Download, Open in browser and
Close (Esc, the dark area, or a swipe down on phones). The header shows the stored
`size`; narrow screens truncate the author and time first. The client-reported
pre-compression size (`assets.source_byte_size`) stays internal. URLs come from
each client's fresh-URL cache, and a refused load asks for one refresh. Audio and
other files keep their inline behaviour; pending rows never open the viewer. The
iPhone/iPad swipe dismissal keeps the file's release offset while the cover
dismisses.

Desktop has no platform video player or AVIF decoder, so it ships a trimmed,
static FFmpeg built from pinned sources by
[joswayski/ffmpeg-desktop](https://github.com/joswayski/ffmpeg-desktop) (FFmpeg
9.0.2, x264, dav1d 1.5.4, zimg 3.0.6; GPL-2.0-or-later, run as a separate program
so the app stays Apache-2.0). `build.sh` and `build.ps1` download its
`ffmpeg-9.0.2-r2` release, check the zip's SHA-256 and package `ffmpeg`
(`ffmpeg.exe`) beside the app, with its licences, `SOURCES.txt` and recipe in
`ffmpeg-licenses/` (`/usr/share/doc/caper-desktop/` in the .deb). A newer build
is a new release revision and a new pin. The app uses it to:

- compress videos before upload with the shared rules, reporting
  \"Compressing… N%\" on the draft chip; any failure uploads the original;
- play videos in the viewer: one FFmpeg decodes RGBA frames into an egui texture,
  another decodes 48 kHz stereo PCM for rodio, and audio is the clock. FFmpeg has
  no TLS, so it streams the signed CDN URL through a loopback range proxy
  (`media_proxy.rs`, bound to 127.0.0.1 with an unguessable per-video token).
  Seeking restarts both decoders;
- decode AVIF and HEIC stills (including iPhone tile grids) for uploads, the
  editor, profile pictures and the viewer.

It is found beside the executable (`/usr/lib/caper-desktop` for the .deb) or at
`CAPER_FFMPEG`. Without it, videos upload unchanged and the viewer offers the
browser. The ignored real-FFmpeg tests use `CAPER_FFMPEG` for the shipped build
and `CAPER_FFMPEG_FIXTURES` for a full FFmpeg that makes their inputs.

### Deletion and purge

Selecting a file starts uploading before Send. Removing a composer file cancels
local work and best-effort calls authenticated `POST /api/assets/{id}/cancel` on
every client. The endpoint soft-deletes only the owner's unsent asset and returns
204, including repeats; missing/foreign files return 404 and sent files 409. Its
row lock serializes with Send, so a late cancellation cannot delete a sent file.
Failed uploads and discarded composer files also request cancellation. Moving a
file into a pending send or cleaning up a confirmed send's local copy must not
cancel it. An unknown send outcome keeps the original IDs for a safe retry.

An API loop (about every minute, idempotent across replicas):

- Marks uploads never completed after 1 hour, or completed but never sent after
  24 hours, as deleted.
- Removes R2 objects for deleted rows in batches of 25: unsent files 30 minutes
  after the reservation (once upload URLs have expired plus a margin, so a reused
  URL can't store uncounted bytes); sent files 24 hours after deletion. Only
  successful deletion of both original and preview sets `purged_at` and releases
  quota. Failures retry.
- Revisits deleted, unsent rows, including purged ones, in a separate 25-row
  batch, because presigned PUTs can race deletion. A cursor with a fixed ceiling
  per pass prevents starvation. `purged_at` records the first successful delete,
  not permanent absence. Monitor `asset_delete_retry`, `asset_purge_retry`,
  `asset_sweep_retry` and R2 request volume as history grows.

An open draft does not extend the server's 24-hour lifetime. Clients keep the
local prepared file and preview until removal or delivery confirmation. On
focus/foreground or Send, uploads at least 23 hours old are silently uploaded
again. Desktop keeps an owned copy rather than re-reading a picked path.
Concurrent triggers share the active upload; successful files in a partially
failed batch are reused on Retry.

Send retries failed uploads and continues once the entire selected batch is
ready; another failure keeps the text and files in the composer instead of
sending a subset. Failed tiles have **Retry upload** and a warning that Send
retries. Native Send attempts are cancelled if their composer or destination
changes while waiting.

Send enforces expiry even before cleanup and returns 409 `attachment_unavailable`.
Clients treat this as a definitive refusal, silently re-upload and send with a new
message UUID and attachment IDs, at most once per send attempt. A failed upload
shows only **Failed to upload.** with **Retry**, preserving the pending message
and any newer draft. A timeout is an unknown outcome: retry the original command
first, and never cancel or replace its files until acceptance or a definitive
refusal. Replacement uploads reserve quota normally; cancelled bytes stay charged
until purge succeeds.

History marks deleted attachments `unavailable` without URLs. Files replaced by an
edit keep their `message_id`, so the loop never purges them. There is no
user-facing sent-file delete; account deletion should set `deleted_at` on the
owner's assets when that flow lands.

### Configuration

| Variable | Where | Meaning |
| --- | --- | --- |
| `R2_ACCOUNT_ID`, `R2_BUCKET`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` | API | Bucket-scoped R2 Object Read & Write key. All four or none; none leaves upload routes returning 503. |
| `R2_ENDPOINT` | API, local only | Loopback S3-compatible fake (`http://127.0.0.1:…`). Rejected otherwise. |
| `ASSET_QUOTA_BYTES` | API | Per-person stored bytes. Default `10737418240` (10 GiB). |
| `ASSET_IMAGE_FORMAT`, `ASSET_AVIF_QUALITY`, `ASSET_IMAGE_QUALITY`, `ASSET_IMAGE_MAX_EDGE`, `ASSET_PALETTE_COLORS`, `ASSET_PREVIEW_EDGE`, `ASSET_VIDEO_MAX_HEIGHT`, `ASSET_VIDEO_BITRATE_KBPS`, `ASSET_VIDEO_SOURCE_BITRATE_PERCENT`, `ASSET_VIDEO_HIGH_FPS_BOOST_PERCENT`, `ASSET_AUDIO_BITRATE_KBPS` | API | Client compression settings above; invalid values stop startup. |
| `ASSET_CDN_ORIGIN`, `ASSET_CDN_SIGNING_SECRET` | API **and** gateway | Worker origin and shared HMAC secret (≥ 32 characters). Without them messages carry attachment metadata but no URLs. |

The bucket needs a CORS rule allowing `PUT` from the web origin with the
`content-type` and `content-disposition` headers (managed in
`joswayski/infrastructure`). The Worker (`apps/cdn/wrangler.toml`) binds the
bucket as `MEDIA`, runs on `cdn.caper.chat` / `cdn.staging.caper.chat`, and holds
`ASSET_CDN_SIGNING_SECRET` as a Worker secret:

```sh
cd apps/cdn
npx wrangler secret put ASSET_CDN_SIGNING_SECRET --env staging
npx wrangler deploy --env staging
```

`npm run test:cdn` runs the Worker tests.

### Platform status

| Platform | Send files | Show files |
| --- | --- | --- |
| Web (desktop and mobile layouts) | Upload a file on desktop; attach drawer (Photos and videos, Files, and Camera on touch devices), clipboard files/images, drag and drop with an overlay | Images, video, audio, file cards; images and videos open in the in-app viewer |
| Apple (iOS and macOS) | Attach drawer with the photo library and camera on iPhone/iPad (Upload a file on macOS), file importer, clipboard files/images, drag and drop with an overlay | Images, video (AVKit), audio, file cards, "File removed"; in-app viewer |
| Android | Attach drawer with the photo library and camera (system photo picker without library access), document picker, clipboard content URIs, drag and drop from other apps, draft chips with final size and progress; on-device compression (Media3) | Images, in-app video/audio playback (Media3), file cards, "File removed"; in-app viewer |
| Rust desktop (Windows, Linux) | Upload a file (Win32 dialog; XDG desktop portal on Linux), clipboard files/images, drag and drop with an overlay; stills and videos compressed per the shared rules (videos via bundled FFmpeg, else unchanged with metadata stripped) | Inline images and video posters (decoded off the UI thread); in-app viewer and player; audio and files open in the system browser or player; "File removed" |

Every client refreshes signed URLs through `POST /api/assets/urls` when `exp` is
past or within an hour, or once after a 403/404, and decodes attachment lists
tolerantly (a malformed entry is skipped). Apple re-encodes video with
`AVAssetReader`/`AVAssetWriter` at the shared size and bitrate rather than export
presets, which cannot take a bitrate.

#### Attach drawer

Choosing a new profile picture or space icon uses the same picker as the composer
on every client: the attach drawer in single-picture mode on iPhone, iPad and
Android (photos only, camera for a photo, Photos and Files for one image; tapping
one opens the crop editor), the Camera / Photos / Files sheet on touch web, and
the file dialog directly on desktop web, Mac and Rust desktop.

Attach (the composer's `+`, in channel, thread and profile-DM composers) opens the
attach drawer on phones and an attach menu on desktop. The smiley opens a
separate Emojis / GIFs / Stickers drawer on mobile. Phones offer both a photo
library and files; desktop web, the Mac and Rust desktop offer one unfiltered
"Upload a file".

- **Width**: mobile drawers reach both window edges; controls keep safe-area
  padding. Android and touch web do not cap the width on tablets or landscape.
- **Limit line**: nothing below the limit. At 10 files it shows only \"Remove a
  file to add another.\" The drawer and menu stay openable; choosing anything then
  shakes the line sideways (4 px, `--motion-slow`, none under reduced motion) and
  buzzes on phones instead of opening a picker.
- **iPhone/iPad** (`AttachDrawer.swift`): an edge-to-edge drawer in a transparent
  full-screen cover, initially half-height. Drag its header or tap the handle to
  expand/collapse; drag down, tap outside, Close, or VoiceOver escape dismisses.
  Camera tile and PhotoKit library show newest first. Tapping a tile copies the
  original (from iCloud if needed) and adds it to the draft at once. Numbered
  terracotta badges show draft order, which follows tap order
  (`AttachDrawerQueue`). Tapping again or removing the chip deselects. At the
  limit other tiles dim. A floating Send sends once uploads settle. The bar holds
  Photos and Files. Access is asked when the drawer first opens
  (`NSPhotoLibraryUsageDescription`); limited access shows \"Manage\"
  (`presentLimitedLibraryPicker`; the automatic limited-access alert is off);
  denied access offers Settings, the system Photos picker and the camera
  (`UIImagePickerController`, `NSCameraUsageDescription`).
- **Android** (`AttachDrawer.kt`): the same drawer as a bottom sheet over
  MediaStore. It asks when first opened: `READ_MEDIA_IMAGES` and
  `READ_MEDIA_VIDEO`, plus `READ_MEDIA_VISUAL_USER_SELECTED` on Android 14+
  (partial access, with \"Manage\"), or `READ_EXTERNAL_STORAGE` up to Android 12.
  Without access it offers Settings, the system photo picker and the camera. The
  camera tile uses `TakePicture`/`CaptureVideo` into the FileProvider cache (no
  `CAMERA` permission). Drafts map back to tiles by `pickedUri`.
- **Web**: desktop `+` opens a menu with \"Upload a file\". Touch layouts open a
  bottom sheet with Camera (`capture`), Photos and videos, and Files.
- **Rust desktop** (`attach_drawer.rs`): a menu above `+` with \"Upload a file\".
  Profile messages add Paste from clipboard.

iOS and Android animate local GIF originals in Photos tiles, with a GIF badge.
Leaving the viewport, backgrounding or reduced motion stops them. Android decodes
at 120dp; iOS caps each animation at 240px, 200 frames and 2 MiB decoded and
plays frames with a Core Animation keyframe animation; it rejects encoded data
above 16 MiB and does not download cloud-only originals for autoplay. Videos show
a play icon and duration. While a composer is on screen (with access already
granted; it never prompts), both apps prepare the 60 newest thumbnails (and iOS
their GIFs) in a process-wide cache (Android LRU 64 MiB; iOS `NSCache` 64 MiB plus
96 MiB of decoded GIF frames) keyed by item and modification time. iOS reads the
GIF flag from `PHAsset.playbackStyle` and applies PhotoKit change details; Android
coalesces MediaStore change bursts and cancels decodes for tiles scrolled away.

Google Play: photo/video permissions need the Photo and Video Permissions
declaration in Play Console (App content); the text is in
[store-declarations.md](store-declarations.md). If Play refuses it, removing the
three `READ_MEDIA_*` lines from the manifest leaves the drawer working through
the system photo picker.

#### File drop overlay

Dragging files over a channel or DM conversation that accepts uploads covers it
with a drop overlay on every client: a random Caper character (one of the 800
branding illustrations, picked on each open) hops in front of three neutral file
cards (image, document, video Lucide glyphs) that drift apart behind it, on a
surface card with a dashed terracotta outline. The title is `Attach to #channel`
(`Attach to Name` in a DM) and the detail counts remaining slots (`Up to 10
files…`, `7 more files…`, `10 files attached. Remove one to add another.`). Every
client's tests run `shared/messages/drop-cases.json`. Dropping adds the files as
drafts, like the attach button. Only drags carrying files open it; the overlay
never takes input.

Motion is shared and seeded: each open picks a random seed and every client
derives the same motion from it (mulberry32; `drop.ts` `dropPlan`/`dropPose`,
checked against `shared/messages/drop-motion-cases.json`). After a 140 ms fade and
the card rising 10 px, the cards spread over 1.3–1.9 s each (staggered, ease-out
cubic) to a resting pose jittered by up to 3 px/3°, then float on two slow
(4–9 s) sine waves per axis. The character rises, then hops on gravity arcs 5–12
px high with a small rebound, irregular pauses and damped-spring squash and
stretch, with a slow 2° tilt. Reduced motion shows the seed's resting pose.
Parity fixtures use character 0 and seed 0. After a drop on macOS the composer
takes focus back.

- Web: `DropOverlay.tsx` over `.chat-panel` or `.chat-thread-panel`. Files dropped
  elsewhere in the app are refused instead of navigating. Thread drops attach to
  that thread.
- Rust desktop: `file_drop.rs`, painted from egui's hovered files.
  `--fixture parity-file-drop` (and `-narrow`) hold it open.
- Apple: `FileDropOverlay` on the conversation's `dropDestination` on iOS and
  macOS, taking photos/videos/audio/PDFs as media and other files as URLs. The
  macOS composer (`NSTextView`) accepts only text drags.
- Android: Compose `dragAndDropTarget` for drags from other apps, with
  `requestDragAndDropPermissions` before copying. Android names a drag only by
  MIME type until the drop, so plain-text files are attached with the button.

#### Draft tiles and galleries

Composer tiles are clickable before sending, including while uploading or after a
failure. Web images and videos use the in-app viewer with local object URLs;
audio, PDFs and plain text open in the browser and other formats download. Apple
uses Quick Look; Android grants the content URI to a system preview app; Rust
desktop opens the file in the system browser. Draft video tiles show a poster
with a small play indicator (web and Apple decode an early frame, Android reuses
its prepared poster, Rust uses FFmpeg). Long press or context click offers
**Remove attachment**, affecting only the draft. Ready chips show only the final
upload size on Android, iOS and web mobile/touch (≤ 760px or coarse pointer);
desktop web, macOS and Rust desktop show original-to-final when compression saved
space. The channel/DM attachment control sits inside the left edge of the message
box on every client.

Sent images and videos form a count-aware gallery on every client: two side by
side, three with a large left tile, four in a 2×2 grid, and larger sets in rows of
two or three. Galleries cap at 480 logical pixels, with 4px gaps, 12px outer
corners and 1px interior corners. Thumbnails crop to fill; the viewer shows the
full media. A single image keeps its natural proportions. Processing, failed and
removed media keep their slots. Audio and document cards follow the gallery, side
by side where space permits.

## Voice messages and AI

The composer's microphone does two things. A tap or click **dictates**: the
speech is transcribed by the AI layer and the words are inserted at the caret, to
edit and send as text. Holding it on a touch screen records a **voice message**,
which plays inline with the sender's waveform. Transcripts appear under voice
messages Slack-style (three lines, then "View transcript").

### Dictation

`POST /api/dictation` (signed in) takes the raw recording as the body, in any
container ffmpeg reads, up to 8 MiB and 10 minutes (its own body limit; the rest
of the API keeps 256 KiB). It converts to 16 kHz mono FLAC, runs
`AI_TRANSCRIBE_MODELS` in order and answers `{"text": …}`. Nothing is kept: not
the audio, and not the text, which only lives in the draft. `ai_calls` records
each attempt with task `dictate`, no job, and the transcript replaced by its
length. Each account may dictate 120 times an hour (on `space_invite_limits`,
429 past it); without a provider it answers 503.

Every client shows the same bar while dictating: discard, timer, live levels,
**Voice note** and a pulsing terracotta **Done** check (Enter is Done, Escape
discards; on iPhone/iPad and Android the microphone itself becomes Done). Done
shows "Transcribing…" with Cancel above the composer, then inserts the words
over the selection, adding a space where they touch a word. "Voice note" keeps
the recording as a voice message instead, so people who only tap still find
voice messages. A failed transcription keeps the audio: "Couldn't insert your
words." with Retry, "Keep as voice note" (or "Send as voice note" with the
setting on) and Discard. A staged voice message only disables "Voice note" and
holds; dictation still works. The fixture (`scripts/native-parity-fixture.mjs`)
answers dictation with "TEST FIXTURE dictated words".

### Flow

1. The client records in whatever its platform does best (Opus/WebM in browsers,
   AAC/M4A on Apple and Android) and reserves an ordinary attachment with
   `voice: true`, `durationMs` (1 ms to 10 minutes) and an optional `waveform`:
   base64 of 64 bytes (1 to 256 accepted), the peak level of each equal slice,
   normalised so the loudest slice is 255.
2. `complete` re-encodes the recording to mono AAC in MP4 at 64 kbps
   (`audio/mp4`, named `Voice message.m4a`) through the same metadata-stripping
   step as other media. A minute is about half a megabyte and counts toward quota.
3. When an AI provider is configured, `complete` also marks the transcript
   `pending` and queues an `ai_jobs` row, so transcription starts before the
   message is sent; an upload that is never sent is cleaned up as usual and its
   job stops.
4. The transcriber (one per API pod, leased jobs, three attempts with backoff)
   converts the stored AAC to 16 kHz mono FLAC, sends it to
   `POST {AI_BASE_URL}/audio/transcriptions` and tries each model in
   `AI_TRANSCRIBE_MODELS` in order until one succeeds.
5. A transcript that finishes before sending is copied into the message when it
   is sent. One that finishes later updates the message and emits
   `message.attachments` (seq-gated by `attachmentsSeq`).

Voice attachments carry `voice`, `waveform` and `transcript`
(`{"status":"pending"}`, `{"status":"ready","text":…}` or `{"status":"failed"}`;
absent when no provider is configured). Push notifications say "Sent a voice
message".

### AI layer

`apps/api/src/ai.rs` calls one OpenAI-compatible gateway (OpenRouter by default).
Features ask for a task; the models serving it are configuration. Every provider
attempt is stored in `ai_calls` with its model, outcome, HTTP status, latency,
cost (as reported by the gateway), full request (inline audio replaced by its size
and asset) and full response, for comparing models on real traffic:

```sql
SELECT model, count(*), avg(latency_ms)::int AS ms, sum(cost_usd) AS usd,
       count(*) FILTER (WHERE NOT succeeded) AS failures
FROM ai_calls WHERE task='transcribe' GROUP BY model;
```

Spending limits live on the OpenRouter key itself.

| Variable | Where | Meaning |
| --- | --- | --- |
| `OPENROUTER_API_KEY` | API | Enables transcripts. Without it voice messages still work, with no transcript. |
| `AI_BASE_URL` | API | OpenAI-compatible gateway, default `https://openrouter.ai/api/v1`. HTTPS only (loopback HTTP for tests). |
| `AI_TRANSCRIBE_MODELS` | API | Comma-separated models in fallback order, default `microsoft/mai-transcribe-2,elevenlabs/scribe-v2`. |

All three are read from `production/apps/caper` like other API settings;
changing them needs only an API restart.

### Recording and sending

A voice message comes from holding the microphone on a touch screen, or from the
dictation bar's "Voice note". Recording starts on touch down, but a touch counts
as a tap (dictation) until held for 300 ms. Held longer it becomes a voice
message with a haptic tick (unless one is already staged), and a hint ("Release to add ·
Slide up to lock", or "Release to send" with the setting on) appears with a lock
pill above the microphone; releasing stops, and sliding up 72 pt/dp/px locks the
recording hands-free with a terracotta Stop that pulses once a second. There is
no slide-to-cancel: a released hold is staged, and
a locked one has Discard. When a hands-free recording continues after the finger
lifts, touch layouts close the keyboard. Recordings under a second are dropped and
10:00 stops automatically.

By default stopping does not send. The recording becomes a voice chip in the
composer beside any staged files (play, waveform, duration, remove), so text,
photos and the voice note go out together on the next Send. It uploads at once;
removing the chip cancels the upload. One voice note per message. The per-device
setting "Send voice messages as soon as I stop recording" (off by default) sends
on stop instead. The profile card's quick message works the same way.

Waveforms are per-slice RMS loudness on a -50..0 dB scale, lightly smoothed and
normalised (Android approximates RMS from MediaRecorder's amplitude readings).
Players draw evenly spaced rounded bars by linear interpolation, with per-frame
progress.

The live recording meter is the same on every client. It adds a bar every 75 ms
(80 px a second) holding the loudest level in that slice, and a bar may fall to
35% of the one before. The bar still forming is drawn at the right edge at full
size from the first reading (the louder of the slice's peak so far and the
previous bar's fall). The meter draws `clamp((v - 0.28) / 0.47) ^ 1.2` of its
height. Bars are 3 px pills 6 px apart and at least a 3 px dot; loud bars warm
from grey toward eggshell. Within 40 px of the left edge bars pinch to 30% of
their height, and they fade out over the left 10%. It scrolls continuously,
stepping under reduced motion. A pending "Transcribing…" breathes (opacity 1 to
0.4 and back every 2 s), still under reduced motion.

On the web the recording runs through the call's microphone processing
(`captureMicrophone`: DPDFNet-8 HR with its DPDFNet-2/RNNoise fallbacks, the
call's "Voice enhancement" strength, default 25%, and its echo/gain choice), so
the file, meter and waveform are what listeners hear. It borrows the call client's
noise assets and spare model worker. During a call, or when processing cannot
start, it records with the browser's echo cancellation, suppression and gain
instead; the recording bar's `data-processing` attribute names the path.

Native clients record through their call chains too, at the saved "Voice
enhancement" strength with input gain 100%. Android uses AudioRecord
(VOICE_COMMUNICATION, 48 kHz, device AEC/NS where present) through local-only
denoise entry points that never touch publication epochs and refuse input past a
40 ms backlog, then MediaCodec AAC into MPEG-4. Apple feeds an AVAudioEngine sink
node through `CaperVoiceNoteProcessor` (DPDFNet-8 HR or RNNoise, then
`CaperProcessVoice`) into AAC; on iOS outside a call the session uses `.voiceChat`
with voice processing, and during a call it shares the call's session. Desktop
runs each frame through the call's `ProcessedVoice`. Each falls back to its
simpler recorder (Android MediaRecorder, Apple AVAudioRecorder, desktop gain only)
when processing can't start, logging one line without audio.

### Platform status

| Platform | Record | Play + transcript |
| --- | --- | --- |
| Web desktop and mobile layout | MediaRecorder of the call's processed microphone (browser processing during a call or as fallback); click/tap dictates, hold on touch or "Voice note" records | Player, seek, one at a time, 3-line transcript |
| Android | Processed AudioRecord to AAC/M4A (MediaRecorder fallback); tap dictates, hold records | Media3 player, transcript |
| iOS / macOS | Processed AVAudioEngine to AAC/M4A (AVAudioRecorder fallback); tap dictates and hold records (iOS), click dictates (macOS) | AVPlayer, transcript |
| Desktop (Windows, Linux) | Call-processed audio to bundled FFmpeg AAC/M4A (WAV fallback); click dictates | Bundled FFmpeg audio-only player |

The ignored Postgres tests `voice_messages_store_aac_and_deliver_transcripts`
and `dictation_returns_text_and_keeps_none_of_it` exercise the full server paths
against a fake provider; watch the first production voice messages and
dictations in `ai_calls`.

## GIFs and stickers (KLIPY)

Mobile web, iOS and Android composers have one **smiley** button opening
**Emojis / GIFs / Stickers**, defaulting to Emojis, in channel, DM, thread and
profile-card composers. Emoji selection inserts at the caret; choosing a GIF or
sticker sends it at once as its own message, keeping the typed draft and staged
files. Media tabs show an unavailable message without KLIPY. Desktop keeps
separate emoji and **GIF** controls; the GIF control is hidden until the API has
`KLIPY_APP_KEY`. Media tabs use "Search KLIPY" (KLIPY requires that placeholder)
and trending results until something is typed.

KLIPY's [integration requirements](https://docs.klipy.com/integration-requirements)
shape the design:

- Search, trending, share and media requests go straight from each device to
  `api.klipy.com` and `static*.klipy.com`. The API never proxies, caches or
  re-hosts KLIPY media, so KLIPY sees the viewer's IP address and user agent.
- Results keep KLIPY's order and mix, ads included. Filtering happens only in the
  Partner Panel; we request `content_filter=off` and filter nothing.
- A sent GIF keeps KLIPY's media URLs exactly as returned.

### API contract

`GET /api/klipy` (signed in) returns `{"appKey","customerId"}`, or 404 when GIFs
are off. The app key is KLIPY's client key and ships to devices by design.
`customerId` is 32 hex characters, an HMAC of the account id keyed from
`AUTH_SECRET`: stable per account and meaningless to KLIPY.

Sending adds an optional `gif` to the usual message body. It never travels with
`attachmentIds`, and text may then be empty:

```json
{"clientMessageId":"…","text":"","gif":{"provider":"klipy","kind":"gif",
 "slug":"hello-hi-662","title":"Hello","width":498,"height":498,
 "gif":"https://static.klipy.com/…/a.gif","webp":"https://static.klipy.com/…/a.webp",
 "mp4":"https://static.klipy.com/…/a.mp4","still":"https://static.klipy.com/…/a.jpg"}}
```

The API stores it as `content.gif` (`content.type` stays `"text"`, so older
clients show the text alone). Rules, shared by the API (`apps/api/src/klipy.rs`)
and every client through `shared/klipy/cases.json`:

- `provider` is `klipy`; `kind` is `gif` or `sticker`.
- `slug` is 1–200 letters, digits, `-` or `_`; `title` is at most 200 characters
  without control characters.
- `width` and `height` are whole numbers from 1 to 4096.
- At least one of `gif` and `webp` is present.
- Every URL is at most 2048 characters: HTTPS on `klipy.com` or a subdomain, with
  no credentials, port or fragment. A query string is fine.

Clients render `content.gif` only when it passes these rules; otherwise they show
the text alone. Edits change only the caption. Forwards copy it. Reply quotes and
push notifications of a captionless GIF read `GIF` / `Sticker` and `Sent a GIF` /
`Sent a sticker`. The retry hash covers the text and the GIF.

### KLIPY requests (from the device)

Base `https://api.klipy.com/api/v1/{appKey}/`, with `{kind}` either `gifs` or
`stickers`:

| Purpose | Request |
| --- | --- |
| Trending | `GET {kind}/trending?page=N&per_page=24&customer_id=…&content_filter=off&locale=xx` |
| Search | `GET {kind}/search?q=…&page=N&per_page=24&customer_id=…&content_filter=off&locale=xx` |
| Share (after a successful send, best effort) | `POST {kind}/share/{slug}` with JSON `{"customer_id","q"}` (`q` is the search text, empty for trending) |

`locale` is the device region as ISO 3166 alpha-2. Searches wait 300 ms after
typing stops, and pages load as the grid nears its end while `has_next` is true.

Parsing (`parse` in `shared/klipy/cases.json`) keeps KLIPY's order:

- **Ad items** (`type: "ad"`) need a non-empty `content` HTML string. They become
  `{type:"ad", html, width, height}`; the size is null when absent or not positive.
- **Other items** need a valid slug.
  - The sent GIF comes from the first of `file.md`, `file.hd`, `file.sm`: `gif`,
    `webp` and `mp4` from it, and `still` its `jpg`, or `png` for stickers.
  - URLs that break the rules are dropped. Width and height come from the size's
    first valid `gif`, then its `webp`; without them the item is skipped.
  - `kind` is the tab that requested it. Control characters are stripped from the
    title, which is then cut to 200 characters.
  - The grid tile comes from the first of `sm`, `xs`, `md`, `hd`: its `webp` or
    else `gif`, plus a `jpg`/`png` still when that size has one.

### Ads

Ads come on with the Ads API toggle in the Partner Panel, and KLIPY serves them
only to mobile user agents.

- **Who asks for them:** Android, iPhone and the touch-layout web picker add
  `ad-min-width=50`, `ad-max-width=<grid width in CSS px or dp>`,
  `ad-min-height=50` and `ad-max-height=250`, plus the device fields they can read
  without permission prompts: `ad-os`, `ad-osv`, `ad-make`, `ad-model`,
  `ad-device-w`, `ad-device-h`, `ad-pxratio` and `ad-language`. Native apps send
  their WebView's user agent (Android `WebSettings.getDefaultUserAgent`, iOS
  `WKWebView` `navigator.userAgent`). Carrier fields, advertising IDs and location
  are never sent, and no tracking prompt is shown.
- **Where they don't appear:** desktop web, macOS, Windows and Linux omit the ad
  parameters.
- **How they show:** an ad fills a full-width grid row labelled **Sponsored**,
  sized to the ad or else 100 px tall. The HTML renders in an isolated web view (a
  sandboxed `iframe srcdoc` with scripts and popups but no same-origin access on
  the web; a non-persistent WebView on Android and iPhone). Taps open in the
  system browser, and an ad can never reach Caper's session.

### Rendering

- **Size:** a GIF shows below any text at its aspect ratio, at most 300 px on its
  longer edge (stickers 160 px). Stickers have no background.
- **Format:** the web prefers a muted, looping, inline `mp4` for GIFs and the
  `webp` for stickers; a failed MP4 falls back to the animated image. Native apps
  use the animated format their decoder supports (`webp` or `gif`); Windows/Linux
  decode both within the frame-memory budget.
- **Opening:** clicking or tapping opens the media viewer. Provider URLs stay
  external, never re-signed through Caper.
- **Reduced motion** shows `still` (or the first frame).
- **Labels:** the accessible label is the title, or `GIF` / `Sticker`.
- **Failures:** a GIF that fails to load leaves a quiet "GIF unavailable"
  placeholder.

### Configuration and rollout

| Variable | Where | Meaning |
| --- | --- | --- |
| `KLIPY_APP_KEY` | API | KLIPY Partner Panel key (letters, digits, `-` or `_`). Unset or malformed disables mobile media tabs and hides desktop GIF controls. Needs `AUTH_SECRET`. |

1. Create the key in the [Partner Panel](https://partner.klipy.com/api-keys) with
   the Ads API on. A test key allows 100 requests an hour for the whole app;
   request production access before real traffic.
2. Add `KLIPY_APP_KEY` to `staging/apps/caper` and `production/apps/caper` in AWS
   Secrets Manager.
3. Roll the API so it reads the new key.
4. Set the Partner Panel content filter to off, matching `content_filter=off`.

### Platform status

| Client | Picker | Ads |
| --- | --- | --- |
| Web | Channel, thread and profile-card composers; popover, or bottom drawer on touch layouts | Touch layouts |
| Android | Channel, thread and profile-card composers; unified bottom sheet | Yes |
| iPhone / macOS | Channel, thread and profile-card composers; sheet / popover | iPhone only |
| Desktop (Rust) | Channel/DM and thread composers; popup | No |

The shape of KLIPY ad objects is undocumented beyond `type` and `content`.

## Message reactions

Desktop web exposes **Add reaction** on message hover or keyboard focus; clicking
a message or reaction does not keep the toolbar visible after the pointer leaves.
Reaction chips have no outer focus ring; keyboard navigation uses an inset cue.
Mobile web, Android and iPhone hide the per-message add button: press and hold a
committed message to open a bottom drawer with five fixed quick reactions
(👍 ❤️ 😂 🎉 👀), the searchable emoji picker, **Copy text** and **Copy message
ID**. Copy uses the exact text or the public message `id`, never the client retry
UUID or channel-local sequence. Read-only previews allow copying but not reaction
mutations. Web offers a keyboard/screen-reader message-actions button; iPhone
provides a VoiceOver action and Android a labelled long-click action. macOS and
Rust desktop keep their hover reaction controls.

Reactions appear below the message with counts; your own chips have a terracotta
tint and `aria-pressed=true`. Clicking a chip adds or removes your contribution,
as does a quick reaction; choosing an emoji in the picker only adds it. Pending
messages cannot be reacted to. Every client applies additions/removals
immediately, without a saving label, spinner or disabled controls. Requests
serialize per message while the newest local choice stays visible over incoming
snapshots. A failed current choice rolls back only your contribution to the
latest server state and offers a retry; superseded failures do not undo a newer
choice. Cached history and replay cursors hold authoritative state, not optimistic
counts.

Web, macOS and Rust desktop group the message hover actions in one neutral,
rounded, bordered bar, kept visible while its menu or picker is open; thread rows
omit Reply in thread. Narrow web, Android and iPhone use message-actions sheets.
Thread actions use a thread-spool glyph and Reply a longer curved arrow;
`shared/message-icons.ts` supplies both to web and the native SVG exporter, and
the icon parity test checks Android's vectors. On macOS the reaction picker is a
352 × 420-point popover anchored to the fixed conversation header, with Cancel,
Escape and outside-click dismissal.

### Emoji picker

The picker offers standard Emoji 15.0 artwork, self-hosted from `@twemoji/svg` and
attributed at `/emoji/NOTICE.txt` (Twemoji graphics, CC BY 4.0). The web build
verifies artwork for every offered emoji and makes no emoji-CDN requests. The
picker is part of the chat bundle. Signed-in writable web chat preloads the first
128 smileys/people images when chat mounts; hovering or focusing **Add reaction**
reuses that preload, and hovering or focusing a category tab preloads its first
128 images. Preloads are shared per category; failures evict only that category.
The web grid uses React Virtuoso (not the emoji library's IntersectionObserver
gate). Category selection renders only that category, resets scrolling and clears
search; search covers all eight categories. Versioned `/emoji/twemoji-15/**`
artwork is served with a one-year immutable cache, so changed artwork must use a
new path. Category tabs sit above search with a terracotta underline for the
selected one.

Native clients have the same eight categories and search behavior, using bundled
raster sprite sheets generated from the same artwork with
`node scripts/native-emoji.mjs` (needs npm dependencies and ImageMagick 7). Android
and Rust read `shared/emoji`; SwiftPM needs a generated copy inside its target,
checked byte-for-byte by CI, including licenses. Rust desktop's fixed 40px cells
fill the picker width and match the virtual scroller's row height.

Picker names use dashes (`grinning-face`); catalogs also include underscore and
spaced aliases. `apps/web/src/chat/emoji-labels.json` supplies the CLDR short name
where the package's last name is slang or a category; regenerate it with
`node scripts/emoji-labels.mjs`, then `node scripts/native-emoji.mjs
--catalog-only`. Composer suggestions rank exact, prefix, keyword and substring
matches, preferring shorter names. Names are labels, not stored identifiers.
Message text does not convert `:name:` shortcodes. Custom emoji would need an
authorized catalog, stable asset IDs, API support alongside Unicode, and rendering
in every client; the API accepts only standard Unicode emoji.

### Reaction API

`PUT /api/chat/channels/{channel}/messages/{message}/reactions` accepts
`{emoji,active}` and `X-Caper-Chat-Token`. It checks the same channel, member,
private grant, channel participation (`channel_joins`) and session permissions as
sending. Readable previews keep reaction snapshots but cannot add, remove or retry
reactions; even no-op writes require participation, and a write queued behind a
leave rechecks permission after acquiring the space lock. DM reactions require one
of the two active participants. Unicode qualification variants are canonicalized;
arbitrary text, multiple emoji and emoji newer than 15.0 are rejected. Active
membership is unique by message, canonical emoji and user, so separate chat
sessions cannot inflate counts. Removing a reaction sets `deleted_at`; adding it
again inserts a new row. Only public `users.external_id` values appear in
`authorIds`.

The response and outbox use
`{type:"message.reactions",schemaVersion:1,channelId,seq,messageId,reactions}`,
each reaction `{emoji,authorIds}`. `seq` participates in the ordered channel
stream. History contains the current `reactions` and `reactionSeq`; the message's
own `seq`, timestamp and content do not change. A no-op returns the current
revision (or `"0"`) without allocating an event. HTTP snapshots never advance a
replay cursor, and per-message revisions stop stale replies overwriting newer
reactions. A full history resync keeps older cached pages only when the refreshed
messages account for every sequence since the applied cursor (or the head is
unchanged); otherwise cached pages are discarded. A visible DM marks sequenced
reaction events read. Reactions do not generate push notifications.

Limits: 50 emoji kinds and 1,000 total contributions per message, and 60
mutations per actor/channel/minute (no-op retries are free). Reactions, message
snapshots, sequence allocation and outbox commit together.

### Who reacted

Names load on demand from
`GET /api/chat/channels/{channel}/messages/{message}/reactions`, authorized like
history (read-only previews and DMs included). The response lists each emoji in
snapshot order with its active reactors in reaction order (`id`, `username`,
`displayName`, `avatarId`) and the message's `reactionSeq`; clients cache per
message and refetch when `reactionSeq` changes.

Every client uses the same wording: "You, Alex, Maya and 2 others reacted with
:thumbs-up:" (you first, up to three names, then a count; the dash-separated
catalog name, or the glyph when none exists). Until names load, a count is shown.
Pointer platforms (web, macOS, Rust desktop) show a tooltip with a large emoji on
hover or keyboard focus. Touch platforms open a Reactions sheet on press-and-hold:
one tab per emoji with its count, then each person's avatar, name and @username.
On web, right-click opens the same panel as a popover, and message actions include
**View reactions**. A tap or click still toggles your own reaction. Clients keep
known names across revision refreshes, project them onto the displayed reactor
IDs, and name your optimistic contribution "You" immediately.

The parity fixture serves this endpoint, and its `incomingReaction` control
accepts an optional `userId`. Desktop has `--fixture parity-reactions`.

### Channel participation and private consent

Channel access and participation are separate. `channel_members` is the private
authorization grant; `channel_joins` records the conversations a member has
chosen. The sidebar shows joined channels. Browse channels searches the accessible
directory and opens read-only previews without joining, preparing voice or
requesting a microphone. Public history and live text subscriptions are readable
before joining; sending, typing and every channel media operation require
participation. Joining a channel never enters its voice call.

`Channel.joined` is returned by space details and channel mutations; clients
default a missing field to true. Public join/leave uses POST/DELETE
`/api/spaces/{space}/channels/{channel}/membership`. Public leave removes the
sidebar entry but keeps preview/rejoin access. Private non-owner leave also
removes the grant and warns that another invitation is required. Both set
`deleted_at`. The owner keeps implicit administration and can rejoin from the
directory. Leaving a channel stops its local call, not an unrelated call.

Owner POST `/api/spaces/{space}/channels/{channel}/members` sends a private
invitation, not a grant. GET returns active `members` and pending `invitations`;
DELETE `/members/{user}` cancels a pending invite or removes a grant and
participation. The recipient's space detail adds `channelInvitations` with only
channel metadata and the owner's public `inviter`. The pending channel is absent
from the directory, and its history/media stay unauthorized. POST/DELETE
`/invitation` accepts/declines; acceptance grants and joins in one transaction.
Stale, revoked, declined or expired invitations cannot be accepted. Unauthorized
resources return generic 404s.

Channel invitations expire after seven days and keep a 24-hour pair cooldown after
decline, cancellation or removal. They share the space-invitation attempt budget
(20 per owner per ten minutes), with live-pending limits of 50 per recipient and
100 per channel. Invitees must already belong to the space.

Accepting a username invitation joins one public starter channel, preferring
`general`. Invite links use their selected public channel, falling back to the
oldest public channel. Creating a channel joins only its creator. Space removal
clears participation and revokes channel invitations. Clients discover
invitations and joins on account/space reload and after mutations; the browser
also refreshes on focus/visibility and every 15 seconds.

## Message pins

Pins are channel-wide, not personal bookmarks. Any joined channel member can pin or
unpin any message; the two participants can do the same in a DM. Private grants and
active space membership still apply. Preview readers can see pins but cannot change
them, including no-op writes. Actions ask no confirmation. "Pinned by …"
attribution appears above each pinned message in the timeline, threads and Pins:
"Pinned by you" when the pinner's account ID matches the viewer, otherwise the
pinner's name, which opens their profile. The header says Pins without a count.

Desktop message controls appear on hover or keyboard focus: two 14px glyphs in
24px targets, 2px apart. Touch clients use long-press actions; narrow web reserves
text space beside its 44px action target. Browse channels lives in the space-name
menu. Pins opens a dismissible overlay while the conversation stays mounted
(outside click or Escape on desktop/web; Close on mobile; a macOS popover and an
iOS sheet). Each row shows the author, avatar, date/time, text and attribution,
with Go to message; Unpin lives in the message-actions menu.

Channel and thread history accept exactly one of `before`, `after` or `around`.
`around=<message ID>` returns the target plus up to 30 messages before and 30
after, ascending, with `hasMore` and `hasNewer`. Limits count messages, not
edit/reaction sequence events. Anchors resolve inside the authorized
channel/thread; others return 404. `after=<sequence>` returns the next 50 rows
ascending. Clients scroll to and highlight the target, expose older/newer paging
and keep a bounded window so latest rows are never spliced across a history gap.
Context snapshots merge independently of the replay cursor. Pinned non-broadcast
replies navigate into their thread. Back to latest, and sends from historical
context, reload the latest page.

`PUT /api/chat/channels/{channel}/messages/{message}/pin` accepts
`{active:boolean}` and `X-Caper-Chat-Token`; clients cannot supply a pin author.
The response and outbox event are
`{type:"message.pin",schemaVersion:1,channelId,seq,message}`. The message snapshot
has nullable `pin:{author,createdAt}` and decimal-string `pinSeq`; the event's
`seq` equals `message.pinSeq`, and the message's own `seq`, content and time are
unchanged. Pin and reaction revisions merge independently, including into the
separate pinned-message snapshots (a newer pin revision must not restore older
reactions). HTTP acknowledgements do not advance replay/read cursors.
Authoritative history clears pins removed while offline; stale pages must not
restore an unpin.

Every first history page includes complete `pinnedMessages`, newest pin first,
captured under the same channel lock as its cursor; older pages return an empty
list, and clients keep the first page's pins plus live events. Old pins never
enter the timeline page. At most 100 active pins per channel and 60 actual
mutations per actor/channel/minute; idempotent retries keep the original
attribution and consume neither an event nor budget. Pin snapshots, rate-limit
activity, sequence allocation and outbox commit atomically. Pins send no system
messages, sounds or push.

For desktop hover assertions in browser scripts, set `MESSAGE_TEST_CHROME` to a
Chromium wrapper adding
`--blink-settings=primaryHoverType=2,availableHoverTypes=2,primaryPointerType=4,availablePointerTypes=4`;
`scripts/fine-pointer-chromium.sh` is one (`CAPER_CHROMIUM` picks the browser).
Stock headless Chromium reports no hover device.

## Message threads

Reply in thread sits beside desktop message reactions and in message actions.
Roots show a reply count and up to five distinct repliers' avatars (first-reply
order). Desktop/wide clients use a right panel; narrow web, Android and iPhone take
over the conversation screen. The selected root has a slightly stronger pin-gold
wash. No nested threads: replying from a broadcast opens its original root. The
same contracts apply to DMs.

On phones, rightward drags over thread history/header move the thread over the
channel beneath it. Releasing past 72px/pt/dp returns to the channel; short or
cancelled drags settle back, using the shared 140ms fast timing. Vertical
scrolling, selected text and composer gestures stay separate; drafts survive.
iPhone read-only message text can start a swipe too.

The unchecked **Also send to channel** option publishes one reply in both places,
not a copied root. IDs, content and reactions are shared. Drafts and the checkbox
are per root for the session; the checkbox clears after each sent reply. Retries
freeze the root and broadcast choice, so an uncertain outcome cannot become a
duplicate or a channel message. A delivery problem stays with its reply in the
thread; the channel offers **Review reply** only when that thread is closed or
another is open. Replies use the existing participation and read permissions.

Thread composers accept the same files as ordinary messages, including
attachment-only replies, through each client's regular pickers, clipboard and
drop. Files belong to their channel and thread root: closing/reopening a thread or
switching conversations keeps them, and sending in one composer does not consume
another's files. The 10-file limit applies per composer. Thread composers share
the channel composer's `:` emoji and `@` mention suggestions.

`POST /api/chat/channels/{channel}/messages` takes optional `threadRootId` and
default-false `broadcast` (and `attachmentIds`). The root must be a same-channel,
top-level message. `GET /api/chat/channels/{channel}/messages/{root}/thread?before={seq}`
returns `{root,messages,cursor,hasMore}`; pages hold 50 replies ascending and
`before` is exclusive. Channel history contains roots and broadcast replies only.
The thread response's channel head must never advance a client's replay cursor.

Each reply's `message.created` event includes `threadRootId`, `broadcast` and
`thread:{replyCount,participants,seq}`. The parent summary and reply/outbox commit
atomically under the channel lock. `thread.seq` is a summary revision, **not a
content revision**. Summary, reaction and pin revisions are independent. Clients
merge summaries monotonically, keep one broadcast identity, and keep roots loaded
through thread GET out of channel pagination until channel history loads them.
Reconnect refetches the active thread and fences obsolete responses.

### Unread thread replies

Every client shows a gold (`#E4C76A`) dot and **Unread** beside the terracotta
reply count when another account replies in a thread the viewer started or
replied to. Participation comes from authorship, not the displayed avatars. Own
replies and blocked/deleted authors create no unread state; sending a reply does
not acknowledge earlier replies.

`GET /api/chat/channels/{channel}/threads/unread` returns
`{threads:[{rootId,seq}]}` with the latest qualifying unread reply sequence.
`PUT /api/chat/channels/{channel}/messages/{root}/thread/read` accepts `{seq}` and
returns 204. Both use account authentication and conversation read permissions.
`thread_reads` stores a per-account/root cursor, advanced monotonically and
clamped to the thread's latest reply. Personalized state never enters shared
payloads or replay.

Clients acknowledge rendered replies only in an active, visible thread's latest
window; channel visits, hover prefetch and older windows do not clear the badge.
Failed acknowledgements keep it for retry, and delayed responses cannot erase
newer unread replies. Live events refresh the list; the 15-second foreground
refresh reconciles reads from another device. Push and channel unread indicators
are unaffected.

## Live message forwarding

**Forward message** is one action, always live. Signed-in readers can forward a
readable message into a joined, writable channel in any of their spaces or an
existing DM, with an optional note. The picker discloses that sharing includes
future edits, reactions and replies and that destination readers can re-forward
it. There is no snapshot/live choice or source-owner consent step.

Destination readers see a read-only original and its direct conversation replies
without source membership. This is an intentional disclosure boundary, not a
grant to the source channel: ordinary source history, threads, reaction mutation
and edit/version routes keep their access requirements. Sharing a single reply
does not expose its parent or siblings. Destination replies are an independent
thread. Re-forwarding flattens to the canonical original, without intermediate
notes or destination replies. Future source replies share automatically. Do not
assume a private channel's content stays inside that channel once a member
forwards it.

The API stores `messages.forward_source_id`; it does not copy the original into an
independently editable message. The destination wrapper has its own author,
optional note, creation sequence, reactions, pins and thread summary.
`@mentions` in the note resolve like a message's but never notify; the forward
notifies as an ordinary message. Shared source snapshots omit source pins and
parent linkage. Removing destination membership/grants revokes the scoped read.
Deleted source spaces/channels/messages render an unavailable original.

### Forwarding API and replay

- `GET /api/chat/forward-destinations` returns writable joined channels and
  existing DMs for the signed-in account.
- Pickers support multiple destinations. Search matches case-insensitive words
  across space and channel/DM names in either order (including `#channel`);
  filtering keeps hidden selections. Each row keeps the space subtitle below its
  name. Clients send sequentially with a separate UUID per destination. Confirmed
  destinations leave the selection; an ambiguous failure freezes the remaining
  keys and note for retry; a definitive rejection unlocks only the remaining
  selection/note. This is not an atomic batch.
- `POST /api/chat/channels/{destination}/forwards` uses `X-Caper-Chat-Token` and
  `{sourceChannelId,sourceMessageId,clientMessageId,text?}`. The source must be
  readable and the destination writable. Retries reuse the exact UUID and note;
  conflicting UUID reuse returns 409. Shared payloads are always loaded by the
  server. Every change to an original rewrites each of its forwards, so one
  original accepts at most 100 forwards, 10 per account; beyond that a new forward
  returns 409 `this message can't be forwarded again` (retries of committed
  forwards still succeed). Sends and forwards share the per-account rate limit.
- `GET /api/chat/channels/{destination}/messages/{wrapper}` hydrates the forward
  like history, so a deleted source is never served from the stored snapshot.
- The message has `forward:{message,seq}` (original or null, and source snapshot
  cursor) and `forwardSeq` (destination projection revision).
- `GET /api/chat/channels/{destination}/forwards/{wrapper}/thread?before={seq}`
  authorizes through the destination wrapper and returns
  `{root,messages,cursor,hasMore,source}`, with 50 ascending replies and exclusive
  `before`. Its source cursor must not advance destination replay. `source` is
  null without source-channel access; otherwise `{spaceId,message}` with the
  original's real thread parent (`spaceId:null` for a DM). Clients check it on
  every **View conversation** click and navigate to the original (opening its
  thread for a reply); destination-only readers keep the read-only excerpt view.
- The outbox publisher projects source creation/reactions/edits into durable
  destination `message.forward` events (schema version 1,
  `event.seq == message.forwardSeq`). Projection and publisher completion commit
  together; replay is idempotent. Source events are never granted to
  destination-only readers.
- Clients merge source snapshots and destination event revisions independently,
  including unloaded and pinned wrappers. An open shared view refetches its loaded
  reply range on source updates.

Wrappers (including their notes) cannot be edited: the API author predicate
rejects `messages.forward_source_id IS NOT NULL` and every client's edit gate
excludes them. Source originals stay author-editable; destination readers see
projected edits but not source edit history.

## Feedback in place

`@feedback`, `@caper` or any other team name (`@capers`, `@caperchat`,
`@caper_feedback`, `@caper_support`, `@support`, `@team`, `@staff`, `@admin(s)`,
`@administrator(s)`, `@mod(s)`, `@moderator(s)`; all reserved usernames) in any
channel, thread, DM, message request or personal note asks the Caper team to look
at that conversation. Only accounts can report: a guest's alias, an alias added by
an edit, and an alias in a forward's note stay plain text. Nothing is copied or
moved. The message shows its alias as a mention pill and is marked **Caper team
has been notified 🫡** for everyone there.

- **Report.** The send transaction writes `feedback_reports` (message, channel,
  reporter, and the thread replies belong in: the message itself when top-level).
  Reports are kept.
- **Access.** `feedback_access(channel, account)` is true for an active
  `feedback_team` member once that channel has any report. The team then takes
  part like a joined member: every read path (history, threads, single messages,
  versions, reactors, gateway subscription and replay), sending, replies, files,
  reactions, editing their own messages, typing, pins, voice and forwarding into
  the conversation. They are not listed as members and keep no read markers.
  Responses carry `"visitor": true` when the reader isn't a member, which clients
  use only to name the space in the header. A visitor's DM reply neither accepts a
  pending request nor is affected by the participants' blocks. Removing someone
  from the team (`deleted_at`) ends their access everywhere at once.
- **Answering.** Team members' messages carry `author.admin`, stamped at send time
  and badged **ADMIN** on every client. Top-level reports open at the message in
  its conversation; reports made in a thread reveal that thread. The team can
  start a thread; an @mention reaches the reporter as usual.
- **Notifications.** Kind `feedback`, title `Sender · Feedback`, for every team
  member, ignoring mutes. See [notifications](notifications.md#who-gets-a-notification).
- **Inbox.** `GET /api/feedback[?before=cursor]` (404 unless on the team) lists
  reports newest first, 50 per page, with reporter, place, a 280-character preview,
  reply count, `archived` and `answered`. `answered` is computed on read: true
  while the newest message in the report's thread is from the team. Clients group
  reports by thread. `PUT /api/feedback/archive {channelId, threadRootId, archived}`
  archives or restores a whole thread for the team (`archived_at` on its reports);
  anyone else posting in the thread brings it back, and `?archived=true` lists
  archived threads. `GET /api/account/me` adds `feedbackTeam: true` for team
  members.
- **Waiting badge.** `GET /api/spaces/unread` also returns `feedbackWaiting`: the
  total unarchived conversations waiting for a team reply, grouped by
  channel/thread, independent of inbox pagination (zero for non-team accounts).
  Every client shows a terracotta count on Feedback through its unread refresh
  loop. Viewing does not clear it; replying or archiving changes it on the next
  refresh. Failed refreshes keep the last count; zero hides it.

**Messaging Caper.** Any team name in **New message** opens an accepted DM with the
Caper account (`users.team_account`, username `caper`, no email, so nobody signs
in as it; the API maps every alias to it). Each top-level message sent there is a
report and lands in the inbox as its own thread; replies stay part of it. The team
answers as itself, in the thread or the DM. The DM's `peer.team` is true, and
clients replace the empty state's privacy line with *Messages here go to the Caper
team.* When the team posts in any DM it visits, everyone in the conversation is
notified as for a DM message.

**Privacy.** Tagging the team is consent for it to read that conversation,
earlier context included. The `@feedback` suggestion says so: *Caper team · can
read and reply here*. Access is per conversation, never per space; the team
cannot browse conversations nobody reported.

**Operating the team** (run with the migration role; the runtime role can only
read):

```sql
-- Add
INSERT INTO feedback_team (user_id) SELECT id FROM users WHERE username = '<username>' AND deleted_at IS NULL;
-- Remove (kept for history)
UPDATE feedback_team SET deleted_at = now() WHERE user_id = (SELECT id FROM users WHERE username = '<username>') AND deleted_at IS NULL;
-- Who is on it
SELECT u.username, t.created_at FROM feedback_team t JOIN users u ON u.id = t.user_id WHERE t.deleted_at IS NULL;
```

Migrations grant the runtime role `SELECT` on `feedback_team`,
`SELECT, INSERT, UPDATE` on `feedback_reports` and its sequence, and `EXECUTE` on
`feedback_access`, which every access check calls.

## Message editing

Message actions and pins on every client offer **Edit message** to the original
author account while it can participate in the conversation: channel messages,
thread roots, hidden and broadcast replies, and DMs. Ownership follows the
original session's account, so rotating a chat capability does not lose
authorship. There is no age limit or space-owner override. Save uses the normal
4,000-code-point, nonblank text rules (text may be empty when files remain);
desktop Ctrl/Cmd+Enter saves. Conflicts and failed saves keep the draft;
**Discard draft and load latest** explicitly replaces it. Channel/account/access
changes close obsolete dialogs.

An **edited** indicator opens retained versions, also available from message
actions and pins. Everyone with current read access can inspect the original and
later versions, including members previewing an unjoined channel; the edit dialog
explains this before saving. History shows the previous version on the left and
the current on the right, with changes highlighted; **View previous
versions** selects an older change without replacing that pair. Web uses Pierre
Diffs (`@pierre/diffs`, `lineDiffType: "word"`): lines are compared first, each
changed line paired with its counterpart is compared by words, and a changed
line without a counterpart is highlighted whole. Native clients highlight the
same characters through `shared/messages/history_diff.rs`, a port of that
pipeline, and run `shared/messages/history-diff-cases.json`, which web checks. There is no version purge or restore-to-version action.

### Editing API and durable delivery

- `PUT /api/chat/channels/{channel}/messages/{message}` accepts the sending
  capability and `{text,expectedRevision}` and returns the current message. An
  optional `attachments` list also changes photos, videos and spoilers
  ([Editing attachments after sending](#editing-attachments-after-sending)).
  Legacy messages implicitly have `revision:1`. Changes add `revision`, `editedAt`
  and decimal-string `editSeq`. Same-text saves are no-ops; an exact retry of the
  preceding successful edit creates no new version/event. A stale draft returns
  409. The limit is 30 actual edits per author/channel/minute.
- `GET` at that URL reloads the current snapshot. `GET .../versions?before={revision}`
  returns `{messageId,versions:[{revision,content,createdAt,filesChanged?}],hasMore}`,
  newest first, 50 per page with exclusive `before`. Only the author sees files an
  edit replaced, with URLs. Reads recheck membership and private grants.
- `message_versions` is append-only (the runtime role has SELECT/INSERT only); the
  original content is captured lazily on first edit.
- The channel lock atomically commits version, current message, outbox event and
  channel head. Events are
  `{type:"message.edited",schemaVersion:1,channelId,seq,message}` with
  `seq == message.editSeq`. Creation `seq`/time, ID, idempotency hash, author,
  thread/broadcast identity, reactions and pins stay unchanged.
- Clients merge content by revision, including pinned/thread projections and late
  pages. Editing does not insert unloaded messages, chime or announce a new
  message. HTTP edit acknowledgements and history pages never advance replay
  cursors. Excess unloaded edit snapshots cause resync rather than silent eviction.

For Android source/unit work, use JDK 17 and an SDK with platform/build-tools 36
(the native build also needs its pinned NDK/CMake). `.agents/setup` provisions
these on x86_64 orbs; it does not install an emulator. This fits a 4 GB orb:

```sh
apps/native/android/prepare-fonts.sh
apps/native/android/gradlew -p apps/native/android --no-daemon --max-workers=1 \
  '-Dorg.gradle.jvmargs=-Xmx768m -XX:MaxMetaspaceSize=384m -Dfile.encoding=UTF-8' \
  :app:compileDebugKotlin :app:testDebugUnitTest
```

## Message deletion

Authors can delete their own messages on every client, from the message actions
after a confirmation. Nobody else can delete them; space owners have no moderation
delete.

`DELETE /api/chat/channels/{channel}/messages/{message}` (chat session token, like
an edit) is idempotent and returns a tombstone: `deletedAt`, empty text, no files
or mentions, a bumped `revision`, and `editSeq`/`attachmentsSeq` at the new
sequence. It is delivered as an ordinary `message.edited` event, followed by a
`message.pin` unpin when it was pinned, so older clients still blank the message
and drop its files. Current clients hide the deleted row entirely in channels, DMs
and threads, before grouping and date separators are calculated. Tombstones stay
in sync state and pagination; an open thread keeps its surviving replies when its
root is deleted. Replies quoting it carry `replyTo.deleted` with empty text and
show "Original message was deleted." with no "View original"; forwards of it
become unavailable.

Nothing is purged (`messages.deleted_at`). The content stays in
`message_versions`, the files are retired (readable only by the author), and
stored replay events and reply quotes are rewritten so replays never hand the
content out again. Edits, reactions, pins, quoting, forwarding and version history
are refused for a deleted message; thread replies under it are still allowed.
Deleted messages stop counting as unread and never notify. At most 60 deletions
per person per channel per minute.

Notifications already shown are taken back. Deleting writes a
`notification_retractions` row (operational, pruned after a week) for every device
a push reached, and a push delivered moments later gets one too. A worker beside
delivery sends them with the same lease, backoff and 24-hour lifetime:

- Android (FCM data): `{"kind":"message.deleted","deletedMessageId",...}` without
  `messageId`, so older releases ignore it; the app removes the message from its
  conversation notification, or the notification when nothing remains.
- iPhone (APNs): message alerts carry `apns-collapse-id: <messageId>`; the
  retraction reuses it with "This message was deleted.", no sound and the
  `passive` interruption level, replacing the original in place. The notification
  service extension also removes originals delivered before collapse ids existed.
- Running web, desktop and Mac clients get `notification.retracted` (`messageId`
  and its conversation) on the account `profiles` feed and close what they showed
  (best effort, not replayed).

What was read on a lock screen cannot be taken back. Message files are signed for
one to two hours (`CdnSigner::attachment_url`), and clients refresh links ten
minutes before expiry through `POST /api/assets/urls`, which no longer signs a
deleted message's files for anyone but its author, so others lose access within
about two hours. Cached copies remain.

## Message text: emphasis, links and grouping

Message content stays a string on the wire
(`content: {version: 1, type: "text", text, mentions}`). Clients interpret
emphasis before drawing mentions and links. The API resolves mentions against the
displayed text too, so formatting `@user_name` does not change whom a message
notifies. Stored text and edit-history diffs keep markers.

**Bold and italic.** Select text in a message composer, edit dialog or forwarding
note to use B/I, or press Ctrl/Cmd+B and Ctrl/Cmd+I. Recognized `**bold**` and
`_italic_` render as styled text while editing, without showing delimiters.
Formatting requires selected text. Different styles nest and can span lines.
Surrounding selection whitespace stays outside the markers; formatting cannot
expand a draft beyond 4,000 characters. Unmatched and backslash-escaped markers,
underscores in identifiers and URL contents stay literal. HTML, custom links,
headings, lists and other Markdown are not interpreted; older clients show the
source markers. All clients' parser tests share
`shared/messages/formatting-cases.json`; the API, desktop, Android and Apple share the Rust parser.

Selecting part of a URL disables formatting (including shortcuts); selecting the
whole URL is allowed. Selections the grammar cannot express (such as part of an
identifier) are rejected rather than showing styling recipients won't see. The web
composer uses ProseMirror for selection, composition and undo, converting to the
string draft at its boundary; native composers project the stored source into
styled text with offset mapping. Emoji/mention completion uses visible text and
keeps formatting. Web paste accepts plain text, not HTML or hidden link
destinations; copy exports visible text. Sending clears the web editor's undo
history. The formatting toolbar is one borderless bar whose buttons fill its
surface, using a background fill for hover, press and focus.

**Extending Markdown later.** Raw source is the persistence, transport and edit
contract; `FormattedRun` and its native equivalents are rendering-only. The shared
send/edit validation test preserves headings, code, quotes and lists verbatim, so
adding them needs no storage migration: add block parsing before inline parsing
and render block nodes separately. Code nodes must bypass emphasis, autolinks and
mention resolution; update the shared fixtures and API mention traversal with all
clients.

### Message links

`http://`, `https://` and `www.` URLs (case-insensitive) become links,
following GitHub Flavored Markdown's autolink literals: a link starts at the
beginning, after whitespace, or after `( [ { < " ' * _ ~`; runs to whitespace or
`<`; drops trailing `? ! . , : ; * _ ~ " ' >`, a trailing `&entity;`, and
unbalanced `)`/`]`; and needs a host of ASCII letters, digits, `_`, `.` and `-`
with at least two labels (three after `www.`) and no `_` in the last two. `www.`
links open as `https://`. Nothing else (`javascript:`, `mailto:`, `ftp:`, bare
domains) is linked, and visible text is never shortened. Links open in the system
browser; on web in a new tab with `rel="noopener noreferrer nofollow ugc"`. They
apply to message bodies in the timeline, threads, Pins and forwarded cards, not to
edit-history diffs or the composer.

The algorithm lives in `apps/web/src/chat/links.ts` and `shared/messages/links.rs`
(API, desktop, Android and Apple); every client's tests assert the
same segments for every case in `shared/messages/link-cases.json`. Change the rules
and that file together.

**Compatibility with richer messages.** Full Markdown and multi-part bot messages
should be added as optional fields on the same version-1 text content (a format
flag or a structured `blocks`/`embeds` list), with `text` always carrying a plain
fallback. Current clients ignore unknown content fields but reject unknown
`version`/`type` values. An expanded renderer should reuse the same bare-URL
detection, and explicit `[label](url)` and bot links should use the same
http/https-only, open-externally handling. Link previews are server-generated
`content.embeds` (see "Link embeds"), never part of `text`.

### Message grouping

A message is shown compactly, without avatar and name, when the
previous visible row in the same list (timeline or a thread's replies) is by the
same account, at most five minutes earlier, on the same day, and neither is a
blocked-message placeholder; "Replied to a thread" broadcasts and thread roots
always keep their header. Compact rows keep the avatar column empty and show the
time there on hover; an "edited" marker moves after the text. Grouping is
presentation only.

## Link embeds

A message's links get preview cards after it is sent: the message posts and
reaches everyone immediately, and the cards arrive a moment later for everyone in
the conversation, DMs and private channels included. The text is never rewritten
and nothing is reposted. Previews are built by Caper itself.

**Flow.**

1. The outbox publisher publishes `message.created` (or `message.edited`). For a
   live message with links it starts a background job (`apps/api/src/embeds.rs`);
   sending and delivery never wait for it.
2. The job picks the message's first five distinct links with the shared link
   rules (`embeds/links.rs`). A link written `<https://like.this>` stays
   clickable but gets no card.
3. Each link goes to the embed Worker (`apps/embed`, `POST /v1/unfurl`, Bearer
   `EMBED_API_SECRET`). The API checks the answer's shape and size again.
4. In one transaction the API stores the cards as `content.embeds` on the message,
   if it is still at the revision the job read (a newer edit runs its own job; a
   deleted message gets none), and on the outbox event replays read for that
   revision. Then it publishes an unsequenced `message.embeds` event
   `{type, schemaVersion: 1, channelId, messageId, revision, embeds}` on the
   typing topic, even when nothing changed or no link got a card, so a sender's
   provisional card is confirmed or removed.
5. Clients apply it when the revision matches. History, replays and pins read the
   stored copy.

An edit keeps cards for links it still contains and runs a new job for the rest.
Deleting a message removes its cards.

**Composer preview.** `POST /api/embeds/preview {url}` (signed-in accounts, 30 a
minute) returns `{embed}` for one link so the composer can show the card before
sending. The Worker caches answers for 30 minutes. Removing the card (×) sends the
message with `suppressEmbeds: true`, stored as `content.suppressEmbeds`; no job
runs and edits keep it. A card already shown at send goes with the pending row and
stays (locally) until `message.embeds` replaces or removes it, so the row has its
final height from the start. Cards that load while a row is on screen fade in
(`--motion-fast`, opacity only; none with reduced motion) and the timeline carries
the rows they push.

**Compatibility.** Older clients ignore `content.embeds`. Older desktop builds
treat any unknown chat event as a broken stream, so the gateway sends
`message.embeds` only to subscriptions that set `embeds: true` in `subscribe`;
every client that draws cards sends it.

**Players.** Web loads the provider's embed page in a sandboxed cross-origin
iframe only after Play (YouTube through `youtube-nocookie.com`; Twitch with
`parent` set to the page's host). Native apps load `play`
(`https://embed.caper.chat/play/{provider}/{id}`) in a web view: a page that holds
only that provider's player, because YouTube needs a real website referrer and
Twitch a `parent` domain. The page runs no script of its own, its CSP allows only
the three players' frames, and it cannot be framed. Apps load `play` only from
`embed.caper.chat` (or `embed.staging.caper.chat`), in a web view with no bridge
to app code, no persistent storage, and every navigation away opened in the system
browser.

**Embed format (version 1).** `{version: 1, url, site: {name, domain}, blocks}`.
`url` is the link as posted and is what the card opens. `site.domain` comes from
that URL, never from the page, and clients always show it. Blocks are drawn in
order; unknown block types are skipped:

| Block | Fields | Drawn as |
| --- | --- | --- |
| `author` | `name`, `handle?`, `url?`, `icon?` | small avatar, name and handle; opens `url` |
| `title` | `text`, `url?` | bold line; opens `url` |
| `text` | `text` | body text, plain (no Markdown or HTML), line breaks kept |
| `images` | `items: [{url, width?, height?, alt?}]` (1-4), `thumbnail?` | gallery; `thumbnail` means one small image beside the text |
| `video` | `url`, `poster?`, `width?`, `height?`, `durationMs?`, `gif?` | the client's own player playing a direct MP4/WebM; `gif` loops muted without controls |
| `player` | `provider` (`youtube`, `vimeo`, `twitch`), `id`, `play`, `poster?`, `width?`, `height?`, `start?` | poster with a play button; Play loads that site's player in place |
| `footer` | `text?`, `timestamp?` | small muted line (counts, date) |

Every image, avatar and poster URL is a signed `embed.caper.chat/m/...` URL: the
Worker fetches and caches the image, so sites never see who is reading. Direct
videos stream from the site itself (X's `video.twimg.com`), or for TikTok through
a Worker redirect to a fresh CDN address. Clients open every link in an embed with
the same http(s)-only, open-externally handling as message links, and treat every
string as plain text. Every client tests `shared/messages/embed-cases.json`.

**The Worker** (`apps/embed`, Cloudflare, `embed.caper.chat`):

- Runs outside the AWS network, so fetching posted links cannot reach the cluster,
  databases or the EC2 metadata service. It also refuses IP literals, internal
  names (`.local`, `.internal`, `.svc.cluster.local`, ...), non-default ports and
  credentials in URLs, and re-checks every redirect (at most five).
- Reads at most 1 MiB of HTML and stops at `</head>`. Images are at most 20 MiB
  and must be PNG, JPEG, GIF, WebP or AVIF; SVG is never served. Media responses
  carry `nosniff` and a `sandbox` CSP.
- Never renders anything from a page: tags become length-limited plain strings.
  Iframe players from arbitrary pages are ignored; only the three providers get a
  `player` block.
- TikTok and Instagram media addresses expire, so cards point at
  `/m/{sig}/tiktok/{id}/…` and `/m/{sig}/instagram/{code}/…`, which look up a
  fresh address on each view (cached ten minutes) and redirect video to it.
- Media URLs are signed with `EMBED_MEDIA_SECRET` (Worker only), so it is not an
  open proxy.

Site readers (`apps/embed/sites.mjs`):

| Site | Source | Card |
| --- | --- | --- |
| X / Twitter (and links from fix sites) | the public endpoint behind X's embed widget (`cdn.syndication.twimg.com`), unofficial | author, text, photos or MP4 video/GIF, quote, likes and replies |
| TikTok (including `vm.`/`vt.` share links) | the post page's own web-app data, unofficial | author, caption, video (re-resolved on each play) or photos, likes and comments |
| YouTube, Vimeo | official oEmbed | channel, title, `player` |
| Twitch | page tags | title, `player` (clip, VOD or channel) |
| Instagram (posts, reels, carousels) | the logged-out query Instagram's own web app makes for a post, unofficial; page tags as Meta's crawler if refused | author, caption, video or photos (re-resolved on each view), likes and comments |
| Facebook, Threads | page tags as Meta's crawler | title, description, image |
| everything else | OpenGraph / Twitter Card / `<title>` | site, title, description, image or direct video |

X, TikTok and Instagram use undocumented endpoints that change without notice
(Instagram's `doc_id` is retired from time to time), so
`node apps/embed/live-check.mjs` runs every reader against real posts (the `Embed
live check` workflow runs it nightly). Instagram limits logged-out requests per
address; if Cloudflare's addresses are limited, cards fall back to the page's
image and caption.

**Configuration.** API only: `EMBED_ORIGIN` (`https://embed.caper.chat`) and
`EMBED_API_SECRET` (at least 32 characters), together or not at all. Without them
nothing is unfurled and the preview route answers 503. In production both go in
the `production/apps/caper` secret, which the API loads through
`caper-api-account`; the gateway needs neither. The Worker:

```bash
cd apps/embed
npx wrangler secret put EMBED_API_SECRET --env production    # same value as the API
npx wrangler secret put EMBED_MEDIA_SECRET --env production  # a different random value
npx wrangler deploy --env production
EMBED_ORIGIN=https://embed.caper.chat EMBED_API_SECRET=... node live-check.mjs
```

Staging uses `--env staging` and `embed.staging.caper.chat`.

**Clients.** Every client draws cards, takes `message.embeds` live, previews the
composer's first link, can send without previews, and plays players inline.
Desktop loads `play` after Play in the operating system's web engine, only for the
player box (`wry`): WebView2 in process on Windows; on Linux WebKitGTK in the
separate `caper-web-player` helper, so the app never links GTK and starts without
it. The native view sits exactly over the card, as a child of Caper's window on
Windows and X11; Wayland cannot embed another process's surface, so it plays in
its own small window and the card offers Show window/Close. It hides, still
playing, while the box is partly out of view or anything egui paints (menus,
popups, tooltips, dialogs, the viewer) meets it, with the poster marked "Playing";
one plays at a time, and it closes on another Play, a conversation change, the
card staying out of view, or quit. Same rules as mobile web views: ephemeral
storage, no bridge, no devtools or downloads, autoplay, only `play` in the main
frame (WebView2 decides in its main-frame navigation handler; the WebKitGTK helper
stops any later main-frame load before it commits), and new windows and links open
in the system browser. Without WebKitGTK or the WebView2 runtime the card says
what to install and offers the browser. Desktop GIF-style videos play on click
rather than looping inline. Linux YouTube playback needs
`gstreamer1.0-libav`/`-plugins-bad`.

## Shared call state and rolling deployments

Account channels each have an isolated room key, capped at 12 participants.
Membership and history live in Postgres; active voice state lives in Valkey.
Typing indicators are transient Pub/Sub events.

- Valkey stores the call capability hash, Caper-to-Cloudflare session mapping,
  track/subscription metadata, mute/deafen state, leases, operation ownership,
  temporary join reservations, cleanup jobs, and the latest ephemeral TURN
  credentials for retry-safe renewal. Treat this store as secret-bearing: TURN
  passwords grant relay access until expiry. It never stores audio or SDP (only an
  offer hash for renewal replay validation). A successful join is not returned
  until its mapping is committed.
- Room state is a hash (`caper:{general}:v1:state` plus a `:channel:<id>` suffix)
  with one field per participant plus bounded metadata. Short WATCH/MULTI/EXEC
  transactions arbitrate concurrent pods; only changed fields are written.
  Cloudflare calls never run inside a retried transaction, and an uncertain EXEC
  or provider mutation is not blindly replayed.
  [ElastiCache Serverless supports WATCH within one hash slot](https://docs.aws.amazon.com/AmazonElastiCache/latest/dg/ServerlessWatch.html);
  state keys and the notification topic share the `{general}` hash tag.
- The same transaction publishes a change notification. Each API has **one**
  Pub/Sub connection and listener; `tokio::sync::watch` coalesces local
  notifications. Each stream reads current shared state before sending its
  projection. Pub/Sub is not a history: a reconnect sends the latest snapshot.
  Subscription loss makes the API unready; read/write failure returns 503, never
  an in-memory fallback. `/health` is liveness; readiness uses `/readyz`.
- SIGTERM rejects new media commands and lets in-flight requests finish.
  Shared-mode shutdown does **not** remove participants or close provider tracks;
  healthy media stays client ↔ Cloudflare while signaling moves pods with the same
  capability. No sticky sessions are needed.
- Explicit leave publishes immediately. A vanished client's 45-second lease
  expires, with a five-second sweep (up to about 50 seconds after the last
  renewal). Disconnect alone is not leave. Active calls have no age cap. Provider
  cleanup is asynchronous, with shared 30-second claims and safe retries.
  Abandoned joins and uncertain operations expire after 30 seconds; uncertain
  operations invalidate only the affected call. ICE renewal's replayable offer has
  a replaceable 30-second claim instead.

Normal in-call and spectator updates have no polling interval: commit → Pub/Sub →
API/gateway → client. The 15-second request is a lease renewal, not the
notification path. Speaking indicators are client-side audio analysis.

The legacy HTTP/SSE presence endpoints (`/api/media/presence/events`, kept for
older clients) send ten-second heartbeats; streams opting into `handoff=1` get
`migrating` on SIGTERM and keep forwarding state for a ten-second window while
the client opens a replacement, cancelling the old stream only after the
replacement delivers a snapshot at least as recent as its last revision. Older
streams get `draining` and close. At most two streams are open per logical
subscription.

Clients keep lease renewal, mute/deafen synchronization and SDP negotiation
independent. Rapid mute/deafen changes coalesce to the latest intent. State writes
time out after five seconds and retry transient failures after 250 ms; each write
carries an increasing per-client `sequence`, and the API ignores older or
duplicate sequences. A newer self snapshot that disagrees with local intent
triggers repair. Join carries the current `muted` and `deafened` values, committed
with the participant (omitted fields mean false).

| Trigger | Recovery behavior |
| --- | --- |
| Join/publish/subscribe/negotiate/close reaches a draining API | Explicit `503` + `code: api_draining` proves admission rejection before mutation. Retry up to three times after 250/500/1000 ms within the original request deadline. Generic 5xx/timeouts do not authorize replay. |
| A speaker leaves before subscription starts | `404` + `code: track_gone` skips only that track. Authentication errors still invalidate the session. |
| A speaker leaves while a subscription is being created | Keep the listener, finish any returned SDP offer, then close the unwanted MID from the latest roster. |
| Cloudflare explicitly rejects an unavailable track | A successful HTTP response with one `not_found_track_error`/`empty_track_error`/`track_error`, no MID/SDP, and explicit `requiresImmediateRenegotiation: false` becomes `track_gone`; release the lock and keep the listener. HTTP errors, partial offers, allocated MIDs and session errors remain ambiguous. |
| Closing a departed track fails or times out | Remove local playback immediately; retry cleanup on later reconciliation/heartbeat without rejoining. API close commits track removal and a cleanup job atomically. |
| Live updates fail while lease renewals work | Retry the stream independently and keep voice/layout stable. |
| A previous call still has a pending device request | New signaling/media queues do not wait on old work. |
| Several successful recoveries over time | Reset the consecutive retry budget after each successful rejoin. |

Track close uses Cloudflare's `force: true`
([no WebRTC renegotiation](https://developers.cloudflare.com/realtime/static/realtime-api-2024-05-21.yaml));
it is cleanup, not evidence that a connection failed. A Cloudflare HTTP 200 body
containing `internal_error` is a failed operation; Caper returns 502 and does not
retry the subscription, since a MID/SDP mutation may have occurred. Unknown
outcomes of provider SDP mutations, invalid sessions, sustained network loss and
failed local capture are not solved by retries.

**Call duration and TURN renewal.** Sessions are never expired for age; heartbeats
renew the 45-second lease. TURN credentials use Cloudflare's maximum **48-hour
lifetime**. Leave and lease expiry enqueue credential revocation (unrevoked
credentials stay usable until expiry). Calls and both mic-test peers request fresh
credentials halfway through (24 hours), then restart ICE on the **same** peer and
SFU session, keeping tracks, subscriptions, capture and mute/deafen.
`setConfiguration()` alone does not update an existing TURN allocation.

- `POST /api/media/turn` caches one current credential generation; a lost
  response returns the same generation across pods. Issuance is limited to two
  attempts/minute; expired passwords are pruned and valid usernames kept for
  revocation. At most three generations overlap.
- `POST /api/media/restart-ice` accepts only an audio ICE-restart offer (no novel
  sending MID, video, data channel, duplicate MID or sendrecv). It uses
  Cloudflare's `POST /sessions/{id}/tracks/new` with `autoDiscover:true` and
  requires an answer with zero new tracks. `PUT /renegotiate` rejects restart
  offers; do not substitute it.
- Generation, sequence, offer hash and ownership claim live in Valkey; retries
  replay the **exact** offer. The client holds its signaling queue until it
  applies the answer and receives an idempotent `POST /api/media/restart-ice-ack`.
  Heartbeats, mute/deafen and leave do not wait behind it.
- Transient failures retry with 1/2/4/8/15/30-second capped backoff; permanent
  rejection uses bounded session recovery. A stale completion cannot clear a newer
  claim or restore a departed participant.

References:
[Cloudflare credential lifetime](https://developers.cloudflare.com/realtime/turn/generate-credentials/),
[expiry behavior](https://developers.cloudflare.com/realtime/turn/faq/), and
[WebRTC configuration semantics](https://w3c.github.io/webrtc-pc/#set-the-configuration).

Do not remove the 12-person limit and call this a large-room system: all-to-all
audio needs active-speaker/subscription limits and load tests. Changing Valkey
endpoints or losing its data loses live calls; API replacement does not. A crash
after Cloudflare creates a resource but before it is recorded can leave a resource
until provider expiry.

### Activation and verification

Production uses a private ElastiCache Serverless Valkey endpoint. Follow the
infrastructure repository's `docs/shared-valkey-runbook.md` for any change of
mode or endpoint:

1. Keep **one** desired API replica until every API pod uses the same
   `VALKEY_URL` and compatible state schema.
2. Deploy compatible API/web images while still in single-process mode.
3. Perform the **one-time empty-channel cutover** from process memory to Valkey
   (the old binary cannot hand off its state), then allow more than one pod.
4. Never overlap in-memory and shared-mode callers, and never roll back to a
   local-only image.

Subsequent same-schema deployments use RollingUpdate with `maxUnavailable: 0`,
`maxSurge: 1`, `/readyz`, and a 65-second termination grace including a
five-second preStop serving window. Shared-mode shutdown must not close healthy
Cloudflare tracks.

The shared-store integration tests need a disposable loopback Valkey:

```bash
# In an orb; do not point TEST_VALKEY_URL at a shared/production service.
amp orb service start caper-test-valkey --command 'valkey-server --bind 127.0.0.1 --port 6389 --save "" --appendonly no' --port 6389
valkey-cli -p 6389 PING # wait for PONG before starting the tests
TEST_VALKEY_URL=redis://127.0.0.1:6389 cargo test --locked -p caper-api tests::shared -- --ignored
```

Zero-interruption claims need live checks: two browsers on different pods, an API
replacement with unchanged peer/session IDs and continuous audio, a killed pod and
a short Valkey interruption.

### Local Compose shared-state testing

`npm run dev` starts Valkey 9.1.2 alongside the API, web, gateway and Postgres. The
API waits for Valkey's health check and defaults to `redis://valkey:6379`. Only
Compose sets `VALKEY_ALLOW_INSECURE=true`, which allows plaintext to the exact
hostname `valkey`. Production requires authenticated TLS. No Valkey port is
published. Leave `VALKEY_URL` absent from `staging/apps/caper` and unset in `.env`
to use local Valkey; a Secrets Manager URL takes precedence.

With Docker running and the staging AWS profile logged in:

```bash
npm run dev -- --scale api=2 --scale chat-gateway=2
```

In another terminal:

```bash
docker compose -f compose.staging.yaml exec valkey valkey-cli ping
docker compose -f compose.staging.yaml ps
docker compose -f compose.staging.yaml restart api chat-gateway
```

Expect `PONG` and two of each container. Voice needs `MEDIA_ENABLED=true` and
staging Cloudflare credentials. Local Valkey has persistence disabled: restarting
it clears active calls; restarting only the API does not. Stop with Ctrl-C;
`docker compose -f compose.staging.yaml down` removes containers (add `--volumes`
only to delete local Postgres data).

## Provisioned resources and configuration

Cloudflare resources: Realtime SFU app `caper-voice-mvp`
(`fe12588b1edc7a4c05691fc5beaa7ad2`) and TURN key `caper-voice-mvp`
(`d487cddbbd5b033ceb9a601155e0d6d8`). Credentials are server-only, outside Git.

| Variable | Meaning |
| --- | --- |
| `APP_SECRET_ID` | Optional AWS Secrets Manager JSON record loaded by the API before other configuration. Settings in this record take precedence over process environment. AWS credentials, region and this ID stay outside the record. |
| `MEDIA_ENABLED` | `true` enables voice; absent/false disables it |
| `VALKEY_URL` | Shared live call state (API and gateway). Empty uses single-process development mode outside Compose; Compose defaults to local Valkey. Hosted endpoints require `rediss://username:password@host:6379` with certificate verification. Plain `redis://` is allowed on loopback, or the Compose hostname with the opt-in below. Never project this into the web container. |
| `VALKEY_ALLOW_INSECURE` | Local-development opt-in (`true` or `1`); allows the exact hostname `valkey` without TLS/password. Set by staging Compose, absent in production. |
| `CF_SFU_APP_ID` | SFU app ID, not account ID |
| `CF_SFU_APP_SECRET` | SFU secret, server-only |
| `CF_TURN_KEY_ID` | TURN key ID |
| `CF_TURN_API_TOKEN` | TURN key secret for generating short-lived credentials |
| `MEDIA_BIND` | Default `0.0.0.0:3001` |
| `RUST_LOG` | Default `caper_api=info,tower_http=info` |
| `LOG_FORMAT` | `json` for structured stdout; otherwise human-readable |
| `ENVIRONMENT` | Log resource environment; default `development` |
| `AXIOM_TOKEN` | Optional server-only ingest token; absent/empty disables export |
| `AXIOM_DATASET` | Defaults to `caper` |
| `AXIOM_ENDPOINT` | Required when a token is set: the dataset's HTTPS Axiom edge URL ending in `/v1/logs` |
| `DATABASE_URL` | Runtime URL: pooled port `6432`, database `/caperchat`, restricted app role, verified TLS. Loopback development may use `sslmode=disable`. Missing configuration fails account access closed. |
| `MIGRATION_DATABASE_URL` | API-only migration URL: direct port `5432`, database `/caperchat`, separate schema-changing role, verified TLS. Required when `DATABASE_URL` is set; never falls back to it. Startup grants the runtime role access to migrated tables. Neither DB secret belongs in web. |
| `DATABASE_MAX_CONNECTIONS` | Optional pool size per API or gateway process, 1–100; default `5`. Keep replicas × size within the pooler limit. Invalid values stop startup. |
| `DATABASE_ALLOW_INSECURE` | Local development only; set by Compose for the private `postgres` service. |
| `AUTH_SECRET` | API-only random secret of at least 32 bytes. Enables account login and HMAC-protects low-entropy codes/IP rate-limit keys. Keep stable across replicas. |
| `DEBUG_USERS` | Optional comma-separated username allowlist for client diagnostics. Exact case-insensitive matches; blank disables. Restart the API after changes; open tabs pick it up when refocused or when User Settings opens. |
| `RESERVED_USERNAMES` | Optional comma-separated additional usernames that `/api/account/profile` cannot newly assign. `caper`, `capers`, `admin`, `admins`, `administrator`, `administrators`, `mod`, `mods`, `moderator`, `moderators`, `support`, `feedback`, `security`, `staff`, `official`, `system`, `team`, `trust`, `safety`, `caper_support`, `caper_feedback`, `jose`, `caperchat`, `aaron`, `joswayski`, `everyone` and `here` are always reserved. Existing owners may keep a reserved username. Restart all API replicas after changes. |
| `AUTH_REVIEW_EMAIL`, `AUTH_REVIEW_CODE` | Optional pair for App Store and Google Play reviewers. Set both or neither (startup fails otherwise). Requesting a code for that address emails nothing, and the fixed 6-character code (sign-in alphabet, no `0`, `O`, `1`, `I`, `L`, `U` or `V`) completes the normal single-use challenge with the usual limits. Keep that account to demo content; rotate the code if it leaks. |
| `AUTH_CODE_ATTEMPTS` | Attempts per code; default `3`, allowed `1`–`10` |
| `AUTH_EMAIL_15M_LIMIT` | Code requests per email from one source in 15 minutes; default `3` |
| `AUTH_EMAIL_DAILY_LIMIT` | Code requests per email from one source in 24 hours; default `5` |
| `AUTH_EMAIL_DAILY_CAP` | Code requests per email from all sources in 24 hours; default `20`. A new code cancels only the same source's earlier codes. |
| `AUTH_IP_HOURLY_LIMIT` | Code requests per source per hour; default `10`. A source is an IPv4 address or an IPv6 /64 (IPv4-mapped counts as IPv4), hashed with `AUTH_SECRET`. |
| `AUTH_GLOBAL_HOURLY_LIMIT` | Code requests across the service per hour; default `500` |
| `NOTIFICATIONS_WEBHOOK_URL` | Optional server-only HTTPS URL for best-effort application notifications (`user.created`). URLs with credentials, query strings or fragments are rejected. Redirects are never followed. |
| `CAPER_API_ORIGIN` | Web-only. Origin the web server uses for server-rendered account and invite lookups (`http://caper-api:3001` in-cluster, `http://api:3001` in Compose); the only place the server sends a visitor's session cookie. Unset outside development, the homepage renders a placeholder and invite pages show unavailable. Lookups time out after three seconds; responses are private, no-store. |
| `NOTIFICATIONS_ENABLED` | `true` starts phone-push workers in the API role (requires `CHAT_ENABLED`). Default off: `GET /api/push/config` lists no platform. |
| `PUSH_PLATFORMS` | Comma-separated `apns`, `apnsSandbox`, `fcm`. A platform is advertised only when listed and its credentials load. |
| `APNS_TEAM_ID`, `APNS_TOPIC` | Apple team ID; topic defaults to `chat.caper.ios`. |
| `APNS_KEY_ID`, `APNS_PRIVATE_KEY` | Production APNs auth key (`.p8` PEM; real newlines or `\n` escapes) for `apns`. |
| `APNS_SANDBOX_KEY_ID`, `APNS_SANDBOX_PRIVATE_KEY` | Sandbox key for `apnsSandbox` (Xcode builds, staging). |
| `FCM_SERVICE_ACCOUNT_JSON` | Firebase service-account JSON key (one string) for `fcm`. |
| `AWS_REGION` | SES region; production and staging use `us-east-1` |
| `SES_FROM_ADDRESS` | Verified Caper sender, including the friendly name |
| `SES_CONFIGURATION_SET` | Required SES transactional configuration set |
| `SES_WELCOME_FROM_ADDRESS`, `SES_WELCOME_REPLY_TO` | Optional, set together. Each new account (not the review account) gets one welcome note from this sender, replies to the Reply-To inbox. Production: `Jose Valerio <jose@caper.chat>` and `contact@josevalerio.com`; Cloudflare Email Routing forwards `jose@caper.chat` there too. The sender's IAM policy must allow the address. Failures are logged and never affect sign-in. |

Use `.env.example`; Rust does not auto-load dotenv files. Export a private env file
before `cargo run -p caper-api`, and run the web with `npm run dev:web`. Remote
browsers require HTTPS. In an orb use supervised services and portal URLs.

`npm run secrets:check` reads `staging/apps/caper` with the `staging` AWS profile
and reports key names as `Secrets Manager`, `in sync`, `differs`, `fallback only`,
or `missing`; it never prints values. Pass `--environment production --profile
production --cluster` to also report each production ExternalSecret's Ready
condition from the current kubectl context.

Image CI builds `apps/api/Dockerfile` and publishes
`production/caper:api-<full-40-character-sha>` from main via `api-image.yml`, using
the `production-caper-ecr-publisher` IAM role. The image run dispatches `Ship` in
`joswayski/infrastructure`, which rolls the API, then the gateway, then the web
forward to the newest published images on `main` (see AGENTS.md, Shipping).
Deployment/Service/ExternalSecret are named `caper-api`; the container is `api`.
The application-wide Secrets Manager record is `production/apps/caper`; each
consumer receives only its fields (the API's Cloudflare projection is Kubernetes
Secret `caper-api-cloudflare`). The API keeps one desired replica with
`RollingUpdate`, `maxSurge: 1`, `maxUnavailable: 0`, a 65-second termination
grace, port 3001 and `/api/media` routing. The web Deployment, Service, Ingress,
PDB and container are named `caper-web`, with two replicas.

`api-image.yml` runs only when `apps/api`, workspace Cargo files or that workflow
change. `aws-image.yml` (web) runs only when the website Docker context,
`apps/web`, `shared`, npm workspace manifests or that workflow change. Ship rolls
out the web only after the API build for the same range has rolled out. New
services should get their own workflow and path allowlist.

The API needs outbound HTTPS to `rtc.live.cloudflare.com`. AWS needs no public
media UDP ports. Clients use SFU plus TURN UDP and TCP/TLS fallback; alternate port
53 is filtered from both STUN and TURN URLs. `/api/media/status` reports the
feature flag; web `/health` is independent of provider availability.

## Axiom API logging

The Rust API exports structured **log events**, including events outside request
spans, through OpenTelemetry OTLP/HTTP to the `caper` Events dataset. Stdout
remains available; browser and native-client telemetry are not collected. Axiom
setup: https://axiom.co/docs/send-data/opentelemetry.

`AXIOM_TOKEN` must be an ingest-only API token authorized for `caper`. Do not
commit it, prefix it with `VITE_`, put it in command-line arguments, or paste it
into a PR/chat. `AXIOM_ENDPOINT` is the dataset's Axiom edge URL plus `/v1/logs`
(for example `https://<your-edge>.axiom.co/v1/logs`); the API requires HTTPS, an
Axiom hostname, and no embedded credentials/query/fragment, and assumes no
region. Do not set `OTEL_EXPORTER_OTLP_HEADERS` or
`OTEL_EXPORTER_OTLP_LOGS_HEADERS`: they can override the authorization/dataset
and disable this exporter. Missing or invalid configuration leaves the API on
local logging; errors never echo supplied values.

Request logs contain `event_name=http_response`, status, numeric `duration_ms`
(time to response headers), router-owned `http_route`, method, and a generated
request ID. Resources identify `service.name=caper-api`, package version,
environment and pod `HOSTNAME`. Only Caper events and Tower's HTTP failure events
are exported; span attributes are limited to method, matched route and request
ID. No headers, query strings, SDP, credentials, raw media, provider session
IDs/MIDs, unmatched URLs or dependency debug output are exported. Future log
statements must follow the same rules; there is no redaction engine.

The SDK worker batches up to 128 records, flushing every second, with a
2,048-record queue and a two-second export timeout. Enqueue is non-blocking; full
queues drop records. Failed batches are not retried. Export failures print a
static stderr warning; SDK internal logging is disabled because its errors can
contain upstream bodies/URLs. Shutdown allows at most three seconds to flush.

### Enabling and disabling export

Production uses a separate optional `caper-api-axiom` ExternalSecret/envFrom. Add
`AXIOM_TOKEN` and `AXIOM_ENDPOINT` to `production/apps/caper`, preserving **every
existing property**, wait for the ExternalSecret to be Ready, then restart or
redeploy the API (secret updates alone do not change a running process; follow
the infrastructure runbook). To disable export, set `AXIOM_TOKEN` to empty, sync
and restart; deleting the property can leave ESO's last token in place.

With `LOG_FORMAT=json`, check the startup marker:

```sh
kubectl -n default logs deployment/caper-api -c api --since=5m \
  | jq -c 'select(.event_name == "telemetry_initialized") | {event_name, axiom_enabled}'
```

`axiom_enabled: true` means configured, not delivered; confirm recent
`service_starting` and `http_response` records in the `caper` dataset.

## Deployment behavior

- Web-only deployments do not reload open tabs. Keep API/gateway changes
  compatible with old tabs and native clients.
- Web responses send `Strict-Transport-Security: max-age=31536000` (no
  `includeSubDomains`), refuse framing (`frame-ancestors 'none'`,
  `X-Frame-Options: DENY`), and set `nosniff` and
  `Referrer-Policy: strict-origin-when-cross-origin`. HSTS only helps once plain
  HTTP redirects to HTTPS at the edge (Cloudflare "Always Use HTTPS").
  Server-rendered account lookups go only to `CAPER_API_ORIGIN`, never to an
  address derived from the Host header.
- Infrastructure configures **API and gateway** with `RollingUpdate`,
  `maxUnavailable: 0`, `maxSurge: 1`, `/readyz`, a five-second `preStop` and a
  65-second termination grace. All overlapping API and gateway pods must use the
  same `VALKEY_URL` and compatible state schema.
- Gateway SIGTERM fails `/readyz` and rejects new sockets/commands. Existing
  sockets receive `migrating` and keep delivering for 20 seconds. Clients open a
  replacement without closing the old socket, resume chat from the applied
  sequence, and switch only after every subscription has caught up (chat replay
  checkpoint, current media revisions and any watched presence snapshot). Failed
  candidates retry while the old socket stays usable; overlapping messages are
  deduplicated. In-call roster streams keep their media capability and do not
  rejoin. Accepted commands finish after socket loss and cache results in Valkey;
  the gateway stays alive 36 seconds before HTTP draining.
- An API-only rollout does not restart the gateway. API SIGTERM rejects new media
  commands and drains in-flight HTTP for up to 30 seconds. **Shared-mode shutdown
  does not remove voice participants, close Cloudflare tracks or revoke TURN
  credentials.** Healthy audio stays client ↔ Cloudflare.
- Memory-only development has no such guarantee: shutdown removes local
  participants and spends up to 20 seconds on provider cleanup (skipped if HTTP
  draining overruns). Never mix local-mode and shared-mode pods.
- Ship deploys the API before the gateway; keep migrations compatible with both
  overlapping versions. A crash, an outage beyond the handoff window or an expired
  lease can still interrupt updates or force voice recovery.

## Limits and lifecycle

- 12 registered sessions per room, one microphone publication each, 11
  subscriptions. A running Mic test uses two additional private sessions within
  that cap.
- 30 joins/minute per room, at most 10 by one account; 120 provider
  mutations/minute per participant; 256 KiB request bodies. One account holds at
  most 4 of a room's 12 slots (participants, mic-test monitors and in-flight joins
  across its devices); beyond that 409 `already in this call on too many devices`.
  These are **not a spending cap or DDoS defense**.
- No call-age limit; 48-hour TURN credentials renewed at 24 hours; 45-second
  presence lease, swept every five seconds. Background browser throttling can force
  a reconnect.
- Leave stops local capture immediately, invalidates the capability, force-closes
  SFU tracks/subscriptions and revokes TURN credentials. Failed cleanup has a
  bounded retry backlog; shared mode persists it in Valkey.
- Negotiations are serialized with per-participant operation guards. Ambiguous
  provider creation is not retried; the session is invalidated and cleaned up.
- Transient heartbeat failures (network/timeout, HTTP 408/429/5xx) retry after
  three seconds without tearing down media (healthy cadence 15 seconds). Snapshots
  have a five-second deadline including body reads; failures lasting 30 seconds
  trigger recovery on the next failed poll. Invalid sessions trigger recovery
  immediately. Failed mute/deafen sync retries the latest state independently.
- A WebRTC `disconnected` state gets ten seconds to recover in place; `failed` or a
  sustained disconnect triggers up to three consecutive rejoin attempts keeping
  mute/deafen and device choice; success resets that budget. Voice diagnostics
  report `lastReconnect` (trigger such as `connection lost`, `receive connection
  lost` or `lease renewal failed: HTTP 401`, its time, and both peer connection
  states); it survives the rejoin, resets on explicit Join, and carries no token,
  credential or SDP.
- All microphone subscriptions are automatic. Deafen mutes playback, not
  forwarding/bandwidth. Mute disables the local track and detaches it from the
  sender. Opus is preferred; browser echo cancellation and gain control are off for
  headphones. Output selection requires `setSinkId`; otherwise use OS settings.
  Joining requires microphone permission.

### Codec-managed silence suppression (Opus DTX)

The web voice engine requests `usedtx=1` for Opus in remote publication answers
and subsequent subscription offers, including the private mic-test sender.
[RFC 7587](https://www.rfc-editor.org/rfc/rfc7587.html#section-7) defines this as a
receiver preference, so the **remote** description controls our outgoing encoder.
The preference is reapplied during renegotiation and kept when replacing the
microphone track; non-Opus codecs are left alone. There is no application volume
threshold, noise gate, speech-triggered detachment or added buffering; DPDFNet,
bitrate, packet duration and FEC are unchanged. DTX is not lossless: if listening
tests reveal degradation, prefer continuous transmission over an aggressive gate.
`node scripts/test-opus-dtx.mjs <vite-url> <speech.wav>` measures sender RTP with
and without DTX in Chromium.

## Leaving voice

Explicit Leave/Cancel and page unload share synchronous local teardown: stop
capture and processed tracks, close the PeerConnection and event stream, clear
playback/roster state, and invalidate the call generation. Leave then shows an
enabled Join without awaiting provider cleanup. The old capability is sent to
`/leave` with Fetch `keepalive`; its outcome never changes the next call's state.
Late peer events and microphone replacement completions are session-guarded.
Automatic reconnect and join-error teardown also stop media synchronously and
clean up the old token in the background. The API removes registry membership
and tokens and notifies listeners before queueing TURN revocation and track
cleanup, and returns without waiting for Cloudflare. If Leave cannot reach the API,
remote presence persists until the 45-second lease expires.

On iOS, Leave gates microphone publication and playback and clears call state
immediately; ending a microphone comparison, disabling tracks, closing the old
peer (including VoiceProcessingIO disposal) and deactivating `RTCAudioSession` run
off the main actor. A new join or microphone preparation awaits pending cleanup
before reusing the shared audio session; cancelling that join invalidates it
before it can activate audio.

## Provider failures and cleanup

Caper HTTP 502 does **not** prove Cloudflare returned 502. Provider HTTP errors,
timeouts, network failures, body failures, invalid JSON, error envelopes and
rejected response shapes return `502 {"error":"media provider unavailable"}` with
an `x-caper-error-id` header matching the Rust warning's `id` (also passed through
the development adapter; not the edge's `cf-ray`).

`Cloudflare operation failed` logs include the operation, failure kind, upstream
HTTP status when available, upstream `cf-ray`, bounded error code and elapsed
milliseconds; `media request failed` links the returned reference to them.
Successful calls log duration at debug level. No bodies, SDP, raw media,
credentials, full URLs, session IDs, MIDs or free-text upstream errors are logged.
Response bodies are bounded to 256 KiB; after a non-success status, reading its
error code has a 250 ms budget.

Subscription failure keeps the original provider error. An ambiguous mutation
invalidates that participant. Unknown subscription MIDs are discovered through a
background session read and closed. Registry removal, token revocation,
dependent-subscription removal and roster invalidation never wait for provider
cleanup.

Cleanup uses an in-memory queue of at most 512 jobs and one worker with four
concurrent slots. Local changes wake it immediately, retry deadlines at their
time, and a five-second reconciliation poll recovers shared work from other
replicas; a separate five-second expiry sweep cannot be held up by provider IO.
Each job has a 12-second bound; Cloudflare requests a 10-second timeout. Only
session reads, `force:true` track closure and TURN revocation are retried; session
creation, TURN issuance, tracks/new and renegotiation are **never replayed by the
transport**. Retries cover network/timeouts and HTTP 408/429/500/502/503/504, at
most five attempts with 2/4/8/16-second delays plus up to 1.02 seconds jitter. A
full queue drops new jobs with an error; exhausted jobs are logged as abandoned.
Shutdown spends up to 20 seconds draining cleanup after HTTP drains. TURN expiry
is the 48-hour backstop; this is not a durable cleanup system.

For a failure, copy its `x-caper-error-id`, UTC timestamp and endpoint (never SDP
or tokens) and correlate:

```sh
kubectl -n default logs -l app.kubernetes.io/name=caper-api -c api --since=30m --prefix --max-log-requests=5 | rg '<error-id>'
```

A 502 without that header may come from ingress or an older image.

## Join startup and preparation

The enabled channel screen starts one DPDFNet-8 worker before Join: it downloads
the model/runtime (about 27 MB), initializes ONNX inference and runs a synthetic
warm-up, then starts its live state fresh. The homepage does not start this work.
Preparation never requests microphone permission, captures audio, creates an
AudioContext, provisions an SFU session or publishes a track. It is preparation
for the next capture, not a pool of spare workers during a call; initialized
workers are not persisted across page loads.

Join opens the microphone and initializes a dedicated worklet, taking exclusive
ownership of the prepared worker. If that worker is still compiling, Join does not
wait: when the browser confirms its own noise suppression is active, audio flows
through the same processed track with browser suppression and DPDFNet is swapped
in when ready (browser suppression is released first); otherwise Join waits for
DPDFNet. A cold join never publishes raw audio. The connection panel splits
Microphone into device and processing time, or notes that the model was still
loading.

Each engine gets one fresh-instance retry for initialization errors or crashes
before advancing from DPDFNet-8 to DPDFNet-2 HR, then RNNoise; Join fails if none
initializes. A transient DPDFNet underrun outputs silence while refilling the
three-hop reserve. Sustained overload or worker failure switches the processed
track to a bypass (preserving the call) and enables the browser's noise
suppression constraint while DPDFNet-2 HR loads; only if that fails does RNNoise
load. A ready fallback replaces only the processing node: track identities, mute
state, input gain and voice enhancement are preserved, and browser suppression is
then disabled to avoid stacking. If RNNoise cannot initialize, the browser
fallback remains; the reported track setting determines whether the UI says
browser suppression is active. An unrecoverable RNNoise processor error stops the
microphone without restarting the connection.

Preparation has a 60-second readiness timeout; failed workers are terminated and
evicted. Explicit Leave stops the used worker and starts fresh preparation; a
worker that has processed microphone samples is never reused. Device changes and
automatic recovery create fresh workers. Join duration measures click to locally
ready to talk, not first word heard remotely.

ICE: the API excludes port 53 from STUN and TURN URLs (string and list forms).
`localDescription()` returns the SDP immediately after `setLocalDescription()`
rather than waiting for gathering to complete, as in
[Cloudflare's echo example](https://github.com/cloudflare/realtime-examples/blob/main/echo/index.html);
SDP availability alone never enables microphone audio.

### Combined join and publication

The client creates its sendonly audio transceiver and offer while the microphone
opens and sends that offer with `media.join` (`publish`). The API creates the
session and TURN credentials, commits the participant, then publishes through the
`publish` state machine before returning `publish` (the answer and `trackId`) with
the capability. A refused or failed publication removes the participant; the
capability is never returned half-joined. TURN servers arrive with the response
and are applied with `setConfiguration` before the offer is set locally. The
microphone is attached (disabled) to the sender before the answer is applied. The
authenticated roster/lease request overlaps ICE. An API without this field
rejects it with 422 before any mutation, and the client joins and publishes
separately.

Subscription negotiation waits for the listener's connected transport:
Cloudflare holds a pull into a negotiated but unconnected session for about 11 s
and then answers 425 `session_error`, after the API's 10-second provider timeout.
A pull into an **empty** session (no negotiated PeerConnection; Cloudflare's
[receive pattern](https://developers.cloudflare.com/realtime/sfu/get-started/connection-patterns/#receive-a-published-track))
is answered at once, and a pull of a publisher that has not connected gets a
per-track `not_found_track_error`. Received audio stays withheld from playback,
and the microphone disabled, until transport, live updates, roster and state have
all completed.

When several sources are missing, `media.subscribe` sends `trackIds` instead of
one `trackId`. The API pulls them in one `tracks/new` (up to 64 remote entries)
and one SDP exchange, returning
`{ tracks: [{ trackId, mid }], gone: [trackId], sessionDescription?, requiresImmediateRenegotiation }`.
Per Cloudflare's [batch operations](https://developers.cloudflare.com/realtime/sfu/api/#batch-resource-operations)
each result is matched by its echoed `sessionId`/`trackName`: a MID without an
error is allocated; `not_found_track_error`, `empty_track_error` or `track_error`
without a MID allocated nothing and goes in `gone` (as do sources that already
left). Anything else is uncertain and takes the recovery path (clean up allocated
MIDs, discover orphans, remove the listener). The single `trackId` form is
unchanged; a 422 for `trackIds` makes the client pull one source at a time for the
rest of the call.

When the provider refuses a listed source that is not sending media yet
(`track_gone` while still in the roster), the listener re-checks after 0.25, 0.25,
0.5, 0.5, 1, 2, 4 and 8 seconds instead of waiting for a roster change or
heartbeat; a successful reconciliation resets that budget. When several people
leave at once, a listener closes their MIDs with concurrent `close` requests.

Diagnostics separate microphone capture from session + publication (both from
Join), report ICE time within transport, and show the selected candidate pair's
connectivity checks (sent vs answered). The API logs every successful Cloudflare
call's `elapsed_ms` at info level (`Cloudflare operation succeeded`).

### Prepared joins for signed-in members

`media.prepare` (account channels only, through the member-checked
`/api/channels/{id}/media/prepare` route) creates the provider session and TURN
credentials moments before a join, which then takes them. The browser sends it
when the pointer or keyboard focus reaches a Join button (at most every 4 seconds
per channel) and as a mouse or pen pointer comes within 120 px of one (one document
listener, at most one geometry check per frame). On coarse-pointer screens the
viewed channel is prepared once when its Join button first becomes visible.

An unused Cloudflare session is disconnected (410 `session_error`) after about 15
seconds, so a prepared join is kept for 8 seconds, one per account per room.
Revocation of each prepared TURN credential is scheduled 12 seconds after issue;
a join that takes the credentials cancels it. Cloudflare bills only egress
([pricing](https://developers.cloudflare.com/realtime/sfu/pricing/)), so sessions
and credentials cost nothing. Join re-checks channel access concurrently with
provisioning and applies the result before committing the participant. An older
API or gateway rejects `prepare`; the client then joins normally.

### Warm sessions: pre-connected for signed-in members

While a signed-in member is on an account channel, voice is available and they are
not in a call, the browser keeps a **warm pair**: two provider sessions with no
tracks, both connected.

- `media.warm` (signed-in accounts only) takes two browser offers, each carrying
  only a negotiated data channel (no track, no in-band messages). The API calls
  Cloudflare `sessions/new` with each offer (the only form Cloudflare accepts for a
  session without tracks), issues TURN, and returns both answers, the ICE servers
  and a ticket.
- The ticket is not stored. It is signed with HMAC-SHA256 for that account's
  session hash, using a key derived from the provider secret every media pod holds,
  and names both sessions, the TURN credentials, the issuing channel and the issue
  time, so a join in **any** of that account's channels can adopt it.
- Join sends `warm: <ticket>` with a microphone offer made on the warm connection.
  The API verifies the ticket against the joining account, requires it under 25
  minutes old, refuses one whose sessions a participant already uses, then
  publishes into the warm session and pulls everyone into the warm receive session
  (which becomes the participant's pull session even in an empty room). The
  response carries `warm: true`; diagnostics say "pre-connected".
- A refused ticket (409 `warm_unavailable`: foreign, altered, expired, replayed or
  sent with a monitor) is refused before anything is reserved; the client closes
  the pair and joins the ordinary way in the same Join, as it does on 422.

The client replaces the pair after 20 minutes, rebuilds it when either connection
fails or stays disconnected for 5 seconds or a hidden page becomes visible, and
closes it during a call, on sign-out and when leaving the call page. Unadopted
TURN credentials are revoked 30 minutes after issue by a job in the issuing
channel's registry; adoption cancels it, even from another channel. A room accepts
one warm request per account per 10 seconds and 48 per 10 seconds in total, and
refuses warms (429) while its cleanup backlog is half full. An idle connected
session carries only connectivity checks, so a warm pair costs no media.

### Join-time pulls on a receive-only connection

A client publishing inside `media.join` also sends `receive: true`. When others are
publishing, the API creates a second, receive-only provider session (or takes one
from a prepared join), publishes the microphone and pulls everyone present
concurrently, and returns
`receive: { tracks, gone, sessionDescription, requiresImmediateRenegotiation }`
with the publication answer. The client answers on a second `RTCPeerConnection`.
Subscriptions, their answers (`negotiate`) and closes then use the receive session;
`close` takes `subscription: true` because MIDs may repeat across the two sessions.
A single session cannot mix local and remote tracks in one `tracks/new`.

Join completes when the microphone connection is connected; it does not wait for
the receive answer or handshake. Audio from people present plays once the receive
connection is up, reported as "Hearing others" (`hearingMs`). If that answer fails
or the receive connection is not connected within 12 seconds of Join completing,
the call rejoins with `lastReconnect` `hearing others failed`. With no offer or an
uncertain pull result, the API cleans up and the client pulls after connecting.
Media that arrives before the roster lists its owner is held until it does. Older
APIs reject `receive` with 422 and the client retries without it. Monitors, native
clients and empty rooms get no receive session. On relay-only networks the receive
connection keeps its initial TURN credentials (48 hours); only the microphone
connection renews them, so a longer call on such a network rejoins once.

### Required live-update and audio readiness

Join waits for the selected audio processor before publication. Live-update
readiness and publication run concurrently with the published track disabled.
After both, mute/deafen synchronization and the lease-renewing roster request
overlap the transport handshake; subscription negotiation waits for transport.
Only after live-update readiness, transport connection, initial
roster/subscription negotiation, state synchronization and a final live-stream and
track check does the client enable audio and show Connected, respecting mute and
monitor state. This gates the joining client only, not remote playout.

The legacy SSE endpoint `GET /api/media/events?snapshots=1` (for older clients)
uses `x-caper-media-token`, never a query-string or cookie capability. It emits
`ready`, an initial `snapshot`, coalesced snapshots on roster mutations, and
`heartbeat` after ten seconds without another event; snapshots contain
`participants` and a monotonic `revision`. One stream per participant; a new one
replaces the previous. Auth/expiry is rechecked for every event, and SSE never
renews the lease. A transient (5xx) database recheck failure skips that event;
denials, setup failures, three consecutive transient failures (about 30 s) and a
shared-store outage end the stream. No handshake within ten seconds or no valid
event within 25 seconds fails it. The API sends `X-Accel-Buffering: no`; production
intermediaries must stream without buffering (Traefik routes `/api/media` to the
API without buffering middleware).

During an established call, a failed live stream reopens with the same capability,
backing off from 250 ms to three seconds while keeping audio. Restored streams get
current state, not a replay. Invalid sessions or sustained heartbeat failure
trigger session recovery. Cancel/leave aborts the stream, startup waits and delayed
retries.

## Received-audio microphone test

Mic test does not play the microphone directly. It creates a private sender and
receiver, sending the selected processed microphone through WebRTC/Opus to the
Cloudflare SFU and decoding it on a separate PeerConnection, with the same codec
preference and SFU/TURN provisioning as participants. Only the received track is
played. The receiver attaches a muted sink as soon as its track arrives so the
jitter buffer drains continuously.

Pressing **Mic test** establishes the private return from the denoised Natural tap;
**Mic Test** starts recording (timer, received-audio meter, **Stop Testing**),
which stops after 30 seconds. The client records one timestamped Opus return, then
renders a WAV Enhanced comparison from that take at the current voice-processing
strength. Natural plays first, then Enhanced (which makes no second SFU trip).
Nothing is uploaded or persisted; testing again, ending, leaving, reconnecting or
unmounting discards both. Reconnects and device/filter changes return to Record.
A decoded recording with no samples above 0.001 amplitude shows a no-signal
warning. Blocked autoplay leaves native Play controls; output-device denial is
reported separately.

Starting detaches the public sender before enabling test audio and saves
mute/deafen, marking both true; stopping restores them. Switching a filter/device
replaces the private sender track. Failure never falls back to direct local
playback and leaves the channel muted until explicit Stop. Use headphones: this is
not a speakerphone double-talk test.

Private sessions are tied to the authenticated channel capability, hidden from the
roster, have sender/receiver roles, cannot subscribe to public tracks or another
person's test, and are cleaned up on Stop/leave/expiry. They share join/capacity
limits. `join` accepts `monitor: "sender" | "receiver"` with the parent's bearer
capability; `publish` returns the opaque `trackId`; the receiver can subscribe only
to its sibling sender. No provider session identifier or arbitrary provider
operation is accepted from the client.

### Audio setup and speech consistency

Headphones natural input is the default, with browser echo cancellation and
automatic gain control **off**; there is no speakerphone echo selector. OS-level
processing may still apply.

Voice processing defaults to 25% after noise suppression, adjustable 0–100%. At
100% it applies Web Audio nodes on-device: a 75 Hz high-pass, restrained warmth and
presence EQ, 3:1 compression, 1.35× makeup gain and final peak limiting. High-pass
frequency, EQ gain, compression ratio and logarithmic makeup gain scale with
strength; peak limiting stays fixed. Zero bypasses the chain; DPDFNet stays active
at every setting. This is fixed dynamics processing, not AGC. The setting lasts
for the page lifetime. The voice-level control is 0–200% (100% is unity). Device
selectors show the current default hardware by name, omit a synthetic “System
default” row and collapse duplicate labels.

## On-device noise suppression

DPDFNet-8 48 kHz HR is the default microphone mode: capture (browser AEC, AGC and
noise suppression off) → input gain → 48 kHz mono DPDFNet → voice processing →
MediaStream track → WebRTC Opus sender → Cloudflare SFU. There is no external
denoising API, license server, per-minute inference fee or raw-audio upload.

The model and runtime (~27 MB) are vendored and served from `/audio/dpdfnet8-v2/`,
versioned and cached as immutable, included by the web build. License notices,
provenance and checksums are in the adjacent README. A test checks the shipped
runtime's size, SHA-256 and WASM validity. Changed `/audio` assets must use new
file names.

### iPhone and iPad audio routes

The models run at 48 kHz. An iPhone route at another rate (24 kHz has been seen
with AirPods) sent silence when the capture graph was forced to 48 kHz. On
iPhone/iPad WebKit (every iOS browser, and iPadOS in desktop mode), capture probes
the route rate with a default context. When it is not 48 kHz, the graph runs at
the route's rate and the worklet converts to and from 48 kHz:
`resampler-v1/resampler.js` (windowed-sinc, Blackman window, 16 zero crossings,
cutoff at 95% of the lower rate's Nyquist) is added to the AudioWorklet first, and
`dpdfnet8-v2/worklet-v4.js` / `noise-v1/worklet-v2.js` wrap their 48 kHz
processing in it. The conversion adds about 1.6 ms. If the converted pipeline
cannot start, capture sends the browser's voice-processed track and the status
names the route rate. Capture diagnostics include `routeSampleRate`. Other
platforms and 48 kHz routes load no resampler. A route change mid-call is not
detected until the microphone is reopened.

### UI sounds on phones

Join, leave, mute and other UI sounds play on touch devices too; the Caper sound
effects switch turns them off. Where the browser has the Audio Session API
(Safari/iOS WebKit 17+), a UI sound played while nothing captures the microphone
sets `navigator.audioSession.type = "ambient"`, so it mixes with other apps' audio
and follows the silent switch. Every microphone capture sets `"auto"` before
`getUserMedia`, and the type is never changed while a capture is live.

### Retained engine implementations (no user-facing selector)

| Mode | Purpose |
| --- | --- |
| DeepFilterNet balanced | 20 dB attenuation limit, retaining about 10% original spectral amplitude. |
| DeepFilterNet gentle | 12 dB limit, retaining about 25% original amplitude; more voice **and noise** return. |
| DeepFilterNet strong | Original 40 dB limit, retaining about 1% original amplitude. |
| RNNoise | Independent lightweight 48 kHz neural model, 3.6 MB same-origin download. No VAD gating. |
| DPDFNet-8 HR (default) | 48 kHz model; approximately 27 MB model/runtime download. Worker-based ONNX inference; substantially heavier than RNNoise. |
| Browser suppression | Built-in baseline retained internally; support varies by browser/device. |
| Off | No requested noise suppression. |

DeepFilter presets blend the enhanced and time-aligned original spectrum; post-filter
beta is zero. No model fixes clipping introduced by hardware, gain control or echo
cancellation. RNNoise provenance is in `apps/web/public/audio/rnnoise-v1/README.md`;
the shared adapter lives at `/audio/noise-v1/`.

DPDFNet-8 HR is the only in-channel filter offered. Changing a device clears the
old mic-test recording and disables recording during initialization. Use
`node scripts/vendor-dpdfnet.mjs 8` (or `2`) to reproduce models, metadata and
licenses; offline authoring needs uv 0.12.23 and runs ONNX metadata export in the
locked `audio` dependency group. The active status appears only after the processor
acknowledges initialization.

Recovery order is DPDFNet-8 HR → DPDFNet-2 HR → RNNoise, for initialization
failure, sustained backlog or processor failure. Healthy 8 HR is never downgraded
based on CPU count or a single slow hop. Initialization errors/timeouts and
processor crashes get one fresh worker/worklet retry per engine per capture,
shared across startup and runtime; sustained backlog advances immediately.
DPDFNet initialization is bounded to 60 seconds per attempt, RNNoise worklet
readiness to 15 seconds (asset download 30 seconds). DPDFNet-2 HR uses the same
48 kHz framing/DSP with its own 10.5 MB model, loaded only on fallback and reusing
the ONNX runtime (provenance in `apps/web/public/audio/dpdfnet2-v1/README.md`). A
downgrade lasts for the current capture. If every local engine fails to
initialize, capture rejects without publishing raw audio; if fallback and browser
suppression both fail at runtime, the status reports bypassed audio.

Device changes replace the outgoing track and release the previous hardware track
and AudioContext. Cancel/leave stops capture and processed tracks; a late
permission grant is released. The RNNoise/DeepFilter adapter adds 10 ms buffering.
DPDFNet uses a 20 ms analysis window, 10 ms hops and three output hops of startup
buffering; an eight-hop backlog terminates inference and starts fallback. No
inference runs inside the AudioWorklet callback.

DPDFNet provenance, checksums, licenses and reproduction are in
`apps/web/public/audio/dpdfnet8-v2/README.md` (CEVA code/weights Apache-2.0; ONNX
Runtime MIT). Following [upstream DPDFNet](https://github.com/ceva-ip/DPDFNet),
keep the 960-point unnormalized FFT, 480-sample hop, Vorbis window and
metadata-initialized recurrent normalization; do not normalize again outside the
model or add a gate/AGC. `node scripts/test-noise-fallback.mjs <vite-url>` injects
startup failures and stalls through the whole fallback chain;
`node scripts/test-mic-playback.mjs <vite-url>` covers the mic-test recorder.

Native clients implement their own audio and noise processing; see
[native acceptance](#native-distribution-and-acceptance).

## Cost and acceptance

[Cloudflare SFU/TURN pricing](https://developers.cloudflare.com/realtime/sfu/pricing/):
**outbound GB × $0.05** after the free tier; the combined SFU/TURN path is not
double-charged. AWS application costs are separate.

Before wider testing, use physical clients on two networks: verify audible speech,
device unplug/change, permission denial, repeated join/leave, restart,
mute/deafen and a long session. Force relay in a test-only PeerConnection override
and inspect selected ICE stats, then test a restrictive network including TCP/TLS
fallback. Do not infer TURN success from ordinary Wi-Fi.

## Runbook

- Unavailable: inspect the feature flag, `/health`, ingress routing, secret names
  and outbound HTTPS. Never print credentials, SDP or raw media in logs.
- Silent: check microphone permission, mute/deafen, output selection and the Play
  audio fallback, then incoming/outgoing RTP and selected ICE candidate stats.
- Stale roster: check that all API/gateway pods share one `VALKEY_URL`, that live
  updates arrive (snapshots and heartbeats flush through intermediaries) and that
  the lease sweep runs. Do not paste auth-bearing HAR files.
- Provider cleanup failure: use private operator credentials to inspect/close
  tracks, never a public admin proxy. Disable new joins during provider outages.
- Rotation: create a replacement app/key, update the private deployment secret,
  restart via the approved deployment path, verify two clients, then revoke the old
  resources. Never put these secrets in browser bundles or desktop builds.
- Take down: set `MEDIA_ENABLED` false and restart the API (GitOps in production).
  The website can stay up.

## Accounts

The web login page sends email codes and then requires a unique username and
display name. Account login is enabled when `AUTH_SECRET`, the database URLs and
SES settings are configured. When a valid account bearer token or session cookie
accompanies a voice join, the API uses the stored display name. Country codes are
neither stored nor shown.

Each code is six uppercase characters from `ABCDEFGHJKMNPQRSTWXYZ23456789`
(excluding ambiguous `0/O`, `1/I/L` and `U/V`). Codes expire after 10 minutes, are
stored only as HMAC-SHA-256 values and allow `AUTH_CODE_ATTEMPTS` attempts
(default 3). Request limits (see the configuration table) are enforced in
PostgreSQL across replicas. Past a limit the API still returns a challenge ID but
sends nothing, so clients cap resends: the code step offers **Resend code** 60
seconds after each code is sent (with an m:ss countdown), at most twice per email
entry, then shows "Still nothing? Check your spam folder, or try again in 15
minutes." **Email me a new code** (after all attempts are used) counts as a resend.
**Use a different email** resets the count.

Session tokens contain 256 random bits; only their SHA-256 hashes are stored.
Sessions lapse after 30 days unused: an authenticated request renews the expiry
to 30 days, at most once a day (phone push devices a session registered stop with
it). Challenges are pruned a week after creation; expired and revoked sessions are
kept as sign-in history.

`users` has a bigint identity PK, a unique random public ID, nullable unique email,
verification/deletion timestamps and profile fields. Usernames are lowercase ASCII
letters/digits/underscore, 3–32 characters and globally unique. Display names are
global, nonunique, 1–64 Unicode characters (see [Name characters](#name-characters)).
Both start NULL and are completed together. Format and length validation live in
the application, not SQL `CHECK` expressions. Server functions keep CSRF
middleware. The web service owns no `/api` routes; Traefik sends same-origin
`/api/*` requests directly to Rust.

### Notifications webhook

Set the server-only `NOTIFICATIONS_WEBHOOK_URL` in the AWS Secrets Manager record
(`production/apps/caper`; staging uses `staging/apps/caper`). The
`caper-api-account` ExternalSecret extracts that record into the API pod. Preserve
every existing property when updating the record, wait for `caper-api-account` to
become Ready, then deploy or restart the API. Never put this URL in web
configuration or a `VITE_` variable.

After the database transaction creates a user, Caper asynchronously POSTs once:
`{"event":"user.created","user":{"id":"<external id>","email":"<verified email>"}}`.
The request has a three-second deadline; failures are recorded only as bounded
event/status diagnostics. No webhook URL or payload is logged, and delivery failure
never affects registration. Delivery is best effort: neither retried nor durable.

### Public API: web and native clients

The public API is **`https://caper.chat/api/*`**, routed by Traefik directly to the
Rust `caper-api` service; `https://caper.chat/*` serves the website through
`caper-web`, and `https://api.caper.chat` is a direct API alias. There is no
native-specific backend.

| Method | Public endpoint | Purpose |
| --- | --- | --- |
| GET | `/health`, `/api/health` | Unauthenticated health checks |
| GET | `/api/updates/native` | Unauthenticated cached, signed desktop update metadata |
| POST | `/api/auth/email/request` | Request a 6-character, 10-minute email code; returns a challenge ID |
| POST | `/api/auth/email/verify` | Consume a challenge and create a session (30 days, renewed by use) |
| POST | `/api/auth/logout` | Revoke the current session |
| GET | `/api/account/me` | Read the authenticated public account |
| POST | `/api/account/profile` | Set a unique username and display name |
| any | `/api/channels/{channel}/media/{operation}` | Member-checked voice: `status`, `join`, `prepare`, `warm`, `turn`, `restart-ice`, `restart-ice-ack`, `snapshot`, `publish`, `subscribe`, `negotiate`, `close`, `state`, `leave`, `presence/events` |

In-call media operations also require the issued `x-caper-media-token` capability
header. Tokens stay in memory, never in URLs, cookies or persistent storage. The
gateway invokes the same channel media handlers for `media.*` commands.

Web verification defaults to an `HttpOnly`, `Secure`, `SameSite=Lax` cookie. Native
clients request `tokenTransport: "bearer"` and store the opaque token in the OS
credential vault (Keychain/Keystore), never plain preferences. Protected routes
accept either transport; logout revokes the same database session. Third-party
browser CORS, API keys and developer OAuth are not implemented.

Bigint account IDs stay internal. The random 12-character `external_id` is returned
as `id`, alongside `username` and `displayName`; changing a username or email does
not change it. Messages store the author's display name at send time, but history,
gateway replay, live publication and idempotent retries serve the author's current
display name and avatar. Deleted accounts keep the stored name. Messages on screen
update on the next history load or reconnect.

### Desktop update metadata cache

`GET https://caper.chat/api/updates/native` returns an envelope with `manifest`
(base64 of the exact `latest.json` bytes) and `signature` (detached base64 Ed25519
signature). It only fetches the fixed GitHub `native-latest` release; request
parameters cannot select an upstream URL. The API keeps one in-memory entry for 60
seconds **per process**, coalesces concurrent misses, bounds each upstream request
to five seconds, and limits the manifest to 64 KiB and signature to 1 KiB.
Successful responses send `Cache-Control: public, max-age=60` and `Age`. After a
failed refresh the API returns `502` with `no-store` and waits 60 seconds before
retrying; it never serves expired metadata.

Desktop release builds check about 20 seconds after launch and every minute,
verify the envelope's signature against their compiled-in public key, and fall
back to GitHub's manifest/signature on errors, malformed data or signature
mismatch. Manual checks are **Caper › Check for Updates…** on Mac and **User
Settings › Settings… › Updates › Check for updates** on Windows/Linux; they share the
automatic worker, restore a dismissed offer and report up-to-date/failure.
Windows/Linux Settings and the Mac's up-to-date result show the installed version,
build number and short source revision; "up to date" means no newer **published
release**. Native releases run on pushes that change the apps (see
`apps/native/README.md`, Releasing); if one did not reach testers, its "Wait for
the servers these apps need" step names what held it.

The desktop updater runs outside the install, with null standard streams and, on
Unix, its own process group. It always relocates itself on the first `apply`
process (comparing paths is unsafe on Windows, where `C:\...` and `\\?\C:\...`
differ). Successful swaps reopen the new app; failed attempts reopen the working
install after the original process exits. macOS uses `/usr/bin/open -n` and checks
its exit status. Errors and launches are logged to `caper-updater.log`
(`~/.local/state/caper` or `$XDG_STATE_HOME/caper` on Linux, the user's temporary
directory elsewhere). Relocation copies, downloads and unpacked builds go in new,
randomly named 0700 folders. Failed attempts leave build-scoped feedback beside the
app (`.app.update-error` for a Windows Setup install), which clients read as
`last_error` and offer the installer as an alternative; a successful update removes
it. Windows/Linux show collapsed, expandable release notes; macOS keeps its AppKit
alert. If an installed updater cannot update itself, run the newer Windows Setup
over the existing installation (do not uninstall). Never reuse a build number.

### Desktop patch updates

Each platform in `latest.json` keeps its full archive (`url`, `sha256`, `size`,
which every updater uses) and may add `patches`: `{from_build, url, sha256,
size}` for the newest earlier builds. Updaters that predate patches ignore the
field. A patch (`<archive>-<from>-to-<to>.caperpatch` on `native-latest`, format
in `apps/native/updater/src/delta.rs`) lists every folder, file (path, mode,
size, SHA-256) and link of the new archive as it unpacks. Each file is copied
from the installed app, rebuilt from the installed file with a bzip2 bsdiff
patch, or carried whole (bzip2). Comparing uncompressed files is what makes
patches small: every Mac Mach-O and signature file changes each release, but
their binary patches are a few KB.

`check` reports `download_size` (the patch from the installed build, else the
full archive) and `patch`; the update reviews show it. `apply` downloads the
patch, checks its signed SHA-256 and size, verifies every installed file it
reads against the hashes in the patch, and rebuilds the release in the private
stage. It then rereads the stage: exactly the listed entries, modes and links,
with every file's size and SHA-256. On Mac it also runs the usual
`codesign --verify --deep --strict` and team check, so the notarized bundle is
byte-identical to the signed release. A missing, mismatched or failing patch is
logged and the same attempt downloads the full archive.

Release tooling: the `patches` jobs in `release.yml` (and `publish-intel` for
Intel) run `.github/actions/update-patches`, which builds `caper-updater` and
runs `scripts/native_update_patches.py generate`. That runs
`caper-updater delta` from each of the six newest retained builds; `delta`
rebuilds the target from the unpacked base exactly as a client does before
writing a patch. Patches larger than half the full archive are dropped.
`publish` lists them with `native_update_patches.py add`; the manifest stays
under 64 KiB by dropping the oldest changelog entries. After the manifest is
live, `scripts/native-update-retain.sh` removes unlisted patches and uploads the
build's full archives as `<build>-<archive>` to the `native-update-bases` draft
release, keeping six builds per archive. Patch steps never fail a release;
without bases, clients download full archives.

Check a patch by hand:
`caper-updater rebuild --platform macos-arm64 --patch FILE --from-build N
--to-build M --install /Applications/Caper.app --output /tmp/stage` rebuilds and
verifies into `/tmp/stage` (with the Mac signature check) without swapping
anything.

### Native distribution and acceptance

The [native clients](../apps/native/README.md) are separately implemented,
browser-free clients using the HTTPS API and application WebSocket, not a
native-only backend.

**CI runners.** All macOS jobs, including signed releases and TestFlight, use
the owner's existing Mac VMs: `josemac-ui` (`ci-mac-1-caper`) for macOS and
`josemac-sim` (`ci-mac-2-caper`) for iOS. They wait when those runners are
unavailable; there is no cloud Mac fallback, including on retries. Intel builds
cross-compile the app and updater for x86_64 and test under Rosetta on the ARM64
VM; this is not physical Intel or macOS 15 acceptance. The fallback watchdog
does not cancel runs with unfinished local Mac jobs.

The existing VM admission hook permits pushes and manual runs on `main`, not
schedules. Apple development suites therefore run only on manual `main` runs;
nightly development builds still cover the other platforms. CI and signed
releases retain their automatic `main` triggers. Coordinate these jobs with
manual testing on the physical Mac, since the VMs share its CPU, memory and disk.

`.github/workflows/mac-route.yml` selects only Windows/Linux runners.
`SELF_HOSTED_MAC` controls its Linux arm64 VM selection, not macOS routing.
Windows and Linux x64 jobs
are explicitly quarantined to GitHub-hosted runners after lost runner
communication, whatever `SELF_HOSTED_WINDOWS`/`SELF_HOSTED_LINUX` say; removing a
quarantine takes a change to that workflow after the host is recovered and a
complete native build passes. Host recovery is in the infrastructure runbooks
([Mac](https://github.com/joswayski/infrastructure/blob/main/docs/mac-ci-runner.md),
[Windows/Linux](https://github.com/joswayski/infrastructure/blob/main/docs/windows-ci-runner.md)):
do not start recovery while jobs or manual tests are active. Use
`./scripts/mac-ci-runner.sh recover` (Mac, refuses to restart a VM with an
active `Runner.Worker`) or
`.\scripts\windows-ci-runner.ps1 recover` (elevated PowerShell on the PC;
`sudo bash /usr/local/sbin/ci-linux-runner.sh recover` inside the Linux VM if its
runner containers stopped). Turning a routing variable off does not redirect
macOS jobs to cloud hardware.

**Download a development build:** request **Native development builds** manually
on `main` to include the Apple clients, then retrieve its artifacts:

```sh
gh workflow run native.yml --repo joswayski/caper --ref main
gh run list --repo joswayski/caper --workflow native.yml --limit 5
# Select the successful run for the intended commit, then substitute its ID:
gh run download RUN_ID --repo joswayski/caper \
  --pattern 'caper-development-*' --dir caper-development
```

Downloads need repository access and expire after 14 days. Each target has its own
directory and `SHA256SUMS`; run `sha256sum --check SHA256SUMS` (Linux) or
`shasum -a 256 --check SHA256SUMS` (macOS) **inside that directory**. On Windows,
compare `(Get-FileHash .\Caper-windows-x64-Setup.exe -Algorithm SHA256).Hash` with
the checksum file. `BUILD.json` records the tested checkout revision. Checksums
detect changed bytes, not publisher identity; these are not signed releases.

Windows Setup stores the app under `%LOCALAPPDATA%\Programs\Caper\app`; the
self-updater replaces that child directory, not the adjacent uninstaller. CI
checks the PE GUI subsystem and exercises silent install/reinstall/uninstall,
payload hashes, shortcuts and Installed apps registration. Linux's `.deb` installs
an application-menu entry with `Terminal=false`; the website's generic Linux link
opens compatibility notes. No universal Linux installer is claimed.

**Signing and stores.** PR CI receives no developer certificates and does not
publish releases or submit to stores.

- **macOS:** Developer ID distribution needs a Developer ID Application
  certificate/private key, hardened-runtime signing, notarization with `notarytool`
  and ticket stapling. Ticket-service exit 68 retries up to nine attempts with
  capped exponential backoff; invalid/revoked tickets fail immediately. See
  [Apple's distribution guidance](https://developer.apple.com/developer-id/).
- **iPhone:** TestFlight needs the registered bundle ID, App Store Connect app and
  an App Store distribution archive. After processing reports `VALID`,
  tester-group assignment can return HTTP 404; distribution retries that for up to
  ten minutes without re-uploading. Release CI archives with a local ad-hoc
  signature (`CODE_SIGN_IDENTITY=-`, `AD_HOC_CODE_SIGNING_ALLOWED=YES`) to keep push
  entitlements without creating a development certificate per runner; only export
  uses the App Store Connect key and cloud-managed distribution signing. Do not
  disable signing or clear entitlements on the archive. The app and
  `CaperNotificationService.appex` must share `MARKETING_VERSION` and
  `CURRENT_PROJECT_VERSION` (otherwise ITMS-90473); the upload script refuses
  mismatches. A correction needs a new binary with a higher build number. See
  [Apple's preparation guide](https://developer.apple.com/documentation/xcode/preparing_your_app_for_distribution).
- **Windows:** unsigned installers can be distributed, but SmartScreen, Smart App
  Control and organization policy may warn or block. Trusted signing may use a CA
  or a signing service. See
  [Microsoft's Smart App Control guidance](https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/overview).
- **Android:** debug-signed APKs are development downloads; fresh CI runners
  generate different debug keys, so replacing a CI build may require uninstalling.
  Supported upgrades need a protected release keystore and Play App Signing. Never
  commit keys.
- **Linux:** the `.deb` and archive target the documented native library baseline.

Signing credentials live in a protected GitHub environment with required
reviewers, limited to reviewed commits, imported into temporary build keychains.
Never give signing secrets to pull-request code or send them in a thread.
Publishing, store submission and changing repository environments require operator
approval.

**Locked-screen calling is a required acceptance gate.** A background-audio
declaration or foreground-service permission alone does not implement a call. A
native media engine must own capture/playback independently of a view,
authenticate Caper's SFU signaling, renew call/TURN leases and release the
microphone immediately on hangup. iOS must configure the audio session and handle
interruption/route changes; Android must start a microphone foreground service
while eligible, show its ongoing notification, and handle focus/routing/teardown.
Record OS/version/device, commit, network/forced-TURN conditions, duration,
lock/background behavior, battery, mute/deafen, Bluetooth/wired route changes and
interruption recovery when accepting it. Caper has no incoming-call ringing.

### Native generated-speech acceptance

The opt-in Linux native test
`generated_speech_public_sfu_two_native_sessions` runs two owned native sessions
against the live SFU with isolated PulseAudio (two null sinks and their monitors,
playback on a different sink from capture), generated flite speech and no physical
devices. It asserts nonzero native publication and remotely decoded PCM peaks,
stays muted until owned-only subscription checks complete, stops on any unexpected
participant or roster uncertainty, bounds the whole operation to 20 seconds with no
retry, and confirms both clients left. It has passed, establishing two-way native
speech through the SFU for that virtual setup only, not physical, Windows, Apple,
phone, TURN, sustained or interruption acceptance.

It is ignored by default and requires `CAPER_SPEECH_SFU_SMOKE=authorized-20s` and
its exact private PulseAudio socket. Each public run needs explicit approval; never
run all ignored tests as a batch.

### Windows microphone access and firewall prompts

Windows desktop capture uses the pinned WebRTC WASAPI implementation's **shared
mode**, so a Discord call does not normally prevent Caper from using the same
microphone. Caper cannot bypass Windows microphone privacy settings or another
application's exclusive access. If initialization/start fails, check **Settings →
Privacy & security → Microphone → Microphone access / Let desktop apps access your
microphone**, or select an available input. Disconnect from Discord temporarily to
isolate a sharing/driver problem (muting Discord may leave capture open).

The vendored `AdmProxy` reports Windows' built-in AEC (the Voice Capture DMO) as
unavailable and refuses to enable it, so WebRTC keeps its software AEC: with the
DMO, WASAPI refuses `StartRecording` unless the same ADM is playing, and Caper's
capture factory never plays.

**System default** means the normal Windows input/output default, including at
startup, replay and microphone reopen (WebRTC's implicit default is the
communications default, which can differ). An explicit saved device never falls
back to another microphone. Failed capture startup closes the publication gate and
reports whether initialization or start failed. A failed input switch restores the
previous route's live/muted intent while keeping old capture epochs fenced.

Before asking the API to join, the desktop checks that a saved microphone or
speaker still exists, without opening it; a missing one fails Join with an error
naming the device and fix, before the person is listed. Other device failures
surface after joining and leave at once. Leaves after a failed setup or a call use
their own connection and retry transport failures, so a device failure never
leaves a stale participant on the roster.

Windows Firewall may ask to allow `caper-desktop` when voice or an audio test first
gathers ICE candidates (the local test's private peer pair opens sockets too;
recordings stay local). Firewall permission is separate from microphone access.
On a trusted home network allow **Private networks**; leave **Public networks**
unchecked unless voice is needed there (Windows can classify a home connection as
public). Cancelling can create a block rule; review Caper's rules in Windows
Firewall if voice later cannot connect. Caper never changes firewall rules. The
installer keeps a stable executable path across updates; portable copies at new
paths can prompt again. **Publisher: Unknown** reflects the unsigned executable;
signing does not grant firewall or microphone access. See Microsoft's
[application firewall rules](https://learn.microsoft.com/en-us/windows/security/operating-system-security/network-security/windows-firewall/rules#applications-rules).
Static `parity-voice-error` and `parity-audio-error` fixtures show the error
presentation.

### Account diagnostics visibility

`DEBUG_USERS` is an optional, server-only comma-separated username allowlist.
Entries are trimmed and compared as exact, case-insensitive usernames; blank
disables it; no wildcards. Own-account payloads from email verification,
`GET /api/account/me` and `POST /api/account/profile` include `debugEnabled`
(default `false`); public rosters and other users' data never do. Eligibility
follows the current username, so it transfers if a username changes.

To change it, update `production/apps/caper` without deleting other keys, then
restart the API:

```sh
kubectl -n default rollout restart deployment/caper-api
kubectl -n default rollout status deployment/caper-api --timeout=15m
```

Open tabs re-read the own-account response when visible again or when **User
Settings** opens. Never expose `DEBUG_USERS` through web configuration or `VITE_`.

Eligible accounts see **User Settings → Audio diagnostics** and expandable
diagnostics inside **Mic test**: capture state (none, opening, failed, active,
ended), processor/fallback status, capture settings, DPDFNet hop counts and
mean/max processing time against its 10 ms budget, raw and processed track state,
peer/ICE states, sender attachment, roster/subscription counts, per-stream RTP
packet counts and audio levels, and playback context state. Counters are local and
reset on refresh. Copying is explicit; no audio, device IDs or credentials are
included and nothing is uploaded. This controls UI visibility only.

### Mobile workspace behavior

At narrow widths, web, Apple and Android navigation uses an inset, rounded channel
surface and one account bar below both the space rail and channel column. Member
panels have an explicit Close action and an outside-tap backdrop; Android Back
closes the visible panel before navigating. Apple custom dialogs are siblings of
the disabled/accessibility-hidden workspace. Exact-username entry disables
autocorrection/capitalization and supports keyboard submission, with pending
submissions guarded. Android workspace content applies IME padding.

Apple's gateway keeps one owner for authorization/socket setup and reconnect
backoff; adding a subscription while authorization is pending cannot replace the
connection task, and failed sockets are cancelled before retry. Transient
transport errors use the connection status instead of chat or member error
banners; genuine subscription/history/send errors stay visible. Swift sends
lowercase message and command UUIDs to match Rust's canonical IDs (the parity
fixture canonicalizes them too); opaque channel, author and subscription IDs stay
case-sensitive.

### Client rendering and motion

Windows/Linux chat does not deep-clone loaded messages on each UI frame. Web
speaking indicators sample every 32 ms only for live, unmuted streams, stop
sampling in hidden tabs (clearing rings without stopping capture/playback), and
keep the RMS threshold and 180 ms release. Chat timestamps share one formatter per
render. Audio popovers and profile dialogs use a 140 ms opacity/4px entrance,
disabled by reduced motion; typing labels fade/translate without blur. Virtuoso
owns incoming append-follow; local sends always reveal the pending message.

Web, Apple and Rust desktop channel/thread timelines hide scroll indicators
(Android's `LazyColumn` already omits them); Pins keep theirs. Delivery never
focuses the composer. Composer resize corrects the bottom position only when its
height changes and the reader was within 80 CSS pixels of the bottom.

Channel and thread arrivals carry existing visible messages from their old painted
positions to the new layout over the shared 260 ms ease-out, alongside the incoming
row's fade/rise. Bottom pinning and history anchoring stay immediate; initial
loads, history navigation and manual scrolling start no motion; reduced motion is
instant. Text stays sharp (no blur). Web clips the translated content so it cannot
change scrollHeight. Rust desktop compensates the whole timeline so dividers move
with their rows. Apple tracks scroll phase on iOS 18/macOS 15 and later; the older
drag fallback does not observe macOS wheel/momentum input.

`CHAT_TEST_WEB_URL=http://localhost:5174/spaces node scripts/test-chat-history.mjs`
runs the mocked 3,000-message history regression.

### Fixed wordmarks and app icons, rotating illustrations

Clickable app identities (Dock, launcher, taskbar/window, installed shortcuts and
package icons) and every client wordmark use the original plain green Caper, and
never change on navigation, hydration or rollover. Browser favicons and file-drop
illustrations choose one of the 800 bundled v3 characters per UTC day, remember it
locally and exclude the previous day's choice. Rotation happens on launch/resume or
while running; there are no alarms, background services or icon-change prompts.
Account avatars are unaffected. Native fixtures use design 0 for illustrations. The
[character catalog](../assets/avatars/README.md) names all 100 designs and their
eight hues.

| Surface | Behavior |
| --- | --- |
| In-app wordmarks | Web composes dot-free lettering with the original green face; native clients reuse their bundled original full wordmark. |
| Browser tabs | SVG plus generated 32/192px PNG favicons; localStorage shares the daily choice across reloads and tabs, checked every minute and on focus/visibility/storage events. |
| Website home-screen shortcuts | Dedicated original-mascot Apple touch and manifest PNGs at 180/192/512px; explicit standalone launches skip favicon rotation. The manifest uses `display: browser`, with no service worker. |
| iOS | Original packaged icon; no alternate-icon calls. |
| Android | The launcher and all 800 legacy aliases use the original mascot. Keep alias component names and enabled state for existing entries and pinned shortcuts; the app never switches launcher components. |
| macOS | Original AppIcon; no runtime `applicationIconImage` replacement. |
| Rust desktop | Bundled original-mascot window icon; installer, pinned-shortcut and Wayland identities stay original. |

Regenerate with `node scripts/generate-favicons.mjs` (ImageMagick 7 and librsvg),
`node scripts/generate-android-launcher-aliases.mjs --check` and
`node scripts/generate-wordmark.mjs` (its `--check` runs in web tests).
`node scripts/test-favicons.mjs <web-url> <artifact-dir>` checks favicon rotation
with a controlled clock.

### Saved default avatars

An avatar is the user's profile picture. The mascot is the default picture
everywhere. `users.avatar_id` is a persisted smallint (0–799), assigned by
PostgreSQL once on account creation and never rerolled by login, profile edits or
voice rejoining. Assignments are not exclusive. The field identifies a bundled
image, not a hash.

The collection is **100 designs × eight hue treatments**: character = ID % 100,
colorway = floor(ID / 100). v3 artwork uses real SVG paths, repairing 60 of the v2
traces; the other 40 keep their exact v2 artwork. Old v1 atlases and v2 SVGs stay
published for older clients. Masters live in `assets/avatars/vector-v3`, falling
back to `assets/avatars/vector-v2`; hand-authored details use filled paths so
every client gets the same geometry. Do not rerun the original tracing
(`uv run scripts/vectorize-avatars.py`) against a published version. Regenerate
client exports with `node scripts/generate-avatar-vectors.mjs` (`--check` runs in
web tests). Web and Rust desktop use `public/images/avatars/v3/{id}.svg`, Apple a
preserved-vector asset catalog (resolved explicitly from the framework/package
bundle) and Android VectorDrawables, all bundled offline. Versioned v2/v3 paths get
`Cache-Control: public, max-age=31536000, immutable`; publish changed artwork at a
new versioned path and never reroll IDs.

Account, member and chat author responses carry `avatarId`. HTTP history, gateway
replay and outbox publication enrich old messages from the current user record
without rewriting the log. Voice snapshots carry the authenticated account's
saved avatar, never a client-supplied ID. Missing or invalid IDs render initials;
presence dots and speaking rings stay separate.

People can replace the mascot with their own picture, including animated GIFs
(see [Custom profile pictures and space icons](#custom-profile-pictures-and-space-icons)).
A custom picture never changes `users.avatar_id`: removing it restores the same
mascot. There is no public mascot gallery, and arbitrary image URLs are never
accepted.

### Login email abuse protection

The API rejects reserved `example.com`, `example.net`, `example.org`, `.example`,
`.test`, `.invalid` and `.localhost` email domains, including subdomains, with 400
before creating challenges or calling SES. This is not a mailbox-existence check.
The durable email/IP/global limits apply before sending. SES sending quotas limit
capacity, and SES account-level suppression (not a Caper list) handles suppressed
recipients; monitor SES bounce/reputation metrics separately.

There is no external account-lifecycle provider: lifecycle state is the nullable
`users.deleted_at` column.

### Database migrations and tests

When `DATABASE_URL` is set, API startup first applies embedded migrations through
`MIGRATION_DATABASE_URL`, closes that connection, then opens the runtime pool and
starts serving. Missing or failed migrations prevent startup; there is no Job or
manual migration step. Both secrets are available to the API pod (an accepted
tradeoff); neither belongs in web. SQLx advisory locks serialize concurrent
startup migrations, so migrations must use direct port 5432 (pooled 6432 is
rejected, with no runtime-URL fallback). Both URLs target the existing
`/caperchat` database; startup creates no database or roles. It connects with
`DATABASE_URL` to identify the runtime role, then grants it only the table and
sequence access it needs. The migration connection sets `search_path=public` so a
same-named user schema cannot redirect tables or the `_sqlx_migrations` ledger;
the runtime pool does not (PlanetScale PgBouncer rejects that startup override),
so runtime queries target `public.` explicitly. SQLx validates applied migration
history, so never edit or rename an applied migration and never roll the API back
to an image that predates an applied migration; fix forward.

`caper-api --migrate` (or `cargo run --locked --release -p caper-api -- --migrate`)
migrates and applies runtime grants without serving; without `DATABASE_URL` it
only migrates.

Account login is activated by populating the account, database, SES and auth-limit
keys in `production/apps/caper`, projecting them into the API through
infrastructure, and verifying SES workload identity.

Database tests are ignored in the ordinary Rust suite; CI runs them against
disposable Postgres 17. Locally, use a **disposable** Postgres whose role can create
databases (SQLx creates a fresh database per test; never a production URL):

```bash
DATABASE_URL='postgres://user@127.0.0.1:55432/postgres' \
  cargo test --locked -p caper-api --test accounts -- --ignored
```

Orb setup installs Postgres binaries; initialize an isolated cluster with
`/usr/lib/postgresql/15/bin/initdb -D /tmp/caper-test-pg -A trust -E UTF8` and run
it with `amp orb service start account-test-db --command '/usr/lib/postgresql/15/bin/postgres -D /tmp/caper-test-pg -h 127.0.0.1 -p 55432 -k /tmp'`.
Trust authentication is only for that loopback test server. The full ignored
suite (never against shared databases):

```sh
DATABASE_URL='postgres://user@127.0.0.1:55432/postgres' \
CHAT_TEST_DATABASE_URL='postgres://user@127.0.0.1:55432/postgres' \
CHAT_TEST_VALKEY_URL='redis://127.0.0.1:6379' \
TEST_VALKEY_URL='redis://127.0.0.1:6379' \
  cargo test --locked -p caper-api -- --ignored --skip tests::shared
```

## Account-global direct messages and deferred mobile push

One-to-one DMs belong to two accounts, not to a space. Personal notes use the same
private stream with your own account as both participants. Start by exact username
(the field suggests people you share a space or DM with); the canonical pair
(including a self pair) has one conversation even when started concurrently. The
list appears below channels in every space, including for accounts with no spaces;
narrow web and mobile clients expose it in Browse. Leaving or deleting a space does
not delete DMs, and space owners have no DM access. This is server-authorized
privacy, not end-to-end encryption. DMs support attachments and deletion like
channels; group DMs and DM voice calls are not implemented. Text limits, send
limits, typing, history pagination, idempotent sends, outbox and replay rules are
the same as channels.

The sidebar pins your real account name with a `you` label; opening it lazily
creates or reopens your notes (displaying the row writes nothing). Nobody else can
read, send, mark read or subscribe to your notes, and they never push. Space owners
have an **Invite people** row opening the username invitation form; members and
accounts with no space get **New message**. The heading's plus starts a DM; desktop
web, Rust desktop and macOS reveal it on heading hover or focus, touch clients keep
it visible.

API contracts (account authentication required):

- `GET /api/dms` →
  `{conversations:[{id,peer:{id,username,displayName,avatarId},lastSeq,readSeq,status,blocked}]}`.
- `POST /api/dms` with `{username}` returns that pair's conversation; your own
  username returns personal notes with `peer.id` equal to your account ID.
- `POST /api/dms/{id}/read` with `{seq}` monotonically advances the caller's read
  cursor, bounded to the channel head. Sequences are decimal strings.
- Message history/send/typing and gateway chat subscriptions use the channel
  endpoints with the DM ID. History sets `channel.direct=true` and an empty
  `space.id`. Only the pair can access it; media endpoints reject DMs.

Lists refresh every 15 seconds; the open conversation receives live messages.
Unread means the head is beyond the account's read cursor (account-wide, not per
device). New conversations are limited to 20/minute and 1,000 per initiating
account (requests others start with you don't count) and require a completed
profile (403 `complete profile required`). Each account may start at most 60 an
hour by username, counting lookups that find no one or a person not accepting DMs
(429 `too many conversations; try again later`).

### Message requests, blocking and DM privacy

Anyone can start a DM by exact username. People who share an active space get a
normal conversation. Anyone else's first DM is a **message request**: the sender
sees "Waiting for @name to accept" above an enabled composer and can keep writing;
the recipient sees one **Message requests** row with a count at the top of their DM
list. A request never adds to unread dots, badges or sounds. Opening one shows its
history read-only with **Accept**, **Decline** and **Block** in place of the
composer; opening does not accept it. Messaging the sender (for example with **New
message**) accepts it. Declining hides the request from the recipient only; the
sender is not told, and later messages do not bring it back.

Blocking is available from a DM's header, from message actions on another account's
message, and from a request. Clients confirm first. A block:

- stops DMs both ways. The blocked person can't start a DM or request, and their
  sends in an existing DM fail with the same refusal a privacy setting gives. You
  see "You blocked @name" with **Unblock** instead of the composer.
- stops every other interaction in that DM: reactions, pins, edits and typing get
  403 (`dm_not_accepted` for the blocked person, `dm_blocked` for you), and none
  accepts a pending request. Clients show the refusal on the message, treat a
  refused send as final (Edit or Dismiss), and hide reactions, pins and edits in a
  DM you blocked.
- declines any pending request from them.
- stops their space and private-channel invitations to you: inviting you returns
  the same 404 as an unknown username, and earlier invitations are hidden while it
  lasts.
- hides their messages wherever you share a conversation. Each run of consecutive
  messages becomes one row, "⊘ N blocked messages — Show", in channels, threads and
  DMs; **Show** reveals that run until you hide it or leave. The notice and revealed
  messages share a neutral background and leading border. Their messages play no
  sounds and their typing isn't shown. Pins, who-reacted lists and thread avatars
  still list them.
- isn't visible to the blocked person.

Unblocking keeps the record (`deleted_at`). The web Edit profile dialog and
`/profile`, and native user settings ("Privacy and blocked accounts" on Android and
Apple, Settings on Rust desktop), list **Blocked accounts** with **Unblock** and
hold **Who can start a DM with you**:

- **Anyone** (default): people outside your spaces send a request.
- **People in my spaces**: only people who share an active space.
- **No one new**: nobody can start a new conversation; existing ones stay open.

The same settings end with **Delete account → How to delete your account**, which
opens `https://caper.chat/privacy#delete-account` (web uses `/privacy#delete-account`).
Google Play requires this in-app path; deletion is an emailed request handled by
hand, as that page describes.

API contracts (account authentication required):

- `GET /api/dms` `status`: `accepted`, `outgoing` (you asked) or `incoming` (a
  request for you); `blocked`: you blocked the peer. Declined incoming requests are
  omitted. Clients treat a missing `status` as `accepted`.
- `POST /api/dms` may return 403 `dm_not_accepted` (their setting, or they blocked
  you) or `dm_blocked` (you blocked them), or 429 after ten new requests in an hour.
- `POST /api/dms/{id}/accept` returns the accepted conversation (idempotent).
- `POST /api/dms/{id}/decline` → 204; 404 unless it is a request for you.
- `GET /api/blocks` → `{blocks:[{id,username,displayName,avatarId}]}`, newest first.
- `PUT`/`DELETE /api/blocks/{accountId}` → 204, idempotent. You can't block
  yourself; at most 1,000 accounts. `accountId` is the ID clients see as
  `author.id`, `peer.id` and member `id`.
- `GET`/`PUT /api/account/privacy` with `{directMessages:"anyone"|"spaces"|"nobody"}`.
- Sending in a DM returns 403 `dm_blocked` or `dm_not_accepted` while either person
  blocks the other.

The parity fixture models the same contract: `POST /__fixture/control
{"messageRequest":{}}` adds a request from Jordan (`jordan`, who shares no space),
and `PUT /api/blocks/member000001` blocks Maya, whose two seeded `#general`
messages then collapse.

### Mobile push (phase 1): direct APNs and FCM

Phone push with sender avatars for DMs and channel messages sends directly to APNs
(iPhone) and FCM HTTP v1 (Android); there is no SNS or push vendor. It stays off
until `NOTIFICATIONS_ENABLED=true`, and `GET /api/push/config` advertises a platform
only when it is listed in `PUSH_PLATFORMS` and its credentials load. With it off the
endpoint returns `{"platforms":[]}`, which keeps native permission prompts hidden;
notification levels and mutes still save.
[notifications.md](notifications.md#who-gets-a-notification) holds the contract: who is
notified, the phone hold while you are active elsewhere, payloads, routes, retries
and configuration.

- Each committed account-authored message (sends and forwards, never edits) writes
  one `notification_jobs` row in its send transaction. Workers in the API role
  expand it into per-recipient notifications whenever chat is enabled;
  `NOTIFICATIONS_ENABLED` gates phone delivery, not expansion or local OS
  notifications. Provider sends hold no database connection.
- Registrations belong to the account session: logout revokes them, delivery
  rechecks the session, and dead tokens reported by Apple or Google are revoked.
- Provider keys live only in `production/apps/caper` (and `staging/apps/caper`),
  written by `scripts/store-push-credentials.sh`; never in client builds or source
  control. Logs never contain tokens, keys or message text.
- Provider acceptance does not prove device receipt. Validate on physical Android
  and signed iOS devices before adding a platform to `PUSH_PLATFORMS`.
- Open browser tabs and running macOS/Windows/Linux apps show local OS
  notifications from an account-wide gateway feed, with sender artwork,
  focused-conversation suppression, reconnect deduplication, click navigation and
  logout cleanup. macOS uses an upload or sender-avatar attachment rather than
  replacing the app icon. Closed-browser Web Push, macOS APNs and keep-running-in-
  tray are not implemented.
- Phone and local notifications can preview one image or video through a
  15-minute signed URL: images use their original (up to 50,000,000 bytes); videos
  use only their chat poster (at most 512 KiB), never video bytes. The receiving
  device creates its own temporary PNG; nothing new is stored in R2. Captions
  survive; missing posters, documents, oversized/expired media and failures fall
  back to text. See [limits and client behavior](notifications.md#uploaded-file-previews).
- iOS requires Communication Notifications on the parent App ID, refreshed
  profiles and an embedded `.notifications` extension profile; see
  [notifications.md](notifications.md#apple-apns).

## Phone navigation and drawers

These rules apply to narrow layouts (web ≤760px, Android and iPhone narrow
layouts, and narrow Rust desktop windows). Wide layouts keep their side-by-side
panes.

### Browse slide

- **Layers:** Browse (space rail, channel list, DMs and the account/voice bar)
  stays still underneath. The conversation is one full-height layer above all of
  it, including under the iPhone status bar and home indicator, and clips only
  sideways. With Browse shown, the conversation is fully off screen, travelling
  its width + 48 px/dp/pt so its edge shadow leaves too; it stays laid out
  (keeping scroll position and drafts) but is hidden, inert and not exposed to
  accessibility. Rust desktop draws Browse first and the conversation over it,
  with a backdrop so presses never reach Browse.
- **Buttons:** Back to Browse, Close navigation and choosing a channel or DM
  slide over 300 ms × the remaining fraction of travel (at least 160 ms) with
  `cubic-bezier(0.33, 1, 0.68, 1)`. The member list's button slides use the same
  timing.
- **Swipes:** a horizontal swipe can start anywhere on the content, headers,
  channel rows and space rails, not just at an edge. Right from chat opens
  Browse; left from Browse returns. Inputs and text editors, audio sliders,
  open dialogs/menus, text selections, multi-touch and vertical scrolls keep
  their own gestures (Apple excludes only the channel-search field's bounds).
  Web and Rust desktop navigate on touch only, so mouse selection is preserved.
  Swipes never select a channel.
- **Following and release:** the drag follows the finger 1:1 from the touch slop
  without a jump. A release past halfway, or a flick of at least 300 px/pt per
  second after 40 of travel, commits; otherwise it springs back, and a flick back
  toward the start cancels. Released swipes finish from the held pose with a
  velocity-matched, non-overshooting curve: 320 ms for a held/slow release,
  160–320 ms for a flick, with initial speed capped at three times the remaining
  distance divided by the duration. Rust desktop uses
  `cubic-bezier(0.2, 0.2·slope, 0.35, 1)` and measures release speed against an
  earlier frame so a same-frame flick still commits.
- **Catching:** a finger landing on a moving slide picks it up where it is
  drawn.
- **No scrolling mid-slide:** once a drag follows a slide, the timeline or list
  it started on stops scrolling until the finger lifts (Android consumes moves in
  the Initial pass; Apple sets `scrollDisabled`). Web cancels the slide when the
  browser takes a vertical pan.
- **Release taps:** the lift that ends a swipe never activates the control under
  the finger (images, links, mentions, avatars, reactions, thread/edit links,
  voice controls, rows, rail). Apple's `SwipeTapGuard` keeps a window renewed by
  every move (0.35 s) and by the lift (0.12 s), and `DrawerSwipe` stops
  hit-testing content once a slide follows the finger. Android consumes moves
  once a drag claims them and cancels presses after twice the touch slop. Web
  suppresses the release click but never a keyboard/assistive click
  (`detail` 0). In Rust desktop the swipe owns the drag.
- **Apple pose:** `DrawerPose` draws each drawer layer from one value per frame
  (under the finger, along the recorded `DrawerSettle` curve in a `TimelineView`,
  or at rest), applied through `DrawnOffset`, a geometry effect with no
  animatable data. No SwiftUI animation touches the pose, so a release cannot
  overshoot either end. A released swipe's recorded slide leads its view switch.
  Web, Android and Rust desktop keep one animated position per drawer.
- **Implementation:** web moves the absolutely positioned `.stage` with WAAPI
  while `data-browse-slide` on the room shows both layers (no View Transitions).
  Android uses `Animatable` with `browseTravel`; Apple uses the scoped drawer
  above; Rust desktop uses `browse_travel` and `NavigationSettle`.
- Opening Browse or Members hides the keyboard. Drafts, staged files and history
  survive. Reduced motion (web `prefers-reduced-motion`, Android animator scale,
  Apple Reduce Motion) switches at once.

### Member drawer

Members (Show/Hide member list) is a header icon button right of Pins on every
width. On phones the list slides in from the trailing edge over the
conversation, below its header, with Browse's timing and release rules; its 25%
backdrop fades with it. A left swipe on the conversation opens it; a right swipe
on the panel or backdrop closes it. Header button, Close, backdrop and Android
Back animate it; Android only intercepts Back while the list shows. The closed
list takes no touches. Opening Browse with the list open carries it along. The
panel has 16px outlined corners and the conversation's edge shadow (web
`-12px 0 32px rgb(0 0 0 / 0.45)`, Apple radius 16 / x −12 / 45%, Android 16dp
elevation, Rust desktop egui edge shadow). A 1:1 DM title keeps its menu for
Block/Unblock on Apple and Android, and phones also have Block there on web.

On web between 761 and 1099px, Members sit beside the conversation while it keeps
320px, otherwise they float over its right edge below the header.

### Media viewer swipe-down

Swipe-down closing locks direction on the first move, follows the finger 1:1 and
springs back or carries off smoothly. Android's backdrop fades as it leaves and
double-tap zoom animates.

## Conversation behavior across clients

### Resizable thread panes

Web, wide Android and Apple layouts allow dragging the thread's leading divider
(left widens). Left/Right adjust by 10, Home/End select the bounds and
double-click/tap resets to 340px/pt/dp. Preferred widths range 300–600 and
persist locally. The pane reserves 300 for the main conversation, or splits the
area equally when it is under 600; shrinking the window does not overwrite the
preference. Phone threads are full screen without a handle. Rust desktop uses a
resizable egui `SidePanel` (300–480).

### Optimistic feedback and thread loading

Pin/unpin updates the initiating client's attribution, count and Pins list
before HTTP confirmation. Edits preview immediately while the editor keeps the
draft until confirmation or conflict. Other readers receive only
server-confirmed events. Failed or abandoned intents reveal the latest confirmed
state. Pending edit previews show mentions as plain text until the server
resolves them. macOS buttons register AppKit pointing-hand cursor rectangles
(not for disabled controls).

Selecting the already open thread does not reset it or start another request.
Thread pages keep only pagination metadata; message content stays in the channel
timeline, so reopening reuses rows and older-page boundaries. Channel resets and
access loss discard that cache (it is not offline storage). Web prefetches
threads on hover/focus; a background response never opens a closed panel or
replaces the selected thread, and speculative failures stay silent. Cold loads
show neutral message-shaped skeletons; Retry is explicit.

Web keeps the virtualized conversation mounted, dimmed and inert behind Pins, so
returning shows the existing viewport. Android keeps its lazy-list state across
Pins; Rust desktop uses a separate Pins scroll area; Apple presents Pins as a
sheet.

### Workspace loading and attachment control

Web renders a server-safe branded skeleton during account/list loading and the
first conversation. The landing channel does not wait for the DM list; DM deep
links wait for their access list. Initial message loading uses the neutral thread
skeleton on web, Android and Apple; Rust desktop has its own history skeleton.
Every main composer reserves its attachment control, disabled until upload
availability succeeds (and disabled if it fails); text composition never waits.

### Drafts across conversation switches

Each channel/DM keeps its text, staged files and thread reply drafts until sent,
removed, or sign-out. Uploads continue while their conversation is closed and are
not re-uploaded on return; failed files, or those past the 23-hour renewal point,
are renewed on return (Apple restarts uploads that were in flight). Only explicit
Remove, sign-out or losing access cancels a reservation. Drafts are in memory:
closing or reloading discards them.

### Composer edge and typing status

Timelines scroll right to the composer with no reserved blank strip. Typing status
sits below the input and takes no space when idle. Keyboard and home-indicator
clearance below the composer is kept.

### Profile-card actions and inline DMs

Other people's profile cards offer **Block** (with confirmation) or **Unblock**
inside a More profile actions (ellipsis) menu, plus Open conversation (speech
bubble) and Close in the header. Blocked cards hide Message and Invite. Your own
card says You with **Edit profile**. Authors with an account ID but no known
username can be blocked but not messaged or invited.

**Invite to space** lists the account's owned spaces and sends a seven-day pending
invitation through the owner-only API; the person must accept. Success disables
that destination for the card session; failures stay inline and retryable.
Blocking prevents DMs/requests and invitations and collapses the blocked
person's text; it does not remove membership or mute voice.

Below a divider, each card has an independent per-person composer with the
channel composer's shell (Attach, field, Emoji, eggshell Send): text, emoji and up
to 10 files, including file-only messages. Send resolves the DM, uploads through
the normal pipeline, and on confirmed delivery clears that draft and opens the
conversation. Known incoming requests stay read-only until accepted; block and
privacy refusals keep the draft. Drafts survive close/reopen while signed in.
An unconfirmed POST freezes its text, attachment IDs and client message ID, and
Retry resends that same command (web's Edit warns the earlier copy may have
arrived). Outside clicks/taps close cards (except during sends or open pickers);
iPhone uses a backdrop-only tap recognizer on the sheet's presentation container.

Clipboard files: web accepts paste-event files; Apple uses its native Paste for
file URLs and file-like bytes; Android and Rust desktop put **Paste from
clipboard** in the Attach menu (Android also accepts keyboard `commitContent`;
Rust repairs egui's missing image-only Ctrl+V).

Channel, thread and profile composers have an Emoji button that opens the
reaction picker (popover on web desktop, Rust desktop and macOS; bottom sheet on
touch) and inserts at the caret within the 4,000-character limit.

### Emoji names inside composers

Composers show the catalog name when an inline emoji is hovered or held; joined
emoji, flags and keycaps are one target. Web, Android and iOS use a 400 ms hover
and 500 ms hold, keeping a completed hold readable for 1.5 s; movement, editing,
scrolling or leaving dismisses it. macOS uses text-range AppKit tooltips; Rust
desktop uses egui's hover/long-touch timing. Web's lookup normalizes padded
keycap code points.

### Reply composer focus

Explicit **Reply** and **Reply in thread** (including from Pins) open the
destination and focus its composer without a second tap, preserving drafts and
never sending. Ordinary thread browsing does not raise the mobile keyboard. Web
requests focus inside the initiating click, after mounting the destination and
releasing the menu's focus trap, so iOS accepts it. Web and Rust desktop allow
typing while thread history loads but block sending. Android and Apple carry a
one-shot focus request until covering sheets dismiss.

### Input focus stability

Text-entry controls on narrow or coarse-pointer web layouts use at least 16px
text (including readonly invite links, search fields and selects) to avoid
Safari's focus zoom without disabling user zoom. Inline username suggestions on
web, Android and Apple stay in place when focus moves to another control; choosing
a person, Escape, clearing the query or a pending request dismiss them. Rust
desktop shows them in an overlay.

### Apple interactive keyboard dismissal

Apple channel/DM and thread timelines keep interactive keyboard dismissal but
suspend automatic bottom scrolling while the timeline scrolls; viewport
measurement continues and pending reaction-follow state is cleared, so no
competing `scrollTo` fires after the drag. Incoming messages and pending sends
respect the guard. Android and web dismiss the keyboard outright on a vertical
drag.

### Primary actions and entrance motion

Primary actions (sign-in, dialog confirmations, Join channel, Accept, Send/Save,
Forward, Test microphone) use the eggshell fill (`--action` `#F1EDE4`) with
blackout labels on every client. Hover brightens, press deepens, and disabled
primaries step back to neutral chrome. Sign-in fields use a terracotta border with
a soft 2px focus ring.

Shared tokens: `--ease-out` `cubic-bezier(0.16, 1, 0.3, 1)`, `--motion-fast`
140ms, `--motion-base` 200ms, `--motion-slow` 260ms.

- Sent and live messages fade in with a 6px settle only when they arrive while
  the reader is at the live end (`Arrivals` on every client): never on first
  loads, channel switches, older/newer pages or jumps. A pending send animates
  once.
- Dialogs rise 8px from 98% while the scrim fades; menus, cards and tooltips rise
  4px; touch sheets rise from the bottom; the thread panel slides in 12px;
  attached files pop in and upload progress glides.
- Closing is instant. Reduced motion turns entrances off; Rust desktop has no OS
  signal for entrances.

### Consistency rules

- Loading older messages keeps the reader's place; a growing draft, suggestions
  or the keyboard never push the newest message out of view; resizing keeps the
  newest message in view for a reader already there.
- Dialog headings stay fixed above a scrolling body. Web, Apple and Android
  dialogs fit their content; Rust desktop shells stay fixed-size
  (`modal_shells_do_not_recenter_for_errors_or_pending_content`). Backdrop clicks
  never discard unsaved Channel settings or a renamed space.
- Touch targets are 44px/44pt/48dp; disclosure chevrons rotate; timers use
  tabular digits; keyboard focus returns to User Settings after its dialogs.
  On macOS, native menu activation follows the system's Keyboard navigation
  setting: focus and Space activation require it; pointer reopening does not.
- Every client hides **Copy text** for attachment-only messages and says
  **Message history**, **New direct message**, **Pin message**/**Unpin message**
  and **Reply in thread** in message menus.
- Changing channels or closing a thread closes its emoji picker; files dropped
  over a dialog never reach the hidden composer.
- Android runs edge-to-edge with `adjustResize`, consuming system-bar insets
  before IME padding; typing indicators are throttled like web (at most one start
  per 500 ms, stop after 500 ms idle).
- Rust desktop hover fills use the shared 6% white `--surface-hover` wash.
- Native clients reconcile `GET /api/spaces` and the open space every 15 seconds
  and on returning to the app; a local change made meanwhile wins and a failed
  read keeps what is shown. Removal shows "This space is no longer available."
- Space and channel member forms pre-check members and pending invitations and
  say "Invitation sent. They must accept before joining." / "This person already
  has access to this channel."

## Space and channel unread indicators

Every client marks rail spaces the same way. A space with unread messages or
direct mentions shows a small light dot in the rail margin. A terracotta count on
the tile's bottom-right shows only unread messages that @mention or reply to you
(`mentionCount`, capped at 99+). The selected space has a terracotta ring and
glow that blooms in (skipped with reduced motion); hover adds a neutral outline.
Joined channel rows also show dots, and mentions in unjoined channels show a dot
in Browse channel lists. Notification levels, pauses, OS permission and mutes do
not hide unread activity.

App-icon badges use the separate `badgeCount` field on the same endpoint; see
[the app-icon badge contract](notifications.md#app-icon-badges).

`GET /api/spaces/unread` returns only unread spaces/channels, with decimal-string
`lastSeq` and `readSeq`, `unreadCount` and `mentionCount`. `mentionCount` counts
unread messages that mention the reader or quote-reply to their messages,
excluding forwards. Joined channels count other unblocked, active authors' main
messages and broadcast replies. Accessible unjoined channels and non-broadcast
thread replies count direct personal mentions only. Membership, private grants
and soft removals constrain every result. Edits, reactions and pins do not create
unread messages.

`POST /api/spaces/{space}/channels/{channel}/read` accepts `{"seq":"42"}`. Read
positions (`channel_reads`) are per account/channel, monotonic and capped at the
channel head. Only a loaded foreground conversation acknowledges its committed
cursor; prefetch, background views, mobile Browse and older search windows do
not. Clients refresh on launch/foreground, every 15 seconds and about a second
after live notification activity (one request per burst). Failed reads retry;
account/request revisions fence late responses. Existing history starts unread
(no backfill).

Channel visits capture `readSeq` before acknowledging. Every client places a
terracotta **New messages** divider before the first incoming, unblocked channel
message after it. The divider breaks author grouping and stays for the visit;
returning captures a new boundary. Live arrivals on an already-read visit add no
divider. If unloaded older history may contain the boundary, the divider waits.
DMs, thread panels and feedback visits have no divider; servers without `readSeq`
show none.

The query inspects only channels whose stream moved past the reader's position
and finds mentions through the partial GIN index `messages_mentions` (migration
`202610110002`, built `CONCURRENTLY` outside a transaction).

### Space-rail voice activity

Every client shows a small speaker on a space's icon: white on neutral chrome when
voice is occupied, white on green when this client is connected there. Joining
alone never turns it green. Authenticated `GET /api/spaces/voice-activity`
returns `{ "spaceIds": [...] }`: member spaces with non-monitor participants in
channels the caller has joined and can access (same authorization as joining
voice). Reservations and monitor-only rooms do not count; no identities or
channel IDs are returned. Clients poll every 15 seconds (web skips hidden tabs),
clear activity on failure/logout, and derive the green badge from local
connection state.

## Discord-style message replies

**Reply** sends an ordinary message with a compact quote of the source; the
original keeps its sequence and thread count. Replies can be replied to, edited,
reacted to, pinned, forwarded or threaded. **Reply in thread** is separate.
Replying does not mention or ping the source author.

While composing, clients show **Replying to [author]** with up to two lines of the
original (from the full local snapshot) above the input; attachment-only sources
show **Attachment**; blocked sources show neither identity nor text. The X removes
the target without clearing the draft.

`POST /api/chat/channels/:id/messages` accepts optional `replyToId`, independent
of `threadRootId` and `broadcast`. It resolves within the authorized, locked
channel: channel replies quote top-level/broadcast messages; thread replies quote
that thread's root or messages. Missing, cross-channel or out-of-thread targets
return 404. Retries bind the target; changing it returns 409. A definitive
attachment refusal allows reuploading with a new command ID while keeping the
target and snapshot.

The optional `replyTo` field is `{id, author, text, fullText?, hasAttachments,
threadRootId?, deleted?, attachments?}`. It is a send-time snapshot with no
recursive quotes or resolved mentions; source edits do not rewrite it. `text` is
bounded to 200 Unicode scalars for older clients; `fullText` is added only when
needed, up to 4,000 scalars, and clients display `fullText ?? text`. Optional
`attachments` are projected from the current same-channel source at
HTTP/outbox/replay delivery and signed like message attachments (never persisted
in the quote; deleted sources expose none). Forward projections remove the quote.
Clicking a quote loads the source through authorized history.

Display, shared by every client:

- The quote is a separate neutral card with **View original**. Small pewter cup
  sockets touch the original and replying avatars, joined by a thin curved string
  (web and Rust desktop share the SVG; Android and Apple draw matching vectors).
- Collapsed quotes wrap to three lines with no character cutoff. **Show more** /
  **Show less** appear only when needed, as small leading-aligned link-blue
  underlined text with 44px touch targets; expansion is immediate.
- Scrolling flexes the string and rotates the cups; a damped spring settles with
  no idle motion. Each reply derives its spring from its own client message ID
  with UTF-8 FNV-1a: stiffness 560–960, damping 28–42, scroll impulse 1.0–1.4,
  travel 5–7 logical pixels, speed capped at 16× travel. Reduced motion keeps it
  static; Rust desktop reads the OS animation preference once per run.
- Incoming replies to your messages get a 12% terracotta wash and accent edge.
  Quote cards show up to three 44px attachment previews and an overflow count;
  spoilers and unfinished files stay generic, and videos never load for a
  thumbnail. Clicking a preview opens the original message.

## Own profile refresh

A successful display-name/username save updates retained self identities by
account ID in the editing client: members, invitations, self-DMs, chat authors,
pins, thread participants, forwarded authors, reaction lists, profile cards and
the local voice roster, without leaving a call or losing drafts or replay
cursors. Other sessions use their normal refresh; there is no realtime profile
event for this.

## Release highlights (What’s new)

Every client shows an after-update highlights view and a User Settings history.
The first successful fetch on a new installation records a baseline silently.
Later builds show unread highlighted releases newest-first; maintenance-only
releases advance the baseline without opening anything. Dismissal acknowledges
the displayed snapshot; failed fetches do not, and rollbacks never lower the saved
sequence. Preferences are local per installation/API origin/platform. Automatic
presentation waits for the signed-in workspace and defers during calls or other
modals; missing publications retry at most once a minute.

History groups releases by UTC publication date ("October 18, 2026"). Native
releases link their `0.1.<build>` version (optional `version` metadata in schema-1
snapshots); web and legacy entries link a short commit. The header shows the Caper
face, title and installed version (`Version 0.1.<build>` or `Web build <commit>`).

Mac/Windows/Linux offer **Check for updates** in the account menu and Settings.
Automatic checks show a persistent **Update available** button in its own row
above the account controls (rainbow edge with a light sweep every eight seconds,
stopped by Reduce Motion, disabled controls or background windows). It opens a
centered review with What's new expanded; Later, Escape or the backdrop close it
without forgetting the update. Desktop clients pass their revision to the bundled
updater, which reads current/target snapshots and stacks skipped releases' highlights;
if the installed build's snapshot is missing, it locates it by revision, then by
`version`, else shows only the target and reports incomplete history. Installing
marks that build's highlights read. After a failed update, **Send this to the
Caper team** posts the error, build and target as `@feedback` in personal notes.
Missing metadata never blocks a signed update.

### Generation and storage

After Ship rolls out Caper web, infrastructure dispatches `whats-new.yml` on Caper
`main` with the confirmed image SHA, authenticating as the Godis deploy GitHub App,
and holds the web deploy queue until generation finishes. The native release calls
the workflow twice: a draft while apps build (`--save-answer`, publishes nothing),
then a publish after the Apple Silicon and desktop builds and before the update
manifest (`--answer` reuses the draft only if its evidence hash is unchanged). A
generation failure never holds back a release.

`scripts/whats_new.py` reads the previous per-platform revision, first-parent
commit messages, relevant runtime diffs and recent highlights, and makes one Claude
Messages API request selecting at most five highlights. Output must cite commits in
range, have client/shared runtime evidence and pass schema/text/claim checks; an
empty list is valid. Only repository evidence is sent, never user data. One request
sends at most 400,000 characters; over that, server-only diffs drop and client diffs
shorten (`diffTruncated`), keeping every commit's subject, body and paths.
Highlights describe behavior reachable in the released client, not native migration
progress. The separate AppKit/UIKit parity hosts are excluded from product paths
(Win32/GTK hosts are outside the eligible roots). Shared-code changes still need
evidence of released behavior; mixed commits describe only that behavior. Update
these boundaries and the editorial prompt when replacement hosts actually ship.

Public GitHub prereleases `whats-new-web`, `whats-new-android`, `whats-new-ios`,
`whats-new-macos`, `whats-new-windows` and `whats-new-linux` hold immutable
`<source-sha>.json` snapshots and rolling `latest.json` heads. Immutable assets
publish before heads; exact reruns reuse published bytes. Never overwrite or delete
immutable assets. Each snapshot is at most 64 KiB, holds at most 30 releases with
positive JavaScript-safe sequences, and plain-text titles/bodies of at most 100/500
code points.

The homepage's Latest changes shows the newest eight highlights from
`whats-new-web/latest.json`, cached per web pod for 60 seconds (stale-while-
refresh, 3-second cold fetch limit), falling back to the build-time list or recent
`main` commit titles.

`GET /api/whats-new?platform=<platform>&revision=<40-lowercase-hex-sha>` reads only
that immutable asset from the Caper repository (never `latest.json`), with bounded
fetches, per-revision coalescing and a 60-second success/failure cache. Up to 32
loaded revisions are kept separately from up to 32 in-flight/failed lookups; a new
revision replaces the oldest settled failure and gets `502` only when all 32 are in
flight.

### Configuration and repair

- `ANTHROPIC_API_KEY` is a Caper Actions secret
  (`gh secret set ANTHROPIC_API_KEY --repo joswayski/caper`). Removing it stops
  generation without affecting deploys.
- Ship's dispatch identity is stored in infrastructure with
  `./scripts/store-ship-dispatch-secrets.sh --profile production` (from the
  infrastructure checkout).
- The generator defaults to `claude-opus-5-5` at `high` effort with
  `fallbacks: "default"`. Optional Actions variables `CAPER_RELEASE_NOTES_MODEL`
  and `CAPER_RELEASE_NOTES_EFFORT` (`low`–`max`) override them. A key not scoped
  to one workspace needs the `ANTHROPIC_WORKSPACE_ID` variable (otherwise HTTP 400
  `invalid_request_error`).

Repair a failed generation only for a revision that actually shipped:

```bash
gh workflow run whats-new.yml --repo joswayski/caper --ref main -f audience=web -f git_sha="$CAPER_SHA"
gh release download whats-new-web --repo joswayski/caper --pattern "$CAPER_SHA.json" --dir /tmp/caper-highlights-review
curl --fail --silent --show-error "https://caper.chat/api/whats-new?platform=web&revision=$CAPER_SHA" | jq .
```

Use `audience=native` and the matching platform tag for native releases. Never
generate notes for a branch or unshipped revision.

## Custom profile pictures and space icons

People can replace their saved mascot with their own picture, and space owners can
give a space an icon. Both may be still images or animated GIFs; GIFs animate
everywhere (members, chat authors, replies, threads, DMs, reactions, voice tiles,
invites, the rail), and reduced motion shows the first frame. Removing a picture
restores the same saved mascot (`users.avatar_id` never changes) or the space's
initial.

### Storage, delivery and retention

Pictures reuse the [upload pipeline](#uploads-and-attachments): an `assets` row
(`purpose` `avatar` or `space_icon`), bytes `PUT` to the private R2 bucket at
`original/{id}`, and delivery through the signed CDN Worker
(`/assets/{id}?exp=&sig=`). Migration `202610140001_pictures.sql` adds the
purposes, `assets.space_id`, `users.avatar_asset` and `spaces.icon_asset` (asset
external IDs). Nothing stores a URL: responses carry `avatarAsset`/`iconAsset`,
and API JSON and every gateway frame add a signed `avatarUrl`/`iconUrl` on the way
out (24–48 h, re-signed daily, like attachments).

Message payloads never store a picture. Wherever messages leave (history, threads,
pins, edits, forwards, send responses, outbox and replay), one batched lookup sets
each person's current picture by account ID, including nested people, so an old
snapshot never shows a changed or removed picture.

**Old pictures are kept.** Replacing or removing a picture leaves its row and R2
object in place. They do not count toward the 10 GB file allowance and appear in
**Previous pictures**. Abandoned, cancelled, rejected or orphaned picture uploads
are purged like other unsent uploads; the purger skips completed pictures.
Migration `202610190001_avatar_restore.sql` adds `avatar_selected_at`, so uploads
reserved before a selection or removal cannot overwrite it on completion.

**Rollback warning.** A pre-pictures API image cannot start against the migrated
database (SQLx stops on the unknown `202610140001` version). Never work around
this by deleting migration records or setting `ignore_missing`: the old purge
marks every completed upload without a message as deleted after 24 hours and
removes its R2 objects, which would destroy every picture. Fix forward.

### Live changes

A saved picture, display name or username, and a space's icon or name, reach other
open clients at once (`profiles.rs`). The API publishes each change once to the
broker topic `caper:profiles:v1` with its audience: for a person, everyone sharing
an active space or DM with them plus their own other sessions; for a space, its
members. Gateways deliver to sockets with a `profiles` subscription
(`{"type":"subscribe","kind":"profiles"}`), signing picture URLs:

```json
{"type":"profile.updated","person":{"id","username","displayName","avatarId","avatarAsset"?,"avatarUrl"?}}
{"type":"space.updated","space":{"id","name","ownerId","iconAsset"?,"iconUrl"?}}
```

Absent picture fields mean removed. Clients apply `profile.updated` to every
retained copy of that account (as [own profile refresh](#own-profile-refresh) does
for the signed-in account) and `space.updated` to the rail, lists and open space.
Delivery is best effort with no replay; reconnect reloads repair anything missed.
Older gateways reject the unknown subscription, which clients treat as
unavailable. The same change rewrites that account's name and picture in every
active voice room, so the next room snapshot carries it.

### API

| Call | Behavior |
| --- | --- |
| `POST /api/assets` `{"purpose":"avatar"\|"spaceIcon", "spaceId"?, filename, contentType, byteSize, width, height}` | Reserves a picture (no `channelId`/`preview`). `spaceIcon` requires owning the space (`404` otherwise). PNG, JPEG, WebP or GIF only (`400 picture_type`); GIFs ≤ 5 MiB, others ≤ 1 MiB (`413 picture_too_large`); 1–1024 px per side. 50 picture uploads per person per day (`429 picture_rate_limited`), plus the usual 30/minute and 20 pending. Returns `{id, kind, upload}`; picture IDs are 20 characters. |
| `POST /api/assets/{id}/complete` | Checks size and magic bytes, then the real pixel size from the first 64 KiB (GIF screen, PNG IHDR, WebP VP8/VP8L/VP8X, JPEG frame header behind EXIF), ≤ 1024 px (`422 picture_invalid`). The first successful completion applies the picture and returns `account` or `space`; retries return current state without re-applying. A picture completing after a later-reserved one is kept but not applied. |
| `POST /api/assets/{id}/complete?draft=true` | Avatars only: verifies and retains without applying or broadcasting. Clients call it after Save profile, before the atomic profile commit. A repeated completion never applies it. |
| `POST /api/assets/{id}/cancel` | Cancels before completion; `409` afterwards. |
| `POST /api/account/profile` with optional `avatar: {avatarId,avatarAsset,expectedAvatarId,expectedAvatarAsset}` | Saves names and the chosen mascot/photo atomically, then broadcasts once. `avatarAsset` is an owned, completed, retained avatar ID or null; `avatarId` is 0–799. Stale expected values return `409 profile_changed` without changing names. Omitting `avatar` keeps the picture. |
| `DELETE /api/account/avatar` | Back to the saved mascot; returns the account. |
| `POST /api/account/avatar/regenerate` | Saves a different random mascot ID, returns the account and broadcasts; `409` with a custom picture. (Legacy immediate save; current clients regenerate locally and save via profile.) |
| `GET /api/account/avatar/history?cursor=<asset ID>` | `{pictures:[{avatarAsset,avatarUrl}],nextCursor}` for your own retained, completed avatars, newest first in pages of 40, including the current one. |
| `POST /api/account/avatar/select` `{"avatarAsset", "expectedAvatarAsset" \| null}` | Applies an owned/completed/retained avatar if the current one matches; `409` if changed, `404` for an invalid target. |
| `DELETE /api/spaces/{space}/icon` | Owner only; back to the initial; returns the space. |

`avatarAsset`/`avatarUrl` join every person that has `avatarId` (account, message
and typing authors, members, invitations, DM peers, suggestions, blocks, reactors,
voice participants, invite-link inviters, feedback reporters). `iconAsset`/`iconUrl`
join spaces in lists, detail, invitations, create/rename/accept responses, the
invite preview and feedback reports. Push notifications show the sender's mascot,
not a custom picture.

### Clients

All clients prepare pictures the same way:

- A GIF (detected by bytes) keeps its animation. Within 5 MB and 1024 × 1024 it
  uploads unchanged; a bigger GIF is shrunk on device: every composited frame is
  center-cropped square and scaled to 512 px with its timing, then 384, 256, 192
  and 128 px while still over 5 MB, then every other frame is dropped (timing
  merged). Web (`account/gif-shrink.ts`, in a worker) and Android (`GifShrink.kt`)
  share one decoder/encoder with per-frame palettes of changed pixels; Apple uses
  ImageIO and Rust desktop the `image` crate.
- Still pictures open in the attachment editor with a square crop (reposition,
  resize, rotate, draw, text); the region is scaled to at most 512 px and encoded
  as WebP (PNG or JPEG where WebP is unavailable), dropping location metadata.
  GIFs offer **Keep original animation** or an acknowledged still-frame edit;
  animated PNG/WebP require confirmation before becoming stills.
- Pictures show aspect-filled in a rounded square (corner radius 30%, the
  `--radius-avatar` token); speaking rings follow that shape. Space icons fill
  their tile. The mascot or initial shows while loading or on failure. Decoded
  pictures are cached by asset ID.

Edit profile and owner-only **Manage space** show the picture in a frame with an
edit badge. The profile frame's menu chooses a new picture, browses **Previous
pictures**, and (with a custom picture) edits or removes it; a space without an
icon opens the file picker directly, and its saved icon can be re-edited. Editing
the current picture works from its saved pixels. Dropping a file on the frame
opens the editor (first file only; busy editors refuse drops). Without a custom
picture the mascot holds a "change me!" sign (`assets/avatars/change-me.svg`).

Profile editing is a private draft: new photos, crops (**Apply crop**), history
selection (**Use picture**), removal, names and the circular-arrow **Regenerate
avatar** (only without a custom picture; chooses locally) all apply only on
**Save profile**, which uploads (draft completion) and commits atomically. A warm
**Unsaved changes** indicator marks a dirty draft; unchanged Save is disabled.
Close, Escape and Back ask **Discard changes?** (**Keep editing** / **Discard**);
browser tab closing uses the native unsaved-changes warning. Saving blocks
controls and dismissal; failures keep the draft. A picture changed on another
device rejects a stale draft. Space icons have no history picker.

Clicking a custom picture inside a profile card opens the original in the media
viewer (chat avatars open the profile first). Web keeps native right-click image
actions; native cards offer equivalent context-menu or long-press actions.

`node scripts/test-pictures.mjs <loopback-web-url>` runs the mocked browser suite
(`PICTURE_VIEWER_ONLY=1` and `PICTURE_GALLERY_ONLY=1` narrow it).

## Desktop window restoration

The Rust desktop client keeps its normal window size/position and its
maximized/fullscreen mode across exits and update restarts. Minimized geometry
never replaces the last usable bounds, and maximized/fullscreen bounds never
replace the normal restore rectangle. Invalid tiny saves fall back to the default
window. Windows starts hidden and applies the saved mode after the first painted
frame. Fixtures do not save this state. The record uses eframe 0.37's `window`
format, keeping its monitor/DPI placement and off-screen clamping. eframe's own
mode flags stay off and Caper's mode sits beside them; eframe 0.33 records still
load. The macOS SwiftUI client uses system window restoration.

## Voice-start notifications

See [the notification contract](notifications.md#voice-start-alerts).
Migration `202610190021_voice_notifications.sql` adds account/space preferences,
a durable per-space cooldown and a shared eligibility function/view; API startup
grants the runtime role access. Old clients omit the new preference fields safely.

## Notifications inbox

Web, Android, Apple and Rust desktop have an account-wide Notifications inbox:
messages, mentions, replies in participated threads, reactions to your messages,
and pending invitations/DM requests. Filters, unread-only mode, pagination and
individual acknowledgements use one authenticated API; see
[the HTTP and read contract](notifications.md#notifications-inbox).
The badge total is the inbox's unfiltered unread count, not a push count.

- Automatic refresh revalidates the loaded page window every 15 seconds,
  including after pagination, without dropping older loaded rows or keeping
  stale read/deleted rows. Live activity signals may refresh sooner. There is no
  manual Refresh; errors offer Retry.
- Rows have compact read controls beside the timestamp and an unread-only
  switch with count; there are no message-kind labels.
- Opening a reaction row highlights the original message; opening a thread reply
  opens its thread, highlights the reply and exposes the composer.
- Rows offer inline Reply (quoted or in thread) and React. Inline sends use the
  shared rich-text composer, keep the inbox open, target the exact destination,
  keep stable retry IDs for uncertain sends, and isolate drafts per row.
