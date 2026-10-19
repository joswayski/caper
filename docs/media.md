# Channels, messaging, and voice

## Scope and architecture

The homepage shows a labelled, client-only simulated conversation. Its join
button opens email-code sign-in/sign-up; it does not connect to a real channel.
The former **Public demo / General** is retired. Account-owned spaces contain
unified text/voice channels; history, live messages, presence and calls require
membership. See [spaces and channel access](#spaces-and-channel-access).
Participants use their account display name and saved Caper avatar. This is not an outgoing-call flow.
No camera, screen sharing, or server-side voice recording.

Channels and direct messages occupy independently scrolling sidebar panes on web,
Android, iOS/macOS and Rust desktop. Drag their horizontal divider to redistribute
the height; both remain bounded above the account controls, even with long lists.
The split defaults to half, retains its saved proportion across launches, and
reserves a useful minimum for each pane in short windows. Mouse pointers use the
platform's vertical-resize cursor. Up/Down adjusts the focused divider, Home/End
reaches its bounds, and double-click/tap resets it. Touch resize gestures do not
start a Browse swipe. The labelled homepage simulation keeps its unsplit layout.

Split-sidebar validation (2026-10-19): `npm run check`, `npm test`, and
`node scripts/test-sidebar-sections.mjs` against the local parity fixture passed.
The browser script covers 80 channels and 40 DMs, independent scrolling, drag and
cursor, persistence/reload, keyboard bounds/reset, action menus at minimum pane
heights, collapsed/sparse lists, and 1440×900, 1440×540 and 390×844 layouts. Set
`SIDEBAR_TEST_WEB_URL` if Vite is not on port 5174. Android Kotlin compilation,
498 JVM tests and debug lint passed. Rust desktop build, Clippy, the full suite
(650 passed, 16 ignored) and an actual Linux/Xvfb pointer drag passed.
Apple implementation remains uncompiled without Xcode; native phone,
touch/assistive-technology and Windows checks remain unverified. Narrow Chromium
captures are not physical-device acceptance. No API or migration change.

Mobile channel, DM and thread timelines dismiss the software keyboard when a
vertical scroll drag starts. The unsent draft stays in its composer; tapping it
reopens the keyboard. Automatic following/paging and scrolling within a multiline
draft do not dismiss it. Web waits past touch slop and leaves horizontal navigation
gestures to Browse/Members; iOS uses native immediate scroll dismissal and Android
uses the list's drag interactions. macOS and Rust desktop hardware-keyboard focus
are unchanged.

Scroll-dismiss validation (2026-10-13): `npm run check` and 604 `npm test` tests
pass. The browser regression's keyboard checks were run independently against
overflowing channel, DM and thread fixture timelines: both vertical directions,
retained drafts, refocusing, automatic scrolling and cancelled gestures pass;
horizontal thread drags retain focus. The channel check fails on unchanged main
because its composer retains focus. The full thread browser script stops at a
separate 527px-vs-600px resize assertion. Chromium touch emulation does not show a
real phone keyboard. Android compilation stops at the missing SDK; no Swift/Xcode
toolchain is available here. Added iPhone scroll-dismiss/draft UI checks still need
Xcode and native keyboard/device acceptance before release.

The narrow iOS workspace sizes its sliding layers from the outer, keyboard-aware
`GeometryReader`; those layers ignore additional keyboard safe-area adjustments.
Container safe areas remain intact. The existing iPhone Browse test now refocuses
after its vertical scroll and checks a keyboard-open swipe out/return, restored
composer and timeline geometry, draft retention, cancelled-swipe editing, and
keyboard reopening. This targets the reported gap after returning from Browse;
Apple compilation, before/after rendering, channel/DM device checks and Reduce
Motion acceptance remain unverified in the Linux orb.

Message composers offer `:` emoji autocomplete on web, Android, Apple and Rust
desktop, using the same bundled Emoji 15 catalog. A colon at the start of a draft,
after whitespace or after an opening bracket activates suggestions; URLs, times,
selected text and active IME composition do not. A single character after the
colon (`:D`, `:P`, `:3`) is an emoticon, not a search: it opens no suggestions,
so Enter sends it as typed. The bare `:` still offers the default emoji. Selection inserts Unicode at the
caret, preserves the rest of the draft and respects the 4,000-scalar limit. The
emoji presentation selectors are preserved separately from normalized artwork IDs.
Enter/Tab accepts a suggestion, arrows move the selection and Escape closes it;
Shift+Enter retains newline behavior. iPhone software-keyboard users select a row
and send with the existing Send button; Android's IME Send accepts an active
suggestion first. No API, gateway, database, infrastructure or secret changes are
required. Web and native binaries can deploy independently; merging alone does
not deploy them. Custom emoji uploads remain unimplemented.
Run `node scripts/native-parity-fixture.mjs` beside Vite, then
`node scripts/test-emoji-composer.mjs http://localhost:5173` for desktop/narrow
browser insertion, selection, send and geometry checks against disposable fixture
data. This is not native keyboard/device coverage. Android and Apple tests require
their platform toolchains; physical touch/IME, software keyboard viewport changes
and Windows rendering remain separate acceptance checks.

Composers also suggest people after `@` on web, Android, Apple and Rust desktop.
An `@` at the start of a draft, after whitespace or after an opening bracket
starts a name of up to 32 ASCII letters, digits or underscores; emails such as
`bob@maya.com` and `x/@maya` do not. Space channels suggest the open space's
loaded members except you, then `@everyone` ("Everyone in this channel") and
`@here` ("Everyone online in this channel"). DMs suggest everyone you share a
space or DM with, from authenticated `GET /api/people` (`{"people":[{"id",
"username","displayName","avatarId"}]}`, excluding you, at most 500), fetched when
a DM opens; until it loads the DM peer is offered. Rows show the avatar and display
name with `@username` in small type beneath it (`@everyone`/`@here` put their
description there). Selecting inserts `@username `
without sending; keys and limits match emoji. Anyone who can post may use `@everyone`/`@here`; permissions are deferred.
Mention suggestions span the available composer width on every client, including
channel and thread replies; `:` emoji suggestion sizing is unchanged.

On send, `chat::persist` resolves names inside the message transaction (shared
grammar in `apps/api/src/mentions.rs`) to any non-deleted account with that
username, in channels and DMs alike, so people can point each other at someone
("that's @alex"). The message's `content.mentions` lists resolved
users (`{"type":"user","id","username"}`) and, in space channels only,
`{"type":"everyone"}` / `{"type":"here"}`, in first-appearance order with at most
20 distinct names. Unresolved names stay plain text and the field is omitted when
empty. `content.version` stays `1`; history, live gateway events and replay carry
the same payload. Clients ignore unknown entry types, render resolved tokens as
terracotta pills, and tint messages that mention the reader (their id, or
`@everyone`/`@here` from someone else). `everyone` and `here` are reserved
usernames. Tagging is not access: someone tagged in a conversation they cannot
open still cannot read it, and notifications go only to people who can read
the conversation. Mention ids are the 12-character random account `external_id`,
never the internal sequence key.

Channel, DM, thread and profile quick-message composers preview complete known
`@username` tags and the Caper team's aliases (including `@feedback`) with the
same terracotta background before sending. `@everyone`/`@here` preview only in
space channels. Partial and unknown names, email addresses and overlong name
runs stay plain. People come from the composer's loaded directory; the profile
quick-message field knows its recipient. This is a local preview, not a promise
of delivery: the server still resolves mentions on send. Drafts and payloads
remain plain text. Web uses a background-only mirror of the native textarea;
Android uses identity-mapped text styling; Apple styles text storage outside
marked-text composition; Rust desktop styles the egui editor's layout without
changing font metrics.

Composer-highlight validation (2026-10-12): `npm run check` and `npm test`
(598 tests) passed. Chromium at 1280px and 390px covered channel, DM, thread,
profile, wrapped and scrolled drafts, insertion, deletion, undo and plain-text
send. Rust desktop fmt, clippy, the full suite (476 passed, 12 existing ignored)
and Linux Xvfb channel/thread/profile rendering passed. These used local fixtures,
not live account or voice traffic. Android's test attempt stopped at the missing
Android SDK; Apple code/tests require an unavailable Apple toolchain. Physical
devices, Windows rendering, software keyboards and real IME composition remain
unverified for this change.

Clicking or tapping a person's pill opens a small profile card (`@everyone`/`@here`
pills stay inert): avatar, display name, `@username`, and an inline DM composer
with text, emoji and file attachments. Typing stays local; choosing or pasting
files resolves the DM and starts uploading immediately, like the main composer.
**Send** waits for any remaining uploads and opens the conversation only after
confirmed delivery. Unsent uploads retain the normal 24-hour cleanup; stale
uploads are renewed before a new send, without changing an uncertain retry's
attachment IDs. **Open conversation** opens a DM without sending the draft;
incoming requests still require explicit acceptance. Message-author avatars/names
(channels, DMs, threads and Pins) and member-list
rows open the same card on every client; guest authors stay inert. Your own card
says "You" and has no composer. Card data comes only from what
the client already loaded: space members, then `GET /api/people`, then DM peers;
someone you share nothing with shows `@username` and the generic avatar, and
messaging still works when their privacy settings permit it. Authors absent from
that directory retain their message name and avatar, without a guessed username
or messaging action. Wide layouts (web desktop,
macOS, Rust desktop) show a popover by the clicked identity; web narrow/touch,
Android and iPhone show a bottom sheet. DM errors
stay inline in the card. No API change. Mentions drive phone push kinds
(`mention.user`, `mention.everyone`); see [notifications.md](notifications.md).

No migration, gateway, infrastructure or secret change is required. The API and
clients deploy independently in either order: older clients ignore
`content.mentions`, and newer clients render plain text from an older API. Rolling
the API back only stops resolving new mentions. Validation:
- API: unit grammar tests and a Postgres test of resolution (public and private
  channels, a member without the private grant, a non-member, DMs naming third
  parties, deleted and unknown accounts, history and outbox payloads) and of
  `/api/people`, plus the full ignored Postgres/Valkey suite.
- Web: unit tests, and a real Chromium check against the fixture at 1280px and
  390px (suggestion order, Enter inserts without sending, Escape, no overlap with
  emoji, pills, mentioned-row tint).
- Fixture: `native-parity-fixture.mjs` resolves mentions on send like the API and
  seeds one message mentioning `fixture_owner`.
- Android: JVM unit tests and lint; no device, emulator or IME check.
- Rust desktop: unit and headless egui tests, fmt, clippy and the full desktop
  suite with the pinned toolchain, plus Linux Xvfb rendering of the
  `parity-mentions` and `parity-direct` fixtures; no Windows or IME check.
- Apple: the pure mention logic and its 13 tests ran with a Linux Swift toolchain;
  SwiftUI/UIKit/AppKit code and the parity UI test need Apple CI. SwiftUI,
  Android and egui inline pills are square-cornered background spans.

Apple composer frames use their measured content height rather than filling the
174-point maximum: channel/DM drafts start at 42 points; thread replies retain
their 72-point minimum. Longer drafts grow and then scroll. Both iOS and macOS
keep a 12-point bottom margin below channel/DM and thread controls, matching
web, Android and Rust desktop. SwiftUI's safe area still avoids the docked
keyboard and, when it is closed, the home indicator. The conversation and
thread backgrounds—not their controls—ignore the safe area, filling the exposed
rounded keyboard corners with the chat color instead of the root's blackout
color. iPhone parity tests assert empty/single-line alignment with Send,
multiline growth, the height cap, shrinking after deletion/send, and compact
thread replies with the keyboard open. They also require a 12-point gap above
the keyboard and check the resting safe-area layout after keyboard dismissal
and reopening. In parity mode only,
the app exposes UIKit's keyboard end frame to measure the full system keyboard,
including predictions; iOS 26's XCUITest keyboard element excludes that bar.
The tests retain their two-point tolerance and do not alter keyboard settings.
Run `./apps/native/apple/parity-screenshots.sh ios` and its `macos` counterpart
with Xcode before release and inspect the composer attachments, including both
keyboard corners and the keyboard-closed home-indicator background. These iPhone
checks and native rendering were not executed in the Linux orb; passing web and
fixture tests do not validate SwiftUI/UIKit/AppKit sizing.

Apple pending sends use the confirmed row's full-width, leading-aligned layout.
The channel requests scrolling on the pending-state update and on content-size
changes without yielding another task first. The parity send regression holds
the fixture before persistence/gateway fanout, checks short and wrapping pending
text in the viewport with an empty composer, then checks unchanged horizontal
alignment and one row after confirmation. `npm test -- --maxWorkers=2` passes
450 web/shared tests and `npm run check` passes; the Apple regression and rendered
frame timing were not run in this Linux orb (no Xcode or connected Apple runner).
Run both Apple parity scripts above and inspect the pending-send captures and
iPhone send transition before release. No API/configuration/migration change is
needed; only updated Apple binaries receive this fix.

Voice lives in the sidebar's channel list (design H, revised October 17, 2026, on
web, Android, iOS/macOS and Rust desktop). Channels stay text and voice together:
opening a channel never joins or switches voice, and joining never changes the
conversation you are reading. A live channel's own row grows into a card, in its
place in the list, so no channel appears twice and nothing says "Live now".
Pressing a card anywhere except its controls and `⋯` menu opens that channel's
chat, like its row.

- Your call's card is green: the channel's row line (name, unread, muted, `⋯`
  and the selected tint) ends with "You're in · N" (Connecting… or Reconnecting…
  in amber) and the shared session timer instead of a join. Faces follow, you
  first and then in join order (speaking never reorders them), with a green
  speaking ring and mute/deafen badges, in at most two rows: up to ten faces, or
  nine and a "+N" tile. Then Mute, Deafen and Leave (Cancel while joining).
  There is no "is speaking" line: the ring shows who talks, so the card never
  changes size with speech. Faces and "+N" are not separate tap targets. A
  right-click (pointer), long press (touch) or the accessibility action on them
  opens the full list ("In #channel · N people") with per-person volume and Mute
  for me, beside the sidebar on wide layouts and as a bottom sheet on phones; a
  face opens with that person's controls expanded. On web the context-menu key
  on the card's row opens it too. Escape, an outside press and Close dismiss it;
  on web, focus moves into it.
- Another live room keeps its row (waveform and count) and gains a body beneath
  it: up to three faces, a one-line summary of names, the shared session timer,
  and Join (eggshell) or Switch (neutral) when you are in a call elsewhere.
  Rooms stay in list order; nothing folds away.
- Voice without a row in the listed space sits above the Channels heading, with
  no heading of its own: your call when it is in another space (its card names
  the space and opens that chat there) or in a channel opened from the feedback
  inbox, and such a visited channel's live room. Starting voice in an empty
  visited channel is no longer offered.
- Collapsing Channels hides quiet rows only; live cards stay listed.

Quiet channel rows are one line, with no roster or count expander. Every row
except your call's keeps a one-click headphones join ("Join voice in #x", or
"Switch voice to #x"). Fine pointers reveal it on hover or keyboard focus in
space reserved beside the `⋯` menu. Touch (Android, iPhone, iPad, touch web) and
Rust desktop at 760px or narrower show it dimmed with a 44px target on quiet rows
and hide it on live rows, whose cards carry Join. The conversation header
carries no voice; "N members · M in voice" stays beneath channel names where
shown. While reading any conversation, phones add a 56px strip under its header:
your call (channel, the card's faces in the card's order, up to five or four and
"+N", Connecting…/Reconnecting… while not connected, timer and a 44px Mute; the
rest opens Browse), or, out of a call, the open channel's live room with Join.
The strip names no speaker. The bottom dock and its pinned roster are gone;
voice errors keep their place. Mic, deafen and device settings stay in the
account bar, including before joining, and mirror the card's Mute and Deafen.
Speaking rings appear only for your own call, since other rooms carry no audio.
Browsing other chats, spaces or DMs does not leave voice. That includes a space
with no joined channel, whose empty state replaces only the conversation (your
call card remains above it), and a pending invitation opened from the rail,
which opens over the room. Voice still ends when its own channel is left,
deleted or revoked. Joined-channel menus offer Leave channel to members;
owners also have the existing privacy/member settings. Leave channel requires
confirmation and no longer appears in the chat header on web, Apple or Rust
desktop; Android already uses its channel menu.
Web tooltips follow their control while open, because a card arriving or
leaving shifts the rows below it.

`VOICE_TEST_CHROME="$PWD/scripts/fine-pointer-chromium.sh" node
scripts/test-voice-controls.mjs http://localhost:5174` runs the mocked web flow.
Each `VOICE_TEST_*=1` mode adds a focus:

- `REDESIGN`: live cards in their rows (no heading, fold or duplicate row),
  pressing a card, right-click and long press for the full list, crowded calls,
  collapsed lists, the phone strip's faces, join and focus, and row join/switch.
- `CHANNEL_ROWS`: row geometry, permissions and pending/error/cancel states; run
  it with stock Chromium too, for the touch rows.
- `SIDEBAR`: DMs, empty spaces and rail invitations during a call.
- `DOCK`: the account bar while connected, and the card's links.
- `TIMERS` and `AVATARS`.

This is browser UI coverage, not live SFU or physical-device validation.
`VOICE_TEST_PREVIEW=1 node scripts/test-voice-controls.mjs` serves the
explicitly simulated, interactive UI on port 5180 beside local Vite; it does not
capture a real microphone or contact the SFU. No production mock route is added,
and there is no API, protocol, configuration or migration change.
Browser modals use viewport-bounded, stable-height shells with scrollable overflow;
errors, pending states and member loading do not resize or recenter them. Channel
settings reserve the hidden save bar, and profile forms reserve error space so
retrying a save does not move its button. Profile, channel and space settings and new
direct-message dialogs dismiss on outside click/tap; nested delete confirmation
dismissal leaves settings open. Rust desktop also dismisses profile and new direct messages
on outside clicks; Apple keeps platform sheet dismissal, and Android keeps its
existing outside-tap behavior. Edit-profile save actions are compact and right-aligned;
web keeps a 148×44px target and leaves sign-in/onboarding button sizing unchanged.
`PROFILE_TEST_WEB_URL=http://localhost:5174/spaces node scripts/test-profile-refresh.mjs`
covers profile sizing (including pending/error), inside clicks/drags, backdrop and
Escape dismissal at desktop, narrow and short viewport sizes with mocked APIs.
October 13 profile-control validation: `npm run check` and all 604 Vitest tests
pass, as do that script's profile-control checks and Rust desktop's backdrop
regression and Clippy. The browser script subsequently times out at its existing
member-profile refresh assertion; the unmodified script fails at the same assertion.
Android/Apple profile sizing is implemented but not built or rendered in this Linux
orb (no Android SDK or Xcode); touch-device acceptance remains unverified.
Web dialogs handle Escape on key-down as well as native cancellation; child-owned
Escape (suggestions or a native select popup) closes that layer first, and IME
composition Escape leaves the editor open. In-flight write guards remain in place.
Apple's shared dialog close button has the standard cancel shortcut on macOS and
iOS hardware keyboards; Android and Rust desktop retain their existing Escape handling.
Run `KEYBOARD_ONLY=1 SPACES_TEST_WEB_URL=http://localhost:5174/spaces node scripts/test-space-controls.mjs`
for desktop/narrow Escape, nested dismissal, Tab/Shift+Tab, initial/opener focus,
pending writes, and synthetic IME/child-owned Escape checks with mocked APIs.
Apple shortcut compilation and physical-keyboard behavior require an Apple toolchain/device;
Chromium checks do not validate Safari or native clients.
Run `MODALS_ONLY=1 SPACES_TEST_WEB_URL=http://localhost:5174/spaces node scripts/test-space-controls.mjs`
against Vite for desktop, narrow and short-viewport geometry, overflow reachability
and backdrop regression checks with mocked APIs. Android, Apple and Rust desktop
use independently implemented stable shells; browser results do not validate
native rendering. Rust desktop has headless egui geometry/backdrop regression
checks. Native CI supplies Android/Apple platform builds and simulator/emulator
UI checks; macOS CI also captures the rendered desktop client. Physical-device
keyboard, scrolling and outside-tap checks remain separate release validation;
CI is not physical-device coverage. Desktop leave/delete-space handling clears the
departed conversation and fences late navigation/chat results before selecting
another space, rather than falling back to the retired General channel.
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
Join carries the initial mute/deafen state; the browser sends a follow-up state
write only if that intent changed while Join or microphone capture was pending.
The authenticated initial snapshot starts as soon as live updates are ready,
overlapping remaining publication/state work. Waiting for live-update readiness
preserves snapshot ordering on older APIs without roster revisions. Subscription
reconciliation still waits for publication, and the microphone remains silent
until roster, current state, and transport readiness have all succeeded. These
are web-only startup changes; device latency improvements require live measurement.
The Rust API replaces a signed-in participant's submitted name
with the account display name. Guest names can collide and are not verified or
reserved; participant IDs, not names, distinguish people. With `VALKEY_URL` configured,
live call state is shared in Valkey; without it, development uses process memory.
Up to 12 people can join with microphone permission,
mute, deafen, choose devices, and leave. Other visitors may record audio.
Caper does not collect or project country codes, and no client renders country
flags. Old Valkey participant records still decode; their country fields are
ignored and dropped when rewritten, without forcing a leave/rejoin. Rust
snapshot/presence tests cover this recovery path; `scripts/test-country-flags.mjs`
checks desktop/narrow web rosters with a mocked legacy country payload.
Android and Apple compilation and physical native checks have not been run for
this removal in the Linux orb; Rust desktop validation is tracked separately.
Visitors see the public roster before joining through a `media` subscription on
the application WebSocket, without joining voice. That projection
includes each participant's session ID, name, mute,
and deafen state, but never media track IDs, session tokens, or audio.
The HTTP/SSE endpoints remain available for older clients and inspection, but the
current browser uses neither SSE nor HTTP polling for live rosters. Spectators
never renew call leases.

Channel names show a shared voice-session timer to everyone authorized to view
the channel's voice roster, including people not in the call. It starts from the
first accepted participant's **Join click**, not completion of provider or audio
setup, survives that participant leaving while others remain,
and clears when the last participant leaves or their lease expires. Microphone
test monitors do not start or extend a session. Authenticated and spectator
snapshots carry nullable `sessionStartedAt` (UTC Unix milliseconds); clients
render `MM:SS`, then `H:MM:SS`. Web, Android, Apple and Rust desktop show a local
provisional timer during a pending join, preserve an occupied channel's existing
start, and adopt the shared timestamp when admission completes. Cancelled/failed
joins clear provisional time; they do not clear another participant's session.
Spectators see only accepted membership, not another client's pending click.
Join requests carry optional `joinStartedAt` (integer UTC Unix milliseconds).
The API bounds it to the 30 seconds before request receipt, clamping future
client clocks to receipt time; clients without it start at request receipt.
Preparation/warm sessions never start timers. Older servers without the snapshot
field hide the shared timer after pending joins finish.
Valkey stores the start with room metadata. Existing occupied rooms initialize
from their oldest remaining participant, since an earlier departed participant's
join time cannot be recovered. Devices calculate elapsed time using their system
clocks, so clock skew can offset the display; future starts clamp to `00:00`.
The timer describes continuous channel occupancy, not each person's time in voice
or guaranteed audio connectivity. Access permissions and roster limits do not change.

Run `VOICE_TEST_TIMERS=1 node scripts/test-voice-controls.mjs http://localhost:5174`
against Vite for delayed join, occupied-room/spectator time, cancellation and
permission-failure checks at desktop and narrow widths. These use synthetic audio
and mocked signaling/WebRTC. API tests cover pre-provisioning time, bounded clock
skew, cross-member snapshots and cross-instance Valkey persistence/fanout.

Web desktop/narrow and Linux desktop rendering are checked with explicitly labeled
fixtures, not live SFU calls. Apple/iOS and Android implement the same timestamp
and display contract but require their platform builds and rendered/device checks;
browser validation does not establish native acceptance. Deploy the updated API
and gateway before web/native releases; both server roles embed media handlers.
No new infrastructure, secrets, Postgres migration or room-schema change is required
for click-time starts. Native releases sending `joinStartedAt` require both updated
server roles: old handlers reject unknown join fields. Web retains its legacy
join fallback. Roll back clients before rolling back server support for the new
request field; keep that support while updated native clients remain in use.
Never delete shared room state to roll back a display feature.

Browser → same-origin `/api/chat/events` WebSocket → Rust gateway → existing
authorized media handlers → Cloudflare control API. Fifteen-second `media.snapshot`
commands renew call leases; pushed snapshots discover roster/track changes.
Browser ↔ Cloudflare Realtime SFU/TURN still carries audio. No audio relays through
the gateway, AWS, Workers, Durable Objects, RealtimeKit or PlanetScale.

Production activation is staged: keep the API at one replica until every pod uses
the same Valkey endpoint and schema. The first switch from process memory requires
an empty-channel maintenance window; the old binary cannot hand off its state.
After activation, API pods can be replaced without closing healthy Cloudflare
media sessions. Their application connections still reconnect; TCP connections cannot
move between pods. See [shared call state](#shared-call-state-and-rolling-deployments)
for the rollout and remaining validation. Each tab receives an
unguessable short-lived call capability. Clients use Caper track IDs, not
arbitrary SFU session IDs. Cloudflare terminates transport encryption; this is
**not E2EE**.

### Channel navigation

Web, Android, Apple and Rust desktop treat clicks on the displayed channel as no-ops;
clicking it while another channel opens cancels the pending transition. Repeated
clicks on one pending destination do not duplicate navigation. Hover/focus on
desktop web, macOS and Rust desktop speculates read-only history, never a sending
session or voice join. Visited timelines resume from their retained replay cursor;
Apple and Rust desktop clicks still recheck space access before using speculative history.
Channel switches keep the displayed title and loaded messages together until the
target is ready, rather than covering them with a skeleton during cache lookup,
prefetch or authorization. Loaded empty channels count as ready conversations.
Only a first load without a ready conversation uses a message skeleton. Android
retains up to 20 channel/DM snapshots in account-scoped memory, rejects incomplete
context windows, and clears removed/revoked conversations and sign-out data;
the authorized gateway replays from each retained cursor.
Apple displays prepared history while its sending session opens and retains the
same space's member page/presence subscription. Rust desktop retains member
statuses and pagination, but reconnects its combined chat/presence gateway on
channel changes. Web keeps its unchanged member subscription mounted.

Web direct messages use page-scoped prepared histories like channels. Hover/focus
speculates only history reads (never a session, read receipt, request acceptance
or voice join); clicking shares that request and commits the title and messages
together. Cold loads retain the displayed conversation. Visited DMs retain their
latest timeline and replay cursor, then catch up through the authorized gateway
without refetching the first page. The cache keeps at most 20 visited DMs and four
five-second speculative reads, and disappears when the Spaces page unmounts.
DM-only switches do not reload the selected space or dim its rail icon; icons
retain the same corners when hovered or selected. Switching away aborts a pending
DM history read and the previous channel's voice-availability request. Loading
failures remain visible with an explicit Retry; cancellation is not a UI error.
`node scripts/test-direct-messages.mjs --navigation` checks shared hover/click reads, cold-load
cancellation, cached switches, stable message/rail frames, replay and failure/retry
at desktop/390px using the disposable native parity fixture beside Vite.
`scripts/test-channel-joining.mjs` checks voice-status cancellation with its
own browser mocks; run it without the parity API/gateway listening.

Android and Apple retain DM snapshots and replay cursors in their account-scoped
navigation state; Rust desktop uses its existing bounded navigation cache for
global DM targets. Native cold DM loads retain the current conversation, and
superseded results cannot replace a newer selection. macOS and Rust desktop DM
rows prefetch on hover/focus; iOS and Android have no hover requirement. Apple and
Rust desktop rail icons also keep fixed corners. Native clients are independent
implementations: browser fixture results do not prove their rendering or gateway
behavior. Apple tests/rendering require Xcode; Android requires its SDK and device
checks; Rust desktop tests and rendering use its Linux build prerequisites.
No API, gateway, migration, infrastructure or secret change is required. Web,
Android, Apple and Rust desktop releases can deploy independently.

Run `NAVIGATION_TEST_WEB_URL=http://localhost:5174/spaces node scripts/test-desktop-navigation.mjs`
against Vite for mocked desktop/390px repeat-click, hover/click sharing, cancellation
and stable presence checks. Web build/tests and Rust desktop tests/Clippy run in
the Linux orb; Apple regressions require Xcode and macOS/iOS render validation.
iOS shares the Apple repeat-click/presence fixes without hover speculation.
Android retains channel history on revisit but does not preload on hover;
physical device and live-network checks remain separate.

### Application gateway and account presence

One WebSocket per tab multiplexes authorized chat, voice-roster, and member-status
subscriptions. HTTP keeps durable messages/history and space/account management.
Temporary typing and media commands use the socket, but invoke the same Rust
handlers and permissions as HTTP. The protocol is not tied to React or browser
focus and can be implemented by future desktop/mobile clients.

- `hello {idleTimeoutSeconds,serverTime}` establishes the session. Heartbeats carry
  `activityAgeMs`; input activity is throttled. Focus, visibility, incoming audio
  and heartbeat traffic do **not** reset activity. Native clients may report their
  own actual input activity. The browser exports `reportActivity()` for future
  local speech detection; merely sending audio packets is not treated as speech.
- `subscribe {id,kind,channelId?,after?,token?,spaceId?,userIds?}` opens one logical
  stream. `event {id,event}` routes updates to it; `subscribed {id}` follows its
  initial snapshot/replay. `unsubscribe {id}` releases interest. There are at most
  32 subscriptions per socket and 100 users per presence subscription. Credentials
  never appear in the URL. Sessions and memberships are checked again during use.
- Account presence is **online** if any live session has recent input, **idle**
  otherwise, and **offline** only after every session lease expires. Default
  `PRESENCE_IDLE_TIMEOUT_SECONDS=600` is ten minutes; valid values are 1–86400.
  Each socket renews its own 40-second lease. The per-user hash expires within
  80 seconds of its last renewal and has at most 32 socket records. A user staying
  online reuses those records indefinitely; no status history accumulates.
  Records represent connected sessions, not installed devices. Multiple browser
  tabs each have a connection. A suspended phone whose heartbeats stop loses its
  lease; native background/voice execution still needs platform-specific support.
- The space member list watches only its current 25-member page. Pagination swaps
  the subscription; collapsing releases it. Initial member *metadata* still comes
  from the space detail endpoint, but presence never loads the entire population.
  Gateways batch expiration checks once per watched user per pod every two seconds,
  then push only changed snapshots. Heartbeats do not broadcast user lists. During
  an outage the UI shows unknown/updating rather than inventing offline status.
- Chat/typing and room invalidations use channel-specific broker topics. Each pod
  subscribes only where it has local interest, and releases unused subscriptions.
  Committed chat cursors repair missed messages; versioned room snapshots repair
  current mute/deafen/roster state. Typing expires and is intentionally not replayed.

Typing is best-effort: it is sent only over an already connected socket, never
queued/replayed after reconnect, and never stored in the command ledger. Lost
start/stop pulses are harmless; the receiving UI expires typing after six seconds.
Authorization and one-second per-author/per-channel rate counters still apply.

Media commands include a UUID and fixed `issuedAt`. A shared Valkey ledger binds the
request to its identity and body, and retains the result for 150 seconds; requests
older than 120 seconds are rejected rather than executed again. Reconnects retry
the **same** command, including pending/draining responses. Accepted commands
continue after socket loss. Mute/deafen also retain their existing monotonic
intent sequence. This is bounded retry/deduplication, **not an exactly-once claim
across a Valkey data loss or a crash during a Cloudflare operation**. An unknown
outcome is surfaced for current-state recovery, not blindly executed twice.
Use a non-evicting, monitored shared broker; losing it clears temporary leases and
command receipts, not Postgres messages. Memory grows with current sockets and
recent commands, not account age. Capacity at 10k concurrent users is not measured.

**Rollout on top of spaces/channels:** PR #144 is merged and included in this
branch. Merge [infrastructure PR #120](https://github.com/joswayski/infrastructure/pull/120)
first and wait for Flux's in-place gateway rollout so gateway pods receive
`MEDIA_ENABLED` and the four Cloudflare settings, but **not** email-login signing,
SES or migration credentials. Existing opaque sessions are verified from the DB.
Then merge Caper PR #146 into `main`, wait for both immutable images, and deploy
that merged SHA **API first**, **gateway second**, **web last**, matching the existing
deployment workflow's API-image prerequisite. The API rollout includes outstanding
spaces/channel migrations; the gateway does not run migrations. The previous
one-channel-per-socket protocol (`/api/chat/events?channelId=…`) has since been
retired: gateways answer it with `410 Gone` before any database work, and only
web tabs loaded before this rollout ever used it (native clients never did).
Do not expose new web clients to old gateways. Old SSE endpoints remain solely for
compatibility; mixed old API pods may delay old-tab roster updates until repair.
The older SSE-specific sections below describe that compatibility path and its
historical validation, not the new browser transport.

**No Terraform/OpenTofu apply is needed for these two PRs.** The production
cluster, cache, secrets and deployment permissions already exist. Staging has no
hosted app cluster/cache: it uses local Compose with staging AWS secrets and SES.
The Compose/CI/orb pin is Valkey 9.1.2; the existing AWS-managed cache reports
Valkey major version 9, not a verified 9.1.2 patch. No hosted upgrade is requested.

Before merging, verify staging from the Caper PR checkout:

```bash
gh pr checkout 146 --repo joswayski/caper
npm ci
aws sso login --profile staging
npm run secrets:check
docker compose -f compose.staging.yaml config --images | grep -Fx 'valkey/valkey:9.1.2-alpine'
npm run dev -- --scale api=2 --scale chat-gateway=2
```

In another terminal, confirm `PONG`/the engine version, exercise calls and member
presence, and restart application containers while leaving Postgres/Valkey intact:

```bash
docker compose -f compose.staging.yaml exec valkey valkey-cli ping
docker compose -f compose.staging.yaml exec valkey valkey-cli INFO server
docker compose -f compose.staging.yaml ps
docker compose -f compose.staging.yaml restart api chat-gateway
# After testing, Ctrl-C the dev command, then stop containers without deleting data:
docker compose -f compose.staging.yaml down
```

For production, follow the [copy-paste infrastructure runbook](https://github.com/joswayski/infrastructure/blob/main/docs/caper-chat-gateway.md#merge-and-production-rollout-order).
It contains guarded PR merge commands, Flux environment/replica verification,
production authentication and ECR image checks, and exact API → gateway → web
workflow commands with pinned-image checks after each rollout. Use the immutable
PR #146 merge SHA, not a moving `main` tip. Watch the exact run URL returned by
each dispatch; do not select the latest run or let an already-ready old Deployment
stand in for workflow completion. No manual `kubectl apply` or production restart
is needed.

These are operator instructions, not actions performed in development. Keep two
ready gateways, `maxUnavailable: 0`, and the existing 65-second termination grace.
Rolling replacement of an application pod is different from losing the shared
broker. Durable replay, lease expiry and command dedup tests use disposable local
services; they do not establish production capacity or physical-device behavior.

Validation for this transport change:
- After merging the latest PR #144 website, `cargo test --workspace` passed 91
  tests; all 23 explicitly enabled ignored integrations passed against disposable
  loopback Postgres 15 and Valkey 9.1.2. Another 14 tests passed against Valkey
  9.1.2 in cluster mode. The gateway test holds Join inside a mocked
  provider while the original socket disappears, retries on another gateway, and
  verifies one participant and the original result. The outbox test locks one
  claimed row and verifies a second publisher can publish the remaining rows.
- `npm run check`, all 222 web tests, Rust formatting/Clippy, and the API release
  build passed. Typing regressions cover start/stop publication without receipts,
  authorization, and dropping interrupted pulses rather than retrying them.
  Orb setup completed twice with the pinned native Valkey binaries. The Docker
  daemon was unavailable; the web and Rust image build stages were executed
  directly, not claimed as built Docker images.
- Chromium exercised real member presence with seeded local accounts: online/idle,
  25-to-5 member pagination, collapse/unsubscribe, and desktop/narrow layouts.
  After the website merge, `scripts/test-space-controls.mjs` and
  `scripts/test-voice-controls.mjs` passed with mocked HTTP/WebSocket/WebRTC and
  synthetic audio, including deletion confirmation/focus/retry, member
  subscriptions, voice menus, and narrow layouts. No live SFU, physical mobile,
  mass reconnect, authenticated staging-secret, or production capacity test was
  performed for this change.

### Public text demo

Retired September 2026. `LiveWindow.tsx` now renders a local scripted
conversation with fictional participants, typing, visual speaking indicators,
and a bundled AI-generated capybara meme (`public/images/demo-tiny-hat.webp`).
The illustration has an accessible simulation label but no visible caption or
badge inside the channel. Its bottom-left profile and microphone/headphones/settings
icons are passive illustration, not live audio controls. The card uses grab/grabbing
cursors for tilt-dragging, while the join link keeps a pointer and the message
history retains native scrolling. Satoshi
uses `font-display: swap` so names, messages and the join label remain visible
in the fallback font while fonts download. It plays no audio and requests no
chat/media capability or gateway subscription. Reduced-motion visitors get a
static illustration. Click/Tap to
join targets `/spaces`: signed-out visitors reach the existing email-code
account flow, while returning members skip login. This illustration
does not claim that image uploads are implemented in account chat.

The centered join CTA is the demo's only action. The `general` and `feedback`
channels, voice roster, composer and emoji reaction counts are read-only; there
are no join notices, reaction picker/hints, or playback controls. Speech,
messages, typing and delayed reactions use independent randomized timings.
Only seven of the 24 scripted posts (memes and a few standout replies) receive
one to three delayed reactions; ordinary messages stay unreacted.
Messages remain in local page history for one earlier animation loop; older
loops are dropped so a long-open homepage stays light, and the clock pauses
while the window is off screen or the tab is hidden. The full-height
conversation follows new posts unless the visitor scrolls back to read older
messages; reloading starts a fresh illustration. No reaction API is called.
The four profile pictures in
`public/images/demo-avatars.webp` were generated using `caper-face.svg` as the
reference (beanie, headphones, glasses, and a star variation). The five bundled
Twemoji 15.0 SVGs carry their CC BY 4.0 attribution in `images/demo-emoji`.
The homepage contact actions link to X and copy the email address, with success
feedback or a visible address when the browser denies clipboard access.
`node scripts/test-home-demo.mjs http://localhost:30701` checks the real local
homepage with a controlled browser clock: history retention across two loops,
auto-follow/scrollback, delayed passive reactions, removed controls/notices,
both channels on mobile, reduced motion, clipboard-denial fallback, and join →
email entry or authenticated spaces. Mobile touch emulation is
browser-only coverage, not native-client or physical-device verification.

The API no longer seeds General or exposes `/api/chat/general` or unscoped
`/api/media/*`. Chat session creation requires an account. Shared authorization
rejects every demo-space channel for history, sends, typing, and subscriptions,
even for authenticated users with an explicit membership. Existing demo data
is retained, not deleted. `CHAT_ENABLED` and `MEDIA_ENABLED` still control account
features and must not be disabled to retire the demo. Web, Android, Apple, and
Rust desktop bootstrap from account spaces without inserting General. `/spaces`
sends signed-out visitors to `/login`.

#### Demo retirement deployment order

1. No infrastructure, secret/configuration change, or new database migration is
   required. Wait for immutable API and web images for the merged commit. Set
   `MERGED_SHA` to that full commit SHA; run operator commands from an authenticated
   checkout with the production Kubernetes context. Merging does not deploy.
2. Deploy **API, then gateway**, waiting for each new-image workflow to finish
   successfully before checking rollout. All old replicas must be replaced:

   ```bash
   gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # Wait for the run for this SHA to complete, then:
   kubectl -n default rollout status deployment/caper-api --timeout=15m
   gh workflow run deploy-caper-gateway.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # Wait for the run for this SHA to complete, then:
   kubectl -n default rollout status deployment/caper-chat-gateway --timeout=15m
   ```

   Old gateways embed the old media router, so API deployment alone does not
   revoke all access. Existing demo calls cannot renew through updated services;
   their 45-second leases expire and the five-second sweep schedules provider
   cleanup. Provider cleanup is asynchronous and may retry: rollout is not proof
   of immediate SFU disconnection. Verify the retired roster empties and cleanup
   completes without interrupting account calls; never clear shared Valkey state.
3. Deploy web after both server roles, then publish updated Android, Apple, and
   desktop clients through their normal release workflows. Native releases can
   proceed independently of one another after the server rollout. Old clients
   cannot access General but may need an update to load account spaces.

   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # Wait for the run for this SHA to complete, then:
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```

4. Verify `/api/chat/general` and `/api/media/status` return 404, old demo channel
   IDs reject history and gateway subscriptions, and an account member can still
   read, send, and join voice in an account-owned space. Verify homepage CTA →
   email entry → code flow. Browser rendering is not native-device verification.
5. Rollback: web/native presentation can roll back independently, but older
   clients expect retired routes. Rolling back API or gateway authorization
   restores public access and is incompatible with keeping the demo retired;
   prefer a forward fix. No data restoration is needed because demo data remains.

The same Rust image has two independently deployable roles:

- `caper-api`: HTTP commands/history plus transactional-outbox publishing (3001).
- `caper-api --gateway`: application WebSocket sessions and media/typing commands
  (3002, `CHAT_GATEWAY_BIND`). It uses the runtime `DATABASE_URL`, never
  `MIGRATION_DATABASE_URL` or email-login signing credentials. It requires `MEDIA_ENABLED`
  and four Cloudflare media settings as the API, with the same shared `VALKEY_URL`.
  `/health` is liveness; `/readyz` requires DB/broker readiness and no drain.

Both need Postgres, the **same** authenticated TLS `VALKEY_URL`, and
`CHAT_ENABLED=true`. The flag defaults off outside Compose. Compose defaults it
on when no override exists; change the example `.env`'s false value to true to
enable it. Secrets Manager overrides process environment settings. API startup
runs migrations and runtime grants but no longer seeds a public demo.
Start the API before the gateway. The
gateway retries startup in Compose if migrations are not ready yet.

IDs follow the existing random alphanumeric convention: **12 characters** for
spaces/channels/guest identities, **15** for messages, with unique constraints.
Internal joins use bigint. IDs contain no timestamp and do not order messages.
Per-channel decimal-string `seq` values are reconnect cursors; browser code uses
BigInt, never floating-point numbers. The channel row lock allocates positions
in commit order. There is no cross-channel ordering or sharding in this demo.

HTTP contract (same origin, no cache):

- `GET /api/chat/general`: retired; returns 404.
- `POST /api/chat/session {name}`: opaque sender token and public author. Login
  and a completed profile are required (403 `complete profile required`
  otherwise); the account's display name is always the author name and the
  submitted `name` is ignored. Tokens are stored hashed in Postgres and expire
  after 30 days. Account-linked chat tokens also stop working after parent
  logout/expiry, or if the account has no username and display name. The web
  client forgets its stored token on logout.
- `POST /api/chat/channels/{id}/messages {clientMessageId,text}` with
  `X-Caper-Chat-Token`: returns the committed message. The browser retries the
  same UUID and original text after ambiguous errors, using Enter or Retry send.
  A reused key with different text or a different sender returns conflict.
- `GET /api/chat/channels/{id}/messages?before={seq}`: earlier history, up to
  50 messages. Requires account membership and any private-channel grant;
  retired demo channels are inaccessible.
- `POST /api/chat/channels/{id}/typing {typing:boolean}` with
  `X-Caper-Chat-Token`: checks the same sender/channel permissions as sending;
  returns 204 after best-effort publication, 429 when throttled. No draft text is
  accepted, saved, or broadcast. There are no new database migrations.

Messages preserve literal Unicode text (up to 4,000 code points, nonblank;
control characters other than newline/tab are rejected). Content is versioned
`{version:1,type:"text",text}`. `prepare_text` is the pre-publication boundary for
future replacement rules such as BO2 → Wardogs: transform once before persistence
and broadcast, not by deleting and reposting. No example replacement is enabled.
An eventual transformation feature must separately retain the submitted original
in restricted audit storage while publishing only the transformed content. The
current hash is not an audit copy. Stable message IDs and channel event sequences
support same-ID edits and retained content versions (see [message editing](#message-editing)).
Restricted transformation audit storage and integrations are not implemented.
Message bodies, chat capabilities, and account credentials are never logged.
Messages and author snapshots are saved in Postgres and visible to authorized
channel members; there is no automatic retention purge. Browser tokens use local
storage; drafts/pending sends survive reconnects but not closing/reloading a tab.
Each channel and DM keeps its own unsent draft (text, staged files and thread
replies) while the app stays open on web, Android, Apple and Rust desktop:
switching conversations in any direction and coming back restores it, sending
clears it, and signing out discards every draft. Drafts stay in memory
and are never sent to the API. Restoring one does not send a typing indicator.
`scripts/test-desktop-navigation.mjs` (channels) and
`scripts/test-direct-messages.mjs` (DMs) check this in Chromium. Desktop, Apple
and Android unit tests cover switching and sign-out.
Signed-in startup obtains a fresh capability
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
WebSocket/history confirmation aborts the now-redundant send POST immediately,
without cancelling the chat client or the next command. This releases client
request resources; it does not undo a server-side commit.

Typing uses channel-scoped `caper:chat:v1:typing:{channelId}` Pub/Sub topics; it
never enters the transactional outbox, history, or message sequence. Gateways
subscribe only for locally interested channels and deliver `typing.updated`
on that channel's logical chat subscription. Each chat subscription rechecks
the sign-in session and channel access (one query) before every durable event
and every ten seconds while idle; typing and upload progress reuse a check from
the last second. Losing access ends the subscription with an `error` frame
within those bounds.

Typing publications are limited across API replicas to four per author/channel
per second and 120 per channel per second. The browser pulses at most every 500ms
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
   a lost wakeup. `FOR UPDATE SKIP LOCKED` claims disjoint outbox batches across
   API replicas without a global publisher lock. Publication can arrive out of
   order; committed per-channel sequences remain the ordering authority.
3. Publisher sends the full event on Valkey Pub/Sub, then marks it published.
   The event remains in Postgres for replay. A crash in between may republish it;
   gateway/client sequence deduplication prevents a second visible message.
4. `GET /api/chat/events` upgrades to the application WebSocket. The browser sends
   `subscribe {id,kind:"chat",channelId,after}`. Channels require a signed-in
   account and current membership; demo channels are denied. Each subscription buffers
   before reading the committed high-water mark, replays through it, emits
   `ready {cursor}`, then merges live events in order. Sends still use HTTP.
5. One DB head check per gateway every two seconds repairs missed events even if
   the final Pub/Sub event was lost and no later message arrives. Broker outage
   does not erase accepted messages. Normal delivery uses full broker payloads,
   not a per-recipient payload fetch. Session/membership authorization is still
   checked per subscription; this is not a claim of zero per-recipient SQL work.

Limits: 30 new messages/account/minute in a channel (shared by all of that
account's chat sessions), 120/channel/minute, and 60 new sender
sessions/minute per account (a site-wide budget let a few accounts block
everyone). Signed-in clients reuse one sender session per account across
conversations (web per page, Apple/Android/Rust desktop per sign-in) and mint a
new one only after a 401 or a profile rename; a 403 refusal never replaces it.
Gateway sockets require a signed-in account; anonymous upgrades get 401 before
taking a connection slot. `GATEWAY_MAX_CONNECTIONS` defaults to 4096
per pod (an admission limit, **not measured capacity**). Each channel has a bounded
256-event ring. A lagging receiver replays from Postgres in batches of 128.
Application sockets use 4 KiB read buffers, a 256-frame outgoing queue, and a
256 KiB WebSocket write-buffer cap. Writes time out after three seconds; replay
is capped at 2,000 events. Older/impossible cursors or missing retained events get
`resync_required`, and the UI reloads recent history with older-page access.
Application heartbeats run every ten seconds, with a 30-second liveness deadline.
These are pre-launch demo limits, not an internet-scale abuse-prevention system.

**Planned deployment handoff:** keep two ready gateway replicas, maxUnavailable 0,
maxSurge 1, five-second preStop, and 65-second termination grace. SIGTERM rejects
new upgrades/readiness and sends `migrating`, but old sockets continue delivering
for 20 seconds. The runtime remains alive for at least 36 seconds to finish
accepted commands bounded to 35 seconds. The browser
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
| Transaction rollback, concurrent retry/order, replay, duplicate publish, lost last event, overlap | Automated integration test with disposable Postgres and real broker/WebSockets; also runs in CI with Postgres 17 + Valkey 9.1.2 |
| Typing auth/rate limits, cross-gateway fanout, unchanged durable cursor | Automated disposable Postgres/broker integration; browser unit tests cover throttling, expiry, overlap and nonblocking sends |
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

CI runs this against cluster-mode Valkey 9.1.2 with one primary owning all hash
slots. This enforces the multi-key Lua restrictions that standalone mode misses:
the typing rate counters share a per-channel `{caper:chat:v1:typing:channelId}` hash tag. It tests
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

Owners invite existing accounts by exact username and can remove members or
cancel pending invitations. Every client confirms before removing a space member
("Remove {name}?") or a private-channel member ("Remove {name} from #channel?");
cancelling an invitation stays one click. Invitees must accept before becoming members.
Existing memberships are unchanged; the migration does not retroactively ask
existing members to accept. Owners can also share [invite links](#invite-links). There are no custom roles,
ownership transfers or public space discovery yet.

Username fields suggest people the client already has, on web, Android, Apple and
Rust desktop. The space invite field offers `GET /api/people` (people you share an
active space or visible DM with) minus current members. The private-channel
grant field offers the space's members minus the owner and current grantees.
Both also list people with a pending invitation, their `@username` line ending
"· Already invited", so an owner can tell them to check their invitations instead
of wondering why they're missing; inviting them again is still refused as a
duplicate. **New direct message** offers `GET /api/people`. Each list is fetched or refreshed when its dialog opens and uses
the `@` ranking (exact username, username prefix, display-name word prefix, then
username substring; at most six, nothing until you type). Rows match `@`
suggestions: avatar, display name, and `@username` in small type beneath it, shown
inline below the field. Choosing one fills in the exact username and never submits;
Up/Down, Enter and Escape work from hardware keyboards, and Escape closes the list,
not the dialog. Nothing is searched server-side: there is no account directory,
and lookups, the shared invitation-attempt limit and block handling stay
exact-username, so typing a username by hand still works. No API, migration or
configuration change.
Non-owner members can leave a space themselves. Removing a space member also
removes their private-channel grants. The owner cannot be removed. Removal is a
soft delete: space memberships, private-channel grants and channel joins get
`deleted_at` and stay in Postgres. Rejoining inserts a new row, so earlier periods
are kept; it does not restore old private-channel grants.
Counts include active resources only. Defaults are 20 owned spaces per
account (`SPACE_OWNED_LIMIT`), 100 total memberships including owned spaces
(`SPACE_MEMBERSHIP_LIMIT`), and 100 channels per space (`SPACE_CHANNEL_LIMIT`).
All three accept positive integers from the API environment or the existing
Secrets Manager record (which takes precedence). Invalid values fail startup.
Restart all API replicas with the same settings after changing them; the browser
reads current limits from `/api/spaces` on load. Lowering a limit does not remove
existing resources or access; it blocks new creates/additions until below that
limit. Quota checks and creation run under database locks to prevent racing
requests exceeding those limits.

Space and channel external IDs use the existing 12-character cryptographically
random alphanumeric generator with database uniqueness constraints; internal
joins use bigint IDs. Names are not authorization tokens. Space names contain
1–80 Unicode characters after trimming, with no control, invisible or
text-direction characters (see "Name characters" below). New channel
names contain 1–80 characters matching `[a-z]+(-[a-z]+)*`: lowercase ASCII
letters with single dashes between segments. Channel names are unique among
active channels of one space. Renaming does not change IDs. Deletion is a
soft delete: it removes access and frees the quota, but retained message rows
are not physically purged by this feature. There is no restore UI.

The account APIs live at `/api/spaces`, `/api/spaces/{space}`, and their
`/channels`, `/members`, `/invitations`, `/invitation`, and
`/channels/{channel}/members` subresources. Browser cookies or account bearer
authentication are required. Only owners manage resources, except a member
removing their own membership or an invitee accepting/declining their own invite.
Text history and
commands use `/api/chat/channels/{channel}/...`;
WebSocket subscriptions still use `/api/chat/events`. Both command and gateway
paths check membership and channel visibility, including replay and live delivery.

#### Name characters

Display names, space names and guest names refuse characters that can make a
name look empty, hide text or reorder what it shows: control characters, the
Unicode `Default_Ignorable_Code_Point` set (bidi embeddings, overrides, isolates
and LRM/RLM/ALM, zero-width space, word joiner and invisible operators, BOM,
soft hyphen, combining grapheme joiner, Hangul fillers, tag characters,
Mongolian vowel separator) and U+2800 BRAILLE PATTERN BLANK. Emoji and scripts
keep what they need: a single ZWJ/ZWNJ between two visible characters, one
variation selector right after a visible character, and tag characters only
inside a subdivision flag (U+1F3F4 … U+E007F). The rule is
`apps/api/src/names.rs`, mirrored for inline errors by `apps/web/src/names.ts`,
Android `model/Names.kt`, Apple `NameCharacters` and desktop `src/names.rs`.
New writes are checked; stored names are read unchanged and no data is
rewritten. A display name saved before the rule stays savable while it is
unchanged, so editing only the username works; a space rename must use a valid
name. Attachment file names come from the sender's file system, so those
characters are removed instead of failing the upload (control characters still
fail as before). Message text is not checked by this rule.

### Invite links

Owners create Discord-style shareable links at `/invite/<code>`. Codes are 10
random alphanumeric characters (about 59 bits), so the unauthenticated preview
cannot be enumerated. **Invite people** opens an invitation-only dialog on web,
Android, Apple and Rust desktop; renaming, icons, member removal and deletion
remain in **Manage space**. Opening prepares a ready-to-copy link for **100 total
joins within 30 days of creation**, initially selecting the oldest public channel.
Under the space lock, the server reuses the newest link with those settings and
that landing channel only if it is still live,
so reopening or opening on multiple devices does not reset its use count or expiry. Expired,
exhausted or deactivated links are not reused. Deactivating stops new joins, not
existing memberships; it leaves the dialog without a prepared link until the owner
creates another or reopens Invite people. No QR code is generated.

The main dialog shows a ready-to-copy link without a destination picker. The server
chooses the oldest public channel, with no special preference for a channel named
`general` and no space-wide destination setting. With no public channels, space-only
invitations remain usable. Owners can choose a different public destination when
creating a separate link under **Custom links**; previously shared links keep their
destinations. The API returns the actual destination, which appears in link metadata.

**More invite options** sits beside the primary Copy action. **Regenerate link**
revokes the displayed code and creates a new code for the same public channel,
with 30 full days and 100 available joins in one transaction. If the old destination
became private or was deleted, regeneration and joining fall back to the oldest
public channel. Regeneration and deactivation (including list revocation) require
confirmation on every client. Cancel leaves the shared URL working; the warning
explains that the previous URL stops new joins and regeneration resets the limits.
Failures leave the previous link intact;
simultaneous regenerations of the same code mint only one replacement (the stale
request returns 404). Other links and existing memberships are untouched. The
newest matching link for each selected channel is used, so it cannot revert to an older
compatible link after regeneration, exhaustion or deactivation. Separate links
created with exactly these settings are also eligible for selection. The UI shows remaining joins
and remaining time, rather than presenting the initial lifetime as time left.

**Custom links** in the options menu creates and manages separate links,
not settings for the primary shared URL. Existing links keep their settings.
Owners type the expiry as a number of minutes, hours or days (the API accepts
any `maxAge` from 60 to 31,536,000 seconds) or choose Never, and optionally type max
uses (1 to 10,000; empty is unlimited; spaces have no member cap). Custom forms
also start at 30 days and 100 joins. The existing custom-create API still treats
omitted/null limits as unlimited for compatibility. A link can also land joiners
in a public channel; a landing channel deleted or made private later
falls back to the starter channel and is no longer named in previews. Links are
retained in `space_invite_links` (`revoked_at`, never deleted) and each join is
recorded in `space_invite_link_uses`.

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
have at most 100 live links. Reusing the default link does not consume that
budget. Deploy the API before these clients; no database migration is required.
The browser regression scenario runs with the disposable native parity fixture
and Vite: `node scripts/test-invite-people.mjs <loopback-web-url>`. It checks
invitation-only controls, copy, reuse, keyboard options, regeneration, custom links,
deactivation, username invitations, narrow-layout scrolling and failed-request retry.
This is mock-backed UI validation, not live join validation; the Rust invite-link Postgres integration
test covers the server lifecycle, concurrent opens/regenerations and rollback.

Invite-people native validation follow-up: with SDK 36 installed, Android main and
instrumentation sources compile (`compileDebugKotlin`, `compileDebugAndroidTestKotlin`)
and all 411 JVM tests pass, including 17 invitation parsing/default/route tests.
`lintDebug` passes with one worker and a 768 MB Gradle heap; the initial concurrent
lint invocation was killed with exit 137. The report has zero errors, 1,684
warnings and two hints; warnings have not been suppressed or resolved here.
Instrumentation was compiled, not executed: no Android emulator or device is
available in the orb. APK/native-media assembly and rendered Compose acceptance
remain unverified. Apple compilation, XCTest and rendering still require Xcode;
no Mac runner is connected. This follow-up does not change invite defaults or UI.

Joining checks the membership quota under the same
space lock as invitation acceptance, consumes one use, joins the landing channel,
and fulfils any pending username invitation. Existing members get the space back
without consuming a use, so a link doubles as a shortcut. Leaving does not block
rejoining by link, but an owner removal or an owner's block returns 403 until the
owner invites the account by username. A removal is recorded in
`space_invitations.removed_at` (migration `202610140002_space_removals.sql`,
backfilled from `status='removed'`) and holds until an invitation is accepted:
a live re-invitation can be fulfilled through a link, but one that was
cancelled, declined or expired leaves the 403 in place.
Expired, revoked and exhausted links return the same 404 to outsiders; existing
members can still preview and open them as shortcuts. Unknown links and deleted
spaces return 404 for everyone. The preview checks active membership using the
cookie/bearer session, never space names or the client's cached space list, and
does not join or consume a use. Like other account responses it is `no-store`.

The web `/invite/<code>` page previews the link and sends signed-out visitors to
`/login?invite=<code>` (then the profile step if unfinished) and back. Web and
native apps also accept a pasted link or bare code (web: under **Create a space**).
Existing members see **You’re already a member** and **Open space**, not a join
prompt. Clicking it uses the idempotent join endpoint to open the landing channel;
newcomers still explicitly choose **Join space**. Older servers that omit
`alreadyMember` keep the previous join presentation. Deploy API before web/native
clients to enable membership-aware previews; no migration or configuration change
is required. The gateway needs no update for this change.

Opening links in the apps: every native app registers the `caper://` scheme, and
the invite page links to `caper://invite/<code>` ("Open in the Caper app"). An
opened link fills in the join dialog and loads its preview, after sign-in and
the profile step when needed. Platform specifics:

- iPhone: universal links for `https://caper.chat/invite/*` through the
  `applinks:caper.chat` Associated Domains entitlement and
  `/.well-known/apple-app-site-association` (team `NAUU43LATL`, `chat.caper.ios`).
  Automatic signing with `-allowProvisioningUpdates` should add the capability to
  the App ID; if a TestFlight export fails on the profile, enable Associated
  Domains for `chat.caper.ios` in the Apple developer portal.
- Mac: `caper://` only. The Developer ID build has no provisioning profile, which
  universal links require.
- Android: verified App Links for `https://caper.chat/invite/` against
  `/.well-known/assetlinks.json` (`apps/web/src/server/app-links.ts`). It lists the
  upload key that signs the GitHub APK. Play re-signs store installs, so add the
  Play app-signing SHA-256 (Play Console → App integrity → App signing) there for
  Play installs to verify; until then they open the browser.
- Windows/Linux desktop: `caper://` is registered per user on launch (HKCU
  `Software\Classes\caper`; `~/.local/share/applications/chat.caper.links.desktop`
  plus `xdg-mime`) and by the installer/`.deb`. A link clicked while Caper runs is
  handed to the open window over loopback (a token file in the app data folder)
  instead of starting a second copy. Real launches and invite activations maximize
  the resizable window, not exclusive fullscreen. Native window geometry is no
  longer saved, and startup overrides previously saved sizes after window creation;
  other app preferences and account persistence remain independent.

**To do (owner): finish automatic app-link opening.**

- [ ] Android Play installs: once Play App Signing exists for `chat.caper.android`
  (after the first app bundle upload), copy **App signing key certificate →
  SHA-256** from Play Console → Test and release → App integrity → Play app
  signing, add it to `ANDROID_CERT_FINGERPRINTS` in
  `apps/web/src/server/app-links.ts`, and deploy web. Until then Play installs open
  invite links in the browser ("Open in the Caper app" still works); GitHub APK
  installs already open directly.
- [ ] iPhone: confirm the first TestFlight build with `applinks:caper.chat` signs.
  If the export fails on the provisioning profile, enable **Associated Domains**
  for `chat.caper.ios` at developer.apple.com → Certificates, IDs & Profiles →
  Identifiers, then rerun the release. Verify by tapping a `caper.chat/invite/...`
  link in Messages on a device with the TestFlight build.

### Invitation consent and abuse limits

`GET /api/spaces` returns active `spaces`, metadata-only pending `invitations`
(space ID, name, owner ID and `inviter: {username, displayName}`), and `limits`.
Only owners can invite; inviter metadata uses the owner's current public profile,
not a snapshot at send time. Clients also accept older metadata without `inviter`.
This additive response needs no migration beyond the invitation tables below.
Pending invitations live in
`space_invitations`, not `space_members`: they grant no space-detail, membership,
channel, history, presence, WebSocket or media access. Acceptance alone inserts
membership and checks the current total-membership quota under database locks.
No private-channel grants are implicit. Existing authorization remains based on
active membership, not invitation status.

| Operation | Route | Result |
| --- | --- | --- |
| Owner sends an invitation | `POST /api/spaces/{space}/members` with `{username}` | 201 with invitee member metadata; not an active member |
| Owner lists pending invitees | `GET /api/spaces/{space}/invitations` | `{members: [...]}` |
| Owner cancels an invitation | `DELETE /api/spaces/{space}/invitations/{user}` | 204 |
| Invitee accepts | `POST /api/spaces/{space}/invitation` | 200 with space metadata |
| Invitee declines | `DELETE /api/spaces/{space}/invitation` | 204 |

Username fields normalize like profile selection: lowercase ASCII letters,
digits and underscores, 3–32 characters. The API trims/lowercases and rejects
invalid syntax (400 `invalid username`); it does not silently strip punctuation.
An authorized owner gets 404 `user not found` for a missing account, or 409
`user already in space` / `user already invited`. Authorization precedes account
lookup. Unauthorized/missing spaces and expired/cancelled invitations retain
generic 404s; the client says “This space is no longer available” without
revealing whether the space exists for somebody else.

Invitation attempts use a durable Postgres fixed-window counter, shared across
replicas: **20 per owner per 10 minutes across all spaces**, including invalid
usernames, failed lookups and duplicates. Attempt 21 and subsequent attempts
return 429 until the window resets. Each invite expires after **7 days**. Decline,
cancel and member removal impose a **24-hour cooldown** on the space/user pair.
At most **50 live pending invites per recipient** and **100 per space** are
allowed; reaching either cap returns 409. Expired/deleted-space invitations do
not count. One row per pair retains state/cooldown, rather than an invitation
event log. These are fixed product limits, not new configuration/secrets.

The browser opens an inert empty shell behind a blurred consent dialog; it never
downloads private content to blur it. Consent shows the space name and inviter's
display name and `@username`, without explanatory paragraphs. Expired invites disappear
from pending lists and cannot be accepted; accepted memberships do not expire.
Decline has initial keyboard focus. Owners
see pending invitees separately from active members and can cancel them. Browser
space lists refresh on focus/visibility and every 15 seconds while visible;
revocation removes the rail entry, invalidates cached navigation and clears the
selected space. An outage is not treated as revocation. Native clients use their
existing blocking/dimmed consent modals without loading the invited space;
they currently discover invites at account load and prune stale spaces when
navigation returns 404, rather than using the browser's periodic list refresh.
Server access checks apply identically to every client.

Space consent uses “You’re invited!” and the same 32px decorative Twemoji
incoming-envelope artwork on web, Android, Apple and Rust desktop. The canonical
PNG and CC BY 4.0 attribution are in `apps/web/public/images/invitation`.
Android/Apple resource copies are checked byte-for-byte by the web test suite;
desktop embeds the canonical PNG. Native layouts retain their existing modal
patterns. This presentation change adds no API, migration, secret or reaction
dependency. Keep the Apple invitation asset catalog and attribution resources
when integrating other native resource changes.

The separate message-reactions work must not be merged blindly into channel
participation: reaction mutation authorization must require `channel_joins`,
including for cross-channel tokens and mutations queued behind leave. Preview
history reads must remain available. Borrowing this fixed artwork does not
import reaction behavior or resolve that integration requirement.

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
kubectl -n default rollout status deployment/caper-chat-gateway --timeout=15m
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

## Uploads and attachments

Signed-in members attach up to 10 files to a message. The API reserves quota
and presigns an exact upload to private R2 staging. On completion it streams
those bytes through temporary disk, removes media metadata without recompression,
and publishes separate server-written objects. The `apps/cdn` Worker serves
only published objects after checking a signature the API issued.

### Data model

One `assets` row per logical file (`202610090010_assets.sql`). `purpose` is
`attachment`, `avatar` or `space_icon` (see
[Custom profile pictures and space icons](#custom-profile-pictures-and-space-icons-october-14-2026)). An optional
preview is a fixed derived object beside the original, not another row:

| Object | Key |
| --- | --- |
| Original | `original/{asset id}` |
| Preview (≤ 640 px, ≤ 512 KiB, WebP/JPEG/PNG) | `preview/{asset id}` |
| Private upload sources (never a CDN route) | `staging/original/{asset id}`, `staging/preview/{asset id}` |

`message_id`/`position` link a sent file to one message; a file an edit
replaced stays linked with `retired_at` set (see
[Editing attachments after sending](#editing-attachments-after-sending)).
Message content keeps
its version 1 `type:"text"` shape and gains an optional `attachments` array
(`id, kind, contentType, name, size, width, height, durationMs, preview`).
**Stored payloads never contain URLs.** Rows are never deleted (matching the
retain-records policy): `deleted_at` hides a file and `purged_at` records that
its R2 objects were removed.

### Upload flow

1. `POST /api/assets` (account cookie/bearer) with channel, name, type, exact
   byte size, optional dimensions/duration and preview size. The API checks
   channel access, locks the user row, enforces quota, 30 reservations/minute
   and 20 pending uploads, inserts a pending row, and returns presigned `PUT`
   URLs (15 minutes). The signature covers `content-type`, `content-length`
   and the download name in `content-disposition`, so R2 rejects other sizes
   and headers (the signature does not authenticate the body).
2. The client uploads the preview, then the original, directly to R2.
3. `POST /api/assets/{id}/complete`: the API `HEAD`s each object, requires the
   exact reserved size, and for inline types checks magic bytes from a 64-byte
   ranged `GET`. It then streams the full source, checks its length and signature
   again, strips metadata from original and preview, and PUTs the results to
   distinct CDN keys. A mismatch or unsafe media soft-deletes the reservation
   (`422`); there is no fallback to the source. Quota and returned sizes use the
   actual cleaned bytes. A row lock serializes completion and cancellation.
   Slow jobs continue after a `409`; clients poll completion rather than hold
   one HTTP request through a proxy timeout.
4. `POST /api/chat/channels/{id}/messages` accepts `attachmentIds`. Inside the
   send transaction the API locks the sender's ready, unattached uploads for
   that channel, embeds their descriptions and links them. Text may be empty
   only when files are attached. Attachment IDs join the idempotency hash;
   text-only hashes are unchanged.

Attachment spoilers use optional `spoilerAttachmentIds` on that send request:
distinct IDs from `attachmentIds`. Marked descriptions store `spoiler:true` in
message JSON; missing means false. The choice is part of the retry hash and
survives history, text edits, forwards and URL refreshes; the author can change
it later by editing the message's files. Each composer offers a
per-file toggle. Updated clients conceal images and video posters with an
on-device blur of a still thumbnail (the image original when no preview exists),
without starting playback. Files without a usable visual preview use a neutral
cover. The eye-off spoiler badge always explains how to reveal it (click or
tap); hover only emphasizes the cover. Revealed attachments keep a labelled
"Hide spoiler" action, including on touch devices. Hiding in the media viewer
closes it and covers that attachment back in chat; hiding also removes its
inline player. Draft badges omit the reveal instruction and retain their
separate spoiler/edit/remove actions. Reveal is local UI state, not a database
write; hiding one attachment leaves its neighbours unchanged, and opening
another image does not reveal hidden gallery items. Push notifications
omit spoiler thumbnails and filenames. This is a presentation preference, not
access control: authorized clients still receive URLs, and older clients do
not understand the flag. Text spoilers and slash commands are not implemented.

Spoiler presentation validation (October 18, 2026): web checks and 682 unit tests
pass. The browser fixture covers desktop/narrow galleries, hover concealment,
keyboard focus, independent reveal/hide and viewer hiding; Chromium touch
emulation verifies the visible instruction and 44px hide target. A real MP4
playback check confirms hiding pauses and removes inline/viewer players.
Android builds the debug APK and release R8 output, passes lint and 483 JVM
tests; the instrumentation tests compile but have not run on a device.
Linux desktop builds, passes application Clippy and the two spoiler regression
tests, and its hidden/revealed/viewer states were inspected under Xvfb.
Apple is implemented but uncompiled here (no Swift/Xcode or connected runner).
Windows, Safari and physical-device behavior remain unverified; browser and
native fixtures are not live-account or live-storage validation.

`GET /api/assets/usage` returns `{used, limit}`. `POST /api/assets/urls`
returns fresh URLs for visible attachments (like Discord's refresh endpoint)
for tabs open longer than a URL's lifetime; a file an edit replaced is
refreshed only for its owner. Files of a forwarded message, and of its direct
replies, are also refreshed for readers of a channel holding a live forward.

### Limits and quota

- Per person, across every space: `ASSET_QUOTA_BYTES` (default 10 GiB) of stored
  bytes, original plus preview. One file may be up to 5 GiB (R2's single `PUT`
  limit); larger reservations get 413 `file_too_large`. Cleaning gets 2 minutes
  plus a second per 20 MB, up to 15 minutes. Space owners are not charged for members' uploads.
- Quota counts what is **stored** (after client compression and metadata removal), and reservations
  count immediately so parallel uploads cannot overshoot.
- Bytes stay counted until the purge loop successfully deletes the original and
  preview. Files replaced by an edit stay stored, so they keep counting. Late
  storage writes may briefly recreate uncharged bytes until the
  reconciliation sweep removes them; quota is accounting, not a strict ceiling
  on instantaneous R2 bytes. Reservation rate limits remain in place.
- Single `PUT` uploads (R2 allows up to 5 GiB). Multipart/resumable uploads are
  not implemented.

### Delivery and URL signing

Every response or socket frame that carries a message signs URLs as it leaves:
history, the send response, and the gateway's live and replay paths. URLs look like
`https://cdn.caper.chat/assets/{id}?exp={unix}&sig={base64url HMAC-SHA256("{key}\n{exp}")}`.
The public `/assets/{id}` path maps to the existing `original/{id}` R2 key;
the HMAC still signs that storage key. Previews stay at `/preview/{id}`.
The Worker also accepts already-issued `/original/{id}` links with their
existing signatures, and both main-file paths share the same edge cache entry.
No stored objects, upload URLs or database records move. Roll out the updated
web/native notification downloaders and the CDN Worker before the API and
gateway begin issuing `/assets` URLs. Older clients can still open files, but
their notification downloaders reject the new path and show text only.
Expiry snaps to UTC day boundaries for profile pictures and space icons, so
their URL is identical for a day (browser cacheable) and lives 24–48 hours;
message files snap to the hour and live one to two hours (see message
deletion). The Worker rejects anything signed more than
three days out, serves allowlisted image/video/audio types inline, forces every
other type to download as `application/octet-stream`, and always sends
`nosniff` plus a sandboxing CSP. PDFs are the one document type that opens in
a browser tab (`inline` with their file name and no page CSP, which would stop
Chrome's viewer); the API only completes a PDF whose bytes start with `%PDF-`.
Range requests (video seeking) are supported.
The edge cache is keyed by object, after signature verification. A leaked URL
works until it expires, as with Discord's signed attachment links.

### Client compression and previews

Clients compress before upload, using settings the API serves with
`GET /api/assets/usage` (`compression`), so operators tune them with
configuration and an API restart, not a client release. When a platform cannot
meet a rule it uploads the original instead of a worse file. What each kind of
file keeps:

- **Screenshots and graphics** stay lossless (identical pixels).
- **Photos** are re-encoded at very high quality (SSIMULACRA2 ≈ 83 on 9–14 MP
  photos): differences are rarely visible at normal size, though not identical
  at 100% zoom; photos larger than `ASSET_IMAGE_MAX_EDGE` are resized.
- **Video** is converted to H.264 so it plays in every browser and app. 4K and
  HDR footage loses resolution (down to a 1440 short edge), some fine detail
  in busy or dark scenes, and its HDR range compared with the original. The bitrate and
  frame-rate settings below are the levers, and they are delivered per
  request, so they could later differ by plan.

Measured in October 2026 with VMAF against the uploaded file (simulated phone
HEVC originals of Xiph/Netflix test clips; x264 `veryfast` as the client
encoder, so hardware encoders may score somewhat lower). 50/60 fps footage, so
the 150% frame-rate boost applies:

| Clip (original) | 6 Mbps, 1080 cap (before) | 8 Mbps, 1080 cap | Defaults: 8 Mbps, 1440 cap | Notes |
| --- | --- | --- | --- | --- |
| Park scene, 1080p50, 12 Mbps HEVC | 61 | 81 | 81 | Foliage smeared at 6 Mbps |
| Crowd running, 1080p50, 12 Mbps HEVC | 56 | 77 | 77 | Deliberately extreme motion and detail |
| Lean HEVC, 1080p50, 3 Mbps | 89 (78 with #519's 100% cap) | 89 | 89 | 200% cap restores it; output is twice the original's size |
| Market, 4K60, 50 Mbps HEVC (1080p / 4K screen) | 80 / 74 | 92 / 85 | 95 / 89 | 1440p60 output ≈ 170 MB/min |
| Driving, 4K60, 50 Mbps HEVC (1080p / 4K screen) | 85 / 78 | 92 / 87 | 95 / 92 | |
| Retina screen recording, 2880×1800 text, scored at full size | — | 94.5 (7.2 Mbps) | 99.9 (12.8 Mbps) | 1080 visibly softens small text |

VMAF 93+ is close to indistinguishable for most viewers; about 6 points is one
just-noticeable difference.

| Setting | Default | Effect |
| --- | --- | --- |
| `ASSET_PALETTE_COLORS` | 256 | Stills with at most this many distinct colours become a **lossless** indexed PNG (the exact-palette path from Captures). 0 disables. |
| `ASSET_IMAGE_FORMAT` | avif | Photo format: `avif`, or `webp`. Clients that cannot encode AVIF use WebP. |
| `ASSET_AVIF_QUALITY` | 85 | AVIF photo quality (libavif scale). 85 matches WebP q92 on SSIMULACRA2 at 19% fewer bytes on 12 MP photos. |
| `ASSET_IMAGE_QUALITY` | 92 | WebP photo quality (JPEG where WebP encoding is unavailable). 100 disables lossy photo encoding in either format. |
| `ASSET_IMAGE_MAX_EDGE` | 4096 | Longest still edge; 0 keeps the original size. Never upscales. |
| `ASSET_PREVIEW_EDGE` | 640 | Preview size for images and video posters (≤ 512 KiB). |
| `ASSET_VIDEO_MAX_HEIGHT` | 1440 | Short-edge cap for videos ("1440p", so portrait phone video and Retina screen recordings keep their detail); 0 uploads videos unchanged. |
| `ASSET_VIDEO_BITRATE_KBPS` | 8000 | Target bitrate at 1080p30, scaled by pixel count and frame rate (hardware encoders need more headroom than x264 to avoid blocking). |
| `ASSET_VIDEO_SOURCE_BITRATE_PERCENT` | 200 | A re-encode never exceeds this percentage of a non-H.264 source's own bitrate (H.264 sources: 100%, since more bits cannot improve them). H.264 needs about twice HEVC's bitrate for the same quality. 0 removes the cap. |
| `ASSET_VIDEO_HIGH_FPS_BOOST_PERCENT` | 150 | Above 30 fps the target grows with the frame rate (fps ÷ 30) up to this percentage, so 60 fps video does not get half the bits per frame. 100 turns it off. |
| `ASSET_AUDIO_BITRATE_KBPS` | 128 | Audio bitrate for transcodes (AAC, or Opus where AAC encoding is unavailable). |

Rules shared by every client:

- **Lossless stays lossless.** PNG, BMP, TIFF and lossless WebP (screenshots,
  UI, drawings) are never encoded lossily: the smaller of an indexed PNG (when
  the colours fit) and libwebp lossless WebP (`method 3`, `exact`), else the
  original. Pixels are identical and the ICC profile is kept. Real screenshots
  have thousands of colours from text anti-aliasing, so this matters: lossy
  WebP visibly blurred text in testing. Lossless WebP made 2880×1800
  screenshots 70–74% smaller.
- **Photos** (JPEG, HEIC/HEIF, lossy WebP) are re-encoded as AVIF at
  `ASSET_AVIF_QUALITY` when `imageFormat` is `avif`, else (or when AVIF
  encoding fails, or for HDR/other colour spaces) as lossy WebP at
  `ASSET_IMAGE_QUALITY`, and kept only when at least 10% smaller. HEIC is
  always converted (browsers cannot show it). Re-encoding applies the EXIF
  orientation and drops EXIF/XMP/GPS. A missing `imageFormat` (older API)
  means WebP.
- **Originals that upload unchanged lose their metadata losslessly:** JPEG
  APPn segments other than JFIF, ICC and Adobe are dropped (orientation is kept
  as a minimal Exif segment), PNG `eXIf`/`tEXt`/`zTXt`/`iTXt` chunks are
  dropped, and MP4/MOV `udta`/`meta` boxes under `moov` and each track are
  renamed `free` and zero-filled in place, so locations are gone and sample
  data is byte-identical.
- **Video** is transcoded to H.264/AAC MP4 only when needed: the short edge is
  above `ASSET_VIDEO_MAX_HEIGHT`, the codec is not H.264 (HEVC, VP9 and others
  do not play everywhere), or the bitrate is more than 1.25× the target.
  Otherwise the original uploads unchanged, so already-efficient phone video is
  never re-compressed. The output bitrate is the target at the output size and
  frame rate, capped at `ASSET_VIDEO_SOURCE_BITRATE_PERCENT` of the source's
  own bitrate (100% for H.264 sources; the 1.5 Mbps floor still applies). A size-only
  transcode is kept only when at least 10% smaller; the audio track is never
  dropped.
- **HDR video** (iPhone/Android HLG, HDR10) is tone mapped to SDR BT.709 (Hable,
  1000-nit peak) where the platform can do it; otherwise the original uploads
  unchanged rather than a washed-out SDR copy.
- **GIF, AVIF, audio, documents and other files** are not re-encoded locally;
  the server metadata policy below still applies.

AVIF quality parity. `avifQuality` is libavif's quality scale as of
libavif 1.0 (`avifenc -q`). On five 9–14 MP photos, quality 85 scored
SSIMULACRA2 83.3 against 83.0 for WebP q92 at 81% of its bytes. libavif 1.3+
with aom 3.13+ switches stills to aom `tune=iq`, which shifts that scale, so
Android and Apple pin `tune=ssim`; desktop's rav1e maps quality 85 to
quantizer index 55 (calibrated to within 0.05 points on average).

Why H.264 and not HEVC/AV1: chat apps send H.264 (WhatsApp, Telegram and
Signal encode it on the phone; Slack and Teams deliver it; Discord plays HEVC
only where the device already can). Streaming services that ship software AV1
decoders also store an H.264 rendition, which would defeat the storage saving,
and browsers cannot use a bundled decoder for `<video>` (Safari plays AV1 only
on devices with AV1 hardware).

The server never transcodes, but it independently removes embedded media
metadata before publication, including when an older client skips stripping.
Client compression remains optional; server metadata removal does not.

### Metadata privacy on completion

- JPEG, PNG, GIF, WebP, AVIF and HEIC/HEIF use ExifTool to remove EXIF/GPS,
  XMP, comments and other writable descriptive metadata. Only image orientation
  and colour-rendering metadata are copied back. GIF/WebP frames, loop settings
  and delays are not re-encoded.
- MP4/MOV, WebM, AVI and the supported audio containers (MP3, MP4/M4A, AAC,
  Ogg, WAV, FLAC, AIFF) are remuxed by FFmpeg with stream copy. Global tags,
  per-stream tags, capture dates, chapters, cover art and non-audio/video tracks
  (including GPS/timecode/data/subtitle tracks) are omitted. Audio/video samples,
  rotation, colour information and playback timing remain.
- Detection uses bytes, not just filenames or MIME types; labelling a known
  media file `application/octet-stream` does not bypass cleaning. Unsupported
  declared media (including SVG, BMP and TIFF originals) is refused rather than
  shared unchanged. Clients normally convert BMP/TIFF to a supported still format.
- This is **container metadata removal, not complete anonymisation**: it does
  not redact visible/audible locations, proprietary information embedded inside
  copied codec samples, ICC profile contents, or documents/archives and their
  embedded files. Non-media downloads retain their contents. Already-published
  uploads are not rewritten by this change.
- Upload URLs target only staging, so a repeated PUT after completion cannot
  replace a cleaned file. Sources are erased after success; the bounded purge
  sweep repeats cleanup after the 15-minute PUT lifetime, including late writes.
  Rejected/cancelled sources follow the existing purge/reconciliation loop.
- The Linux API image includes FFmpeg, ExifTool and `prlimit`. One job per pod
  streams to a private temporary directory, removed on success/error/cancellation.
  Processes have address-space/heap, CPU, output-size and wall-clock limits,
  closed stdin, no shell/config-file interpolation and no network protocols.
  Neither command output nor embedded metadata is logged. The production API
  needs a disk-backed writable `/tmp` (12 GiB for a 5 GiB R2 PUT plus its result)
  and 512 MiB memory; infrastructure provisions these before the API release.
  Storage/process failures fail closed. Cleaning may take up to two minutes
  per object. `/complete` responds within ten seconds with the result or a
  retriable `409`, leaving slow work running. Clients allow a queued batch of
  ten files to finish; a process restart safely restarts from private staging.

Apple's “Location Included” picker/drag badge describes the **selected source**,
before Caper processes it; it is not an inspection of the shared object. Verify
privacy by inspecting the downloaded result. Reservations created by the previous
API during a rolling update may need to be selected/uploaded again (their signed
PUT used the old final key, not staging).

Web specifics: WebCodecs via Mediabunny (loaded on demand). Google Chrome on
Linux encodes H.264 but not AAC, so those uploads carry Opus audio in MP4;
open-source Chromium builds without H.264 keep the original file.

### Encoders by platform

| Platform | Photos (AVIF) | Lossless | Video and HDR | Size cost |
| --- | --- | --- | --- | --- |
| Web | libavif 1.0.1 + aom 3.7.0 WASM (`@jsquash/avif` 2.1.1), single-threaded worker, speed 9 (≈3 s per 12 MP here) | `@jsquash/webp` 1.5.0 worker; indexed PNG | Mediabunny/WebCodecs H.264; HDR tone mapped on WebGL2 | 3.5 MB WASM (1.1 MB gzipped), fetched only when a photo is encoded |
| Android | libavif 1.4.2 + aom 3.15.1 JNI built from pinned sources (`prepare-avif.sh`), speed 8, `tune=ssim`; AOMedia decoder shows AVIF on API 26–30 | Bitmap lossless WebP; indexed PNG | Media3 Transformer H.264; HDR tone mapped with OpenGL | +1.8 MB download per ABI (3.75 MB installed) |
| Apple | libavif 1.4.2 + aom 3.15.1 static XCFramework from [joswayski/libavif-apple](https://github.com/joswayski/libavif-apple) (built from pinned sources, Neon on arm64, SSE/AVX on x86_64), speed 6, `tune=ssim`; ImageIO decodes | libwebp 1.6.0 lossless; indexed PNG | AVAssetWriter H.264 at the shared size and bitrate; HDR to BT.709 via AVVideoComposition | ≈4.3 MB uncompressed per arm64 slice (estimate) |
| Desktop (Windows, Linux) | rav1e 0.8.1 + avif-serialize, speed 10, quantizer 55 for quality 85; x86 assembly when nasm is present (1.4 s vs 4.6 s per 14 MP) | libwebp 1.6.0 lossless (`webpx`); indexed PNG | Bundled FFmpeg 9.0.2: Windows' Media Foundation H.264 encoder (hardware on most PCs), else x264 `veryfast`; HDR tone mapped with zimg (Hable, as on web) | ≈2.4 MB uncompressed (1 MB gzipped), plus FFmpeg ≈16.6 MB (6.1 MB gzipped) |

### Server-side processing (parked)

A server pipeline was built and then deliberately not shipped: clients
uploaded originals to S3, an arm64 Lambda (`apps/media-worker`) compressed
them (lossless WebP/AVIF, H.264 with HDR tone mapping, FLAC, gzip) into R2,
and the API pushed `message.attachments` updates. Encoding on our servers
costs roughly $0.003–0.02 per video minute, which a $5/month plan cannot
absorb for heavy uploaders, while client encoders are free. It remains in the
history of joswayski/caper#246 (contract `9d3709b`, API `5f23d15`, worker
`08797de`, `72f8514`, `c5f57ea`) and joswayski/infrastructure#129 (`8ae9ace`) for when it
is needed. Clients still accept the optional `status`
(`processing|ready|failed`), `animated` and `attachmentsSeq` fields and the
`message.attachments` / `attachment.progress` events it used. Today's API
sends only `attachmentsSeq`, on edits that change a message's files, and an
absent `status` means ready.

Follow-ups:

- Desktop still uploads HEIC unchanged and cannot edit HEIC/AVIF. The bundled
  FFmpeg decodes AVIF inline images/spoiler previews and AVIF/HEIC full-size images.
- Server metadata stripping regression checks use real FFmpeg/ExifTool on
  synthetic geotagged fixtures plus a disposable Postgres and fake R2. Live R2,
  Apple picker/share workflows and physical native-device checks remain separate.
- HEVC/AV1 delivery once every client can play it; chunked parallel encoding
  if server processing returns.
- JPEG XL: Chrome 155 (October 2026) decodes it by default, Safari since 17
  and Firefox from 158. Its draw for us is lossless JPEG recompression
  (about 20% smaller and byte-for-byte reversible), but Android has no
  platform decoder, desktop and older browsers would need one, and AVIF
  already gives about 19% over WebP on photos. Revisit once Android and our
  native clients can display it.

A text edit keeps a message's attachments, and a message with files may be
edited to empty text. Its author can also replace its photos and videos or
change spoilers, but not add or remove files (see
[Editing attachments after sending](#editing-attachments-after-sending)).
A forward embeds a snapshot of
its source message, including its attachments, whose URLs are signed with the
forward; they refresh when the forward is reloaded, not through
`POST /api/assets/urls`. Pinned messages and thread replies show their files.
Thread replies may carry files through the API; the client composers attach
files only in channels and DMs.

Pasting copied files or images into a channel/DM text box uses the same draft
attachments as the file picker: preparation/upload starts immediately, but no
message is sent until Send/Enter. Existing text stays intact, and ordinary text
and links still paste at the selection. The 10-file limit, upload errors, retry,
removal and conversation permissions are unchanged. Web accepts the files the
browser exposes in the paste event (not HTML images or URLs to download); Apple
accepts native file URLs and image data; Android accepts clipboard content URIs
through the text box's Paste menu or Ctrl+V; Rust desktop accepts native file
lists and images with Ctrl+V. Android keeps its value-based composer and adds a
file-aware text menu/keyboard handler rather than changing selection/mention
editing. Rust's egui integration suppresses image-only paste key-downs, so its
key-up supplies the missing event without duplicating text-bearing pastes.

No API, gateway, database, infrastructure or secret change is needed. Clients
can release independently against the existing upload API; rolling a client
back only removes its new paste entry point, not sent attachments. Before a
native release, verify clipboard access on the actual OS, including screenshots,
copied documents, ordinary text, a full tray and disabled uploads. Browser
coverage lives in `scripts/test-chat-paste.mjs` against the local
`native-parity-fixture.mjs`, not real account/storage traffic.

Paste validation (October 12, 2026): `npm run check` and 604 `npm test` tests
pass. The browser regression passes at 1280px and 390px: real Chromium clipboard
image/text paste, synthetic multi-file paste, preserved draft text, removal,
confirmed channel/DM sends, the 10-file boundary and unavailable uploads. Both
attachment-tray captures were inspected; narrow Chromium is not a phone test.
Rust desktop fmt, Clippy, Linux build and 484 tests pass (12 existing ignored).
Android's Gradle attempt stops at the missing SDK; Apple has no Swift/Apple
toolchain in this orb. Native OS clipboard/device acceptance remains unverified.
No Docker daemon is available; the web production build was checked directly.

### Editing attachments before sending

A pencil on a draft's tile opens a full-window editor for still images and
videos before they are sent, on web (desktop and mobile layouts), Android,
iPhone/iPad/Mac and Rust desktop. Spoiler, Edit and Remove share one solid bar
in the tile's top-right corner, like Discord's (with a pointer, 28 px buttons;
on touch layouts the bar spans the tile's top edge and its 44 px buttons share
the width); hovering a button only lightens it. Edit appears only for files the
editor can open. A spoilered file stays editable: the editor and preview show
it unblurred to its author, its tile shows the blurred edited render, and Send
carries both the edit (already in the uploaded file) and the spoiler flag.
Opening a draft's preview (a tap or click on the tile) shows **Crop**, **Draw**
and **Text** under the picture for files the editor can open; each closes the
preview and opens the editor on that tool (the pencil opens Draw). Web uses
its media viewer; Android, Apple and desktop open such files in the app's own
preview instead of the system viewer (QuickLook on Apple, the system app on
Android and desktop), which can't host the tools, and keep the system viewer
for other files. Each preview zooms pictures like that app's media viewer
(pinch or, on desktop, Ctrl+scroll; double-tap or double-click; drag to pan
while zoomed); video doesn't zoom except desktop's still frame. Desktop's
preview also has Open in default app, which opens what will be sent: the
original when unedited, otherwise the prepared edited file, waiting with
"Available when your edit is ready" while it prepares (a profile card file is
prepared when it's clicked); and for videos when its FFmpeg lacks the overlay
filter, it disables Draw and Text as the editor does. On web phones the editor
takes over the preview's history entry, so Back closes the editor. The editor offers:

- **Crop** with corner and edge handles (drag inside to move; with a keyboard,
  arrow keys nudge the frame on web and Android) and **Rotate left** in quarter
  turns.
- **Draw** freehand with round lines, and add **Text** (Satoshi Bold, one line,
  up to 200 characters): click or tap to place it, click it to change it, drag
  to move it. With a mouse, trackpad, pen or iPad pointer, Draw shows a ring
  the size and colour of the brush in place of the cursor (at least 6 px).
  Both take one of six colours (red, yellow, green, blue, white, black) or any
  colour from the platform's colour picker, and a size slider (2–40 pt lines,
  14–96 pt text, as seen on screen).
- **Undo**, **Redo** (Ctrl/⌘+Z, Ctrl/⌘+Shift+Z, Ctrl+Y) and **Reset** to the
  original; Escape closes a text field, then cancels. Done with no change just
  closes.

**When it happens.** Uploads still start the moment a file is added, so
unedited files send as quickly as before. The pencil is available straight
away (editing uses the local original, not the upload). Saving an edit cancels
that file's preparation and upload (`POST /api/assets/{id}/cancel`, the same as
Remove; the unattached upload is purged by the existing cleanup) and prepares
the **original** again with the edit applied, then uploads the result in the
same tile. The draft keeps the original and a description of the edit, so
reopening shows the earlier crop, drawings and text still editable, and edits
never stack generation loss. The profile card composer, which uploads at Send,
applies the edit when it prepares the file. The pencil is hidden while a send
that carries the files is pending.

**Never the unedited file.** People crop to hide things, so an edit that cannot
be applied (the image cannot be decoded, the platform cannot encode H.264, the
transcode fails) fails the tile with "Couldn’t apply your edits." and its
Retry, rather than uploading the original. Send retries it once, like any
failed upload, and never sends the message without it.

**The model** (`apps/web/src/chat/edit.ts`, mirrored by each client): quarter
turns of the upright original (EXIF applied), a crop as fractions of the
turned picture, then marks painted in order: strokes (points as fractions,
width as a fraction of the longer edge) and text centred on a point (size as a
fraction of the longer edge, always upright). Every client's tests run
`shared/messages/edit-cases.json` (turns, crop rounding including even video
crops, and when a file counts as edited).

**What gets uploaded.**

- Images are painted at the original's own resolution, then encoded by the
  source's class under the rules above: photos (JPEG, HEIC, lossy WebP, AVIF)
  become AVIF/WebP at the server quality and `ASSET_IMAGE_MAX_EDGE`, and
  screenshots/graphics (PNG, BMP, TIFF, lossless WebP) stay lossless. The
  smallest encoding is always kept (the original is not an option), with PNG
  as the last resort. Output is upright and carries no EXIF.
- Videos are always transcoded to H.264, even when the original would have
  uploaded unchanged. The turn and crop are baked into the frames, the crop
  starts and measures on even pixels, its short edge is
  capped by `ASSET_VIDEO_MAX_HEIGHT` (0 keeps the crop's size) and the bitrate
  target is for the output size. Drawings and text are one still overlay over
  the whole video (the editor says so and shows a still frame from just after
  the start). HDR is
  tone mapped as usual; where the platform cannot tone map it, editing fails
  rather than uploading an unedited or washed-out copy. Audio is kept.
- Editable: still JPEG, PNG, WebP, AVIF, HEIC/HEIF, BMP and TIFF the platform
  decodes, and videos it can transcode. GIFs, animated WebP/PNG, audio and other
  files show no pencil.

No API, gateway, database, infrastructure or secret change is needed: the
server sees an ordinary upload, and cancelled reservations already exist.

| Client | Editor | Images | Video |
| --- | --- | --- | --- |
| Web | `AttachmentEditor.tsx` (loaded on demand), full-window dialog; Back closes it on phones | Canvas at full resolution, then the existing encoders | Mediabunny/WebCodecs: rotate, crop and resize, then the overlay in `process`; HDR through `hdr-video.ts` with the tone-mapped frame drawn turned and cropped. Shown only where the browser encodes H.264 (open-source Chromium has no encoder) |
| Android | `AttachmentEditor.kt`, a full-screen Compose dialog; Compose has no colour picker, so the custom swatch opens a small hue and saturation/value picker | `EditPainter` (android.graphics) on the decoded crop region at native size, then the existing encoders | Media3: `ScaleAndRotateTransformation`, `Crop`, `Presentation`, then a `BitmapOverlay` of the marks; HDR needs Android 10+ to tone map. Like the existing transcode, portrait output may carry rotation metadata. AVIF is editable from Android 12 and HEIC from 9; TIFF is not (no platform decoder) |
| Apple | `AttachmentEditor.swift`: a full-screen cover on iPhone/iPad, a window overlay on Mac (inside the profile popover there); the system `ColorPicker` | `AttachmentEditRenderer` (CoreGraphics/CoreText) at native size, then the existing encoders | The existing BT.709 composition (the HDR tone map) read as BGRA, each frame turned, cropped, scaled and overlaid with Core Image, written through a pixel-buffer adaptor |
| Rust desktop | `attachment_editor.rs`, a full-window egui modal with egui's colour picker | `attachment_edit.rs` on the decoded original at native size: tiny-skia strokes, Satoshi Bold through ab_glyph, then the existing encoders. HEIC and AVIF are not editable (the desktop pipeline does not decode them) | FFmpeg filter graph: `transpose`/flips, `crop`, then scale or the HDR tone map. Marks need `overlay` with a PNG input (`-f png_pipe`), which the bundled trimmed FFmpeg 9.0.2 lacks, so Draw and Text are disabled for videos (with an explanation) until joswayski/ffmpeg-desktop enables the `overlay` filter, `png` decoder and `png_pipe` demuxer; the app checks the binary and uses them once present. Videos show a frame from 1 s in, the desktop poster rule |

Validation (October 17, 2026): `npm run check` passes and `npm test`
passes apart from three tests outside this change (`noise-assets` fails the
same way on the base commit here; the two GIF-shrink tests pass alone and time
out only under load). Browser runs against the loopback fixture at 1280px and
390px (Chromium) covered drawing, text placement, dragging and re-editing,
crop handles, rotation, undo/redo shortcuts, Reset, Escape, the pencil only on
still images, and that the earlier upload is cancelled and the uploaded bytes
have the expected size, crop, turn and colours. That Chromium has no H.264
encoder, so web video edits were checked with Mediabunny and VP9 (turn, crop,
size and overlay position); H.264 and HDR video edits in a browser remain
unverified. Android: `testDebugUnitTest` (445 tests, including the shared
cases), `lintDebug` and Kotlin compilation pass with a local SDK. Rust desktop:
fmt, Clippy and 570 tests pass; ignored real-FFmpeg tests turn, crop, tone map
and draw on video with a full FFmpeg and refuse marks with the bundled one;
Xvfb captures of the editor were inspected. Native development builds on the
branch: Android test, lint, build and emulator smoke pass (the smoke does not
open the editor); Linux and Windows desktop builds pass; the macOS
`xcodebuild test` passes, including CoreGraphics rendering and an H.264 clip
turned, cropped and drawn on (one rerun crashed earlier, in `APIClientTests`,
when a request reached a mock URL handler the previous test had cleared); the
iOS simulator build passes. Its parity UI tests passed before the rebase onto
#664. After it, `testLatestReactionsStayAboveOpenKeyboard` timed out twice with
the reaction fully visible above the composer: each check read four frames,
each a slow accessibility query, within a 5-second wait (`main`'s passing run
took 4.8 s for one chip). The check now reads each frame once, and with it all
39 iOS parity UI tests pass (one run stopped earlier at `GatewayLifecycleTests`,
which this change doesn't touch). The macOS parity UI step fails the same nine
tests as `main` does without this change (Live now, thread and composer layout
checks; the Intel Mac passed one of them in the latest run), so it says nothing
about the editor. Not yet run: the editors on an emulator or physical device,
Media3 effects and encoders, Windows (Media Foundation) and real storage
uploads from native clients.

Validation of the tile bar, preview tools and brush ring (October 18, 2026):
web `npm run check`, the typecheck and `npm test` pass (apart from the same
two tests as above), and Chromium at 1280px and 390px checked the bar's order
and solid hover, the preview's tools opening the editor on their tool, Back on
phones closing that editor without a stray history entry, and the ring's size,
colour and hiding. Android: compilation, unit tests (including the tile
actions, initial tool and ring diameter) and lint pass, and in Native
development builds the build and emulator smoke pass (the smoke doesn't open
the preview or editor). Apple, in Native development builds: the macOS and iOS
simulator builds compile, the core tests pass on both (including the new tile,
tool and ring tests), all 39 iOS parity UI tests pass, and the macOS parity UI
step fails only `main`'s tests as above; no UI test opens the new preview or
the ring. Rust desktop: fmt, Clippy and 577 tests pass, Xvfb captures of the
bar, preview and ring were inspected, and the Linux and Windows builds pass.
Not yet checked anywhere: the Mac blank cursor and iPad pointer hiding, stylus
hover, and the bar on physical touch screens.

Preview zoom and desktop's Open in default app (October 18, 2026): each app's
media viewer zoom moved into a part both screens share (Apple `MediaZoom`,
Android `ZoomableImage`, desktop `viewer::Zoom`), so the viewers keep their
behaviour. Android compilation, unit tests and lint pass; desktop fmt, Clippy
and 581 tests pass (zoom, panning, the button's states and the temp copy it
opens), with Xvfb captures of the zoomed preview and the waiting tooltip;
in Native development builds, Apple's macOS and iOS simulator builds and core
tests pass, all iOS parity UI tests pass (including those opening the media
viewer whose zoom moved into `MediaZoom`), the macOS parity UI step fails only
`main`'s tests, and the Android emulator smoke passed on a rerun after the
emulator's own System UI crashed in the first attempt. Pinch on real touch
screens and trackpads, and a default app actually opening, are unchecked.

With spoilers (October 18, 2026): merged with attachment spoilers, the tile's
bar holds Spoiler, Edit and Remove on every client. Web `npm run check`, the
typecheck and `npm test` pass (apart from `noise-assets`, which fails the same
way on `main`), and Chromium at 1100px and 390px showed the three-button bar
inset on desktop and spanning the tile on a phone, with the spoilered tile
blurred. Android compilation, 473 unit tests (including the bar's order, its
labels and the picture a spoilered tile blurs) and lint pass; desktop fmt,
Clippy and 612 tests pass, with Xvfb captures of the pointer and narrow bars,
the spoiler cover and the video badge. Native development builds on the
merge: the macOS and iOS simulator builds compile and their core tests pass,
Linux desktop builds, and Android tests, lint and the build pass; the UI steps
fail only the tests `main`'s nightly fails the same way (one iOS invite test,
eight macOS layout tests, and Android's reaction picker and What's New links).

### Editing attachments after sending

**API only so far:** the server accepts these edits, but no client offers them
yet; the web, Android, Apple and Rust desktop editors follow. The author of a
message can replace any of its photos and videos with an edited render, put
back the file first sent in that slot, and turn any file's spoiler on or off.
Files can't be added, removed or reordered after sending.

**Request.** The text edit `PUT /api/chat/channels/{channel}/messages/{message}`
takes an optional `attachments` list: the message's files in their current
order, one entry each.

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
  upload: the author's, completed in this channel within 24 hours and not
  sent, of the same kind. `edit` says how it was rendered from the slot's
  first-sent file, in the pre-send editor's model. Its rules and limits (quarter
  turns, a crop inside the picture, at most 500 marks and 100,000 point
  values, one line of 1–200 Unicode scalar values, `#RRGGBB`, 128 KiB of
  JSON, fractions rounded to a millionth when saving) are the `wire` section
  of `shared/messages/edit-cases.json`. The API tests every case; each client's
  editor is to test them too.
- `{id, replaces}` where `id` is the slot's first-sent file undoes every edit
  of that file: no upload and no `edit`.
- Without `attachments` the request behaves exactly as before; clients send it
  only when files change (an older API refuses the field with 422). A spoiler
  toggle alone is an edit, with a revision, the edited marker and a version.
  Text may be empty while files remain.
- The response and the `message.edited` event carry the new list, and
  `attachmentsSeq` becomes the edit's `editSeq` whenever the files change, so
  released clients, which keep a message's files unless `attachmentsSeq` is
  newer, take them. Edits never notify.
- Resending the same body after a lost response returns the stored message;
  this is checked before any upload is linked. A different list at an old
  revision is 409, like a stale text draft.

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

**Who sees what.** Replaced files stay stored and linked to their message,
marked `retired_at` (migration `202610180001_attachment_edits.sql`, additive);
a render records its first-sent file (`edit_source_id`) and the description
(`edit`).

- Everyone sees the current files wherever the message appears: history,
  threads, pins, single messages, forwards and notifications (a notification
  queued before the edit shows the new file).
- Only the author can still open replaced files: `POST /api/assets/urls`
  refreshes them only for their owner, and `GET …/versions` signs every
  version's files for the author, replaced ones included.
- Others get versions without URLs, in which files no longer on the message
  are placeholders: `{id, kind, contentType, name: "Replaced file", size: 0,
  unavailable: true}`. Both views mark a revision whose `(id, spoiler)` list
  differs from the one before with `filesChanged: true`.
- Stored events (`message.created`, `message.edited`, pins and forward
  snapshots) still name earlier files, so both gateway replay paths, the outbox
  publisher and notification content turn replaced files into the same
  placeholder for every viewer; the later `message.edited` brings the current
  files.
- The edit description appears only in the author's `attachment-sources`,
  never in the message, its versions or its events.
- Not taken back: URLs already issued work until they expire (one to two hours;
  the CDN checks only the signature), as do downloaded copies.
- Replaced files are never purged, still count toward the author's
  `ASSET_QUOTA_BYTES`, and cancelling one returns 409.

**Re-editing** starts from the first-sent file, so edits never compound.
`GET /api/chat/channels/{channel}/messages/{message}/attachment-sources`
(author only: 403 for others, 404 without access or for a forward) lists each
current photo and video with the first-sent file to edit from, freshly signed,
and the current file's edit (`null` when unedited):

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

The first-sent file is the compressed upload, with any edit made before
sending already in it, and it keeps its EXIF orientation, so editors decode it
upright as they do before sending.

**Rollout.** Ship rolls the API first (the migration runs at startup), then
the gateway; until the gateway updates, its replays are not concealed, but no
released client can make such an edit yet.

Validation (October 18, 2026): fmt, Clippy and the 179 default workspace tests
pass, including the planner and every shared `wire` case. With a disposable
Postgres 16, cluster-mode Redis 7 and ExifTool, all ignored database tests
pass, including new ones for replacing, restoring and chained edits, retries,
409s and each refusal, URL refresh for the author and others, versions
(placeholders, signing, `filesChanged`), `attachment-sources`, purge, quota and
cancel, both gateway replays, the publisher, forward projection and
notifications. The migration was also applied over existing rows. No client,
live R2/CDN or multi-account check yet.

### Inline GIF playback

GIF uploads remain ordinary `image/gif` originals; the current upload path does
not convert them to videos. Upload preparation can also produce a **still**
WebP/JPEG preview. That preview is useful for placeholders and notifications,
but choosing it as the ready chat image freezes the animation. The legacy
`animated` video flag is not required for ordinary GIFs.

Ready GIF tiles now load the original on web, Android, Apple and Rust desktop;
still photos retain their smaller previews. The native clients must also use
animation decoding, not just select a different URL. Android uses the existing
Coil GIF decoder per request. Apple reuses its ImageIO frame loader and timeline;
Rust advances composited GIF frames on the visible tile's existing texture.
Local first-frame thumbnails cannot overwrite a loading/playing desktop GIF.
Click/tap still opens the existing viewer. No upload, wire, API or database
change is needed, and existing GIF attachments do not need to be uploaded again.

Reduced-motion readers see a still inline, including GIFs with no uploaded
preview. Web draws a bounded canvas still; native clients use their existing
motion preference. Apple and Rust inline decoding is limited to a 720px long
edge, 600 frames and 32 MiB of retained frame pixels per GIF. When the pixels
would exceed that budget, playback resolution shrinks while every frame and
delay is preserved. A 720×400 GIF exceeds the budget after just 29 frames at
full resolution; it must not silently become a still. Apple chooses the size
from the frame count and actual decoded row stride, while Rust shrinks retained
frames geometrically during its single decode pass. Files exceeding the
600-frame decode-work ceiling still fall back to one still, not a truncated loop.
Rust also applies decoder allocation/dimension limits and the existing 40 MiB
network limit, so files exceeding those limits may fail inline. Apple releases
frames on disappearance and Rust releases inactive animation cache entries.
Animated WebP/APNG timeline support is not added by this GIF fix.

Validation (October 13, 2026): `npm run check` passes; `npm test -- --maxWorkers=2`
passes 605 tests. Chromium exercised actual `prepareFile` and `MessageAttachments`
with a generated 720×400 GIF, with and without its generated still preview,
and a still-photo control. Before the fix, the preview-backed GIF changed zero
pixels between captures; after, both GIF tiles changed 33,893 pixels without a
click. The photo control and both reduced-motion GIFs changed zero pixels.
Desktop/narrow (390px)/reduced-motion captures and a playback recording were
inspected. Cold reduced-motion loads also load the original and display a still.
This is a labelled local component fixture, not live R2, Safari or phone evidence.
Rust desktop tests pass (487 passed, 12 existing ignored), including real GIF
frame pixels, asymmetric timing, looping, reduced motion, re-signed URLs and a
late local thumbnail. Apple has a real ImageIO decode/budget regression test,
but no Apple toolchain/device acceptance is available in this Linux orb.
Android's compile/unit-test attempt stops at `SDK location not found`.
Android/Apple builds, native visual/device checks and live R2 acceptance remain
required before release. No Docker daemon is available; the web production
build was validated directly.

Memory-budget follow-up (October 14, 2026): a generated 720×400, 60-frame GIF
reproduced the freeze in both the real Mac attachment view and Rust's decoder:
only one frame survived. The Rust regression now verifies all 60 frames within
32 MiB, the last frame, looping and reduced motion; desktop tests pass (510
passed, 12 existing ignored), as do formatting and Clippy. The rebuilt Linux
client against the loopback chat/media fixture changes 9,308 GIF pixels between
captures while the adjacent photo changes zero; playback resumes after switching
channels and back. This is local fixture evidence, not a production-upload or
Windows-device check. The published Linux package could not run in this orb
because it requires glibc 2.39; the local source build was used instead.

The real Mac attachment renderer now animates the same 60-frame GIF: 28/28
consecutive captures changed initially and 29/29 after remove/remount. The
29-frame and small-GIF controls keep playing, the PNG stays unchanged, and
Reduce Motion holds all three GIFs on frame 1 before and after reload. ImageIO
retains 60 frames at 500×278 (33,360,000 bytes), trading some Retina sharpness
for complete playback within 32 MiB. Focused GIF XCTest passes 3/3 on the final
source; the full CaperCore suite passes 365/365 on identical attachment source.
macOS and iOS-simulator Xcode builds pass. This is a local renderer harness,
not an installed-release or physical-iPhone check; viewer open/close is not yet
verified. No web, Android, upload, API or schema change is required for this
follow-up; only Apple and Rust desktop need the corrected native release.

Deployment order:

1. Infrastructure, secrets/configuration, migrations, API, gateway and CDN:
   **no changes or deployment required**. Keep the current signed delivery and
   private R2 bucket; do not publish a bucket or rewrite stored assets.
2. The original web playback fix uses the normal image/release process; current
   main pushes build changed web code and dispatch Ship after image publication.
   This native memory-budget follow-up needs no web deployment.
   In a real channel and DM, send a GIF large enough to have
   a preview; verify playback before clicking and after reloading, narrow layout,
   viewer opening, reduced motion and an unchanged still-photo control.
3. Release Android, iOS/macOS and Rust desktop through their normal client
   channels; these can deploy independently of web and one another. First build
   and validate each platform, including scrolling offscreen/back, URL expiry,
   replacing a pending thumbnail, reduced motion and memory with multiple GIFs.
4. Roll back the web image if necessary; native rollback uses a corrected release
   with a higher build number. No data/config rollback or GIF re-upload is needed.

### In-app viewer

Clicking a sent image or video opens it almost full screen inside the app on
every client (`MediaViewer.tsx` on web, `MediaViewer.kt` on Android, the
`AttachmentViews.swift` viewer on iPhone/iPad/Mac, `viewer.rs` on desktop),
like Discord's. The viewer holds that message's ready images and videos in
message order, starting at the one clicked: arrows, Left/Right or a swipe move
between them (no wrap) with an "n / m" counter. Images show the preview until
the original loads, zoom with double-click/double-tap, pinch or Ctrl+scroll,
and pan while zoomed; animated GIF/WebP play. Videos autoplay with sound and
pause when you move away or close; GIF-style videos (`animated`) loop muted.
Each viewer has Save/Download, Open in browser and Close (Esc, the dark area,
or a swipe down on phones). The header shows the current file's stored size
(`size`, what Save/Download fetches): after the author and time on web and
Android, after the counter on Apple, and after the name on Rust desktop. Web and
Android keep the size whole on narrow screens and truncate the author and time
first; web drafts opened in the viewer show their final upload size once
prepared. The client-reported pre-compression size (`assets.source_byte_size`)
stays internal. URLs come from each client's existing fresh-URL cache, and a
refused load asks for one refresh. Audio and other files keep
their inline behaviour; pending rows never open the viewer.

The iPhone/iPad swipe-dismiss implementation retains the file's release offset
while the full-screen cover dismisses. `GestureState` still resets short or
canceled drags, but cannot snap a successfully dismissed file back to centre.
The distance/fling thresholds, paging and zoom exclusions are unchanged.
Validation (October 11, 2026): Swift 6.1.2 syntax parsing passed for iOS 17 and
macOS 14. A temporary Swift harness executed the production release closure
and offset expression with a simulated gesture-state reset: the old code
failed all four successful-dismiss cases; the fix passed all ten cases,
including threshold boundaries, horizontal/upward drags and cancellation.
`npm run check` and all 565 repository tests passed. This is not SwiftUI
typechecking or rendering: no Xcode or connected Apple runner was available.
Before distributing the iOS update, check slow/held and fast downward releases,
an already-offscreen file, short/canceled drags, paging, zoomed panning and
Reduce Motion on iPhone/iPad. Other clients and services need no rollout;
rollback requires a corrected higher-build-number iOS release.

Desktop has no platform video player or AVIF decoder, so it ships a trimmed,
static FFmpeg built from pinned sources by
[joswayski/ffmpeg-desktop](https://github.com/joswayski/ffmpeg-desktop)
(FFmpeg 9.0.2, x264, dav1d 1.5.4, zimg 3.0.6; GPL-2.0-or-later, run as a
separate program so the app stays Apache-2.0). `build.sh` and `build.ps1`
download its `ffmpeg-9.0.2-r1` release, check the zip's SHA-256 and package
`ffmpeg` (`ffmpeg.exe`) beside the app, with its licences, `SOURCES.txt` and
build recipe in `ffmpeg-licenses/` (`/usr/share/doc/caper-desktop/` in the
.deb). That repo's CI smoke-tests the app's exact commands on Linux and on a
Windows runner, where Media Foundation's H.264 encoder (asked for High profile)
and x264 both pass; a newer build is a new release revision and a new pin. The
app uses it to:

- compress videos before upload with the shared rules (sizes are FFmpeg
  expressions on the rotated frame, so portrait video keeps its shape),
  reporting "Compressing… N%" on the draft chip; any failure uploads the
  original;
- play videos in the viewer: one FFmpeg decodes RGBA frames at a constant rate
  into an egui texture, another decodes 48 kHz stereo PCM for rodio, and audio
  is the clock (the wall clock without sound). FFmpeg has no TLS; it streams
  the signed CDN URL through a loopback range proxy (`media_proxy.rs`, bound to
  127.0.0.1 with an unguessable per-video token). Seeking restarts both
  decoders at the new time;
- decode AVIF and HEIC originals for the full-size viewer.

It is found beside the executable (`/usr/lib/caper-desktop` for the .deb) or
at `CAPER_FFMPEG`. Without it (a development build), videos upload unchanged
and the viewer offers the browser. The ignored real-FFmpeg tests run against
the shipped build with `CAPER_FFMPEG` set to it and `CAPER_FFMPEG_FIXTURES` set
to a full FFmpeg (lavfi test sources, libx265) that makes their inputs.

### Deletion and purge

Selecting a file starts uploading before Send. Removing a composer file cancels
local work and best-effort calls authenticated `POST /api/assets/{id}/cancel` on
web, Android, Apple and Rust desktop. The endpoint soft-deletes only the owner's
unsent asset and returns 204, including repeat requests; missing/foreign files
return 404 and sent files return 409. Its row lock serializes with Send, so a
late cancellation cannot delete a successfully sent file. Failed uploads and
discarded composer files also request cancellation. Moving a file into a pending
send or cleaning up a confirmed send's local copy must not cancel it. An unknown
send outcome keeps the original IDs for a safe retry.

An API loop (approximately every minute, idempotent across replicas):

- Marks uploads never completed after 1 hour, or completed but never sent
  after 24 hours, as deleted. This remains the fallback when a client is offline,
  closes before cancellation completes, or never receives the reservation ID.
- Removes R2 objects for deleted rows in batches of 25: unsent files once their
  15-minute upload URLs have expired plus a 15-minute margin (30 minutes after
  the reservation; no 24-hour wait after explicit removal), so a reused URL
  can't store bytes the quota no longer counts; sent files 24 hours after
  deletion (so moderators can still inspect them). Only successful deletion
  of both original and preview sets `purged_at` and releases quota. Failures retry.
- Revisits deleted, unsent rows, including already-purged rows, in a separate
  25-row batch. Presigned PUT URLs can be reused until expiry and an in-flight PUT
  can race deletion. Rows remain permanent cleanup records; a cursor with a fixed
  ceiling per pass prevents new arrivals or failed deletions starving older rows.
  `purged_at` records the first successful delete pass, not permanent object
  absence. Full-sweep latency and scan cost grow with retained history; no finite
  cutoff is safe without a provider-enforced bound on late writes.

An open draft does not extend the server's 24-hour lifetime. All clients retain
the local prepared file and preview until removal or delivery confirmation.
On focus/foreground or Send, uploads at least 23 hours old are silently uploaded
again, leaving a one-hour margin. Desktop retains an owned copy rather than
re-reading a picked path that may change. Concurrent triggers share the active
upload; successful files in a partially failed batch are reused on Retry.

For unsent composer drafts, Send retries failed uploads and continues once the
entire selected batch is ready. Another failure keeps the text and files in the
composer instead of sending the successful subset. Failed tiles have a **Retry
upload** action and a highlighted warning explaining that Send retries; the
normal uploading status/progress returns during each attempt. There is no new
continuous retry loop. Native Send attempts are cancelled if their composer or
destination changes while waiting; profile composers retain their complete
selection on failure as well.

Send enforces expiry even before cleanup and returns 409 `attachment_unavailable`.
Clients treat this as a definitive refusal, silently re-upload and send with a
new message UUID and attachment IDs. Recovery is bounded to one replacement
cycle per send attempt. A failed upload shows only **Failed to upload.** with
**Retry**, preserving the pending message and any newly composed draft; returning
to the app also retries upload failures. No expiry or reselect explanation is
shown. A timeout still means an unknown send outcome: retry the original command
first, never cancel or replace its files until acceptance or a definitive refusal.

Staged files stay with their conversation across switches for the session
(see below), not across app restarts or page reloads.
No new lifetime-renewal endpoint or schema is introduced. Replacement uploads
reserve quota normally; cancelled bytes remain charged until purge succeeds.

History marks deleted attachments `unavailable` without URLs. Files replaced by
an edit keep their `message_id`, so the loop never deletes or purges them and
cancelling one returns 409. No user-facing sent-message or sent-file delete exists yet; account deletion should set `deleted_at` on
the owner's assets when that flow lands.

#### Upload retry feedback validation (October 17, 2026)

| Platform | Local validation boundary |
| --- | --- |
| Web | `npm run check` and all 657 `npm test` tests pass. `scripts/test-chat-paste.mjs` and `scripts/test-profile-composer.mjs` pass against the disposable parity fixture: held retries, repeated failure, retained text/files, complete-selection delivery exactly once after recovery, channel/thread isolation, and profile queued-file neutrality. Inspected 1280px/390px DPR-2 Chromium failed/uploading states, thread/profile composers, and a recorded two-image failure/recovery interaction. This is not live R2, Safari, touch or physical-device validation. |
| Rust desktop | All 555 tests pass; 12 device/audio tests ignored. Formatting, Clippy and Linux binary build pass. Tests cover complete-selection retry, duplicate Send and stale destination/draft guards. Inspected the Linux `parity-attachments` fixture; no live native upload or Windows/macOS render acceptance. |
| Android | Implementation and Compose regression updated. `:app:testDebugUnitTest :app:compileDebugAndroidTestKotlin` stops during configuration because the Android SDK is absent. No Kotlin build or device rendering verified. |
| iOS/macOS | Implementation and XCTest updated for retry success/failure, duplicate Send and stale composer protection. Swift/Xcode unavailable; no Apple compilation, tests or rendering verified. |
| Containers | Docker daemon unavailable. Web production build validated directly; no infrastructure or API changes. |

#### Cancellation validation and rollout (October 11, 2026)

| Platform | Validation boundary |
| --- | --- |
| API/Postgres | Workspace tests, formatting and Clippy pass. The disposable loopback Postgres attachment test passes with fake S3: owner-only/idempotent cancellation, sent-file protection, Send/removal races, expiry before purge, accepted-send retries after 24 hours, quota held across delete failures, late PUT reconciliation and multi-batch sweep progress. Not live R2 acceptance. |
| Web | `npm run check` and `npm test -- --maxWorkers=1` pass (576 tests after merging current main). Labelled browser fixtures at 1440px and 390px, DPR 2, exercise ready/in-flight/reservation-race removal, synthetic file drop and overlay dismissal, silent focus/Send renewal after 24 hours, failed renewal and chip Retry, preserved same-text newer drafts, identical uploaded bytes and no cancellation of sent files. Ready, pending failure/Retry and chip failure/Retry states inspected; not live R2, Safari or a physical phone. |
| Rust desktop | 416 tests pass, 12 device/audio tests ignored after merging current main; formatting, Clippy and Linux build pass. Includes silent replacement/Retry, preserved newer drafts and a loopback upload test that changes the picked file between uploads to verify retained bytes. Inspected the labelled Linux `parity-attachments` fixture with ready/uploading/failed chips and Retry. Not Windows or physical-device acceptance. |
| Android | Implementation and tests updated. Gradle stops at configuration because Android SDK location is missing; no Kotlin compilation, tests, native render or device acceptance. |
| iOS/macOS | Implementation and XCTest updated; no Swift compiler or Xcode in this orb, so no typecheck, tests, native render or device acceptance. |
| Containers/services | Docker daemon unavailable; web production build and Rust compilation validated directly. No deployment, shared SQL, live R2 or SFU changes. Initial overlapping builds exhausted orb memory; bounded reruns passed. |

**Deployment order.**

1. No infrastructure apply, secrets/configuration, database migration, CDN Worker,
   gateway or Valkey change is needed. Retain existing R2 permissions and the
   established API replica count. Merging does not deploy services. After merge,
   wait for immutable API/web images for the merged revision and set `MERGED_SHA`
   to that full revision.
2. Deploy the API first so clients can cancel reservations:

   ```bash
   gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # After the deployment workflow for this SHA succeeds:
   kubectl -n default rollout status deployment/caper-api --timeout=15m
   ```

   Older clients still use abandonment expiry. The new reconciliation sweep also
   revisits previously deleted, unsent rows; monitor `asset_delete_retry`,
   `asset_purge_retry`, `asset_sweep_retry` and R2 request volume as history grows.
3. After API readiness, web and native clients can release independently:

   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # After the deployment workflow for this SHA succeeds:
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```

   Complete Android/Apple builds and physical-device acceptance before native
   distribution, then use the existing signed release pipeline:

   ```bash
   gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$MERGED_SHA"
   ```

4. Verify a removed, unsent file and its preview leave R2 and stop counting toward
   usage after cleanup (allow for its 25-row batches/retries). Sent files must
   remain accessible. Leave a draft open for over 24 hours: focus/Send must
   silently re-upload and send without expiry UI. Interrupt a renewal, verify
   only Failed to upload/Retry appears, then restore connectivity and Retry.
   Text, bytes and newer drafts must survive; unknown send retries must retain
   their original IDs and never cancel accepted attachments. These live/device
   checks remain outstanding.
5. Roll back API/web to their prior known-good image SHAs using the same deploy
   commands; there is no new schema to undo. New clients tolerate a missing cancel
   route and fall back to expiry. An API rollback loses reconciliation and the
   typed refusal needed for silent Send recovery; it cannot restore already-purged
   unsent bytes. Native rollback requires a corrected higher-build-number release.

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
bucket as `MEDIA`, runs on `cdn.caper.chat` / `cdn.staging.caper.chat`, and
holds `ASSET_CDN_SIGNING_SECRET` as a Worker secret:

```sh
cd apps/cdn
npx wrangler secret put ASSET_CDN_SIGNING_SECRET --env staging
npx wrangler deploy --env staging
```

### Platform status

| Platform | Send files | Show files |
| --- | --- | --- |
| Web (desktop and mobile layouts) | Yes: picker, clipboard files/images, and drag and drop with an overlay | Images, video, audio, file cards; images and videos open in the in-app viewer |
| Apple (iOS and macOS) | Yes: Photos picker, file importer, clipboard files/images, drag and drop with an overlay on macOS and iPad/iPhone | Images, video (AVKit), audio, file cards, "File removed"; images and videos open in the in-app viewer |
| Android | Yes: system photo picker and document picker, clipboard content URIs, drag and drop with an overlay from other apps, up to 10 files, draft chips with final upload size and progress. Applies the server `compression` settings on device per the shared rules and encoder table above (Media3 H.264/AAC transcode with `videoMaxHeight` bounding the short edge and original fallback; previews and video posters). | Images (preview; tap opens the in-app viewer), in-app video/audio playback (Media3), file cards, "File removed". Refreshes signed URLs before expiry and once after a 403/404. |
| Rust desktop (Windows, Linux) | Yes: file dialog (Win32; XDG desktop portal on Linux), clipboard files/images, and drag and drop with an overlay, up to 10. Stills per the shared rules and encoder table above (JPEG only if WebP fails; HEIC uploads unchanged). Videos through the bundled FFmpeg by the shared rules (below), else unchanged with metadata stripped; posters for previews | Inline images and video posters (decoded off the UI thread, cached by attachment id). Images and videos open in the in-app viewer with an in-app player; audio and files open in the system browser or player. "File removed" cards. URLs refreshed before expiry and once after a 403/404 load |

#### File drop overlay

Dragging files over a channel or DM conversation that accepts uploads covers it
with a drop overlay on every client: today's Caper character (the wordmark's
daily branding art) hops in front of three neutral file cards (image, document,
video Lucide glyphs) that fan out behind it, on a surface card with a dashed
terracotta outline. Its title is `Attach to #channel` (`Attach to Name` in a DM)
and its detail counts the remaining slots (`Up to 10 files…`, `7 more files…`,
`10 files attached. Remove one to add another.`). Every client's tests run
`shared/messages/drop-cases.json`. Dropping adds the files as drafts, exactly
like the attach button; nothing uploads until the person sends. Only drags
carrying files open it; text and link drags are ignored. The overlay never takes
input. Motion is shared: a 140 ms fade, the card rising 10 px, the cards fanning
from the middle with a 420 ms overshoot, the character rising, then a 1.6 s hop
and a 2.4 s card sway. Reduced motion (web `prefers-reduced-motion`, Apple
Reduce Motion, Android "Remove animations") shows the resting pose.

- Web: `DropOverlay.tsx` over `.chat-panel` or `.chat-thread-panel`. A file
  dropped elsewhere in the app is refused instead of the browser navigating
  to it. Thread drops attach to that thread, not the channel composer.
- Rust desktop: `file_drop.rs`, painted from egui's hovered files over the
  conversation. `--fixture parity-file-drop` (and `-narrow`) hold it open for
  rendering checks.
- Apple: `FileDropOverlay` on the conversation's `dropDestination`, now on iOS
  as well as macOS, taking photos/videos/audio/PDFs as media and other files as
  file URLs. The macOS composer (`NSTextView`) accepts only text drags, so files
  dropped on it attach instead of typing their paths.
- Android: Compose `dragAndDropTarget` on the conversation for drags from other
  apps (split screen, freeform/desktop windows, ChromeOS), with
  `requestDragAndDropPermissions` before copying. Android names a drag only by
  MIME type until the drop, so plain-text files are attached with the attach
  button rather than by dragging.

Validation: web unit tests plus Chromium with synthetic `DragEvent`s and a
mocked `/api/assets/usage` at 1280px and 390px (overlay, counts, child
enter/leave, text drags ignored, drop adds a draft, outside drops refused); not
a real OS drag. Rust desktop unit tests (shared text, timing, overlay shown
only while files hover a writable conversation) and the fixture rendered under
Xvfb at 1440×900 and 390×844; no real Windows/X11 drag. Android compiles with
the pinned toolchain and its JVM tests cover the shared text, MIME filter and
timing; the overlay was not rendered on a device or emulator. Swift was written
without a Swift toolchain and awaits macOS CI (`FileDropTests` covers the shared
text). Real Finder/Files/Photos drags, iPad and Android split-screen drops,
Windows/X11 drags and physical devices remain to be checked.

Composer attachment tiles are clickable before sending, including while
preparing/uploading or after an upload fails. Web images and videos use the
in-app viewer with local object URLs; audio, PDFs and plain text open in the
browser, and unsupported/active document formats download instead. Apple uses
Quick Look for the staged local file. Android grants the selected content URI
to a system preview app; Rust desktop opens the selected file in the system
browser. Native format support depends on the installed system handlers.
Remove remains a separate control and never opens a preview.

Sent images and videos form a count-aware gallery on web, Apple, Android and
Rust desktop: two side by side, three with a large left tile, four in a 2×2
grid, and larger sets in rows of two or three. Galleries cap at 480 logical
pixels, with 4px gaps, 12px outer corners and 1px interior corners. Thumbnails
crop to fill; opening the viewer still shows the full media. A single image
keeps its natural proportions. Processing, failed and removed media retain
their slots. Audio and document cards follow the gallery, side by side where
space permits; order within each group is preserved. The unsent composer tray
is unchanged.

Gallery validation (October 18, 2026): local-fixture Chromium checks cover
counts 2–10 at 390, 760 and 1280px, plus mixed media and processing/error states;
DOM checks verify row patterns, corner ownership, filled cells and no overlaps.
Rust desktop has a headless egui layout/click regression and a rendered Linux
attachment fixture. Android geometry JVM tests pass, but the Compose build is
blocked by the missing Android SDK in this orb. Apple source and geometry tests
await Xcode; iOS/macOS/Android rendering and physical-device checks remain
unverified. These local fixtures do not validate live uploads or deployment.

Draft tiles show video posters with a small translucent play indicator centred
in the preview area, clear of any progress or retry footer. The whole tile still
opens the preview; the indicator adds no separate hit target. Web and Apple
decode a bounded early frame locally; Android reuses its prepared poster;
Rust desktop uses FFmpeg, including for profile-card files not yet uploaded.
Undecodable videos retain the play
indicator. The visible X sits near the top-right corner without shrinking the
44-point Apple/web touch target or Android's 48dp target. Long press (or desktop
context click) offers **Remove attachment**, affecting only the unsent draft,
not the original library file or a sent message. Sending profile-card files
retain their removal lock. Source byte counts and upload behavior are unchanged.

Validation for these draft controls: Chromium desktop and touch-emulated narrow
layouts with the local API fixture cover posters, long press, tap preview,
movement cancellation, removal, retry and unsupported video. Rust desktop was
built and rendered with its labelled attachment fixture; its removal regression
and a real FFmpeg-backed staged-video poster test pass. Apple and Android builds
and physical-device rendering still need verification; browser captures are not
evidence of native iOS/Android behavior.

Ready draft chips show only the final upload size on Android, iOS and the web's
mobile/touch layout (up to 760px or a coarse pointer). Desktop web, macOS and Rust
desktop retain the original-to-final comparison when compression saved space.
Preparing/compressing, upload progress and error labels are unchanged. Source
byte counts and compression/upload behavior are unchanged; this is presentation
only. Native Android/iOS builds and device rendering remain release checks.

The channel/DM attachment control sits inside the left edge of the message box
on web, Android, iOS/macOS and Rust desktop, sharing its background and outline.
Existing pickers, the ten-file limit and upload availability rules are unchanged.
Web and native binaries can roll out independently; no API, gateway, database,
infrastructure or secret changes are required. Android/Apple native builds and
rendering, Windows rendering and physical-device checks remain release checks;
browser fixtures do not validate those platforms.

Apple notes. Attachments decode tolerantly (a malformed entry is skipped).
Signed URLs are refreshed through `POST /api/assets/urls` when `exp` is past or
within an hour, or after a 403/404, at most once per stale URL; decoded images
are cached by attachment id. Stills follow the shared rules and encoder table
above (exact indexed PNG via a Swift encoder over the Compression framework's
DEFLATE, libwebp lossless, AVIF through the bundled libavif, else WebP/JPEG at
`imageQuality`), scaled to `imageMaxEdge`; HEIC is never uploaded when
conversion succeeds. Videos are re-encoded with `AVAssetReader` and
`AVAssetWriter` (H.264/AAC MP4) at the shared output size and bitrate, with
`audioBitrateKbps` for AAC; the original uploads on failure. Export presets
were dropped because they cannot take a bitrate and the HDR video composition
overrode their size: an iPhone 1440×1920 HEVC clip was stored unscaled at
about twice the original's bitrate. Previews and poster frames use
`previewEdge`. Taps open images and videos in the in-app viewer (below) and
files at their signed URL in the system browser. Audio plays through `AVPlayer` with the app's existing audio
session. Unit tests cover decoding, URL refresh decisions, compression
decisions, the indexed-PNG round trip through ImageIO, the upload request
sequence and a file-only send against a stubbed `URLProtocol`. They were
written without a Swift toolchain and await macOS CI; Photos/file pickers,
drag and drop, AVKit playback, export presets on real media, live R2 uploads
and physical devices are not yet validated.

### Validation

- Rust unit tests: SigV4 presigning matches AWS's published S3 example, signed
  upload headers, day-snapped delivery URLs, a signature vector shared with the
  Worker tests, configuration rules, sniffing and name/type normalization.
- Postgres integration test (fake S3 server): quota with previews, access
  denial, exact-size and magic-byte verification, ordered single-use linking,
  history signing, URL refresh visibility, purge timing and quota release.
- Worker tests (`npm run test:cdn`): signature, expiry, tampering, inline versus
  download headers, ranges, HEAD.
- Web tests: tolerant attachment validation, upload request sequence, storage
  full errors, file-only sends.
- Chromium against a local API, gateway, Postgres 16, Valkey, a fake S3 server
  and the real Worker code: a 1.6 MB PNG uploaded as a 143 KB WebP, a second
  member received it live through the gateway, history reload and a 390 px
  layout rendered, a tampered signature returned 404.
- Google Chrome (stable, Linux) against the same stack: a flat 2560×1440 PNG
  uploaded as a 9.5 KB lossless indexed PNG (from 104 KB), and a 4 s 2560×1440
  VP9/Opus WebM was transcoded in the browser to a 1920×1080 H.264 + Opus MP4
  (2.3 MB → 805 KB) that played back with its poster. Rust and web tests cover
  the settings, the flood limits and an indexed-PNG decode round-trip.
- Not yet validated: live R2 (signature acceptance, signed `content-length`
  and CORS), the deployed Worker and its edge cache, Safari/Firefox encoders
  (including Opus-in-MP4 playback on Safari), and physical phones. Desktop has
  not uploaded to a live API/R2/CDN; its Windows file dialog and drag and drop
  and the Linux portal dialog have not been exercised interactively.
- Rust desktop unit tests: tolerant attachment parsing, URL expiry and
  refresh batching with one retry after 403/404, compression decisions, a
  pixel-exact palette PNG round trip, MP4 header probing, and the upload
  request sequence against a loopback fake server (exact storage headers and
  length, no credentials on storage `PUT`s, `attachmentIds` only when present,
  storage-full errors). The labelled `--fixture parity-attachments` preview
  was rendered under Xvfb (software Vulkan) at 1440×900 and 900×700.
- Android: JVM unit tests cover tolerant attachment parsing, URL expiry and
  refresh decisions, the compression decisions (palette PNG versus lossy versus
  keep, preview sizing, video target height), a lossless indexed-PNG round trip
  through the JDK decoder, and the reserve, presigned `PUT`, complete and send
  sequence against MockWebServer (exact headers, no credentials sent to
  storage). Not yet validated on Android: the Compose attachment UI and pickers
  on an emulator or device, Bitmap/WebP encoding and EXIF handling, Media3
  Transformer transcoding and playback, and uploads to live R2/CDN.
## Message reactions

Desktop web exposes **Add reaction** on message hover or keyboard focus; clicking
a message or reaction does not keep the toolbar visible after the pointer leaves.
Reaction chips have no outer focus ring; keyboard navigation uses an inset cue.
Mobile web, Android and iPhone hide the per-message add button. Press and hold a
committed message to open a bottom drawer with five fixed quick reactions
(👍 ❤️ 😂 🎉 👀), the searchable emoji picker, **Copy text**, and
**Copy message ID**. Copy uses the exact text or globally unique public message
`id`, never the client retry UUID or channel-local sequence. Read-only previews
allow copying but not reaction mutations. Message links and unsupported actions
(bookmarks, reminders, delete) are not exposed. Threads, pins and live forwarding are
described below.
Web offers a keyboard/screen-reader message-actions button; iPhone provides a
VoiceOver action and Android a labelled long-click action. Desktop Apple/Rust
clients intentionally retain their existing reaction controls rather than
adopting the mobile drawer.

Reactions appear below the message with counts; your own
chips have a terracotta tint and `aria-pressed=true`. Clicking a chip adds or
removes your contribution, as does a quick reaction. Choosing an emoji in the
picker only adds it. Pending messages cannot be reacted to or have their
unconfirmed IDs copied. Web (desktop/mobile), Android, iOS/macOS, and Rust desktop
apply reaction additions/removals immediately, without a saving label, spinner,
or disabled controls. Requests serialize per message while the newest local
choice stays visible over incoming server snapshots. A failed current choice
rolls back only your contribution to the latest server state and shows an explicit
retry of that desired state. Superseded failures do not undo a newer choice.
Cached history and replay cursors contain authoritative state, not optimistic
counts; a full resync reconciles abandoned in-flight choices from server history.

Android, iOS/macOS, and Rust desktop also provide searchable standard-emoji
pickers, counted chips, own-contribution highlighting, add/remove, and save-error
retry. Clients preserve per-message reaction revisions independently of the
channel replay cursor.

Web, macOS and Rust desktop group the message hover actions in one neutral,
rounded, bordered bar. Controls retain their existing compact targets, individual
hover/focus feedback and permissions. Web and Rust keep the whole bar visible
while its menu or picker is open; thread rows omit Reply in thread. Narrow web,
Android and iPhone retain their existing message-actions sheets instead of the
desktop bar.

Thread actions use a thread-spool glyph, and Reply uses a longer curved arrow.
`shared/message-icons.ts` supplies both to web and the native SVG exporter;
the icon parity test also checks Android's vectors against the same glyphs.
Action labels, targets and permissions are unchanged.

Hover-bar validation: web check/build/typechecks and 639 web/shared tests pass;
Rust desktop formatting, application Clippy, Linux build and 536 tests pass
(12 existing device/audio tests ignored). Inspected labelled Chromium desktop,
thread, menu/picker, keyboard-focus and 390px layouts, and 2× Linux wide/narrow
renders. Real CDP touch holds and the reaction suite pass with touch emulation
re-enabled before each input (this orb's browser commands otherwise reset it).
The broader Threads and Pins scripts stop at quote-connector and invite-consent
assertions respectively; both reproduce on unchanged main. No local macOS/Xcode,
Windows or physical-device validation is claimed.

On macOS, the reaction picker is a 352 × 420-point native popover anchored to
the fixed conversation header, not the scrolling timeline. Its Cancel button
lives in the popover content; Escape and outside clicks also dismiss it. The
Apple parity suite checks catalog/empty-result bounds, accessible Cancel,
isolated focus styling, and preserving search and the original message target
during live scrolling. iPhone retains its message-actions sheet and navigation
toolbar. These are fixture-backed UI checks, not physical-device or live SFU
validation. No server, schema, infrastructure, or configuration change is needed;
macOS users receive the fix only after an updated native binary is released.

`node scripts/test-message-actions.mjs` uses the disposable
`native-parity-fixture.mjs` server and Vite, with Chromium touch input and verified
coarse-pointer emulation. It covers hold versus tap/scroll/cancel, clipboard
content/ID, quick toggles, search, live-scroll target retention, focus/dismissal,
clipboard errors, read-only previews and the desktop picker. It also holds
reaction requests before fixture delivery to verify immediate selection/removal,
enabled controls, no saving status, rapid-toggle ordering, failure rollback and
retry. This is browser fixture evidence, not native or physical-device
acceptance. The native reaction
UI suites cover the new drawer but require Android instrumentation and Xcode;
run them and inspect iPhone/Android drawers before releasing those clients.
No API, gateway, migration, infrastructure or secret change is needed for this
client-only update. Web, Android, Apple and Rust desktop can release independently;
each needs an updated client to get optimistic reactions. Roll back web by restoring
the previous image. For mobile, release a revert build with a higher build/version
number instead of attempting an installed-app downgrade. No database rollback
is needed.

Optimistic reaction validation: web build and 336 unit tests passed; Chromium
desktop and touch-emulated narrow layouts exercised held requests and failures,
and captures were inspected. Rust desktop passed 189 tests (9 ignored), fmt,
Clippy and a Linux build; its rendered fixture showed immediate removal and
rollback after a delayed mock 503. Android and shared iOS/macOS tests were added
but not executed in the Linux orb (no Java/Android SDK or Swift/Xcode). Run their
unit/UI suites before release. Linux/browser fixtures do not establish Windows,
native phone, macOS, multi-client production or physical-device acceptance.

The picker offers standard Emoji 15.0 artwork, self-hosted from `@twemoji/svg`
and attributed at `/emoji/NOTICE.txt` (Twemoji graphics, CC BY 4.0). The web
build/dev preparation verifies artwork for every offered emoji. It makes no
emoji-CDN requests. The picker code/data is part of the chat bundle, so opening
either a composer or reaction picker needs no additional code-loading panel.
Signed-in writable web chat preloads/decodes the first 128 Emoji 15.0 smileys/people images when
chat mounts, without waiting for a sending session. This covers the opening grid,
not the full catalog, and does not mount or open the picker. Hovering or focusing
**Add reaction** reuses that preload, or retries failed image preparation.
Hovering or keyboard-focusing a category tab preloads/decodes its first 128
Emoji 15.0 images (or the full category when smaller), without selecting it or
scrolling the grid. Category manifests are also generated at build time.
Preloads are shared per category across message buttons and picker reopenings;
failures evict only that category so the next intent can retry. Images beyond
the prefetched window still load on demand; immediate clicks on a cold network
can still precede completion.
The web picker uses the existing React Virtuoso grid, not the emoji library's
separate category IntersectionObserver gate, which could leave a visible category
heading with no emoji buttons. Category selection renders only that category and
resets scrolling; selecting a category clears search. Search covers all eight
categories, regardless of the selected category. The grid loads its mounted image
window eagerly, including after category changes. Versioned
`/emoji/twemoji-15/**` artwork is served with a one-year immutable browser cache;
changed artwork must use a new path. Web category tabs sit above search and use
the same colored Twemoji artwork as native, with a terracotta underline for the
selected category. Pointer clicks have no circular ring; keyboard focus retains
a separate outline.
`node scripts/test-message-actions.mjs` checks startup code/opening-grid preparation
without opening, warm openings/reopenings without loading-panel flashes,
hover/focus reuse without opening or duplicate image requests,
all seven non-opening category preloads without selection/scrolling, cache reuse
across reopening, populated category jumps, eager loading with virtualization, selected
tab styling, and keyboard navigation on desktop and Chromium touch-emulated web.
Android, iOS/macOS and Rust desktop also have eight category controls above search,
category browsing and global search with the same reset behavior. Their independent
implementations use bundled sprite sheets, not browser image loading. Rust desktop
warms the opening category's sheets at startup and category windows on hover/focus;
its fixed 40px cells fill the picker width and match the virtual scroller's row
height, rather than leaving unused width or clipping rows from a mismatched height.
Picker validation: web check/build and 604 tests passed; Chromium desktop and
touch-emulated narrow layouts exercised all eight categories, global search,
preloading/cache reuse, scrolling and reaction persistence. Composer insertion
checks passed. Profile-composer emoji/caret and settled drawer bounds passed,
but its later incoming-request navigation check timed out. Rust desktop passed
484 tests (12 ignored), Clippy and a Linux build; default, Activities, global
search and empty-result captures were inspected. Windows, Android and Apple
rendering remain unverified: no Windows device, Android SDK or Xcode is available
in this Linux orb. Run native build/UI checks and inspect those clients before
releasing them; browser and Linux fixtures do not prove native device acceptance.
These are images, not the operating system's emoji font, so supported reactions
look consistent across all clients. Native apps bundle raster sprite sheets from the same
artwork with `node scripts/native-emoji.mjs` (requires npm dependencies and
ImageMagick 7). Android and Rust read `shared/emoji`; SwiftPM requires a generated
copy inside its target, checked byte-for-byte by CI, including licenses.
Picker names use dashes (for example, `grinning-face`). Web and all native
catalogs also include underscore and spaced search aliases (`grinning_face` and
`grinning face`). The picker package orders its names by length, so its last name
can be slang or a category (`litaf` for 🔥, `looking` for 👀, `animals` for 🐕).
`apps/web/src/chat/emoji-labels.json` supplies the Unicode CLDR short name for those
entries; the package's names stay searchable. Regenerate it with
`node scripts/emoji-labels.mjs` (fetches the Emoji 15.0 `emoji-test.txt`), then
`node scripts/native-emoji.mjs --catalog-only`. Composer suggestions rank exact,
prefix, keyword and substring matches, preferring shorter names within a rank. Names are labels/search terms, not stored reaction identifiers:
selecting any spelling still sends the same Unicode emoji. Existing reactions
need no migration. The browser fixture checks all three search spellings,
dash-separated accessible names/image alt text, and Unicode persistence.
Message-body emoji are unchanged. Custom uploads, frequently used/top-five lists,
and a skin-tone selector are deferred. Message text does not currently convert
`:name:` shortcodes into emoji. Dashes are not a restriction on future custom
emoji, but uploads alone will not enable them. Custom reactions need an
authorized emoji catalog, stable asset
IDs, API/protocol support alongside Unicode, and image rendering in every client;
the current API deliberately accepts only standard Unicode emoji.

`PUT /api/chat/channels/{channel}/messages/{message}/reactions` accepts
`{emoji,active}` and `X-Caper-Chat-Token`. It checks the same channel/member/private
grant, channel participation (`channel_joins`), and session permissions as sending.
Readable previews retain reaction snapshots but cannot add, remove, or retry
reactions. Even no-op writes require participation, and a write queued behind
leave rechecks permission after acquiring the space lock. Direct-message reactions
require one of the two active participants, not space membership or a channel
join. History holds the channel lock through snapshot reads for both channel
types. Unicode qualification variants are
canonicalized; arbitrary text, multiple emoji, and emoji newer than 15.0 are
rejected. Active membership is unique by internal message ID, canonical emoji, and
internal user ID; separate account chat sessions therefore cannot inflate counts.
Removing a reaction sets `deleted_at` and keeps the row; adding it again inserts a
new row. Reactions no longer cascade from messages.
Both reaction membership and rate-limit activity use `user_id` foreign keys to
`users.id`. Snapshot/event generation joins those keys to `users.external_id`;
only public IDs appear in `authorIds`, including the stored history/outbox payloads.
An account is required; the retired public General demo is not writable.

The response and transactional outbox use
`{type:"message.reactions",schemaVersion:1,channelId,seq,messageId,reactions}`.
Each reaction is `{emoji,authorIds}`; IDs are visible to the channel's readers,
as message-author IDs already are. `seq` participates in the existing ordered
channel stream. History contains the current `reactions` and `reactionSeq`
snapshot; original message `seq`, timestamp, and content do not change. A no-op
returns the current snapshot revision (or `"0"`) without allocating an event.
HTTP snapshots never advance a client's replay cursor. Per-message revisions
prevent stale replies or older pages from overwriting newer reactions. A full
history resync retains older cached pages only when the refreshed messages account
for every sequence since the applied replay cursor (or the head is unchanged).
Otherwise, a missed event may be a reaction on an older row, so cached pages are
discarded and can be loaded again. Empty refreshed history also clears cached
pages. Normal reconnects replay missing events, including
during native gateway handoffs. A visible direct conversation marks sequenced
reaction events read; HTTP snapshots do not advance that read cursor. Reactions
do not generate push notifications.

Limits are 50 emoji kinds and 1,000 total contributions per message, plus 60
mutations per actor/channel/minute. No-op retries do not consume that budget.
Expired rate records are removed when that actor next mutates. Reactions,
message snapshots, sequence allocation, and outbox commit together.

### Who reacted

Snapshots and events carry only public reactor IDs. Names load on demand from
`GET /api/chat/channels/{channel}/messages/{message}/reactions`, authorized like
history (account cookie or bearer; read-only previews and DMs included). The
response lists each emoji in snapshot order, with its active reactors in reaction
order (`id`, `username`, `displayName`, `avatarId`) and the message's
`reactionSeq`; clients cache per message and refetch when `reactionSeq` changes.
Removed (soft-deleted) reactions are not listed.

Every client uses the same wording: "You, Alex, Maya and 2 others reacted with
:thumbs-up:" (you first, up to three names, then a count; the dash-separated
catalog name, or the glyph when none exists). Until names load, a count is shown
from the snapshot. Pointer platforms (web, macOS, Rust desktop) show this in a
tooltip with a large emoji on hover or keyboard focus. Touch platforms (iOS,
Android, web on touch) open a Reactions sheet on press-and-hold: one tab per
emoji with its count, then each person's avatar, name and @username. On web,
right-click opens the same panel as a popover, and message actions include
**View reactions**. A tap or click still toggles your own reaction.

Web and Rust desktop retain known names during revision refreshes (including
failed refreshes), project them onto the currently displayed reactor IDs, and
name your optimistic contribution as "You" immediately. They do not temporarily
replace a named tooltip with a count when you toggle an existing reaction.
Removed IDs are excluded immediately; a genuinely unknown person still uses the
snapshot-count fallback until their name loads. Apple already reconciles names
this way; Android uses a separate hold-to-open sheet rather than hover tooltips.
`scripts/test-chat-reactors.mjs` checks every tooltip DOM update while fixture
writes/name refreshes are held, rapid toggles, refresh failure, pointer-only
toolbar visibility, and keyboard focus. Run its desktop checks with a fine-pointer
Chromium configuration (`AGENT_BROWSER_EXECUTABLE_PATH` in headless orbs).

Interaction-fix validation: `npm run check` and all 450 web/native-support tests
pass; both browser interaction scripts pass, including verified coarse-pointer
touch input. Rust desktop passes 275 tests (9 existing ignored), package Clippy,
fmt and a Linux build; its fixture tooltip was rendered under 2× Xvfb and inspected.
These are local fixture checks, not physical-phone, Windows/macOS or live-service
acceptance. No Docker daemon was available; web build stages ran directly.

The parity fixture serves this endpoint, and its `incomingReaction` control
accepts an optional `userId` so tests can react as any fixture account. Desktop
also has `--fixture parity-reactions`, and the Apple parity fixture builds its
lists in the app.

| Platform | Who-reacted validation |
| --- | --- |
| API | Disposable-Postgres test: reaction order, names/avatars, preview readers allowed, removed reactions hidden, outsiders/guests/other channels 404 |
| Web | Unit tests for wording, parsing and emoji names; Chromium with the fixture at 1280px (hover tooltip, right-click popover, tabs, click toggle) and 390px touch emulation (hold sheet without toggling, tap toggles, **View reactions**); screenshots inspected |
| Rust desktop | 202 tests, Clippy and fmt; Linux build rendered under Xvfb at 1440px and 420px with the fixture. Tooltip delay is egui's app-wide 0.5 s; keyboard-focus display not checked |
| Android | 131 JVM tests, lint and androidTest compilation; the new emulator UI test and on-screen sheet are unverified until native CI runs (on `main`, or a dispatched branch build — pull requests get no CI) |
| iOS/macOS | Swift parsed and core logic run on Linux only; SwiftUI views are first compiled, and UI tests first run, by native CI (on `main`, or a dispatched branch build). macOS shows the card on hover, not on keyboard focus |

None of this is physical-device, TalkBack/VoiceOver or live-server acceptance.

### Deployment order for reactions

1. **Prerequisites/infrastructure/secrets:** no infrastructure apply, new service,
   feature flag, or secret is required. Keep the existing PostgreSQL/Valkey/chat
   configuration. Keep one desired API replica and use a maintenance window for
   consent/participation mutations rather than mixing old and new API versions.
   Obtain the immutable merged reaction revision as `MERGED_SHA`; wait for
   that revision's API/web images and native build artifacts before deployment.
2. **Database/API first:**
   `gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"`.
   Watch the exact workflow run, then
   `kubectl -n default rollout status deployment/caper-api --timeout=15m`.
   API startup uses the existing direct `MIGRATION_DATABASE_URL` to apply all
   pending migrations, including `202609290001_message_reactions.sql`, space
   invitations, channel joining, `202610030001_direct_messages.sql`, and
   `202610030002_push.sql`, plus runtime grants before serving. Optional push
   delivery can stay disabled; no push provider setup is required for reactions.
   Already-applied migrations are skipped. No manual writes or separate job are needed.
   Verify `/readyz` and that existing history/sends still work.
3. **Gateway next:** deploy the cumulative gateway so typing/media commands also
   enforce channel participation:
   `gh workflow run deploy-caper-gateway.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"`.
   Watch the exact run, then
   `kubectl -n default rollout status deployment/caper-chat-gateway --timeout=15m`.
   No Valkey/SFU reset is needed. Do not change replicas or close healthy tracks.
4. **Native clients:** after API and gateway readiness, release Android, Apple, and Rust
   desktop clients from the same revision:
   `gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$MERGED_SHA"`.
   Watch that exact run and have native testers update before exposing web
   reactions. Older clients may fail to advance across reaction events; they
   must update. Check add/remove, search, own/other chips, persistence, errors,
   reconnect, and account/channel switching on each platform. Native builds
   against the fixture do not prove physical-device or production behavior.
5. **Web last:**
   `gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"`,
   watch that exact run, then
   `kubectl -n default rollout status deployment/caper-web --timeout=15m`.
   Reload pre-release browser tabs. Verify two users adding/removing the same
   emoji, one user's highlight, counts after refresh and reconnect, and denied
   access to private messages. Public previews must show reactions but reject
   writes until joined; leaving must disable writes. Check desktop hover/keyboard
   and narrow layouts, two-person DM reaction updates/read cursors, and outsider
   denial. Include invitation consent and channel joining checks below.
6. **Rollback:** roll back the web first to reduce reaction writes. Updated
   native apps also expose writes; retain the compatible API while preparing
   corrective client releases. Do not restore old clients that cannot consume
   reaction events. The migration is additive: do not drop reaction
   data or rewrite stored events. Do not roll API/gateway back across the consent
   or participation boundary. Prefer a forward fix. Merging alone deploys
   none of these components; the commands above require operator authorization.

Channel-participation integration validation (October 3, 2026): the disposable
Postgres/Valkey suite passes cross-channel add/remove/no-op denial, join/leave,
and a mutation blocked behind leave. Denial preserves reaction membership,
activity, message snapshot, channel sequence, and outbox. Root Rust tests and
strict Clippy pass. Mocked Chromium desktop/390px checks cover visible preview
counts, disabled mutations, picker closure and disabled retry after participation
revocation, plus normal own/other chips and picker rendering. Screenshots were
inspected; browser mocks are not live transport or native-device evidence.
Native implementations also gate mutations on participation. Rust desktop's
retained-session preview regression passes. Android JVM tests, instrumentation
compilation and lint run locally; instrumentation execution and Apple builds/tests
require platform CI. Apple bundles both emoji and invitation artwork. No
production migrations or deployments have been performed.

Historical validation before channel-participation integration: `npm run check`
and all 292 web tests passed. Rust
formatting, Clippy with warnings denied, and workspace tests passed (110 passed,
24 ignored); the separately executed disposable Postgres/Valkey chat suite passed
all 7 tests, including persistence, duplicate/concurrent adds, removals, account
identity, authorization/revocation, outbox rollback, mutation limits, and both
reaction-cap boundaries. Rust desktop tests passed after integrating the native
reaction UI (124 passed, 9 ignored). `cargo build --locked --release --package
caper-api` passed. Docker had no daemon, so API/web build stages were validated
directly rather than building container images.
Chromium desktop and 390px narrow-layout checks used the real local API, Postgres,
and gateway: two-viewer updates, refresh persistence, chip ownership/removal,
search/empty state, and Escape/focus return passed. A deliberately simulated HTTP
503 verified the visible save error and successful retry after restoring the API.
Screenshots were inspected; the narrow capture is browser layout coverage, not
a physical phone or touch/Safari test. Those browser checks predate the subsequent
merge retiring public General. Android's 83 JVM tests and instrumentation-test
compilation passed locally with JDK 17 and Android SDK 36. Apple compilation and
all simulator/device UI checks run in GitHub CI, not this Linux orb. Check the current PR's
CI results and native screenshots before release. Physical devices, native live
cross-client reactions, and production rollout checks remain required. No
deployment or production database write was performed.

### Invitation rollout and validation (September 30, 2026)

The invitation migration is additive and preserves existing memberships. The
disposable-Postgres spaces test covers privacy, concurrent duplicate invitations
and acceptances, membership quotas, expiry, cooldown, and atomic rate/cap limits:

```bash
CHAT_TEST_DATABASE_URL=<disposable-loopback-postgres-admin-url> \
cargo test -p caper-api spaces::tests::authorization_self_leave_soft_deletion_and_quota_races -- --ignored
# With the web dev server running; API calls are explicitly mocked:
SPACES_TEST_WEB_URL=http://localhost:5174/spaces \
node scripts/test-space-invitations.mjs
```

| Platform/check | Invitation validation |
| --- | --- |
| API | Real disposable Postgres 15 lifecycle/privacy/concurrency and current-owner inviter metadata test passed; default Rust tests and clippy passed |
| Browser | Build/287 unit tests passed; Chromium at 1280px and 390px exercised normalization, missing/duplicate/member/429 errors, cancel, inviter/expiry, long names, legacy metadata, consent retry/decline/accept and focus-triggered revocation. DOM confirms no private requests before acceptance; screenshots inspected |
| Rust desktop (Linux; shared source with Windows) | Native unit tests and Linux build; consent/pending-owner desktop and narrow fixtures rendered and inspected separately from browser. Not Windows execution or live SFU evidence |
| Apple iOS/macOS | Consent, pending/cancel, validation and 404 cleanup implemented; Swift/Xcode tests and device rendering unavailable in this Linux orb |
| Android | Consent, pending/cancel, validation and 404 cleanup implemented. Initial PR CI found a MemberManager callback compile error; fixed with explicit callback arguments in the follow-up. JDK/Android SDK tests and physical-device rendering unavailable in this orb; platform CI must verify the fix |
| Docker/production/live media | No Docker daemon available; image build stages validated directly. No shared database migration, deployment, live SFU or physical-device test performed |

Use the combined **Deployment order** under
[Channel participation and private consent](#channel-participation-and-private-consent)
below for space invitations and channel joining. Merge space-invitation consent
first, but do not deploy it separately: the cumulative API applies both pending
migrations before the gateway and clients roll out. Merging does not deploy.

### Channel participation and private consent

Channel access and participation are separate. `channel_members` remains the
private authorization grant; `channel_joins` records the conversations a member
has chosen. The sidebar shows joined channels. Browse channels searches the
accessible directory and opens read-only previews without joining, preparing
voice, or requesting a microphone. Public history and live text subscriptions
remain readable before joining; sending, typing and every channel media
operation require participation. Joining a channel never enters its voice call.
The current quota remains 100 channels per space; this does not raise it to
thousands or introduce server-side directory pagination.

`Channel.joined` is returned by space details and channel mutations. New clients
default a missing field to true for compatibility with older APIs. Public
join/leave uses POST/DELETE
`/api/spaces/{space}/channels/{channel}/membership`. Public leave removes the
sidebar entry but preserves preview/rejoin access. Private non-owner leave also
removes the authorization grant and warns that another invitation is required;
both leaves set `deleted_at` rather than deleting rows.
the owner keeps implicit administration and can rejoin from the directory.
Leaving a channel stops its local call, not an unrelated call. SFU cleanup after
authorization ends is asynchronous, as above.

Owner POST `/api/spaces/{space}/channels/{channel}/members` now sends a private
invitation, not a grant. GET returns active `members` and separate pending
`invitations`; DELETE `/members/{user}` cancels a pending invite or removes a
grant and participation. The recipient's space detail adds `channelInvitations`
containing only channel metadata and the current owner's public `inviter`
identity. The pending channel is absent from the accessible directory, and its
history/media stay unauthorized. POST/DELETE `/invitation` accepts/declines;
acceptance grants and joins in one transaction. A stale, revoked, declined or
expired invitation cannot be accepted. Unauthorized resources retain generic
404 responses.

Channel invitations expire after seven days and retain a 24-hour pair cooldown
after decline, cancellation or removal. They share the space-invitation attempt
budget (20 per owner per ten minutes), with separate live-pending limits of 50
channel invitations per recipient and 100 per channel. Invitees must already
belong to the space. These are fixed policies, not new secrets/configuration.

Migration `202610010001_channel_joining.sql` joins existing members to all their
currently accessible non-demo channels, preserving conversations during rollout.
Accepting a username invitation joins one public starter, preferring `general`,
if available. Invite links use their selected public channel, falling back to the
oldest public channel. Creating a channel joins its creator, not every space member. Space
removal clears participation and revokes channel invitations. No Valkey/SFU
schema change or data reset is needed.

Validation commands (disposable local services only):

```bash
DATABASE_URL=<disposable-loopback-postgres-admin-url> \
CHAT_TEST_DATABASE_URL=<disposable-loopback-postgres-admin-url> \
CHAT_TEST_VALKEY_URL=redis://127.0.0.1:6379 \
TEST_VALKEY_URL=redis://127.0.0.1:6379 \
cargo test --workspace -- --ignored --test-threads=1
npm run test:native -- tests/native-parity-fixture.test.mjs
CHANNEL_TEST_WEB_URL=http://localhost:5174/spaces node scripts/test-channel-joining.mjs
```

| Platform | Channel participation validation / limits |
| --- | --- |
| API | Disposable Postgres migration/backfill, concurrent join, public read versus participation, private consent/expiry/cooldown/cancellation and removal tests; SQL sends and gateway/media authorization enforce joins |
| Browser | Build/typecheck and unit tests; explicitly mocked Chromium desktop/390px search, preview, retry, persistence, accept/decline and leave. DOM/request assertions verify no private history before consent and zero microphone requests; representative captures inspected |
| Rust desktop | Independent membership/directory/preview/consent implementation, Linux unit tests/fmt/clippy/build and rendered labelled desktop/narrow preview, directory/private consent and starter-channel disclosure fixtures inspected. Not Windows execution, physical audio, or live SFU evidence |
| Apple iOS/macOS | Independent implementation and DTO/route tests added. No Swift/Xcode execution or device rendering in this Linux orb; platform CI and device acceptance remain required |
| Android | Independent implementation and DTO/route tests added. Earlier channel PR CI built the APK and passed five instrumentation tests, but UI smoke expected the retired guest demo. Reconciliation retains current main's sign-in smoke fix; exact-head platform CI and device acceptance remain required. No JDK/Android SDK execution or physical-device rendering in this orb |
| Native refresh | Invitations/joins are discovered on account/space reload and after mutations. Browser additionally refreshes on focus/visibility and every 15 seconds. Native clients do not claim browser-equivalent periodic discovery; server authorization applies immediately |
| Docker/live | No Docker daemon available; validate web/API image build stages directly. No deployment, shared database writes, live SFU or physical-device acceptance performed |

**Deployment order** for space invitations and channel joining (operator actions
only; merge does not deploy). When rolling out the cumulative reaction revision,
use **Deployment order for reactions** above, including its client compatibility
requirements, instead of a separate invitation/channel rollout.

1. Space invitations, channel participation, and invitation presentation are
   merged. Deploy their cumulative images once, not as separate feature rollouts.
   No new infrastructure, secrets or
   configuration is required. Existing direct `MIGRATION_DATABASE_URL`, runtime
   `DATABASE_URL` and shared `VALKEY_URL` must already be configured. Keep one
   desired API replica. Use a maintenance window for channel/invitation mutations;
   do not serve them from mixed old/new API versions, since older versions grant
   private access without consent. Set `BACKEND_SHA` to a full merged revision
   containing channel participation whose `api-$BACKEND_SHA` image is published;
   set `CLIENT_SHA` to a compatible merged revision whose web image and native
   checks succeed. API image builds are path-filtered, so presentation-only
   revisions may not publish an API tag.
   API and gateway must use the same backend SHA. One SHA is valid only if that
   exact revision has the required published API and web images and native checks.
2. Deploy API first; startup uses the existing direct
   `MIGRATION_DATABASE_URL` to apply every pending embedded migration, including
   `202609300002_space_invitations.sql` and `202610010001_channel_joining.sql`,
   before granting runtime table privileges. The channel migration backfills
   existing active members, not pending space invitees. Already-applied migrations
   are skipped. No manual migration/backfill/reset is required. Wait for workflow
   success, readiness and all API pods using the new image before continuing.
   ```bash
   gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$BACKEND_SHA"
   kubectl -n default rollout status deployment/caper-api --timeout=15m
   ```
3. Deploy gateway next so typing/media commands enforce participation too. The
   API and gateway must both be updated before exposing channel-joining clients;
   no Valkey/SFU schema migration/reset is required. Do not close healthy tracks
   for this rollout. Wait for workflow success and readiness. The workflow/app
   key is `caper-gateway`; the Kubernetes Deployment is `caper-chat-gateway`.
   ```bash
   gh workflow run deploy-caper-gateway.yml --repo joswayski/infrastructure --ref main -f git_sha="$BACKEND_SHA"
   kubectl -n default rollout status deployment/caper-chat-gateway --timeout=15m
   ```
4. Deploy web, then release native clients after their exact-head platform tests
   succeed. Web and native releases can proceed independently after steps 2–3.
   Older clients cannot join newly discovered channels or accept private channel
   invitations and may misleadingly display a pending invite as an active member;
   update owner clients before using private invitations. Wait for each workflow
   to succeed before checking rollout.
   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$CLIENT_SHA"
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$CLIENT_SHA"
   ```
5. Verify with two owned test accounts: old conversations remain joined, a new
   member cannot read space/channel history or use media before accepting the
   space invite and starts in one channel afterward; public preview cannot
   send/use voice; join does not enter voice; private metadata precedes acceptance;
   cancellation blocks stale acceptance; public/private leave have distinct
   re-entry behavior. Verify inviter/expiry, accept/decline/cancel and an unrelated
   active call surviving navigation/leave. Treat provider cleanup as asynchronous.
   Prefer a forward fix. Client rollback is possible against the new services,
   with the limitations above; do not roll API/gateway back across the consent
   or participation boundary or drop membership/invitation tables as rollback.

## Message pins

Pins are channel-wide, not personal bookmarks. Any joined channel member can pin
or unpin another member's message; the two participants can do the same in a DM.
Private-channel grants and active space membership still apply. Preview readers
can see pins but cannot change them, including no-op writes. Actions do not ask
for confirmation. “Pinned by …” attribution appears above each pinned message in
the timeline, threads and Pins. It says “Pinned by you” when the pinner's account
ID matches the viewer; otherwise it names the pinner, independently of who wrote
the message. The attribution still opens the pinner's profile. The header says
Pins without a count.
Desktop message controls appear on hover or keyboard focus: two 14px glyphs in
24px targets, separated by 2px. Touch clients retain long-press actions and large
targets; narrow web reserves text space beside its 44px action target.
Browse channels lives in the space-name menu, and the DM divider follows
the channel list; the account/audio dock stays fixed.
Pins opens a dismissible overlay while the main conversation remains mounted.
Clicking outside or pressing Escape dismisses desktop/web Pins; mobile clients
also provide Close. Apple uses a macOS popover and an iOS sheet.
Each list shows the original author, avatar, date/time, text, and shared attribution,
with a visible Go to message action. Unpin belongs in the normal message-actions
menu, revealed on hover/focus or through mobile actions, not directly in the row.
These are independent native implementations, not web wrappers.

Channel history and thread history accept exactly one of `before`, `after`, or
`around`. `around=<message ID>` returns the target plus up to 30 messages before
and 30 after, in ascending order, with `hasMore` and `hasNewer`. Limits count
messages, not edit/reaction sequence events. Anchors resolve inside the authorized
channel/thread; wrong-channel, wrong-thread and inaccessible targets return 404.
`after=<sequence>` returns the next 50 rows in ascending order. Clients scroll to and
highlight the target, expose older/newer paging and retain a bounded visible
window so retained latest rows are never spliced across a history gap. Context
snapshots merge independently of the committed gateway replay cursor. Pinned
non-broadcast replies navigate into their thread. Back to latest and successful
sends from historical context reload the latest page instead of leaving new
messages hidden beyond a gap.

`PUT /api/chat/channels/{channel}/messages/{message}/pin` accepts `{active:boolean}`
and `X-Caper-Chat-Token`; clients cannot supply a pin author. The response and
durable outbox event are
`{type:"message.pin",schemaVersion:1,channelId,seq,message}`. The message snapshot
contains nullable `pin:{author,createdAt}` and decimal-string `pinSeq`. The
event's `seq` equals `message.pinSeq`; original message `seq`, content, and
creation time remain unchanged. Pin/reaction revisions merge independently.
HTTP acknowledgements do not advance replay/read cursors. Reconnect replays
mutations, and authoritative history clears pins removed while offline; stale
acknowledgements/pages must not restore an unpin.

The clients also merge reaction revisions into their separate pinned-message
snapshots, including pins outside loaded history. A newer pin revision must not
restore older reactions, and a no-op pin response can still carry newer reactions.
October 8 regression checks cover these orderings in web and the Rust desktop
model. Android helper and Apple model tests are included but were not run in the
Linux orb (no Android SDK or Swift/Xcode); full native builds/device checks remain
release validation. This client-only correction needs no server or schema change.

Every first history page includes complete `pinnedMessages`, newest pin first, captured
under the same channel lock as its cursor. Older pages (`before=`) return an empty
`pinnedMessages`: every client keeps the first page's pins and live pin events. The pins collection is separate from
the 50-message timeline page: an old pin must not enter that page or skip the
intervening history. There are at most 100 active pins per channel and 60 actual
mutations per actor/channel/minute. Idempotent retries preserve the original
attribution and consume neither an event nor the mutation budget. Pin snapshots,
rate-limit activity, sequence allocation and outbox commit atomically. Unpinning
retains messages and durable mutation events; only expired rate-limit records
are pruned. Pins do not send system messages, play message sounds, or create push.

### Pin validation boundary

| Platform | Evidence and remaining gap |
| --- | --- |
| API/gateway | Disposable Postgres tests additionally cover channel/thread context, exact 30-before/30-after limits despite edit-sequence gaps, forward paging, boundary flags, mutually exclusive anchors and access isolation. Existing pin coverage includes shared delivery/replay and persistence. No production write or deployment |
| Web | 470 web/shared-native-support tests, build/typecheck/lint and `scripts/test-message-pins.mjs` pass: hover-menu optimistic unpin before server echo, outside/Escape dismissal, pinner profile by account ID, nested-menu layering, failed-jump retry, old context with 30 rows either side, centered highlight, Back to newest message, sends from channel/thread context, narrow layout and pinned thread navigation. Unit regressions cover pin jumps superseding same-root prefetches, out-of-order context responses and cached older/newer boundaries. `scripts/test-message-threads.mjs` passes cached reopening, stable repeated clicks, hover prefetch and 30-frame Pins return with an open thread/draft. Latest `scripts/test-chat-history.mjs` passes desktop/latest/live/history and narrow-history Pins round trips, checking 30 painted return frames and stable scrollbars, then fails its narrow live-arrival check with a 3px bottom gap (2px allowed); reproduced on a clean main checkout too. Desktop/narrow screenshots inspected. Browser fixture only, not Safari or a physical phone |
| Rust desktop | Linux build, Clippy, 285 tests passed (9 ignored), including retained Pins viewport/paging anchors, pending-unpin projection, resize/clip bounds, fixture pin counts and cached bounded thread context after forward paging. Inspected labeled static `parity-pins`, `parity-pins-empty`, and `parity-pins-long` renders: populated/empty cards, hover actions, long/multiple-pin scrolling, and a 390px window. Actual Linux menu/outside/Escape dismissal exercised; not live account acceptance. Windows build/runtime and production cross-client checks remain release validation |
| Android | Context/paging models, API, request fencing, Compose dialog/actions/navigation implemented. JDK present; `compileDebugKotlin` blocked by missing Android SDK. Compilation, rendered states and physical-device checks require CI/a native runner |
| Apple | Models, gateway, SwiftUI actions/list and protocol regressions implemented. Swift/Xcode unavailable in this Linux orb; macOS/iOS builds, rendered states and devices require CI/a native runner |
| Containers/live | Docker has no running daemon. Validate web/API build stages directly; no container-image, live account/SFU or physical-device acceptance is inferred from fixtures |

With the disposable fixture and Vite running, use:

```sh
npm run test:native -- tests/native-parity-fixture.test.mjs
MESSAGE_TEST_WEB_URL=http://127.0.0.1:5174 node scripts/test-message-pins.mjs
DATABASE_URL=postgres://user@127.0.0.1:55432/postgres \
  cargo test --locked -p caper-api threads_isolate_replies -- --ignored
CHAT_TEST_DATABASE_URL=postgres://user@127.0.0.1:55432/postgres \
CHAT_TEST_VALKEY_URL=redis://127.0.0.1:6379 \
  cargo test --locked -p caper-api pins_are_shared -- --ignored --nocapture
```

For desktop hover assertions, set `MESSAGE_TEST_CHROME` to a Chromium executable
wrapper that adds
`--blink-settings=primaryHoverType=2,availableHoverTypes=2,primaryPointerType=4,availablePointerTypes=4`;
`scripts/fine-pointer-chromium.sh` is one (`CAPER_CHROMIUM` picks the browser,
otherwise Playwright's newest Chromium).
Stock headless Chromium reports no hover device. The script verifies the fine
pointer before checking hover and later checks a 390px narrow browser layout.
That is not touch emulation or physical-device validation.

Attribution validation (2026-10-15): `npm run check`, all 650 JavaScript tests,
and `scripts/test-pinned-media.mjs` pass. The browser fixture covers self/other
labels in chat and Pins at 1280px/390px, live pin changes, pinner profiles,
unpin removal and thread attribution. Rust desktop passes 545 tests (12 device/
audio tests ignored), formatting, Clippy and a Linux build; its Pins fixture was
rendered and inspected. Android compilation stops at the missing Android SDK;
Apple builds/rendering require Swift/Xcode, unavailable in the Linux orb.

### Deployment order for pins

Merging does not deploy. These are operator actions, not commands run by tests.

1. **Prerequisites/infrastructure/configuration:** no infrastructure apply, new
   service, secret, feature flag, or Valkey/SFU reset is required. Keep existing
   `DATABASE_URL`, direct `MIGRATION_DATABASE_URL`, `VALKEY_URL` and chat config.
   Keep one desired API replica. Set `MERGED_SHA` to the full merged revision;
   wait for its API/web images and native CI. Update native testers before
   enabling pins: old clients do not display pins and may resync across new events.
2. **Database/API first:** startup applies pending embedded migrations, including
   `202610060001_message_pins.sql` (partial pin index and rate-limit table), and
   runtime grants before readiness. No manual data writes/backfill are needed.
   Watch the exact dispatched workflow and readiness, then verify history/send.
   ```sh
   gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-api --timeout=15m
   ```
3. **Gateway next:** deploy the same backend revision before pinning clients.
   Replay and publishing must preserve the original message author's avatar
   when the mutation sequence differs from the message creation sequence.
   Do not change replica counts or close healthy voice tracks.
   ```sh
   gh workflow run deploy-caper-gateway.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-chat-gateway --timeout=15m
   ```
4. **Client releases:** after server readiness, web and native releases are
   technically independent. Release validated native builds first so testers
   can consume pin events before exposing the web action. Wait for each exact
   workflow run; platform compilation/rendering is a prerequisite for release.
   ```sh
   gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$MERGED_SHA"
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```
5. **Verify/rollback:** with two accounts on different clients, pin/unpin each
   other's messages, confirm attribution/list updates, reload/reconnect, pin an
   old message, and switch channels/DMs. Preview, removed membership and revoked
   private grants must deny mutations. Confirm ordinary sends/reactions and an
   unrelated voice call remain healthy. Prefer a forward fix; client rollback
   removes entry points but leaves durable pins. Do not roll server images back
   across this migration: SQLx validates applied migration history. A reviewed
   rollback build must retain the migration and disable pin writes while keeping
   history/events readable. Keep additive tables/indexes and durable events;
   never drop message/pin data or migration records as rollback.

## Message threads

Reply in thread sits beside desktop message reactions and in message actions.
Roots show a reply count and up to five distinct repliers' avatars (first-reply
order). Desktop/wide clients use a right panel; narrow web, Android and iPhone
take over the conversation screen. The selected root has a slightly stronger
pin-gold wash; other messages are not dimmed. No nested threads: replying from a
broadcast opens its original root. The same contracts also apply to DMs.

The unchecked **Also send to channel** option publishes one reply in both places,
not a copied root message. IDs, content and reactions are shared. Drafts and the
checkbox choice are per-root for the current channel session, and the checkbox
clears after each sent reply so later replies stay in the thread; retry commands
freeze the root and broadcast choice. Ordinary sends cannot retry a pending reply
into the channel. Replies use the existing participation and read permissions;
thread reads do not expose another channel's messages.

Normal in-flight sends show no cross-composer warning. A delivery problem stays
with its reply in the thread; the channel offers **Review reply** only when that
thread is closed or another thread is open. Sending still allows one outstanding
command per channel, and retries keep the original ID, text, root and broadcast
choice so an uncertain outcome cannot become a duplicate or a channel message.

Thread composers accept the same files, photos and videos as ordinary messages,
including attachment-only replies. Each client reuses its regular picker,
clipboard/drop entry points, preparation, uploads, previews, removal and Retry.
Files belong to their channel and optional thread root: closing/reopening a
thread or switching conversations retains them, and sending in one composer
does not consume another's files. The 10-file limit applies to each composer.
**Also send to channel** shares the reply and its attachments in both places.
This is client-only; the existing API already accepts `attachmentIds` together
with `threadRootId`, so no service, schema, infrastructure or secret change is
needed. Native clients can release independently against the current API.

Thread-attachment validation (October 13, 2026):

| Client | Checked | Still requires acceptance |
| --- | --- | --- |
| Web | `npm run check`, `npm test`, and `scripts/test-chat-paste.mjs` against the disposable parity fixture at 1280px/390px pass. Additional Chromium checks exercised real MP4 selection/playback, native image clipboard, synthetic drop, pending file-only replies, failed-upload Retry, per-composer limits and disabled uploads; rendered states inspected. | Live storage, Safari and physical devices. The fixture is not production upload acceptance. |
| Android | `testDebugUnitTest lintDebug compileDebugAndroidTestKotlin` pass (380 JVM tests). Shared native-support tests pass (27). A temporary real-ViewModel/HTTP harness covered thread-only files, unfinished-upload blocking, close persistence, frozen retries and rejected Edit/removal isolation. | Phone/tablet rendering, real picker return/navigation, clipboard/drop and media handling; no APK/native DSP assembly or device run. |
| Apple iOS/macOS | Swift 6.1.2 parsing for iOS 17/macOS 14 and four SDK-free model harness groups pass. Three XCTest regression cases added. | Xcode/Apple SDK typechecking, XCTest, iPhone/macOS rendering and real Files/Photos/paste/drop. Linux harnesses do not validate platform APIs. |
| Rust desktop | Earlier full suite: 489 passed, 12 device/audio ignored. Final thread tests: 21 passed; application-package Clippy and formatting pass. Inspected wide/narrow Linux fixtures include upload/error, file-only ready and rejected replies. | Live storage and real OS picker/clipboard/drop, Windows/macOS and device/media acceptance. Local fixture screenshots are not live upload proof. |

`POST /api/chat/channels/{channel}/messages` adds optional `threadRootId` and
default-false `broadcast`. The root must be a same-channel, top-level message.
`GET /api/chat/channels/{channel}/messages/{root}/thread?before={seq}` returns
`{root,messages,cursor,hasMore}`; pages contain 50 replies in ascending sequence
order and `before` is exclusive. Channel history contains roots and broadcast
replies only. The response's channel head must never advance a client's replay
cursor: thread pagination is separate from channel pagination.

Each reply's ordinary `message.created` event includes `threadRootId`, `broadcast`
and `thread:{replyCount,participants,seq}`. The parent summary and reply/outbox
commit atomically under the existing channel lock. `thread.seq` is a summary
revision, **not a content revision**; original message `seq` and creation time do
not change. Summary, reaction and pin revisions remain independent. Clients
merge summaries monotonically, preserve one broadcast identity, and keep roots
loaded through thread GET out of channel pagination until channel history loads
them. Reconnect refetches the active thread and fences obsolete responses.

### Unread thread replies (October 17, 2026)

Web, Android, Apple and Rust desktop show a gold (`#E4C76A`) dot and **Unread**
beside the terracotta reply count when another account replies in a thread the
viewer started or replied to. Participation comes from message authorship, not
the five displayed avatars. Own replies and blocked/deleted authors do not create
unread state; sending a reply does not implicitly acknowledge earlier replies by
others.

`GET /api/chat/channels/{channel}/threads/unread` returns
`{threads:[{rootId,seq}]}`, using the latest qualifying unread reply sequence.
`PUT /api/chat/channels/{channel}/messages/{root}/thread/read` accepts `{seq}`
and returns 204. Both use account authentication and existing conversation read
permissions. The additive `thread_reads` table stores a per-account/root cursor,
advanced monotonically and clamped to that thread's latest committed reply.
Personalized state never enters the shared message payload or gateway replay.

Clients acknowledge rendered replies only in an active, visible thread's latest
window; channel visits, hover prefetch and older windows do not clear the badge.
Failed acknowledgements retain it for retry, and delayed responses cannot erase
newer unread replies. Live events refresh the list; the existing 15-second
foreground refresh reconciles reads made on another device. This does not change
push notifications or channel unread indicators. Earlier thread visits were not
recorded, so existing participated threads can initially show **Unread** until
opened; no migration guesses or backfills past reads.

Validation: `npm run check`, 651 web/shared tests, Rust workspace tests and
Clippy passed. A disposable Postgres HTTP test covers participation beyond five
avatars, account-only access, private-channel/DM denial, own/blocked/deleted
authors, prefetch/channel-read independence and monotonic/thread-clamped cursors.
Chromium at 1280px and 390px exercised the real local API: hover left read storage
unchanged, opening a thread persisted its visible sequence and cleared only that
thread's badge, a background tab kept new replies unread until it regained focus,
and unread state survived a page reload. Narrow Chromium is not a phone/device
test. Rust desktop passed its tests and package Clippy, linked against native
libwebrtc, and rendered the labelled `parity-thread-unread` fixture under
Linux/Xvfb. That fixture verifies layout, not a live native conversation.
Android SDK and Swift/Xcode are
unavailable in this orb, so their new contract tests and native rendering remain
unverified. Docker has no running daemon; the web production build and API
compilation were checked, not container images or an API release build.

### Thread validation boundary

| Platform | Evidence and remaining gap |
| --- | --- |
| API | Workspace tests/Clippy and disposable Postgres integration cover isolation, participation/read access, invalid/nested/cross-channel roots, concurrent retries, broadcast identity, reactions, outbox rollback and the 50-reply paging boundary. No production writes |
| Web | Production build, 374 unit tests and `scripts/test-message-threads.mjs`: empty/populated/loading/error states, selected root/count/avatars, shared broadcast reactions, drafts, retries, focus restoration, full-screen dialog, inert background and mobile Back. Inspected 2x Chromium desktop and touch-emulated narrow captures; `(pointer:coarse)` verified. Not Safari or a physical phone |
| Rust desktop | `DOCS_RS=1 cargo check --manifest-path apps/native/desktop/Cargo.toml --tests` and package Clippy (`--no-deps`) pass after pin integration. This skips the libwebrtc C++ build/link; dependency-inclusive Clippy is blocked by existing missing-safety-doc errors in vendored `webrtc-sys`. No rendered desktop or native runtime acceptance in this orb |
| Android | Compose full-screen/wide panel, models, gateway and retry regressions implemented. JDK/Gradle/Android SDK unavailable here; native compilation, rendering and physical-device checks remain required |
| Apple | SwiftUI iPhone full-screen/wide panel, models, gateway and retry regressions implemented. Swift/Xcode unavailable here; iOS/macOS compilation, rendering and physical-device checks remain required |
| Containers/live | No Docker daemon. Web/API build stages checked directly; no container-image, live multi-account cross-client, SFU or physical-device acceptance is inferred from fixtures |

Thread composers on every client share the channel composer's `:` emoji and `@`
mention suggestions and keys (see [cross-client parity](#cross-client-parity-pass-october-8-2026)).
Native keyboard focus, accessibility, paging and IME behavior need platform
validation before release.

With the disposable fixture and Vite running:

```sh
npm run test:native -- tests/native-parity-fixture.test.mjs
MESSAGE_TEST_WEB_URL=http://127.0.0.1:5174 node scripts/test-message-threads.mjs
DATABASE_URL=postgres://user@127.0.0.1:55432/postgres \
  cargo test --locked -p caper-api threads_isolate -- --ignored
```

Set `MESSAGE_TEST_CHROME` to the fine-pointer Chromium wrapper documented under
pins to also assert desktop hover/focus visibility and compact reply controls.

### Deployment order for threads

Merging does not deploy. These are post-merge operator actions, not development
commands that have been run against shared systems.

1. **Prerequisites/infrastructure/configuration:** no infrastructure apply, new
   service, secret, configuration or SFU/Valkey reset is needed. Keep existing
   `DATABASE_URL`, direct `MIGRATION_DATABASE_URL`, `VALKEY_URL` and chat config,
   and one desired API replica. Set `MERGED_SHA` to the full merged revision and
   wait for its backend/web images and native CI. Older installed clients may
   display live thread replies inline because they do not yet filter roots.
2. **Database/API first:** startup applies embedded migration
   `202610060002_message_threads.sql` after pins, adding nullable root references,
   default-false broadcast and timeline indexes. No manual data write/backfill
   or separate migration dispatch is required. Wait for the exact dispatched
   workflow and readiness; verify ordinary history/send before clients.
   ```sh
   gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-api --timeout=15m
   ```
3. **Gateway:** deploy the same cumulative backend revision after API readiness.
   The thread payload uses existing message-created delivery, without a new
   gateway protocol or service. Do not change replica counts or close healthy
   voice tracks.
   ```sh
   gh workflow run deploy-caper-gateway.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-chat-gateway --timeout=15m
   ```
4. **Clients:** after server readiness, web and native releases can deploy
   independently. Validate/release native clients before testers create threads
   if consistent filtering is required. Compilation and rendered platform checks
   are prerequisites, not inferred from browser tests. Wait for exact workflow
   outcomes before accepting a release.
   ```sh
   gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$MERGED_SHA"
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```
5. **Verify/rollback:** use two accounts on different updated clients. Reply,
   broadcast, react from either surface, reload/reconnect, page over 50 replies,
   switch roots/channels/DMs and use mobile Back. Confirm denied membership/grants
   cannot read or reply and an unrelated voice call remains healthy. Client
   rollback removes entry points but old clients may show replies inline. Prefer
   a forward fix; do not roll backend images back across the migration because
   SQLx validates applied history. A reviewed rollback build must retain the
   migration and thread read/event filtering while disabling new replies. Keep
   all messages, outbox records, columns and indexes; never delete conversation
   data or migration records as rollback.

## Live message forwarding

**Forward message** is one action, always live. Signed-in readers can forward a
readable message into a joined, writable channel in any of their spaces or an
existing DM, with an optional note. The picker discloses that sharing includes
future edits, reactions and replies and that destination readers can re-forward
it. There is no snapshot/live choice or source-owner consent step.

Destination readers see a read-only original and its direct conversation replies
without source membership. This is an intentional disclosure boundary, not a
grant to the source channel: ordinary source history, threads, reaction mutation
and edit/version routes keep their existing access requirements. Sharing a
single reply does not expose its parent or siblings. Destination replies are an
independent thread; they never join or increment the original thread. Re-forwarding
flattens to the canonical original, rather than including intermediate notes or
destination replies. Future source replies share automatically, including replies
from people who did not create the forward. Do not assume a private channel's
content stays inside that channel once a member forwards it.

The API stores `messages.forward_source_id`, retaining both records. It does not
copy the original content into an independently editable message. The destination
wrapper has its own author, optional note, creation sequence, reactions, pins
and thread summary. `@mentions` in the note resolve like a message's
(`content.mentions`), so every client highlights them and opens profile cards,
but they never notify: the forward notifies as an ordinary message only. Shared source snapshots omit source pins and parent linkage.
Removing destination membership/grants revokes the scoped read; an independently
re-forwarded DM remains authorized by that DM, not the intermediate space.
Deleted source spaces/channels render an unavailable original without exposing
their retained history. Individual message deletion is not implemented.

### Forwarding API and replay

- `GET /api/chat/forward-destinations` returns writable joined channels and existing
  DMs for the signed-in account.
- Web, Android, Apple and Rust desktop pickers support selecting multiple
  destinations. Search matches case-insensitive words across space and channel/DM
  names in either order (including `#channel`); filtering keeps hidden selections.
  Each row keeps the space subtitle directly below its channel/DM name.
  Clients send sequentially with a separate UUID per destination. Confirmed
  destinations leave the selection; an ambiguous failure freezes the remaining
  keys and note for retry. A definitive rejection unlocks only the remaining
  selection/note. This is not an atomic batch: earlier confirmed forwards remain
  delivered if a later destination fails or the picker is closed.
- `POST /api/chat/channels/{destination}/forwards` uses `X-Caper-Chat-Token` and
  `{sourceChannelId,sourceMessageId,clientMessageId,text?}`. The source must be
  readable and the destination writable. Same-session retries reuse the exact
  UUID and note; ambiguous failures freeze that intent. Conflicting UUID reuse
  returns 409. Shared payloads are always loaded by the server, never accepted
  from client input. Every change to an original rewrites each of its forwards,
  so one original accepts at most 100 forwards, 10 per account; beyond that a
  new forward returns 409 `this message can't be forwarded again` (retries of
  committed forwards still succeed). Sends and forwards share the per-account
  rate limit.
- `GET /api/chat/channels/{destination}/messages/{wrapper}` hydrates the forward
  like history, so a deleted source is never served from the stored snapshot.
- The returned message has `forward:{message,seq}` (original or null and source
  snapshot cursor) and `forwardSeq` (destination projection revision).
- `GET /api/chat/channels/{destination}/forwards/{wrapper}/thread?before={seq}`
  authorizes through the destination wrapper and returns
  `{root,messages,cursor,hasMore,source}`, with 50 ascending replies and exclusive
  `before`. Its source cursor must not advance destination replay.
  `source` is null without source-channel access; otherwise it contains
  `{spaceId,message}` with the original's real thread parent (`spaceId:null`
  for a DM). Web, Android, Apple and Rust desktop check this on every
  **View conversation** click, navigate to the original and focus it (opening
  its containing thread for a reply). Destination-only readers retain the
  read-only excerpt view. Older APIs omit `source` and keep that fallback.
  Deploy the API before clients to enable navigation; no migration, new
  infrastructure, secrets/configuration or gateway rollout is needed.
- The outbox publisher projects source creation/reactions/edits into durable
  destination `message.forward` events, with schema version 1 and
  `event.seq == message.forwardSeq`. Projection and publisher completion commit
  together; replay is idempotent. Normal source events/subscriptions are never
  granted to destination-only readers. Original creation sequences stay fixed.
- Clients merge source snapshots and destination event revisions independently,
  including unloaded and pinned wrappers. An open shared view refetches its
  loaded reply range on source updates, so old-reply edits/reactions remain visible.

Editing from PR #337 is integrated. The API author predicate rejects
`messages.forward_source_id IS NOT NULL`, and web, Android, Apple and desktop
edit gates exclude wrappers, including their notes. Source originals remain
author-editable; destination readers see projected live edits but do not gain
source edit history. Content revisions and forward projection cursors merge
independently from reactions, pins and destination thread summaries.

### Forwarding validation boundary

| Platform | Evidence and remaining gap |
| --- | --- |
| API | Workspace tests/Clippy plus disposable Postgres/Valkey checks: private cross-space/DM reads, canonical re-forwarding, authorization denial/revocation, stable retry/conflict, 54-reply pagination, separate destination thread, real edit handler, wrapper edit rejection, source-version denial, real outbox Pub/Sub and replay deduplication. The existing editing database regression also passes |
| Web | `npm run check` and `npm test` pass (470 web/shared tests). `scripts/test-message-forwarding.mjs` checks aligned 2px subtitles, space/channel token search, selection surviving filtering, delivery across spaces/channels/DMs, partial unknown-response retries without duplicates, definitive rejection edits to remaining destinations, destination-only live edits/reactions/replies, read-only originals and independent destination threads. Default/selected/search/empty/retry/rejected desktop and narrow captures inspected at 2x. Chromium narrow layout is not Safari, a physical phone or native acceptance |
| Rust desktop | Full libwebrtc-linked Linux build and 288 tests pass, 9 existing opt-in tests ignored. Headless picker tests cover compact same-space rows, token search and hidden selections; retry tests cover partial success and preserved UUIDs. Application Clippy passes with `--no-deps`; unrestricted Clippy still fails on existing vendored `webrtc-sys` safety-documentation warnings. Native default/selected/search renders inspected and one forward per destination confirmed via the disposable HTTP fixture for two spaces and a DM. Not production/Windows/device acceptance |
| Android | Compose multi-select picker/card/read-only view, protocol/gateway/merge and regression tests implemented. JDK is available, but Android SDK/Kotlin compiler are not; compilation, native rendering and device checks remain required |
| Apple | SwiftUI picker/card/read-only view, protocol/gateway/merge and regression tests implemented. Swift/Xcode unavailable in this Linux orb; iOS/macOS compilation, sheet transitions and device checks remain required |
| Containers/live | No Docker daemon; build stages validated directly. No deployed multi-account/cross-client or physical-device acceptance is claimed |

With the disposable fixture/Vite and a disposable local Postgres/Valkey running:

```sh
npm run check
npm test --workspace @caper/web
npm run test:native -- tests/native-parity-fixture.test.mjs
MESSAGE_TEST_WEB_URL=http://127.0.0.1:5174 node scripts/test-message-forwarding.mjs
MESSAGE_TEST_WEB_URL=http://127.0.0.1:5174 node scripts/test-message-edits.mjs
DATABASE_URL=postgres://user@127.0.0.1:55432/postgres \
FORWARD_TEST_VALKEY_URL=redis://127.0.0.1:6388 \
  cargo test --locked -p caper-api --lib forwarding -- --include-ignored
```

### Deployment order for forwarding

Merging does not deploy. Run these only after review/merge against a cumulative
revision; do not deploy an isolated feature branch that omits applied migrations.

1. **Prerequisites/infrastructure/secrets:** ensure main contains threads (PR #335)
   and editing (PR #337), then merge forwarding; set `MERGED_SHA` to the resulting
   full merged revision. The current main baseline contains both features;
   this PR includes that baseline and preserves subsequent client fixes.
   No new infrastructure, service, secret/configuration or SFU/Valkey reset is needed.
   Keep existing database/migration/Valkey/chat configuration and one API replica.
   Wait for backend/web images and native build checks. Use the cumulative
   migration history and the integrated wrapper rejection above.
2. **Database/API first:** API startup applies embedded
   `202610060010_message_forwards.sql` after the threads/editing migrations, adding the
   nullable retained source reference, self-reference check and lookup index.
   No data backfill or separate manual migration is required. Verify normal
   send/history and cross-space forward before clients.
   ```sh
   gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-api --timeout=15m
   ```
3. **Gateway:** deploy the same cumulative backend after API readiness. Existing
   durable destination delivery carries the new event; no source subscription
   or gateway grant is added. Do not change replicas or close healthy voice tracks.
   ```sh
   gh workflow run deploy-caper-gateway.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-chat-gateway --timeout=15m
   ```
4. **Clients:** deploy web after servers. Android/Apple builds, rendering and
   device checks remain release prerequisites; clients can then deploy
   independently. Older clients may display only the note; upgrade before
   relying on forwarding.
   ```sh
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$MERGED_SHA"
   ```
5. **Verification/rollback:** use separate source/destination accounts and a DM
   recipient without source access. Forward, re-forward, edit/react/reply at
   source, reply independently at destination, reload/reconnect/page and revoke
   destination membership. Verify wrapper edits and source version reads remain
   denied. Prefer a forward fix. Rollback builds must retain all applied migrations,
   content, versions, outbox records and read/projection compatibility while
   disabling new forwards. Never delete retained data or applied migration history.

## Feedback in place

`@feedback`, `@caper` or any other team name (`@capers`, `@caperchat`,
`@caper_feedback`, `@caper_support`, `@support`, `@team`, `@staff`, `@admin(s)`,
`@administrator(s)`, `@mod(s)`, `@moderator(s)`; all reserved usernames) in any
channel, thread, DM, message request or personal note asks the Caper team to look
at that conversation. Only accounts can report: a guest's alias, an alias added by an
edit, and an alias in a forward's note stay plain text. Nothing is copied or
moved. The message stays where it was written, shows its alias as a mention pill
and is marked **Caper team has been notified 🫡** for everyone there.

- **Report.** The send transaction writes `feedback_reports` (message, channel,
  reporter, and the thread replies belong in: the message itself when it was
  top-level). Reports are kept.
- **Access.** `feedback_access(channel, account)` is true for an active
  `feedback_team` member once that channel has any report. The team then takes
  part like a joined member: every read path (history, threads, single messages,
  versions, reactors, gateway subscription and replay), sending, replies, files,
  reactions, editing their own messages, typing, pins, voice and forwarding into
  the conversation. They are not listed as members and keep no read markers.
  Responses carry `"visitor": true` when the reader isn't a member, which clients
  use only to name the space in the header. A visitor's DM reply neither accepts
  a pending request nor is affected by the participants' blocks. Removing someone
  from the team (`deleted_at`) ends their access everywhere at once.
- **Answering.** Team members' messages carry `author.admin`, stamped by the server
  at send time and badged **ADMIN** on every client. Top-level reports open at
  the message in its conversation, without opening a thread. Reports made in
  a thread reveal that existing thread so the reported reply is visible. The
  team can explicitly start a thread; its participants see replies, and an
  @mention reaches the reporter as usual.
- **Notifications.** Kind `feedback`, title `Sender · Feedback`, for every team
  member, ignoring mutes. See [notifications](notifications.md#who-gets-a-notification).
- **Inbox.** `GET /api/feedback[?before=cursor]` (404 unless on the team) lists
  reports newest first, 50 per page, with reporter, place, a 280-character
  preview, reply count, `archived` and `answered`. `answered` is computed on read,
  not stored: it is true while the newest message in the report's thread is from
  the team, so anyone replying afterwards makes it wait again. Clients group
  reports by thread. `PUT /api/feedback/archive {channelId, threadRootId, archived}`
  archives or restores a whole thread for the whole team (`archived_at` on its
  reports). Anyone but the team posting in the thread afterwards brings it back, and
  `?archived=true` lists archived threads. `GET /api/account/me` adds
  `feedbackTeam: true` for team members.
- **Waiting badge.** `GET /api/spaces/unread` also returns `feedbackWaiting`:
  the total unarchived conversations waiting for a team reply, grouped by
  channel/thread using the newest report, independent of inbox pagination.
  Non-team accounts receive zero. Web, Android, Apple and Rust desktop show a
  terracotta count on Feedback through their existing unread refresh loop.
  Opening, collapsing or viewing archived feedback does not clear it; replying
  or archiving changes it on the next refresh. Failed refreshes retain the last
  count; zero hides it. This is a shared team backlog, not a personal unread count.

**Messaging Caper.** Any team name in **New message** opens an accepted DM with
the Caper account (`users.team_account`, username `caper`, no email, so nobody
signs in as it; the API maps every alias to it). Each top-level message sent
there is a report, marked like `@feedback`, so it lands in the inbox as its own
thread; replies in that thread stay part of it. The team answers as itself, in
the thread or straight in the DM; either counts as answered. The DM's `peer.team`
is true, and clients replace the empty state's privacy line with *Messages here go
to the Caper team.* When the team posts in
any DM it visits (the Caper DM, personal notes or two people's DM), everyone in
the conversation is notified as for a DM message.

**Privacy.** Tagging the team is consent for it to read that conversation, the
earlier context included. The `@feedback` suggestion says so: *Caper team · can
read and reply here*. Access is per conversation, never per space. The team cannot
browse conversations nobody reported.

**Operating the team** (run with the migration role; the runtime role can only read):

```sql
-- Add
INSERT INTO feedback_team (user_id) SELECT id FROM users WHERE username = '<username>' AND deleted_at IS NULL;
-- Remove (kept for history)
UPDATE feedback_team SET deleted_at = now() WHERE user_id = (SELECT id FROM users WHERE username = '<username>') AND deleted_at IS NULL;
-- Who is on it
SELECT u.username, t.created_at FROM feedback_team t JOIN users u ON u.id = t.user_id WHERE t.deleted_at IS NULL;
```

Migration `202610120002_feedback.sql` adds the Caper account, the two tables
and the function.
`--migrate` grants the runtime role `SELECT` on `feedback_team`, `SELECT, INSERT,
UPDATE` on `feedback_reports`, its sequence and `EXECUTE` on `feedback_access`. Every
access check calls the function, so **run the migration before rolling out the
API image**: an older API keeps working against the new schema.

| Check | Evidence / limitation |
| --- | --- |
| API | `push::tests::feedback_lets_the_team_read_and_answer_where_it_was_asked` against disposable Postgres and Valkey: no access before a report, whole-channel visitor read, private channels, thread replies with `author.admin`, reactions and pins, @feedback and @named visitors notified as `feedback`, APNs payload, inbox order and `answered`, edits never report, DM request stays pending, gateway read and participation (typing, voice), removal ends access. Grant test covers the runtime role. Full workspace suite passes |
| Web | Vitest for aliases, suggestions, pills, notification routes and inbox grouping. Real local API, gateway, Postgres and Valkey in Chromium: inbox, desktop visit at the report's thread, admin reply with badge seen by members, member autocomplete, "Join voice" under the visited report, 390px DM visit opening the thread, DM reply leaving the request untouched |
| Rust desktop | `cargo fmt --check`, `cargo test` (422 passed) and Clippy with libwebrtc: decoding, pills, autocomplete, open order, inbox paging, notification routing, headless egui inbox/visit flows. No real window or live server |
| Android | No Android SDK here. Models, `data/*`, push payloads and autocomplete compiled with standalone kotlinc and their JVM tests pass; Compose UI, view model and FCM taps unbuilt |
| Apple | No Xcode here. Models, `AppModel`, autocomplete, notifications and API client type-checked with Swift 6 on Linux (stubbed Apple-only modules) and their XCTests pass; SwiftUI views, APNs taps and macOS notification clicks unbuilt |
| Waiting badges (October 18) | Disposable Postgres HTTP regression covers 55 conversations beyond the first page, duplicate reports, team-only counts, archive/answer/reopen and removed channels. Web unit test and Chromium 1280px/390px mocked-response checks cover counts, collapse/archive views, outage retention and zero hiding. Linux desktop fixtures visually inspected; 13 feedback-related tests and the unread-divider integration test pass. Apple/Android decoding tests added but not run: Swift/Xcode and Android SDK unavailable in this orb; no physical-device checks |

## Message editing

Web, Android, Apple (iOS/macOS) and Rust desktop message actions and pins offer
**Edit message** to the original author account while it can participate in
the conversation. This includes
channel messages, thread roots, hidden replies, broadcast replies and DMs.
Ownership follows the original session's account, so rotating a chat capability
does not lose authorship. There is no age limit or space-owner override.
Save uses the normal 4,000-code-point, nonblank text rules; desktop Ctrl/Cmd+Enter saves.
Conflicts and failed saves keep the draft. **Discard draft and load latest**
explicitly replaces it. Channel/account/access changes close obsolete dialogs.

An **edited** indicator opens retained versions, also available from message
actions and pins. Everyone with current read access can inspect the original
and later versions, including members previewing an unjoined channel. Editing
does not erase information: the dialog explains this before saving. History
prominently shows the previous version on the left and current version on the
right, with word-level changes highlighted. **View previous versions** below
selects an older change without replacing that latest pair; the original has
no fabricated predecessor. Web uses Pierre Diffs (`@pierre/diffs`); native
clients render native word diffs with equivalent capabilities. There is no
automatic version purge or restore-to-version action.

### Editing API and durable delivery

- `PUT /api/chat/channels/{channel}/messages/{message}` accepts the sending
  capability and `{text,expectedRevision}` and returns the current message.
  An optional `attachments` list also changes its photos, videos and spoilers
  ([Editing attachments after sending](#editing-attachments-after-sending)).
  Legacy messages implicitly have `revision:1`. Actual changes add `revision`,
  `editedAt` and decimal-string `editSeq`. Same-text saves are no-ops; an exact
  retry of the immediately preceding successful edit does not create another
  version/event. A stale draft returns 409. The limit is 30 actual edits per
  author/channel/minute; retries/no-ops do not consume it.
- `GET` at that URL reloads the current snapshot. `GET .../versions?before={revision}`
  returns `{messageId,versions:[{revision,content,createdAt,filesChanged?}],hasMore}`, newest
  first, 50 per page with exclusive `before`. Only the author sees files an
  edit replaced, with URLs; others get placeholders and no URLs. Reads recheck current membership
  and private grants; outsiders have no version-history access.
- Migration `202610060003_message_edits.sql` follows threads. It adds append-only
  `message_versions`; the original published content is captured lazily on first
  edit, without rewriting or backfilling all messages. The runtime role has
  SELECT/INSERT, not UPDATE/DELETE, on this table.
- The channel lock atomically commits version, current message, outbox event and
  channel head. Events are `{type:"message.edited",schemaVersion:1,channelId,seq,message}`,
  with `seq == message.editSeq`. Creation `seq`/time, message ID, original send
  hash/idempotency, author, thread/broadcast identity, reactions and pins stay
  unchanged. `thread.seq` remains an independent root-summary revision.
- The existing publisher and gateway replay these events. Clients merge only
  content by revision, including pinned/thread projections and late pages.
  Editing does not insert unloaded messages, chime or announce a new message.
  HTTP edit acknowledgements and history pages never advance replay cursors.
  Excess unloaded edit snapshots cause resync rather than silent eviction.

### Editing validation and release gaps

| Platform | Evidence and limits |
| --- | --- |
| API | Initial editing baseline: 159 workspace tests, none ignored, including disposable Postgres/Valkey tests for ownership, rotated capability, permissions, roots/replies/DMs, stale drafts, retries, rollback, publisher, rate/paging boundaries and append-only grants. Locked release build passed. Backend is unchanged in the UI/parity follow-up; its default workspace run passed 124 tests with 35 database/integration tests ignored (disposable servers were not running), plus fmt/Clippy |
| Web | Production build/typechecks, 382 web tests, 15 fixture tests and `scripts/test-message-edits.mjs` passed: real saves, pins, rendered split diffs with separate word highlights, fixed latest pair while browsing older changes, original selection, 390px history/conflict/draft/reload, ownership, hidden/broadcast edits, independent summaries, Escape isolation, 55-version paging and retry/rejected saves. Initial threads/pins regressions also passed (one default-pointer pins run needed retry). Inspected 2x desktop/narrow captures. Chromium fixtures do not prove touch devices, Safari or live multi-account acceptance |
| Rust desktop | Native editor/history, cursor-neutral HTTP snapshots and independent gateway merge. Linux build, fmt and 221 tests (9 existing device/integration tests ignored); package Clippy uses `--no-deps` because vendored `webrtc-sys` has existing missing-safety-doc warnings. Actual wide/480px history, original selection and editor captures were inspected; resize/selection repaint fixes keep contents visible. Tests cover retained drafts, late results, author changes, HTTP/replay isolation, stale projections and selected-original placement. Windows and cross-client live acceptance remain prerequisites |
| Android | Native author editor/history from timelines, threads and pins, with retained conflicts, latest/older comparisons and paginated history. Java 17/SDK 36 Kotlin main/test compilation and all 142 JVM unit tests passed, including edit headers/revision/identity, Unicode word diffs and bounded history. Orb setup now includes Java; install the Android SDK separately. No APK/native-audio build, rendered Compose/device or live cross-client acceptance is inferred from compilation/unit tests |
| Apple iOS/macOS | Native SwiftUI editor/history from rows/actions, threads and pins. Linux Swift 6 typechecked Foundation models and executed 10 history/diff/Unicode assertions; changed SwiftUI/API/model sources passed iOS 17 and macOS 14 syntax parsing. These checks do not typecheck SwiftUI/UIKit/AppKit or replace platform XCTest. Xcode compilation, rendered UI, physical devices and cross-client live acceptance remain prerequisites |
| Containers / live | No Docker daemon; API release and web production build stages passed directly, not as container images. Staging secrets check could not authenticate the staging AWS profile; no secret/configuration changes are required. No live multi-account, physical-device or SFU evidence is inferred from fixture checks |

With the disposable fixture and Vite running:

```sh
npm run test:native -- tests/native-parity-fixture.test.mjs
MESSAGE_TEST_WEB_URL=http://127.0.0.1:5174 node scripts/test-message-edits.mjs
CHAT_TEST_DATABASE_URL=postgres://user@127.0.0.1:55432/postgres \
  CHAT_TEST_VALKEY_URL=redis://127.0.0.1:56379 \
  cargo test --workspace chat::editing -- --ignored
```

For Android source/unit validation, use JDK 17 and set `ANDROID_HOME` to an SDK
with platform/build-tools 36 (the repository's native build also needs its pinned
NDK/CMake). `.agents/setup` now provisions these SDK components on x86_64 orbs,
using the app's Gradle version pins, and exposes the SDK in new login shells
inside this checkout. Native asset preparation remains in the Android build
scripts; setup does not install an emulator. The smaller Gradle heap below fits
a 4 GB orb:

```sh
apps/native/android/prepare-fonts.sh
apps/native/android/gradlew -p apps/native/android --no-daemon --max-workers=1 \
  '-Dorg.gradle.jvmargs=-Xmx768m -XX:MaxMetaspaceSize=384m -Dfile.encoding=UTF-8' \
  :app:compileDebugKotlin :app:testDebugUnitTest
```

### Deployment order for editing

Merging does not deploy. These are post-merge operator commands, not shared-state
actions performed during development.

1. **Prerequisites:** merge the threads dependency first, then editing; set
   `MERGED_SHA` to the full cumulative merged revision and wait for its backend/web
   images and native checks. No infrastructure apply, new secrets/configuration,
   service, SFU change or Valkey reset is necessary. Keep the direct
   `MIGRATION_DATABASE_URL`, existing runtime/Valkey configuration and one desired
   API replica. Have a current database backup; never reset retained data.
   Old native builds may miss edits or resync at the next event.
2. **Database/API:** deploy API first. Startup runs embedded
   `202610060003_message_edits.sql` after the pins/threads migrations and grants
   append-only runtime access. No manual data write, backfill or separate
   migration dispatch is needed. Wait for the dispatched workflow and readiness.
   ```sh
   gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-api --timeout=15m
   ```
3. **Gateway:** deploy the same backend revision after API readiness, before
   exposing editor controls. No new gateway service or protocol subscription is
   needed. Do not change replica counts or close healthy voice tracks.
   ```sh
   gh workflow run deploy-caper-gateway.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-chat-gateway --timeout=15m
   ```
4. **Clients:** validate and release the editors/history on Android, iOS/macOS,
   Windows/Linux and web. Clients can deploy independently after server readiness,
   but coordinate release acceptance so users receive equivalent editing/history
   capabilities. Wait for each exact workflow outcome before accepting a release.
   ```sh
   gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$MERGED_SHA"
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```
5. **Verify/rollback:** use two accounts on different updated clients. Edit a
   root, hidden/broadcast reply, pin and DM; compare versions, reload/reconnect,
   page past 50 versions and race two drafts. Confirm broadcast surfaces share
   one content revision, thread summaries stay unchanged, outsiders cannot edit
   or read versions, edits do not chime, and ordinary chat/voice stays healthy.
   Client rollback hides editor controls but cannot undo saved edits/versions.
   Prefer a forward fix; a reviewed server rollback build must retain the
   migration, version reads and edit-event consumption while disabling writes.
   Never drop retained content/outbox/migration records or roll SQLx images back
   across the applied migration.

## Message deletion

Authors can delete their own messages on web, Android, Apple and Rust desktop,
from the message actions after a confirmation. Nobody else can delete them yet;
space owners have no moderation delete.

`DELETE /api/chat/channels/{channel}/messages/{message}` (chat session token, like
an edit) is idempotent and returns a tombstone: `deletedAt`, empty text, no files
or mentions, a bumped `revision`, and `editSeq`/`attachmentsSeq` at the new
sequence. It is delivered as an ordinary `message.edited` event, followed by a
`message.pin` unpin when it was pinned, so released clients that predate
deletion still blank the message and drop its files. Current clients render
"This message was deleted." in place, keep its thread summary, and offer no
other actions. Replies quoting it carry `replyTo.deleted` with empty text and
show "Original message was deleted."; forwards of it become unavailable.

Nothing is purged (`202610180002_message_deletions.sql` only adds
`messages.deleted_at`). The content is kept in `message_versions`, the files are
retired (readable only by the author), and stored replay events and reply quotes
are rewritten so replays never hand the content out again. Edits, reactions,
pins, quoting, forwarding and version history are refused for a deleted message;
thread replies under it are still allowed. Deleted messages stop counting as
unread and never notify. At most 60 deletions per person per channel per minute.

Notifications already shown are taken back. Deleting writes a
`notification_retractions` row (operational, pruned after a week like
deliveries) for every device a push reached, and a push delivered moments later
gets one too. A worker beside delivery sends them with the same lease, backoff
and 24-hour lifetime:

- Android (FCM data): `{"kind":"message.deleted","deletedMessageId",...}` without
  `messageId`, so older releases ignore it; the app removes the message from
  its conversation notification, or the notification when nothing remains.
- iPhone (APNs): message alerts carry `apns-collapse-id: <messageId>`; the
  retraction reuses it with "This message was deleted.", no sound and the
  `passive` interruption level, so iOS replaces the original in place. The
  notification service extension also removes originals delivered before
  collapse ids existed.
- Running web, desktop and Mac clients get `notification.retracted`
  (`messageId` and its conversation) on the account `profiles` feed and close
  what they showed. Like other feed events it is best effort and not replayed.

What was read on a lock screen cannot be taken back.

Message files are signed for one to two hours, on the hour (`CdnSigner::attachment_url`;
profile pictures and space icons stay daily). Clients refresh links ten minutes
before expiry through `POST /api/assets/urls`, which no longer signs a deleted
message's retired files for anyone but its author, so other people lose access
within about two hours. A copy already in someone's browser or app cache stays
there, as a saved file would.

Validation: the ignored Postgres/Valkey test
`chat::deletion::tests::deletion_keeps_the_row_scrubs_content_and_only_lets_the_author_delete`,
web unit tests, the fixture test, and a Chromium check against
`native-parity-fixture.mjs` at 1440px and 390px (menu, confirmation, tombstone).
Rust desktop unit tests, fmt and clippy pass on Linux (not launched; Windows and
macOS unbuilt). Android and Apple add unit tests mirroring the edit tests but were
not compiled here (no Android SDK or Swift toolchain); CI and the native release
are their first build. Device and live-API checks remain open on every native
client.

No infrastructure, secret or configuration change is needed. The migration is
additive and Ship rolls the API out before the gateway and web as usual.

## Message text: links and grouping

Message content stays plain text on the wire
(`content: {version: 1, type: "text", text, mentions}`); there is no server,
database or API change. Clients find links at render time the same way they
find mentions: split the text into mention segments, then run link detection
on the plain segments only.

**Links.** `http://`, `https://` and `www.` URLs (case-insensitive) become
links. The rules follow GitHub Flavored Markdown's autolink literals: a link
starts at the beginning, after whitespace, or after `( [ { < " ' * _ ~`; runs
to whitespace or `<`; drops trailing `? ! . , : ; * _ ~ " ' >`, a trailing
`&entity;`, and unbalanced `)`/`]`; and needs a host of ASCII letters,
digits, `_`, `.` and `-` with at least two labels (three after `www.`) and no
`_` in the last two. `www.` links open as `https://`. Nothing else
(`javascript:`, `mailto:`, `ftp:`, bare domains) is ever linked, and the
visible text is never shortened. Links open in the system browser; on web in a
new tab with `rel="noopener noreferrer nofollow ugc"`. They apply to message
bodies in the timeline, threads, Pins and forwarded-message cards, not to
edit-history diffs or the composer.

The algorithm lives in `apps/web/src/chat/links.ts` and each native client;
all four clients' unit tests assert the same segments for every case in
`shared/messages/link-cases.json`. Change the rules and that file together.

**Compatibility with rich messages.** Markdown and multi-part bot messages
(titles, fields, embeds, buttons) should be added as optional fields on the
same version-1 text content, for example a format flag or a structured
`blocks`/`embeds` list, while `text` always carries a readable plain fallback.
Every current client ignores unknown content fields but rejects unknown
`version`/`type` values, so this keeps already-installed apps showing the
fallback instead of dropping the message. A future Markdown renderer should
call the same link detection for bare URLs (GFM autolink literals), so plain
messages render identically before and after it ships; explicit
`[label](url)` links and bot embed links should go through the same
http/https-only, open-externally link handling. Link previews, if added, are
separate server-generated attachments rather than text.

**Grouping.** A message is shown compactly, without avatar and name, when the
previous visible row in the same list (timeline or a thread's replies) is by
the same account, at most five minutes earlier, on the same day, and neither
is a blocked-message placeholder; "Replied to a thread" broadcasts and thread
roots always keep their header. Compact rows keep the avatar column empty and
show the time there on pointer hover; an "edited" marker moves after the
text. Grouping is presentation only: ordering, paging, unread state and
actions are unchanged.

Validation: web was checked in a browser at desktop and phone sizes. Android
compiles with 221 JVM unit tests passing, including the shared link cases.
Desktop UI and unit tests pass (stand-in fonts; no rendered screenshot in CI).
Apple was not compiled in the implementing environment; its unit and parity UI
tests, and link colour when a message also has mention pills, still need a
macOS run. No physical device, TalkBack/VoiceOver or touch-hover checks yet.

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
| Cloudflare explicitly rejects an unavailable track during subscription | A successful HTTP response with one `not_found_track_error`/`empty_track_error`/`track_error`, no MID/SDP, and explicit `requiresImmediateRenegotiation: false` becomes `track_gone`; release the operation lock and preserve the listener. This includes a connected publisher that is not sending source media yet. HTTP errors, partial offers, allocated MIDs, and session errors remain ambiguous and do not take this path. |
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

The native voice audit extends this safe-rejection path to Cloudflare's documented
[`empty_track_error`](https://developers.cloudflare.com/realtime/sfu/observability/error-codes/).
The regression previously returned 502 and removed the listener; it now preserves
both sessions when no MID or SDP was allocated. This is not a confirmed diagnosis
of the native live smoke's uncorrelated 502. After approval and merge, deploy the
API correction before repeating multi-client acceptance (no migration or secret change):

```sh
MERGED_SHA=REPLACE_WITH_FULL_40_CHARACTER_MERGE_SHA
gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
kubectl -n default rollout status deployment/caper-api --timeout=15m
```

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

Real disposable Valkey integration tests cover independent
API instances, cross-pod notifications and capability replacement, full replacement
with unchanged Cloudflare mappings, capacity/operation races, lease/claim recovery,
and connection loss/recovery without replay. Cloudflare is mocked in these tests.
CI and orb setup use pinned Valkey 9.1.2. Run locally using a disposable loopback server:

```bash
# In an orb; do not point TEST_VALKEY_URL at a shared/production service.
amp orb service start caper-test-valkey --command 'valkey-server --bind 127.0.0.1 --port 6389 --save "" --appendonly no' --port 6389
valkey-cli -p 6389 PING # wait for PONG before starting the tests
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

`npm run dev` starts Valkey 9.1.2 alongside the API, web, gateway and Postgres.
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
| `DATABASE_MAX_CONNECTIONS` | Optional runtime pool size per API or gateway process, a whole number from 1 to 100; default `5`. Every replica holds its own pool, so keep replicas × size within the database's pooler limit. Invalid values stop startup. |
| `DATABASE_ALLOW_INSECURE` | Local development only. Set by Compose so the API may connect without TLS to the private `postgres` service. Hosted databases still default to verified TLS. |
| `AUTH_SECRET` | API-only random secret of at least 32 bytes. Enables account login and HMAC-protects low-entropy codes/IP rate-limit keys. Keep stable across replicas and rotations deliberate. |
| `DEBUG_USERS` | Optional API-only comma-separated username allowlist for authenticated client diagnostics. Exact case-insensitive matches after trimming; blank disables. Restart the API after changes; open tabs pick it up when refocused or when User Settings opens. |
| `RESERVED_USERNAMES` | Optional API-only comma-separated additional usernames that cannot be newly assigned by `/api/account/profile`. `caper`, `capers`, `admin`, `admins`, `administrator`, `administrators`, `mod`, `mods`, `moderator`, `moderators`, `support`, `feedback`, `security`, `staff`, `official`, `system`, `team`, `trust`, `safety`, `caper_support`, `caper_feedback`, `jose`, `caperchat`, `aaron`, `joswayski`, `everyone`, and `here` are always reserved. Existing owners may keep their current reserved username while updating their profile; changing to another reserved username is still blocked. Exact case-insensitive matches after trimming; restart all API replicas after changes. This does not create accounts or deliver chat mentions. |
| `AUTH_REVIEW_EMAIL`, `AUTH_REVIEW_CODE` | Optional API-only pair for App Store and Google Play reviewers, who cannot read emailed codes. Set both or neither; startup fails otherwise. Requesting a code for that one address emails nothing, and the fixed 6-character code (letters/digits from the sign-in alphabet, no `0`, `O`, `1`, `I`, `L`, `U` or `V`) completes the normal single-use challenge with the usual attempt and rate limits. Anyone given the pair can use that account, so keep it to demo content and rotate the code after reviews if it leaks. |
| `AUTH_CODE_ATTEMPTS` | Attempts per code; default `3`, allowed `1`–`10` |
| `AUTH_EMAIL_15M_LIMIT` | Code requests accepted per email from one source in 15 minutes; default `3` |
| `AUTH_EMAIL_DAILY_LIMIT` | Code requests accepted per email from one source in 24 hours; default `5` |
| `AUTH_EMAIL_DAILY_CAP` | Code requests accepted per email from all sources in 24 hours, bounding email bombing; default `20`. A new code cancels only the same source's earlier codes, so a stranger's requests can't lock the owner out or void the code they're typing. |
| `AUTH_IP_HOURLY_LIMIT` | Code requests accepted per source in one hour; default `10`. A source is an IPv4 address or an IPv6 /64 (IPv4-mapped addresses count as IPv4), hashed with `AUTH_SECRET`. |
| `AUTH_GLOBAL_HOURLY_LIMIT` | Code requests accepted across the service in one hour; default `500` |
| `NOTIFICATIONS_WEBHOOK_URL` | Optional server-only HTTPS URL for best-effort application notifications. The current event is `user.created`; URLs with credentials, query strings, or fragments are rejected and leave notifications disabled. Redirects are never followed. |
| `CAPER_API_ORIGIN` | Web-only. Origin the web server uses for server-rendered account lookups, e.g. `http://caper-api:3001` in-cluster or `http://api:3001` in Compose. It is the only place the server sends a visitor's session cookie. Unset outside development, the homepage renders a placeholder and the browser looks the account up itself. |
| `NOTIFICATIONS_ENABLED` | `true` starts the phone-push workers in the API role (requires `CHAT_ENABLED`). Default off: `GET /api/push/config` lists no platform and nothing is sent. Notification settings and mutes work either way. |
| `PUSH_PLATFORMS` | Comma-separated `apns`, `apnsSandbox`, `fcm`. A platform is advertised only when listed here and its credentials load; add one only after physical-device validation. |
| `APNS_TEAM_ID`, `APNS_TOPIC` | Apple team ID; topic defaults to `chat.caper.ios`. |
| `APNS_KEY_ID`, `APNS_PRIVATE_KEY` | Production APNs auth key (`.p8` PEM; real newlines or `\n` escapes) for `apns`. |
| `APNS_SANDBOX_KEY_ID`, `APNS_SANDBOX_PRIVATE_KEY` | Separate sandbox key for `apnsSandbox` (Xcode builds, staging). |
| `FCM_SERVICE_ACCOUNT_JSON` | Firebase service-account JSON key (as one string) for `fcm`. |
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
This is the application control API, not a self-hosted audio relay. After
publishing, the image run dispatches `Ship` in `joswayski/infrastructure`. Ship
rolls the API, then the chat gateway, then the web forward to the newest
published images on `main`, never backwards (see AGENTS.md, Shipping).
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
manifests, or that workflow change. An API-only merge does not publish a website image,
and a website-only merge does not publish an API image. Ship handles a merge that touches both: it rolls out the
web only after the API build for that range has published and rolled out.
Future workers or other services should get their own
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
  requests route straight to Rust. Keep API/gateway changes compatible with old
  tabs and native clients, which will not all update at deployment time.
- Web server responses send `Strict-Transport-Security: max-age=31536000` (no
  `includeSubDomains`), refuse framing (`frame-ancestors 'none'`,
  `X-Frame-Options: DENY`), and set `nosniff` and
  `Referrer-Policy: strict-origin-when-cross-origin`. HSTS only helps once plain
  HTTP redirects to HTTPS at the edge (Cloudflare "Always Use HTTPS").
  Server-rendered account lookups go only to `CAPER_API_ORIGIN`, never to an
  address derived from the request's Host header.
- The production infrastructure manifests configure **both API and gateway** with
  two replicas, `RollingUpdate`, `maxUnavailable: 0`, `maxSurge: 1`, `/readyz`, a
  five-second `preStop`, and 65-second termination grace. This is the desired
  configuration, not proof of the live cluster's state. All overlapping API and
  gateway pods must use the same `VALKEY_URL` and compatible state schema.
- Gateway SIGTERM makes `/readyz` fail and rejects new sockets/commands. Existing
  sockets receive `migrating` and keep delivering chat/media updates for 20 seconds.
  The browser opens a replacement without closing the old socket, resumes chat
  from its applied Postgres sequence, and waits for all subscriptions to catch up,
  including a current media snapshot, before switching. Failed candidates retry
  while the old socket remains usable. Overlapping messages are deduplicated.
  Accepted commands finish after socket loss and cache their results in Valkey;
  retries keep the same command identity. The gateway keeps its runtime alive for
  36 seconds before HTTP draining so those commands can finish.
- An API-only rollout does not restart the separately deployed gateway or its
  sockets. Account/chat state remains in Postgres and active voice state remains
  in Valkey. API SIGTERM rejects new media commands and drains in-flight HTTP
  requests for up to 30 seconds. **Shared-mode shutdown does not remove voice
  participants, close Cloudflare tracks, or revoke their TURN credentials.**
  Legacy opt-in SSE streams use the ten-second overlap described above. Healthy
  audio stays client ↔ Cloudflare, independent of API/gateway connections.
- Memory-only development does not have that voice guarantee: shutdown removes
  local participants and spends up to 20 seconds on provider cleanup. If HTTP
  draining exceeds its deadline, the process exits without provider cleanup
  rather than racing cleanup against unfinished mutations. Do not mix local-mode
  and shared-mode pods; use the staged Valkey cutover before adding replicas.
- Updated Android, Apple, and Rust desktop clients also overlap sockets on
  `migrating`. Chat replay resumes from the applied cursor; promotion requires
  subscription acknowledgments, the chat replay checkpoint, current media
  revisions, and any watched presence snapshot. Messages and media snapshots
  delivered by both sockets are deduplicated. Failed or stalled candidates retry
  without clearing the old stream's chat/voice state. In-call roster streams keep
  the existing media capability and do not rejoin the call during handoff.
  Older installed native versions still briefly reconnect; release the updated
  clients independently against the existing compatible API/gateway protocol.
- Deploy the matching API image successfully before the gateway image; the
  infrastructure gateway workflow verifies that API rollout before changing its
  pin. Keep infrastructure lifecycle settings in place and migrations compatible
  with both overlapping versions. Merging application code does not deploy it.
- Regression coverage uses real local WebSockets, Postgres and Valkey for replay,
  chat delivery through drain overlap, admission rejection, and accepted-command
  recovery. Shared-media tests replace both API instances and assert unchanged
  provider session mappings and zero shutdown track closes/TURN revocations.
  Browser tests cover repeated mixed chat/media handoffs, failed candidates, stale
  snapshots, deduplication, and unchanged mocked voice peers after gateway loss.
  Android JVM tests and Rust desktop tests exercise native handoffs with real
  local WebSockets. Swift actor tests use an injected transport, compiled and
  run in a Linux harness; Xcode/macOS/iOS and Windows builds remain separate CI
  validation. No physical-device handoff acceptance is claimed. Cloudflare is
  mocked; these checks do **not** prove live-cluster routing or continuous audio.
  Run the live two-client replacement checks in
  [Activation and verification](#activation-and-verification) before claiming that.
  A crash, replacement outage beyond the handoff window, or expired lease can
  still interrupt updates or force voice recovery.

## Limits and lifecycle

- 12 registered sessions total, one microphone publication each, 11 subscriptions.
  A running Mic test uses two additional private sessions within that same cap.
- 30 joins/minute per room, at most 10 of them by one account; 120 provider
  mutations/minute per participant; 256 KiB request bodies. One account holds
  at most 4 of a room's 12 slots (participants, microphone-test monitors and
  in-flight joins on any of its devices) and gets 409 `already in this call on
  too many devices` beyond that. These are **not a spending cap or DDoS defense**.
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
  The voice diagnostics report `lastReconnect`: the trigger (for example
  `connection lost`, `receive connection lost` or `lease renewal failed: HTTP 401`),
  its time, and both peer connections' states. It survives the rejoin and resets on
  an explicit Join; it carries no token, credential or SDP.
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

On native iOS, Leave gates microphone publication and playback and clears call
state immediately. Ending a microphone comparison, disabling WebRTC tracks,
closing the old peer (including VoiceProcessingIO disposal), and deactivating
`RTCAudioSession` run off the main actor. Previously those synchronous operations
ran on the main actor, so even an already-idle model could not render until audio
shutdown returned. A new join or microphone preparation awaits pending cleanup
before reusing the shared audio device/session; cancelling that join invalidates
it before it can activate audio. Remote `/leave` cleanup remains independent.

The iOS regression tests hold the audio-session configuration lock while checking
immediate Leave and cancelled rejoin, and use synthetic PCM through local WebRTC
peers to check silence before peer disposal and playback after reopening its gate.
These tests require Xcode/iOS Simulator and have not been run in the Linux orb.
Physical-iPhone Leave timing (speaker and Bluetooth), rapid leave/rejoin, and
leaving during a microphone comparison still need device validation. No new
native latency measurements are claimed here.

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

The enabled channel screen starts one DPDFNet-8 worker before Join. It downloads
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
instance. If that worker is still compiling (right after page load or a leave),
Join no longer waits for it: the microphone is requested with the browser's
noise suppression on, routed through the same processed track, and DPDFNet is
swapped in when ready (browser suppression is released first, as in a fallback
swap). This applies only when the browser confirms its suppression is active;
otherwise Join waits for DPDFNet as before. A model that fails while loading
takes the existing fallback path. Join waits for SSE readiness, transport, and
initial roster/state synchronization before enabling outgoing audio. A cold
join never publishes raw audio.

A September 25, 2026 production join showed Microphone 1,766 ms against
Session + publish 509 ms and Transport 139 ms (ICE 117 ms): the join was held by
microphone startup. In real Chromium with the real assets (fake device), opening
the device took 56-108 ms and a capture with a ready model 28-35 ms, while a
capture during compilation waited for the rest of the compile (about 42 s on
that slow sandbox CPU). With this change a capture during compilation returned in
79 ms with browser suppression carrying audio (published peak 1.01, as in
browser-only mode), then swapped to DPDFNet on the same track. The connection
panel now splits Microphone into device and processing time, or notes that the
noise model was still loading. Each engine gets one fresh-instance retry
for initialization errors or crashes before advancing from DPDFNet-8 to DPDFNet-2 HR,
then RNNoise; Join fails if none can initialize. A transient DPDFNet underrun outputs silence while
refilling the three-hop reserve, then resumes processed audio. Sustained overload
or worker failure switches the existing processed track to a bypass, preserving the call, and attempts to enable
the browser's microphone noise-suppression constraint while DPDFNet-2 HR loads.
Only if 2 HR also fails or overloads does RNNoise load. A ready fallback replaces only the processing node:
outgoing and mic-test track identities, mute state, input gain and voice enhancement
are preserved. Browser suppression is then disabled to avoid stacking filters.
If RNNoise cannot initialize, the browser fallback remains. The reported track
setting determines whether the UI says browser suppression is active or unavailable.
The mic test displays that live status; both Natural and Enhanced use this same
noise-suppressed input, and Enhanced adds EQ/compression rather than more denoising.
The recovery worklet uses `worklet-v3.js` to avoid the immutable cache of older clients; current pages load `worklet-v4.js` (see iPhone and iPad audio routes).
DPDFNet processor errors retry once before advancing to the next tier. An unrecoverable RNNoise
processor error stops the microphone without restarting the connection.

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

### Combined join and publication

Joining used to run four Cloudflare API calls back to back: `sessions/new` and
TURN (in parallel), then `tracks/new` for publication, then per remote track
`tracks/new` and `renegotiate` after transport. Two September 24, 2026 samples
(one browser, direct route, 6–7 ms RTT) took 1,071 ms in an empty room and
1,515 ms with one other listener. Their 31 ms roster phase in the empty room is
one gateway round trip, so the remaining time is mostly provider calls and
their ordering, not the SFU or AWS hop. These are two samples, not a benchmark.

The browser now creates its sendonly audio transceiver and offer while the
microphone opens, and sends that offer with `media.join`. The API creates the
session and TURN credentials, commits the participant, then publishes the offer
through the same `publish` state machine before returning `publish` (the answer
and `trackId`) with the capability. A refused or failed publication removes the
participant; the capability is never returned half-joined. TURN servers arrive
with that response and are applied with `setConfiguration` before the offer is
set locally, so relay gathering still uses them. The microphone is attached to
the sender before the answer is applied, preserving the previous invariant that
transport never connects without the (disabled) microphone. The authenticated
roster/lease request overlaps ICE. Subscription negotiation (`tracks/new` pulls
plus `renegotiate`) still waits for the listener's connected transport, because
Cloudflare does not accept a pull into a session whose PeerConnection has been
negotiated but has not connected. A live September 24, 2026 probe (real
Cloudflare, Chromium offers, the publisher and listener each having applied
their publication answer) found that `tracks/new` for the listener was held for
10.9 s and then answered HTTP 425 `session_error`: "Session is not ready yet.
Please ensure the PeerConnection is connected before making this request." The
API's 10-second provider timeout fires first, so pulling early turned into a
502 and removed the listener. A pull into an **empty** session (no negotiated
PeerConnection, as in Cloudflare's
[receive recipe](https://developers.cloudflare.com/realtime/sfu/get-started/connection-patterns/#receive-a-published-track))
was answered in 275 ms, and a pull of a publisher that has not connected was
answered at once with a per-track `not_found_track_error`. The probe could not
connect WebRTC transport (no UDP and no TURN/TLS egress from that sandbox), so
whether a held pull completes once the listener connects mid-wait is unknown;
either way it would not start media sooner. Received audio remains withheld
from playback, and the microphone stays disabled, until transport, live
updates, roster and state have all completed.

When several sources are missing, the browser sends `media.subscribe` with
`trackIds` instead of one `trackId` per request. The API pulls them in one
`tracks/new` (Cloudflare accepts up to 64 remote entries, from different
publishers) and one SDP exchange, and returns
`{ tracks: [{ trackId, mid }], gone: [trackId], sessionDescription?, requiresImmediateRenegotiation }`.
Per Cloudflare's [batch operations](https://developers.cloudflare.com/realtime/sfu/api/#batch-resource-operations),
partial success is reported per track. Each result is matched by its echoed
`sessionId`/`trackName` and classified: a MID without an error is allocated;
`not_found_track_error`, `empty_track_error` or `track_error` without a MID
allocated nothing and is returned in `gone` (as are sources that already left),
so the browser retries it with the bounded re-check above. Anything else is
uncertain and takes the existing recovery path: enqueue cleanup of any allocated
MIDs, discover orphans, and remove the listener. That includes a missing result,
an error beside a MID, an undocumented code, or an offer that does not match the
allocations. The single `trackId` request and its response are unchanged, so
native clients are unaffected. An API that rejects `trackIds` with 422 makes
the browser fall back to one pull per source for the rest of that call.

When the provider refuses to pull a listed source that is not sending media yet
(`track_gone` while the source is still in the roster), the listener re-checks
after 0.25, 0.25, 0.5, 0.5, 1, 2, 4 and 8 seconds (about 16.5 s in total) instead
of waiting for the next roster change or 15-second lease heartbeat. A newcomer's
track is listed at publication but pullable only once its transport sends media,
typically 0.3–3 s later, so the early checks are dense; a refused pull allocates
nothing and needs no renegotiation (live-captured shape: empty MID,
`not_found_track_error`, `requiresImmediateRenegotiation: false`, no offer). A
successful reconciliation resets that budget.

Diagnostics now separate microphone capture from session + publication (both
measured from Join, since they overlap), report ICE time within transport, and
show the selected candidate pair's connectivity checks (sent vs answered) to
expose retransmitted checks. The API logs every successful Cloudflare call's
`elapsed_ms` at info level (`Cloudflare operation succeeded`), not only failures.

Compatibility: an API without this change rejects the `publish` field with 422
before any mutation, and the browser then joins and publishes separately. Older
gateways converted that plain-text 422 into a 503, so the fallback only covers
the direct HTTP path. Deploy **API first, gateway second, web last**; no
migration, secret, Valkey schema or configuration change. Native clients do not
send the field and are unaffected. After merge:

```bash
MERGED_SHA=<full-merged-caper-commit>
gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
kubectl -n default rollout status deployment/caper-api --timeout=15m
gh workflow run deploy-caper-gateway.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
kubectl -n default rollout status deployment/caper-chat-gateway --timeout=15m
gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
kubectl -n default rollout status deployment/caper-web --timeout=15m
```

Validation: the real Cloudflare HTTP adapter against a local stub returning
documented batch results (partial success in both positions, all refused, and six
uncertain shapes); Rust unit tests with a mocked provider (combined publication,
invalid offers creating no session, refused publication leaving no participant,
the 422 contract), the Valkey/Postgres gateway suite, and browser-client tests
with mocked WebRTC (combined answer, 422 fallback, overlapped lease renewal with
pulls held until transport connects, delayed pull retry, batched pull with a departed source, batch fallback after
422). Real Chromium confirmed that an offer created before
`setConfiguration` applies unchanged and that TURN allocation requests then reach
the later-configured server. Live Cloudflare joins, multi-network TURN, Firefox,
Safari and remote listening latency were **not** measured; compare the new
diagnostics from real sessions before claiming a specific speedup.

### Prepared joins for signed-in members

`media.prepare` (account channels only, through the member-checked
`/api/channels/{id}/media/prepare` route) creates the provider session and TURN
credentials for a signed-in member moments before they join. Their join takes
them instead of calling Cloudflare for both. The browser sends it when the
pointer or keyboard focus reaches a Join button, at most every 4 seconds per
channel, and also as a mouse or pen pointer comes within 120 px of a Join
button (one document listener, at most one geometry check per animation frame,
signed-in members only). Touch has no approach, and pointerdown arrives too close
to the tap to help, so on a coarse-pointer screen the viewed channel is prepared
once when its Join button first becomes visible (an `IntersectionObserver` on
that one button, not the sidebar's). Real Chromium with touch emulation and the
UI fixture sent exactly one `prepare`, for the viewed channel; desktop sent none
from visibility alone. A tap more than about 10 seconds after opening the channel
joins the ordinary way. The retired public demo's unscoped media routes now
return 404. Real Chromium with the UI
fixture confirmed: no request from far away, one when approaching within 100 px
without touching the button, and none again on the hover that follows.

Live measurements (September 25, 2026, this sandbox, real Cloudflare) set the
limits:

- An unused Cloudflare session accepted a publication after 10 seconds and was
  disconnected (410 `session_error`) at 15 seconds. A prepared join is therefore
  kept for 8 seconds, one per account per room; preparing at login would not work.
- Cloudflare bills only egress ($0.05/GB after a shared 1,000 GB monthly free
  tier); creating sessions, issuing TURN credentials and idle sessions are not
  charged ([pricing](https://developers.cloudflare.com/realtime/sfu/pricing/)).
- Revocation of each prepared TURN credential is scheduled 12 seconds after it is
  issued, so it happens even if the room goes quiet; a join that takes the
  credentials cancels it. Registry state gains `prepared` with a serde default,
  compatible in both directions.

Across 12 warm joins per version against live Cloudflare, provider session plus
TURN took a median 142-159 ms of each join (106-282 ms), which a prepared join
skips; a cold join (first after idle, new TLS connection to Cloudflare through
this sandbox's egress proxy) spent 616-657 ms there. Preparing also warms the
API's connection before the join. Production cold-connection cost is expected to
be lower than through that proxy; compare `Cloudflare operation succeeded`
`elapsed_ms` for `create_session` and `publish` in production logs.

Join also re-checks channel access concurrently with provider provisioning
instead of after it; the result is still applied before the participant is
committed or any capability returned.

When several people leave at once, a listener now stops their audio locally and
closes their MIDs with concurrent `close` requests (one round trip, not one per
person). Each close is one atomic registry update with no SDP change, and the
API already removed the listener's subscriptions server-side when the source
left, so these calls only keep state consistent; a transient failure keeps that
MID for the next reconciliation.

Deploy API, then gateway (its allowlist gains `media.prepare`), then web. An
older API or gateway rejects `prepare`; the browser ignores the failure and joins
the ordinary way.

### Warm sessions: pre-connected for signed-in members

A prepared join still leaves the connection handshakes on Join and lasts only
seconds. Signed-in members now keep a **warm pair** instead: while a member is on
an account channel or the public live demo, voice is available and they are not in a call, the browser
creates two provider sessions with no tracks and connects both:

- `media.warm` (signed-in accounts only; anonymous demo visitors receive 404) takes two
  browser offers, each carrying only a negotiated data channel (no track, no
  in-band channel messages). The API calls Cloudflare `sessions/new` with each
  offer, which is the only form Cloudflare accepts for a session without tracks:
  `tracks/new` requires at least one track (live-checked September 25, 2026).
  It also issues TURN, and returns both answers, the ICE servers and a ticket.
- The ticket is not stored anywhere. It is signed with HMAC-SHA256 for that
  account's session hash, using a key derived from the provider secret every
  media pod already holds. It names both sessions, the TURN credentials, the
  issuing channel and the issue time, so a join in **any** of that account's
  channels, including the demo, can adopt it. On the demo, both issuance and
  adoption validate the account cookie/bearer; a cookie alone is insufficient.
  Guest joins still work without a warm ticket.
- Join sends `warm: <ticket>` with its microphone offer, made on the warm
  connection (a new audio section beside the data section). The API verifies
  the ticket against the joining account and checks it is under 25 minutes old.
  It refuses a ticket whose sessions a participant in the room already uses.
  It then publishes into the warm session and pulls everyone present into the
  warm receive session, creating no session and issuing no TURN. Both
  connections are already connected, so no handshake follows. The warm receive
  session becomes the participant's pull session even in an empty room, so
  later pulls use a connected transport too. The response carries
  `warm: true`, and the diagnostics label the join "pre-connected".
- A refused ticket (409 `warm_unavailable`: foreign, altered, expired, replayed,
  or sent with a monitor) is refused before anything is reserved. The browser
  then closes the pair and joins the ordinary way in the same Join. So does a
  422 from an API without the field.

Lifecycle and cost:
- The browser replaces the pair after 20 minutes. It rebuilds the pair at once
  when either connection fails or stays disconnected for 5 seconds, and when a
  hidden page (a backgrounded phone) becomes visible again.
- It closes the pair during a call, on sign-out, and when leaving the call page.
- Closing the connections ends the provider sessions. Unadopted TURN
  credentials are revoked 30 minutes after issue, by a job scheduled in the
  issuing channel's registry; adoption cancels that job there, even from
  another channel.
- A room accepts one warm request per account per 10 seconds, and at most 48
  per 10 seconds in total. It refuses warms (429) while its cleanup backlog is
  half full, so parked revocations never crowd out the immediate cleanup that
  removing a participant needs.
- Cloudflare bills egress only, and an idle connected session carries only
  connectivity checks, so a warm pair costs no media. Cloudflare documents that
  its media inactivity timeout does not define the lifetime of a connected
  session without media.
- Registry state gains `warmed` (rate limiting only) with a serde default,
  compatible in both directions.

Validation:
- Rust tests cover issue, rate limit and validation, adoption, and pulls into
  the warm receive session. They also cover the refusals (foreign, expired,
  altered, replayed, monitor, anonymous demo visitor) and cross-room cancellation of the
  revocation. Each of the replay check, the signature check and the revocation
  cancel was removed in turn to confirm a test fails.
- Web tests cover the warm lifecycle, adoption with no new PeerConnection, the
  409/422 fallback, replacement after failure, and stopping on 404. Signed-in
  demo tests cover public-endpoint preparation and adoption; Rust also checks
  refusal when authentication fails and when another account uses the ticket.
- Real Chromium over loopback, with a second local peer standing in for
  Cloudflare (not Cloudflare): the data-channel-only pair connected in 31 ms.
  Adding the microphone renegotiated without leaving `connected`, and audio
  reached the stand-in 93 ms after starting. A server-style offer on the warm
  receive connection delivered audio in 93 ms.
- Live Cloudflare signaling: `sessions/new` with a data-channel-only offer
  returned an answer that Chromium applied.
- **Not verified:**
  - Publishing into a *connected* warm Cloudflare session; the sandbox cannot
    connect WebRTC.
  - End-to-end join time on a device.
  - How long iOS keeps the pair while the page is visible.

Deploy API, then gateway (it embeds the media handlers), then web. The signed-in
demo extension requires both backend roles to be updated. An older backend
returns 404 for demo preparation (or 400 if it lacks `media.warm` entirely), and
the browser stops asking until reload. No migration, secret or configuration
change. After merge:

```bash
MERGED_SHA=<full-merged-caper-commit>
gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
kubectl -n default rollout status deployment/caper-api --timeout=15m
gh workflow run deploy-caper-gateway.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
kubectl -n default rollout status deployment/caper-chat-gateway --timeout=15m
gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
kubectl -n default rollout status deployment/caper-web --timeout=15m
```

### Join-time pulls on a receive-only connection

A browser that publishes inside `media.join` also sends `receive: true`. When
others are publishing, the API creates a second, receive-only provider session
alongside the main session and TURN (or takes one from a prepared join), then
publishes the microphone and pulls everyone present concurrently, and returns
`receive: { tracks, gone, sessionDescription, requiresImmediateRenegotiation }`
with the publication answer. The browser answers that offer on a second
`RTCPeerConnection`, so both connections come up together. Subscriptions, their
answers (`negotiate`) and closes then use the receive session; `close` takes
`subscription: true` because MIDs may repeat across the two sessions, and cleanup
closes each MID in its own session.

Join completes when the microphone connection is connected; it does not wait
for the receive answer's round trip (`negotiate` through the API to Cloudflare)
or the receive connection's handshake. The publication is queued ahead of that
answer, so it never waits on it. Audio from the people present plays as soon as
the receive connection is up, which the diagnostics report separately as
"Hearing others" (`hearingMs`, from Join). If that answer fails, or the receive
connection is not connected within 12 seconds of Join completing, the call
rejoins with `lastReconnect` set to `hearing others failed`, as a failure would
have failed the join before. An iPhone report attributed about 300 ms of a
1,062 ms join to this step; the saving is not yet measured on a device.

Why a second session: Cloudflare answers a pull into a session with no
negotiated PeerConnection at once (its documented receive pattern, also used
live by this app's microphone monitor), but holds a pull into a negotiated,
unconnected session for about 11 s and then answers 425. A single session cannot
mix local and remote tracks in one `tracks/new`. An earlier prototype created the
receive session on the browser's first pull after joining; that added a session
creation and a late second handshake and was slower, so it was dropped. Pulling
inside the join removes the post-connect pull instead.

Live signaling check (September 25, 2026, this branch's API, real Cloudflare,
Chromium SDP): a join into an occupied room returned the publication answer and a
receive offer; Chromium answered it on a second connection and `negotiate`
returned 200. Join latency into an occupied room was unchanged (median of 8:
370 ms with the join-time pull, 375 ms without), because the pull runs in
parallel with publication. The source could not connect from the sandbox, so the
pull itself was a refusal answered with Cloudflare's inactive placeholder offer;
that shape is covered through the real HTTP adapter in tests.

Rules: without an offer or with an uncertain pull result, the API cleans up and
the browser pulls after connecting as before; the join still succeeds. Media that
arrives before the roster lists its owner is held and assigned when it does.
Older APIs reject `receive` with 422 and the browser retries without it (then
without publication). Monitors, native clients and empty rooms get no receive
session. On relay-only networks the receive connection keeps its initial TURN
credentials (valid 48 hours); only the microphone connection renews them, so a
call longer than that on such a network rejoins once.

### Join benchmark: before and after the join speedups

Measured against live Cloudflare with headless Chromium SDP, local debug-build
APIs, 12 warm runs per version, interleaved; time from Join to the publication
answer applied in the browser (when ICE can start). Baseline is `a6bc4a1`, the
commit before PR #162.

| Version | Cold first join | Warm median (min-max) |
| --- | ---: | ---: |
| Baseline (`a6bc4a1`) | 938 ms | 417 ms (327-477) |
| PR #162 (`4ee0ac7`) | 930 ms | 436 ms (310-504) |
| Prepared join (derived: join minus logged session+TURN) | about 250-300 ms | about 290 ms |

The API ran next to the benchmark, so PR #162's saved browser-to-API round trip
(one gateway RTT in production) does not show here; Cloudflare's calls dominate.
Pull requests measured directly (refused pulls; no source can connect here):
one track 241 ms, three one at a time 661 ms, three in one batch 255 ms, and each
answer (`renegotiate`) 180 ms. So joining a room with three people spends about
435 ms instead of 1,201 ms on post-connect signaling (one batch and one answer
instead of three of each); with one other person it is unchanged.

Modelled full join (measured segments above, plus 345 ms transport and 31 ms
roster from earlier production diagnostics; excludes microphone capture and
gateway round trips, so absolute numbers are lower than real joins):

| Version | Empty room | One other | Three others |
| --- | ---: | ---: | ---: |
| Baseline | 793 ms | 1,214 ms | 1,994 ms |
| PR #162 | 781 ms | 1,202 ms | 1,216 ms |
| Join-time pulls | 781 ms | about 960 ms | about 975 ms |
| Join-time pulls + prepared join (signed in) | about 635 ms | about 815 ms | about 830 ms |

With join-time pulls, audio from people already present needs the receive
answer (180 ms) and the receive connection's own handshake (modelled as the same
345 ms), in parallel with the microphone connection, instead of a pull (241 ms,
255 ms for three) and answer after it.

This is a model, not an end-to-end measurement: WebRTC transport cannot connect
from the sandbox. Confirm with the connection diagnostics panel in real browsers.

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
alone never renews the lease. A transient (5xx) database recheck failure on a
heartbeat or roster change sends nothing for that event and keeps the stream; the
next successful check delivers any missed revision. Denials, setup failures, three
consecutive transient failures (about 30s) and a shared-store outage still end it. There is no durable event log or second registry.

Join/rejoin waits for the selected audio processor before publication. The SSE
handshake and publication run concurrently; the published track stays disabled
(silence). After both finish, mute/deafen state synchronization overlaps the
transport handshake, and so does the lease-renewing roster request (see
"Combined join and publication"). Subscription negotiation still waits for transport. A newer dirty state is repaired before
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

### iPhone and iPad audio routes

The models run at 48 kHz. On an iPhone whose audio route ran at 24 kHz (the rate
a default `AudioContext` reported; the user confirmed AirPods were connected), a
capture graph forced to 48 kHz sent silence: the phone showed a live, running
capture and Opus sent only DTX packets (~0.6 kbps, audio level 0). On the same
phone at a 48 kHz route, DPDFNet-8 HR ran at 5.2 ms mean per 10 ms hop and was
heard normally.

On iPhone/iPad WebKit (every iOS browser, and iPadOS in desktop mode) capture
probes the route rate with a default context. When it is not 48 kHz, the capture
graph runs at the route's own rate and the worklet converts to and from 48 kHz
itself: `resampler-v1/resampler.js` (windowed-sinc, Blackman window, 16 zero
crossings, cutoff at 95% of the lower rate's Nyquist) is added to the same
AudioWorklet first, and `dpdfnet8-v2/worklet-v4.js` / `noise-v1/worklet-v2.js`
wrap their unchanged 48 kHz processing in it. A 24 kHz route carries nothing above
12 kHz, so converting up loses nothing the microphone captured. The conversion
adds about 1.6 ms (38 samples at 24 kHz) and cost about 2.4% of one core in this
sandbox. If the converted pipeline cannot start, capture sends the browser's
voice-processed track rather than failing the join, and the status names the
route rate. Capture diagnostics include `routeSampleRate`.

Other platforms and 48 kHz routes load no resampler and behave as before; the v4/v2
worklets are the v3/v1 code at 48 kHz, under new names because `/audio` assets are
cached as immutable (old pages keep the old files). A route change after joining
(connecting a headset mid-call) is not detected until the microphone is reopened.

Validation: unit tests round-trip 16, 24 and 44.1 kHz through 48 kHz with a fixed
delay and under 1% error, check anti-aliasing (an 18 kHz tone does not fold into
a 24 kHz output), and run both worklets at 24 kHz. In real Chromium at 24 kHz,
worklet-v4 sent 101 hops/s of 480 samples (48 kHz) with an echo in place of the
model and output matched input level; worklet-v2 loaded the real RNNoise and
behaved as at 48 kHz. The sandbox cannot run DPDFNet in real time (25 ms per
10 ms hop), and no iPhone check at a 24 kHz route has been made.

### UI sounds on phones

Join, leave, mute and other UI sounds play on touch devices too; the Caper sound
effects switch still turns them off. Where the browser has the Audio Session API
(Safari/iOS WebKit 17+), a UI sound played while nothing captures the microphone
sets `navigator.audioSession.type = "ambient"`, so it mixes with other apps'
audio instead of pausing their music, and follows the silent switch. Every
microphone capture (a call or the mic test) sets `"auto"` before
`getUserMedia`, and the type is never changed while a capture is live. A capture
whose track ended without release stops counting. Unit-tested with mocks only:
whether iOS keeps other apps' music playing has not been checked on a device.

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

DPDFNet-8 HR is the preferred in-channel noise-suppression filter; no engine selector
is shown. Speakers/Headphones stays fixed to natural headphone input.
Microphone/output selectors remain; output selection also applies to live and
recorded mic-test playback. Changing a device clears the old recording immediately
and disables recording during initialization. A fallback preserves the existing
tracks and clips; record a fresh sample to hear the newly active filter. Its 14.9 MB model and
runtime load lazily from the same versioned asset directory.
Use `node scripts/vendor-dpdfnet.mjs 8` to reproduce its model/metadata/licenses.
Offline authoring requires uv 0.12.23; the script runs ONNX metadata export in the
locked `audio` dependency group, without a global Python package installation.
The active status appears only after the processor acknowledges initialization.
The recovery order is DPDFNet-8 HR → DPDFNet-2 HR → RNNoise, for initialization
failure, sustained backlog or processor failure. Healthy 8 HR is never downgraded
based on CPU count or a single slow hop. Transient underruns rebuffer processed audio.
Initialization errors/timeouts and processor crashes receive one fresh worker/worklet
retry per engine per capture, shared across startup and runtime; successful recovery
does not reset this budget. Sustained backlog skips retries and advances immediately.
Worker-reported inference failures are handled separately from worklet overload.
DPDFNet initialization is bounded to 60 seconds per attempt, RNNoise worklet readiness
to 15 seconds (asset download has its own 30-second timeout). Abort stops recovery.
2 HR uses the same 48 kHz framing/DSP with its own smaller model and recurrent state;
upstream lists 2.42G MACs versus 8 HR's 7.17G (not a measured CPU guarantee).
Its 10.5 MB model loads only on fallback, reusing the existing ONNX runtime.
Reproduce with `node scripts/vendor-dpdfnet.mjs 2`; provenance is in
`apps/web/public/audio/dpdfnet2-v1/README.md`.
A downgrade lasts for the current capture; a fresh capture starts with 8 HR again.
If all local engines fail to initialize, capture rejects without publishing raw audio.
Runtime recovery keeps the existing microphone track live, with browser suppression during transitions.
If both local fallback initialization and browser suppression fail, the status
explicitly reports bypassed audio. This avoids a call-wide reconnect loop.
An unrecoverable RNNoise AudioWorklet processor error still stops the microphone. This does
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
warms up and resets state before readiness. An eight-hop backlog terminates inference
and starts fallback, rather than ending the track or building unbounded delay.
An empty output queue refills the three-hop reserve without abandoning inference.

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

Fallback validation, September 23, 2026:

- `node scripts/test-noise-fallback.mjs http://localhost:5174` runs against local
  Vite, with real model initialization and synthetic white noise. It injects an initial
  8 HR startup failure, verifies a fresh 8 HR retry, then stalls 8 HR,
  verifies real 2 HR processing/attenuation, then stalls 2 HR and verifies RNNoise.
  It verifies unchanged tracks, measurable attenuation
  (at least 6 dB), and unclipped desktop/narrow mic tooltips with Escape dismissal.
- A separate unmodified-worker Chromium run in the orb reproduced sustained overload
  (10.8 ms average inference per 10 ms hop). RNNoise then reduced synthetic noise
  energy by about 40 dB in the locally recorded test track. This is not a quality
  guarantee, physical microphone listening test, or live SFU/TURN validation.
- No configuration or migration is required; ship with the normal web deployment.

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

Native development clients are separate implementations, not wrappers around this
browser adapter. Native audio and noise-processing parity are not established by
these browser checks. See [native acceptance](#native-distribution-and-acceptance);
no production-ready native client ships from this repository.

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

The public demo on the homepage remains guest-accessible. For guests, the browser uses
`unique-names-generator` to assign a readable color-and-animal name for each visit
and keeps it through reconnects. No account or profile is needed. The guest voice
API is database-independent, but the unified browser screen requires chat storage
to resolve the demo space/channel IDs. Existing `General` seed rows retain their
IDs and stored spelling for compatibility; the API returns `general`, and new
seeds use lowercase. No data migration is required.
The API accepts guest names of 1–64 Unicode characters after trimming,
with no control, invisible or text-direction characters. Guest names are
unverified, nonunique, and not reserved.
When a valid account bearer token or browser session cookie accompanies a join, the
API uses the stored display name instead of the submitted name. Country codes
are neither stored nor shown. The web login page sends email codes and then requires a unique
username and display name. Account login is enabled when `AUTH_SECRET`, the database
URLs, and SES settings are configured. Native development account screens use the
same service with bearer sessions; their platform acceptance is tracked separately.
Each code is six uppercase characters from `ABCDEFGHJKMNPQRSTWXYZ23456789`,
excluding visually ambiguous `0/O`, `1/I/L`, and `U/V`. Codes expire after 10
minutes and have three attempts by default. A replacement code consumes the prior
active code for that email. Request limits default to 3/email/15 minutes,
5/email/day, 10/IP/hour, and 500 globally/hour; all are configurable through the
application secret.
Past a limit the API still returns a challenge ID but sends nothing, so clients
cap resends: the code step offers **Resend code** 60 seconds after each code is
sent (with an m:ss countdown), at most twice per email entry, then shows "Still
nothing? Check your spam folder, or try again in 15 minutes." **Email me a new
code** (after all attempts are used) counts as a resend. **Use a different
email** resets the count.

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
# Expected: enabled is true. Then open the homepage window in two browsers and test audio/leave.
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
return an indistinguishable synthetic challenge ID and do not call SES. Challenges
are pruned a week after creation; expired and revoked sessions are kept as sign-in
history.

External-provider middleware, callbacks, token verification, key fetching, session
hooks, and browser forwarding remain removed. Server functions retain CSRF middleware.
The production web service owns no `/api` routes; Traefik sends same-origin
`/api/*` requests directly to Rust.

### Public API: web and native clients

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
| GET | `/api/updates/native` | Unauthenticated cached, signed desktop update metadata |
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
Native desktop and mobile clients request `tokenTransport: "bearer"`
and store the returned opaque token in the OS credential vault (Keychain/Keystore),
never plain preferences or web storage. Protected routes accept either transport. Logout
revokes the same database session for every client type. Guest media capabilities
are not account credentials.

Native development account screens and OS-vault integrations live in `apps/native`;
they are not yet accepted on every physical platform. The web account flow uses
the secure same-origin cookie transport.
Arbitrary third-party browser CORS access, API keys and developer OAuth consent
are not implemented; native/server HTTP clients do not require CORS. Browser WebRTC
does not prove native audio support.

Bigint account IDs stay internal and are never exposed in profile responses.
Random 12-character alphanumeric IDs provide permanent public references without
revealing signup order or the internal sequence. Stored as `external_id` for
integrations and external references, the API returns this value as `id`, alongside
`username` and `displayName`. Usernames are globally unique, changeable handles;
changing a username or email does not change either account ID.

Messages store the author's display name as it was at send time, but history,
gateway replay, live publication and idempotent retries serve the author's current
display name and avatar (like Discord). Deleted accounts and guests keep the stored
name. Messages already on screen update on the next history load or reconnect, not
instantly; the member list and suggestions follow their own refresh.

### Desktop update metadata cache

`GET https://caper.chat/api/updates/native` returns an envelope with `manifest`
(base64 of the exact `latest.json` bytes) and `signature` (the detached base64
Ed25519 signature). It only fetches the fixed GitHub `native-latest` release;
request parameters cannot select an upstream URL. No account, new secret, database
migration, gateway change, or CDN rule is needed.

The Rust API keeps one in-memory entry for 60 seconds **per API process**. It
coalesces concurrent misses, bounds each upstream request to five seconds, and
limits the manifest to 64 KiB and signature to 1 KiB. Successful responses send
`Cache-Control: public, max-age=60` and `Age`; this cache works even if Cloudflare
bypasses `/api/*`. These headers permit HTTP caching but do not establish that an
edge or desktop client caches responses. Restarting a process empties its cache.
After a failed refresh, the API returns `502` with `no-store` and waits 60 seconds
before retrying. It does not serve expired metadata indefinitely.

Desktop release builds check about 20 seconds after launch and every minute. They
verify the envelope's signature against their compiled-in public key before
using any metadata, and fall back to GitHub's manifest/signature on site errors,
malformed data, or signature mismatch (including a rolling-release upload race).
Archive downloads and checksum verification remain unchanged. Android and iOS
distribution is unchanged. Existing desktop builds keep their old check schedule
until they install a new native release.

Manual checks use **Caper › Check for Updates…** on Mac and
**User Settings › Settings… › Updates › Check for updates** on Windows/Linux.
They share the automatic-check worker, restore a dismissed update offer, and
report up-to-date/failure results. The server's 60-second cache still applies;
healthy running clients normally detect a new manifest within about one to two
minutes of publication, not of starting the release workflow. Sleep, offline
clients, upstream outages, and download/install time can delay delivery.

Windows/Linux Settings and the Mac's up-to-date result identify the installed
version, build number and short source revision. Development builds are labelled
separately on Windows/Linux. "Up to date" means no newer **published release** is
available; it does not mean the installed app contains the latest merged code.
The release manifest's build/version/commit identifies what is available, not
what is running on a particular user's machine.

Native releases run by themselves on pushes that change the apps (see
`apps/native/README.md`, Releasing). If one did not reach testers, inspect that
**Native release** run: its "Wait for the servers these apps need" step names
the API image or Ship run that held it.

Deploy the API before publishing the native release to use the cache immediately;
either component can roll back independently because GitHub remains a fallback.
Verify the deployed endpoint returns `200`, the documented cache headers, and a
verifiable envelope; then test **Check for Updates…** on a packaged Mac build and
the manual and automatic checks on Windows/Linux. The one-minute polling/manual
action change is client-only: if the cache is already deployed, no API, web,
gateway, configuration, infrastructure, or database rollout is needed. Publish
the commit with `gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha=MERGED_COMMIT_SHA`
if no native change has released it yet, then update installed clients once.

**Desktop restart recovery:** the shared updater runs outside the install, with
null standard streams and, on Unix, its own process group. Successful swaps reopen
the new app; failed download/verification/swap attempts reopen the working install
after the original process exits. macOS uses `/usr/bin/open -n` through
LaunchServices and checks its exit status. Errors and launch requests are recorded
in `caper-updater.log`: on Linux in `~/.local/state/caper` (or
`$XDG_STATE_HOME/caper`), elsewhere in the user's temporary directory. The updater
stages relocation copies, downloads and unpacked builds in new, randomly named
0700 folders, never fixed paths in a shared `/tmp`. Local Linux subprocess
checks cover relocation, replacement/relaunch, and failure recovery, not a live
signed-release update or a macOS/Windows GUI launch. Those platform checks remain
release acceptance. Install the fixed native release manually once if the old
updater still closes without reopening; no server/configuration rollout is needed.

**Windows repeated update prompt:** relocation is now unconditional on the first
`apply` process. Comparing `current_exe()` against a canonical install path was
unsafe: Windows can return `C:\...` for the executable and `\\?\C:\...` for
the install, skipping relocation and leaving the updater inside the directory it
must replace. The regression now enters through `apply` from a canonical launch
path and an install-directory working directory, not just the relocation helper.
The old condition fails this Windows executable test under Wine; the fix passes.
Wine used a local compatibility shim for its missing `ProcessPrng` API; it is not
a shipped dependency or a substitute for Windows 11 acceptance.

Interrupted/failed attempts leave bounded, build-scoped feedback beside the app
(`.app.update-error` for a Windows Setup install). A successful update removes it;
an independently installed different build ignores it. Both desktop clients read
the optional `last_error` from the existing updater check response and offer the
installer as an alternative. Windows/Linux now use Caper's original wordmark and
collapsed, expandable release notes; only notes/error details scroll, not actions.
Web, Android and iOS do not use this updater. macOS keeps its native AppKit alert.

After merging, publish a new native release as above, then **close Caper and run
the new Windows Setup once over the existing installation** (do not uninstall or
delete user data). An old updater cannot reliably bootstrap its own fix. Check
the next higher-build release from the installed copy: one restart, updated build,
no repeated offer. Read `Get-Content "$env:TEMP\caper-updater.log" -Tail 40` if it
fails. No infrastructure, secrets, migrations, API/gateway/web changes or AI-summary
configuration are needed. Roll back through a corrected higher-build native
release; never reuse an old build number. Native Windows 11 and Mac update/relaunch
and AppKit rendering remain platform acceptance gaps in this Linux orb.

### Native distribution and acceptance

The [native clients](../apps/native/README.md) are separately implemented,
browser-free development clients. They use the existing HTTPS API and application
WebSocket, not a native-only backend. Keep the website available while native
feature parity and platform acceptance remain incomplete. No backend migration,
provider secret, production restart, or infrastructure change is required by the
native development build workflow.

**Mac CI runner quarantine, October 12, 2026:** `.github/workflows/mac-route.yml`
currently routes all macOS jobs to GitHub-hosted runners, even when
`SELF_HOSTED_MAC=on` and the local runner reports online. Self-hosted controls were
interrupted by an inspected application-memory/Force Quit dialog and, separately,
lost runner communication. Online status alone does not prove a usable desktop
session. The underlying host cause has not been established: this orb has no
connected Mac or remote shell access, and its GitHub token cannot administer
runners or repository variables. This quarantine changes no host settings,
credentials, Windows/Linux routing, or the Mac's Linux-arm routing. It neither
deploys an app nor starts a release. Routing-file changes trigger the native
workflow on `main`, so acceptance checks the actual merged policy.

To recover the machines, use the physical Apple Silicon Mac's infrastructure
checkout, with both Tart windows open and admin logged into each desktop. Keep
Caper routing off and wait for all VM jobs to become idle before recovery;
`recover` refuses to restart a VM with an active `Runner.Worker`:

```sh
cd ~/code/infrastructure
gh auth status
gh variable set SELF_HOSTED_MAC --body off --repo joswayski/caper
./scripts/mac-ci-runner.sh recover
./scripts/mac-ci-runner.sh status
```

Inspect the VM desktop, memory pressure and newest runner `_diag` logs; confirm
the OS dialog is absent, the desktop is drawn/unlocked, and both Caper listeners
are online. See [the host recovery runbook](https://github.com/joswayski/infrastructure/blob/main/docs/mac-ci-runner.md).
These host commands have **not** been run from this orb. Do not restore routing
by toggling the variable alone: a follow-up PR must remove the explicit macOS
quarantine after recovery, then validate complete ARM/Intel/iOS native suites on
the intended main revision. Keep hosted routing if that control fails.

**Linux CI runner quarantine, October 12, 2026:** the same routing workflow also
keeps `linux=false`, even when `SELF_HOSTED_LINUX=on` and the runner reports
online. In [native run 38304485868](https://github.com/joswayski/caper/actions/runs/38304485868/job/114997812443),
`ci-linux-caper-1` lost communication during checkout, before any build or test
step started. GitHub's check annotation confirms lost runner communication; no
step log is available. This is not evidence of a compiler or app failure, and
the underlying host cause remains unknown. Windows and the Mac's Linux-arm
routing remain unchanged. No host setting or service was changed from this orb;
there is no connected runner or remote host access, and runner administration
is unavailable with its GitHub token.

From the Windows PC's infrastructure checkout in elevated PowerShell, keep
Caper's Linux routing off while inspecting Hyper-V, guest memory/network health
and runner `_diag` logs. Start stopped services or the VM without restarting
running jobs using the existing recovery command:

```powershell
gh auth status
gh variable set SELF_HOSTED_LINUX --body off --repo joswayski/caper
.\scripts\windows-ci-runner.ps1 recover
.\scripts\windows-ci-runner.ps1 status
```

If the VM is running but its runner containers are stopped, run inside the VM:

```sh
sudo bash /usr/local/sbin/ci-linux-runner.sh recover
```

These commands have **not** been run from this orb. See
[the Windows/Linux recovery runbook](https://github.com/joswayski/infrastructure/blob/main/docs/windows-ci-runner.md).
Removing the explicit Linux quarantine requires a follow-up PR after physical
recovery, then complete main CI and native acceptance on the intended revision.
Leave hosted routing in place if that control fails. No infrastructure, secrets,
database, application-service deployment or client release is required to use
the hosted route; merging the routing change starts main validation, not a
release.

**Windows CI runner quarantine, October 12, 2026:** the routing workflow now
also keeps `windows=false`, even when `SELF_HOSTED_WINDOWS=on` and the runner
reports online. In [native run 38318994763](https://github.com/joswayski/caper/actions/runs/38318994763/job/115042820729),
`desktop-caper-1` lost communication during "Build and package Windows". The
check annotation confirms a runner disconnect, not a compiler/test failure;
the job log is unavailable. Earlier Windows updater success on the same runner
did not establish that it could finish the desktop build. Linux-arm routing is
unchanged. Signed release jobs already use GitHub-hosted runners, so this
quarantine does not restart or change an in-progress release.

No host, secrets, database, service or client deployment is required. The merged
routing change triggers main CI and native development builds; verify the exact
merge commit, including the Windows installer job. To recover the physical PC,
use its infrastructure checkout in elevated PowerShell:

```powershell
gh auth status
gh variable set SELF_HOSTED_WINDOWS --body off --repo joswayski/caper
.\scripts\windows-ci-runner.ps1 recover
.\scripts\windows-ci-runner.ps1 status
```

Inspect the Windows service, network/memory health and runner `_diag` logs before
restoring routing. These host commands have **not** been run from this orb; no
live Amp runner is connected, and its GitHub token cannot administer runners.
Keep the existing Mac/Linux quarantines. Remove the Windows quarantine only in
a follow-up PR after physical recovery and a complete native build on the
intended main revision; toggling the repository variable alone does not remove
it. Roll back a failed recovery to GitHub-hosted routing, not to an unverified PC.

**Live channels in the list, October 17, 2026:** after the owner tried Live now
on an iPhone, the separate Live now section is superseded on every client by
live channel rows that grow into cards in place (see the voice section above):
no "Live now" heading or "N more live" fold, no "is speaking" line (it shifted
the card), the phone strip shows the card's faces in its order, and pressing a
card opens its chat while a right-click, long press or accessibility action on
its faces opens per-person volume. Validation:

- Web: `npm run check` passes, and `npm test` passes 657 of 658 tests (the
  known Node 22 `noise-assets.test.ts` failure). Every `test-voice-controls.mjs`
  mode passes at 1280px and 390px in fine-pointer Chromium, and `CHANNEL_ROWS`
  also passes in stock (touch-style) Chromium; desktop and phone captures were
  inspected. The long press is exercised with synthetic touch pointer events.
  All of this uses a mocked API and WebRTC.
- Android: main, unit-test and androidTest Kotlin compile, the 436 JVM unit
  tests pass and lint is clean. The 14 voice UI tests passed only under a
  temporary, uncommitted Robolectric setup. `connectedDebugAndroidTest`,
  `smoke.py`, `assembleDebug` and devices were not run.
- Apple: the eight `LiveNow` unit tests pass on a Linux Swift 6.1.2 toolchain
  and every changed Swift file parses. The SwiftUI views, `ProtocolTests`, the
  parity UI tests and screenshots need Xcode and were not run here.
- Rust desktop: fmt, `cargo clippy -p caper-desktop --all-targets --no-deps -D
  warnings` and 554 tests pass; the wide and narrow `parity-voice-*` fixtures,
  including the new `parity-voice-elsewhere`, were inspected under Xvfb.

No live SFU, physical-device, touch-hardware or screen-reader validation was
done, and there is no API, protocol, configuration or migration change.

**Live now, October 17, 2026:** the October 11 header voice below is
superseded on every client by the sidebar's Live now (see the voice section
above). Validation:

- Web: `npm run check` passes, and `npm test` passes 655 of 656 tests. The one
  failure is `noise-assets.test.ts`, which needs Node 24's WebAssembly; this orb
  has Node 22, and the test fails the same way on unmodified main.
  `test-voice-controls.mjs` passes in every mode at 1280px and 390px in
  fine-pointer Chromium, and `CHANNEL_ROWS` also passes in stock (touch-style)
  Chromium. All of this uses a mocked API and WebRTC.
- Android: Kotlin and androidTest compile, the 430 JVM unit tests pass, and lint
  is clean. The ten instrumented voice UI tests passed only under a temporary,
  uncommitted Robolectric setup, which checks behaviour but not screenshots.
  `connectedDebugAndroidTest`, `smoke.py` and devices were not run.
- Apple: the Swift sources parse and the six new `LiveNow` unit tests pass on a
  Linux Swift 6.1.2 toolchain. The views were type-checked only against a
  SwiftUI stand-in. Run the parity UI tests (rewritten for Live now) and both
  parity screenshot scripts with Xcode before release.
- Rust desktop: fmt, `cargo clippy -p caper-desktop --all-targets --no-deps -D
  warnings` and 550 tests pass, and its new `parity-voice-*` fixtures were
  inspected under Xvfb.

No live SFU, physical-device, screen-reader or touch-hardware validation was
done.

**Header voice, October 11, 2026:** the October 3 voice rows below are
superseded. Join, Switch here and Leave for the open channel live in the
conversation header on every client; channel rows show voice only while
occupied, plus a hover/focus join shortcut on fine-pointer clients (desktop web,
macOS, Rust desktop). The voice dock with Leave stays visible whenever in voice.
Validation: web build/typecheck/lint and 584 Vitest tests pass;
`VOICE_TEST_REDESIGN=1` and `VOICE_TEST_CHANNEL_ROWS=1` pass at 1280px/390px
(and the 220px sidebar) in stock and fine-pointer Chromium (mocked
API/WebRTC). Rust desktop: fmt, clippy and 429 unit tests pass (headless egui
renders). Android and Apple changes were not compiled or run here (no Android
SDK or Xcode); run `./gradlew :app:testDebugUnitTest
:app:connectedDebugAndroidTest`, `smoke.py` and the Apple parity UI tests
before release. No live SFU, physical-device or touch-hardware validation.

**Stable channel controls, October 3, 2026:** web, Android, shared iOS/macOS
SwiftUI, and Rust Windows/Linux desktop use separate channel-name/settings and
voice rows. Owners get a permanent channel-options menu opening existing privacy
and member settings. Quiet speaking-icon
actions keep their slot through Join voice, Joining, and Switch here. Connected
channels reserve that space without a duplicate Leave action: disconnect lives
only in the voice-connected dock beside the account. Desktop name/voice rows are
32px/28px with no extra gap; mobile retains its touch targets. Empty channels
show no occupancy status; occupied rosters start collapsed.
Selecting text leaves the current call intact. Pending authorization disables
competing joins, and explicit switching still verifies fresh channel access.

Intentional platform differences: Android uses 48dp targets and count-only
occupancy; iOS uses 44pt targets; macOS and Rust desktop use 28pt/28px voice
targets. Apple and Rust desktop drop summary avatars before crowding the count
at narrow sidebar widths. Rust desktop may shorten `N in voice` to `N` at its
220px minimum; its accessible name retains the complete occupancy description.
Android's channel menu also contains Leave channel for joined account channels;
this changes membership and is separate from disconnecting voice in the dock.

Local acceptance: Android unit tests, lint and four-ABI debug packaging, Rust
desktop unit/accessibility-geometry tests and a Linux fixture render, plus web
build/tests and mocked browser interactions. Fixtures do not establish live SFU
behavior. Android smoke/device execution, Apple build/XCTest/UI rendering,
Windows execution, physical touch targets, and live multi-network voice remain
pending platform acceptance. Ship updated native binaries after those checks;
web and native releases can proceed independently without API/gateway changes,
database migrations, infrastructure apply, or new secrets.

**Apple chat confirmation/recovery correction, October 3, 2026:** Swift sends
lowercase message and gateway-command UUIDs to match Rust's canonical result IDs.
Opaque channel, author, and subscription IDs remain case-sensitive. The native
fixture now canonicalizes UUIDs too, including retry keys; echoing uppercase
request IDs previously hid invalid-message/duplicate-pending-row failures. This
complements the mobile workspace correction's pending-socket/retry protections:
transient disconnections use connection status, while genuine send/subscription
errors remain visible. Recovery cannot confirm or erase uncertain sends.

Local validation: ten compiled Swift delivery checks and five HTTP/model checks
against the disposable fixture passed; the latter stubbed Keychain/audio and drove
gateway-state callbacks manually. All six fixture tests and 295 web tests passed,
as did `npm run check`. Full Apple XCTest/UI execution and physical iPhone checks
remain pending: this Linux Swift runtime's libcurl does not support WebSockets,
and the orb has no Xcode. Native tests cover canonical HTTP/gateway confirmation
and real socket disconnect → replay → warning removal. Android, web, and Rust
desktop already generate lowercase UUIDs and require no client changes for this
correction; their native/device behavior is not established by these Swift checks.
Release updated iOS/macOS clients after platform checks; no API/gateway/web
deployment, migration, new secret, or infrastructure apply is required.

**Download a build after merge:** relevant `main` pushes run **Native development
builds** automatically. To request a new build and retrieve its artifacts:

```sh
gh workflow run native.yml --repo joswayski/caper --ref main
gh run list --repo joswayski/caper --workflow native.yml --limit 5
# Select the successful run for the intended commit, then substitute its ID:
gh run download RUN_ID --repo joswayski/caper \
  --pattern 'caper-development-*' --dir caper-development
```

Downloads need GitHub repository access and expire after 14 days. Each target has
its own directory and `SHA256SUMS`; run `sha256sum --check SHA256SUMS` on Linux or
`shasum -a 256 --check SHA256SUMS` on macOS **inside that directory**. On Windows,
compare `(Get-FileHash .\Caper-windows-x64-Setup.exe -Algorithm SHA256).Hash` with the
checksum file. `BUILD.json` records the tested checkout revision, which for a PR
is generally its merge commit. Checksums detect changed bytes, not publisher
identity. These artifacts are not a signed release or proof of voice support.

Windows CI checks the actual app's PE GUI subsystem and exercises silent
install/reinstall/uninstall, payload hashes, shortcuts and Installed apps
registration. Setup stores the app under `%LOCALAPPDATA%\Programs\Caper\app`;
the self-updater replaces that child directory, not the adjacent uninstaller.
Physical Windows checks still include opening from Explorer/Start without a
console, an in-app update/relaunch without a console, and uninstalling while
Caper is closed. These packaging checks do not validate microphone capture.
Linux's `.deb` already installs an application-menu entry with `Terminal=false`;
the generic Linux website link now opens compatibility notes instead of directly
downloading a tarball. No universal Linux installer is claimed.

**Signing and stores are separate from compiling:** PR CI receives no developer
certificates and does not publish releases or submit to stores.

- **macOS:** Apple Developer Program membership supports Developer ID
  distribution outside the Mac App Store. The Account Holder must provision a
  Developer ID Application certificate/private key. A distributable app needs
  hardened-runtime signing, notarization with `notarytool`, and ticket stapling.
  Development/ad-hoc signing does not substitute for Developer ID notarization.
  See [Apple's distribution guidance](https://developer.apple.com/developer-id/).
- **iPhone:** register the final bundle ID and App Store Connect app, then use
  the correct development/ad-hoc provisioning profile for registered devices,
  or an App Store distribution archive for TestFlight. An unsigned simulator
  `.app` cannot run on a phone, even with a paid developer membership. TestFlight
  also has review/distribution requirements; no automatic approval is promised.
  See [Apple's preparation guide](https://developer.apple.com/documentation/xcode/preparing_your_app_for_distribution).
  Release CI archives with a local ad-hoc signature (`CODE_SIGN_IDENTITY=-`,
  `AD_HOC_CODE_SIGNING_ALLOWED=YES`) to retain push entitlements without creating
  an Apple Development certificate on each fresh runner. This intermediate
  archive is not a phone-installable release. Only export uses the App Store
  Connect key and automatic cloud-managed distribution signing/upload; do not
  disable signing or clear entitlements on the archive. Existing development
  certificates do not need to be revoked for this CI path.
  The app and `CaperNotificationService.appex` use the same `MARKETING_VERSION`
  and `CURRENT_PROJECT_VERSION`; XcodeGen defaults otherwise give the extension
  literal `1.0` / `1` versions and cause ITMS-90473 warnings. The upload script
  checks both built Info.plists before export and refuses mismatched versions.
  Correcting an already uploaded build requires a new binary with a higher build
  number, then confirmation that App Store Connect processing reports no mismatch.
- **Windows:** unsigned installers and executables can be distributed, but SmartScreen,
  Smart App Control, and organization policy may warn or block execution. There
  is no guaranteed per-app override. Trusted code signing may use a certificate
  authority or a signing service; a monthly service is not the only option.
  See [Microsoft's Smart App Control guidance](https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/overview).
- **Android:** a debug-signed APK is only a development download. Fresh CI runners
  generate different debug keys, so replacing a prior CI build may require
  uninstalling it and losing local state. A stable, protected release keystore
  and Play App Signing/upload-key setup are needed before a supported upgrade or
  store channel. Do not commit keys or reuse a public debug key for a release.
- **Linux:** the `.deb` and archive target the documented native library baseline.
  Test installation on a clean supported distribution; a CI build does not prove
  all distributions, Wayland compositors, audio servers, or credential vaults work.

Before creating a signed release pipeline, keep credentials in a protected GitHub
environment with required reviewers, limit signing to reviewed commits, and
import certificates into temporary build keychains. Never provide signing secrets
to pull-request code or send certificates/private keys in a thread. Publishing,
store submission, and changing repository environments require operator approval.

**Locked-screen calling remains a required acceptance gate.** An iOS background
audio declaration or an Android foreground-service permission alone does not
implement a call. A native media engine must own real capture/playback independently
of a view, authenticate Caper's SFU signaling, renew call/TURN leases, and release
the microphone immediately on hangup. iOS must configure an appropriate audio
session and handle interruption/route changes. Android must start a microphone
foreground service while the app is eligible and permission is granted, display
its ongoing notification, and handle focus/routing/service teardown. Neither a
simulator nor browser device emulation proves those behaviors on physical phones.

Record each OS/version/device, exact commit, network/forced-TURN conditions, call
duration, lock/background behavior, battery/resource observations, mute/deafen,
Bluetooth/wired route changes, interruptions and recovery before closing this gate.
Incoming direct-call notifications/ringing are not present in Caper's backend and
are not implied by comparing ongoing channel calls with Discord.

### Native generated-speech acceptance

On 2026-09-24 at 21:02:26–21:02:35 UTC, the opt-in Linux native
`generated_speech_public_sfu_two_native_sessions` test passed in 8.80 seconds
against the live General SFU. Source was merged PR148 plus the follow-up test
harness. No server deployment was performed for this test.

- Two owned native sessions, isolated PulseAudio with only two null sinks and
  their monitors, generated flite speech, and no physical microphone/speaker.
  Playback used a different sink from capture to prevent feedback.
- Actual native gain/denoise/contour publication peaks: 22,947 on each client
  over 240/239 observed frames. Actual remote decoded PCM peaks: 22,591 and
  21,505 over 278/183 frames. These are measurements, not packet-count proxies.
- General was empty before joining. Both clients stayed muted until owned-only
  subscription checks completed. A roster watch stopped on unexpected people
  or roster uncertainty; no unrelated participant was observed. The whole
  join/subscribe/speech operation was bounded to 20 seconds, with no retry.
- Local capture and transport closed before assertions. Both remote leave
  requests acknowledged, and a final presence read found neither owned client.
  The private audio service was stopped after the test.

This establishes two-way native speech through the SFU for this Linux virtual
audio setup. It does not explain the earlier silent-input zero-RTP observation,
or establish physical quality, Windows/Apple/phone capture, forced TURN,
multi-network, sustained, Bluetooth, lock-screen, or interruption acceptance.
The test is ignored by default and requires `CAPER_SPEECH_SFU_SMOKE=authorized-20s`
and its exact private PulseAudio socket. Each public run needs explicit approval;
do not run all ignored tests as a batch. Local roster/probe regressions run in CI.

### Windows microphone access and firewall prompts

Windows desktop capture uses the pinned WebRTC WASAPI implementation's
**shared mode**. A Discord call does not normally prevent Caper from using the
same microphone. Caper cannot bypass Windows microphone privacy settings or
another application's exclusive access. If initialization/start fails, check
**Settings → Privacy & security → Microphone → Microphone access / Let desktop
apps access your microphone**, or select an available input in Caper. Disconnect
from Discord temporarily to isolate a sharing/driver problem; muting Discord
may leave capture open. The generic failure does not establish a Discord conflict.

Earlier Windows builds failed every call and microphone test with "Microphone
could not start", whatever the device or privacy setting. The capture track
asks for echo cancellation, so WebRTC switched the capture ADM to Windows'
built-in AEC (the Voice Capture DMO) whenever that DMO existed, which is on
nearly every PC. WASAPI then refuses `StartRecording` unless the same ADM is
already playing, and Caper captures on a factory whose playout stays off, with
speakers on another. The vendored `AdmProxy` now reports the built-in AEC as
unavailable on Windows and refuses to enable it, so WebRTC keeps its software
AEC. Verify on a Windows PC: Settings › Audio test records, and a voice join
connects with the microphone live.

**System default** means the normal Windows input/output default, including at
startup, replay, and fresh microphone reopen. WebRTC's implicit default is the
communications default, which can be a different or unavailable device. An
explicit saved device never falls back to another microphone. Failed capture
startup closes the publication gate and reports whether initialization or start
failed; retry must not be treated as an already-running microphone. A failed
input switch restores the previous route's live/muted intent, while keeping old
capture epochs fenced until the fresh peer is ready.

Before the desktop asks the media API to join, it checks that a saved
microphone or speaker still exists, without opening it. A saved device this
computer no longer has fails Join with an error that names the device and the
fix, without listing the person in the call. Other device failures surface
after joining and leave at once. Leaves after a failed setup or a call use
their own connection and retry transport failures, because the joining
runtime and its pooled connection may already be shutting down. Earlier
builds could lose that leave, so a device failure left a participant on the
roster until its 45 s lease expired while the app offered Join again, and
repeated attempts listed the same person several times.

Windows Firewall may ask to allow `caper-desktop` when voice or an audio test
first gathers WebRTC ICE candidates. Both calls and the local test's private
peer pair open network sockets. Test recordings stay on this machine and are
never uploaded, but the test is not socket-free. Firewall permission is separate
from microphone access and cannot fix a WASAPI capture failure. For a trusted
home network, allow **Private networks**; leave **Public networks** unchecked
unless voice is also needed on networks Windows classifies as public. Windows
can classify a home connection as public, so check the active network profile.
Cancelling can create a block rule; review Caper's existing rules in Windows
Firewall if voice later cannot connect. Caper does not change firewall rules or
disable security prompts automatically. The installer keeps a stable executable
path across updates, but portable copies at new paths can prompt again.

**Publisher: Unknown** reflects the unsigned Windows executable. Adding version
metadata does not replace trusted Authenticode signing, and signing does not
itself grant firewall or microphone access. See Microsoft's
[application firewall rules](https://learn.microsoft.com/en-us/windows/security/operating-system-security/network-security/windows-firewall/rules#applications-rules).

Windows acceptance still requires distinct normal/communications defaults,
explicit input selection, mute/reopen, concurrent Discord capture, denied mic
permission, and fresh firewall-rule checks on private/public profiles. Linux
unit tests and virtual-device checks do not establish Windows behavior. Static
`parity-voice-error` and `parity-audio-error` fixtures check error presentation
without capturing audio or opening a transport.

### Account diagnostics visibility

`DEBUG_USERS` is an optional, server-only comma-separated username allowlist for
client diagnostics. Entries are trimmed and compared as exact, case-insensitive
usernames; blanks disable the allowlist, and wildcards or substring matches are
not supported. Authenticated own-account payloads from email verification,
`GET /api/account/me`, and `POST /api/account/profile` include
`debugEnabled` (default `false`). Public rosters and other users' account data
never include it. Because usernames can change, eligibility follows the current
username and can transfer; use immutable public account IDs in a future design if
eligibility must remain attached to a person.

After merge, preserve every existing property while adding `DEBUG_USERS` to the
existing `production/apps/caper` Secrets Manager record, then deploy the merged
API and web images. No database or vendor is required:

```sh
MERGED_SHA=REPLACE_WITH_FULL_40_CHARACTER_MERGE_SHA
gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
kubectl -n default rollout status deployment/caper-api --timeout=15m
gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
kubectl -n default rollout status deployment/caper-web --timeout=15m
```

For later allowlist changes, update the same record without deleting other keys
and restart the API so it reloads configuration from Secrets Manager:

```sh
kubectl -n default rollout restart deployment/caper-api
kubectl -n default rollout status deployment/caper-api --timeout=15m
```

Open tabs re-read the own-account response when they become visible again or
when **User Settings** opens, so affected users need neither a reload nor a new
login after the API restart. Do not expose `DEBUG_USERS` through web configuration or a `VITE_`
variable.

Eligible accounts see **User Settings → Audio diagnostics**, plus expandable
diagnostics inside **Mic test**. Reports distinguish no capture, opening, failed,
active and ended capture; include actual processor/fallback status, capture
settings, DPDFNet hop counts and mean/max processing time against its 10 ms budget,
and connection statistics when available. The report also carries raw and
processed track state, peer/ICE states, whether the microphone sender is attached,
roster/subscription counts, per-stream RTP packet counts and audio levels, and
the playback context state with any outputs blocked on a user gesture. Counters are local and reset on refresh.
Copying is explicit; no audio, device IDs, credentials or automatic diagnostic
uploads are included. This controls UI visibility, not access to privileged server
data. OpenFeature is an evaluation API/provider standard, not a required database
or service; this single allowlist intentionally has no flag SDK or table.

### Mobile workspace behavior and validation

At narrow widths, web, Apple and Android navigation uses an inset, rounded channel
surface and one account bar below both the space rail and channel column. The rail
has no full-height divider. Member panels have an explicit Close action and an
outside-tap backdrop; Android Back closes the visible panel before navigation.
Long member lists scroll without losing the Close action. Desktop/tablet layouts
keep their existing columns; Rust desktop is unchanged by this mobile pass.

Apple custom dialogs are siblings of the disabled/accessibility-hidden workspace,
and only their background consumes empty-space taps. Exact-username entry disables
autocorrection/capitalization and supports keyboard submission on web, Apple and
Android. Web and Apple guard pending submissions; Android's IME action respects its
existing busy state. Android workspace content also applies IME padding so the
composer can move above the software keyboard.

Apple's gateway now keeps one owner for authorization/socket setup and reconnect
backoff. Adding another subscription while authorization is pending cannot replace
the connection task. Failed sockets are cancelled before retry. Transient transport
errors use the existing connection status instead of becoming persistent chat or
member error banners; genuine subscription/history/send errors remain visible.
This does not establish the cause of every production disconnect or validate
cellular/Wi-Fi transitions.

| Platform/check | Coverage for this mobile pass |
| --- | --- |
| Web, Chromium desktop/narrow/short viewport | Executed mocked API/gateway checks for exact-username add/Enter/pending protection, dialog geometry, navigation/account alignment, member Close/outside dismissal and scrolling; inspected rendered states. Not iPhone Safari or touch evidence. |
| Apple iOS simulator/macOS ARM | CI run `36819347315` passed: all-800 bundled-avatar lookup, representative SwiftUI colored-pixel rendering and pending-connection regression. iPhone 16 simulator parity passed 17 tests, including editable exact-username add, Close/outside dismissal and navigation bounds; inspected native captures. This is fixture/simulator evidence, not a physical iPhone or production-network check. |
| Apple macOS Intel | Bundled-avatar lookup passed, but the SwiftUI pixel test crashed hosted Metal initialization. Follow-up uses CPU rasterization of the same resolved NSImage on Intel, retaining SwiftUI rendering on iOS/ARM; the follow-up requires native CI. No native toolchain is available in this Linux orb. |
| Android | CI run `36819347315` built the APK and passed six instrumented tests, including all-800 resource lookup and representative VectorDrawable pixels. Smoke reached real Compose send/HTTP/gateway delivery, then failed an old exact-author expectation missing `avatarId`; the expectation now includes the saved ID without weakening assertions. Full follow-up smoke and physical keyboard/Back/navigation checks remain required before release. |
| Rust desktop | No UI changes. Root Rust tests/format/Clippy ran; these do not validate native mobile clients. |
| Production networks/voice | Not exercised. Sustained voice, background/resume and Wi-Fi/cellular/TURN checks remain separate acceptance gates. |

Run the focused browser regression against local Vite with
`MODALS_ONLY=1 SPACES_TEST_WEB_URL=http://localhost:31740/spaces node scripts/test-space-controls.mjs`.
The native parity fixture now supplies saved avatar IDs in account, member, chat
and voice responses rather than testing only initials.

### Client performance and motion validation (October 5, 2026)

Windows/Linux chat no longer deep-clones every loaded message, author, text and
reaction on each UI frame. It retains the same variable-height rendering and
paging anchors; layout work still grows with loaded history. This is an allocation
reduction, not full timeline virtualization or a measured FPS/battery claim.

Web speaking indicators allocate no sampling loop for missing/muted streams or
failed analyser setup. Visible indicators sample every 32 ms rather than wake on
every display frame. Hidden tabs stop UI sampling and clear speaking rings without
stopping capture/playback; visibility restores sampling. The RMS threshold and
180 ms release are unchanged. Chat timestamps share one formatter per render,
retaining locale/timezone refresh. Audio popovers/profile dialogs have a 140 ms
opacity/4px entrance, disabled by reduced motion; typing labels fade/translate
without a blur filter. Incoming messages no longer consult a delayed bottom-state
callback to force scrolling; Virtuoso owns append-follow, while local sends still
reveal the pending message.

| Platform/check | Coverage for this pass |
| --- | --- |
| Web | Production build and 340 tests passed. The 3,000-message browser regression passed rapid/settled incoming delivery, prepend anchors, retry, narrow layout, own sends and empty history, with nine rows mounted at latest. Other Chromium fixture checks covered desktop/narrow popover bounds, dialog focus, normal/reduced motion, typing appearance/removal, and synthetic-audio indicator cleanup. Inspected 1440×900 and 390×844 captures; narrow emulation is not physical touch or Safari evidence. |
| Windows/Linux desktop | Linux: 192 tests passed, nine opt-in tests ignored; format and Clippy passed. The added render regression retains authoritative messages/reactions/cursor across 1440/840/390px widths. No Windows execution or live SFU/hardware-audio validation. |
| Apple/Android | Source audit only; no changes in this pass. Apple still forcibly follows incoming messages, and Android lacks explicit latest-message scroll ownership. Safe reader anchoring, tall/pending rows and platform UI tests remain follow-up work. |

An orb Chromium microbenchmark of 100 renders × 30 timestamps took 514 ms with
one formatter per message versus 23 ms with one per render; four rendered fixture
timestamps matched independently formatted expectations. These numbers describe
only formatting work, not end-to-end app speed. Desktop history virtualization,
native reader-aware scrolling, and physical-device resource/voice checks remain
separate work. No server, migration, infrastructure, or secret changes are needed;
web and Windows/Linux client releases can deploy independently. Roll back their
respective image/client build if needed.

With local Vite running, repeat the long-history check using
`CHAT_TEST_WEB_URL=http://localhost:5174/spaces node scripts/test-chat-history.mjs`
(substitute the configured port). Its account/history/gateway are explicit test
mocks; no production data or SFU is used.

### Quiet conversation timelines (October 8, 2026)

Web desktop/narrow, Apple and Rust desktop channel/thread timelines hide scroll
indicators while retaining scrolling. Android's existing `LazyColumn` timelines
already omit indicators. Web keeps Virtuoso's reader-aware incoming-message follow
and explicit own-send reveal; delivery does not focus the composer. Composer resize
corrects the bottom position only when its height changes and the reader was within
80 CSS pixels of the bottom before resizing, using the current DOM scroll extent
rather than delayed virtualizer measurements. Historical context jumps retain their
existing no-follow guard. Pins retain their existing indicators.

| Platform/check | Coverage for this pass |
| --- | --- |
| Web | `npm run check` and all 470 tests passed. Chromium's 3,000-message history, thread and Pins regressions passed desktop/narrow scrolling, hidden-indicator style checks, composer growth, retained focus, reader/prepend anchors, PageUp/End, Pins return frames/context navigation and own sends. Inspected latest/history/thread captures at 1280×800, 1440×900 and 390×844; these use explicit test fixtures, not production data. Newest-content checks measure the visible text, not the virtualizer's estimated empty end padding. |
| Rust desktop | Linux: 286 tests passed, nine opt-in tests ignored; format, application Clippy and build passed. Inspected a 2× static channel/thread fixture. No Windows execution or live gateway/voice validation. Root workspace tests, format and Clippy also passed. |
| Apple/Android | Apple indicator changes are source-only here; no Apple toolchain or native device run in this Linux orb. Android is unchanged. Native follow policies are unchanged, including the reader-aware follow limitations noted above. Physical phones, software-keyboard transitions, touch scrolling and Safari acceptance remain required. |

No infrastructure, secrets/configuration, database migration, API or gateway rollout
is needed. Deploy the web image and release Apple/Rust desktop clients independently;
no Android release is needed for this change. Before distribution, verify native
scrolling and incoming messages on each changed platform. Roll back the web to its
previous image; native rollback requires a corrected higher-build release.

### Fixed wordmarks and app icons, rotating illustrations

Clickable app identities (Dock, launcher, taskbar/window, installed shortcuts and
package icons) and every client wordmark use the original plain green Caper.
Browser favicons and playful file-drop illustrations still choose one of the 800
bundled v3 characters per UTC day, remember it locally, and exclude the preceding
choice. This does not change account avatars. Rotation runs on launch/resume or
while the client is running; it is not an exact midnight job while closed,
suspended or throttled. There are no alarms,
background services, icon-change prompts or new dependencies.

Wordmarks never change on navigation, hydration, or daily rollover, including the
homepage, sign-in, loading, empty and error screens. Web shares one React daily
selection between the favicon and file-drop illustration (including blocked
storage), not the wordmark. Native clients retain their existing local selection
keys for illustrations only; those keys no longer replace app icons or wordmarks.
Apple refreshes its daily selection when the file-drop illustration appears,
resumes or remains open across rollover. iOS and standalone web rotate only the
illustration, leaving their packaged/installed icons original. Different
installations may choose different characters. Native test fixtures use design 0
for deterministic illustrations and the original green Caper for wordmarks.
The [character catalog](../assets/avatars/README.md) names all 100 designs and
explains their eight hues; it does not add mascot reactions to the chat protocol.

| Surface | Behavior and validation |
| --- | --- |
| In-app wordmarks | Web headers/forms always compose dot-free lettering with the original green face; Rust desktop and Apple/Android reuse their bundled original full wordmark. Clock-controlled Chromium checks cover fixed server markup, navigation/reload, rollover, other tabs, unavailable storage, standalone branding, decorative images and desktop/narrow layouts while favicons continue rotating. Apple/Android builds and rendering need native/device acceptance; browser checks do not verify them. |
| Desktop/mobile browser tabs | SVG plus generated 32/192px PNG favicons; localStorage persists the daily choice across reloads and tabs. Checks every minute and on focus/visibility/storage events. Clock-controlled Chromium tests cover UTC rollover, timer refresh, same-day reload/SPA navigation, shared-tab assignment, PNG/SVG pixels and desktop/narrow asset inspection. Safari/Firefox and physical mobile browsers remain unverified. |
| Website home-screen shortcuts | Dedicated original-mascot Apple touch and manifest PNGs at 180/192/512px. Explicit standalone launches skip favicon rotation. Browsers/OSes cache installed icons; no reliable scheduled refresh is available. The manifest uses `display: browser`; no offline service worker or background capability is added. Existing shortcuts may need re-adding, and browser-specific icon selection needs device validation. |
| Native iOS | Original packaged dark-green icon; no alternate-icon calls or alerts. No iOS icon behavior is changed. |
| Native Android | Application/default launcher and all 800 legacy aliases use the original green mascot. Keep alias component names and enabled state for existing entries and pinned shortcuts; the app no longer switches launcher components. An instrumented fixture test covers a same-day upgrade state and next-day branding-only rotation without changing the enabled entry; it requires Android CI/device execution. Signed upgrade/rollback and OEM icon caches remain device acceptance gates. |
| Native macOS | Dock/Finder/package icons use the original AppIcon; no runtime `applicationIconImage` replacement. Swift daily-choice tests cover only illustrations. Bundle compilation and a real Dock check require Apple CI/Xcode/macOS, unavailable in a Linux orb. |
| Rust desktop | Runtime window icons retain the bundled original-mascot PNG; installer, pinned-shortcut and Wayland package identities also stay original. Daily selection/persistence affects only file-drop illustrations, not wordmarks. Earlier X11/Openbox checks confirmed the original window icon across fresh launch and restart with saved character 799. Windows taskbar/Wayland shell caching need platform acceptance. |

Deterministic native parity/Android fixture runs retain the original icon. The web
test uses a labelled clock/storage fixture, not a physical home-screen test:
`node scripts/test-favicons.mjs http://localhost:31095 .amp/in/artifacts`.
Static fallback exports use `node scripts/generate-favicons.mjs` (ImageMagick 7 and
librsvg); launcher declarations use
`node scripts/generate-android-launcher-aliases.mjs --check`. Native wordmarks reuse
their already-bundled original artwork. `node scripts/generate-wordmark.mjs`
exports dot-free lettering to web/Rust, Apple and Android without changing the
original full-logo source; `--check` runs in web tests to detect drift.
Docker is unavailable; web production
build stages and the Rust desktop build were checked directly. Application-only
desktop Clippy passes with `--no-deps`; full native-workspace Clippy still fails on
existing vendored WebRTC safety-documentation warnings.

#### Stable-wordmark rollout order

1. No infrastructure apply, secrets/configuration, database migration, API,
   gateway or SFU rollout is needed. Existing daily preferences and saved profile
   avatars remain untouched. Web and native clients can deploy independently;
   merging does not deploy or distribute them.
2. Set `MERGED_SHA` to the full merged revision. Once its immutable AWS web image
   is available, deploy through the existing operator workflow:
   ```sh
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # After the deployment workflow succeeds:
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```
   Check home → spaces → home, sign-in, and space loading/empty/error screens at
   desktop and narrow widths. Every wordmark keeps its original green mascot
   while browser favicons and file-drop illustrations can keep their daily character.
3. Before native distribution, build/test with the Android SDK, Xcode, and the
   Rust desktop toolchains. Verify original wordmarks and rotating file-drop
   illustrations on Android, iOS/macOS, and Rust desktop, including existing
   saved daily choices. After separate release approval, publish with:
   ```sh
   gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$MERGED_SHA"
   ```
4. Roll back web with the same deployment workflow and the previous known-good
   revision. Native rollback requires a corrected higher-build-number release.
   No database or infrastructure rollback is needed. No deployment or release
   was run for this wordmark change.

#### Fixed-app-icon rollout order

1. No infrastructure, secrets/configuration, database migration, API/gateway or
   SFU deployment is needed. No web deployment or iOS release is needed for this
   revert: web favicons, installed website icons and the iOS AppIcon are unchanged.
   Saved profile avatars and vector artwork stay unchanged.
2. Require native platform builds/tests before release. Run the Android fixture
   launcher regression on API 33+ and a supported older version. Test a signed
   upgrade from a rotating build with an enabled avatar alias, including an
   existing pinned shortcut; check that one clickable original-mascot entry
   remains. Verify the running macOS Dock and Windows/Linux window/taskbar icons,
   including an installation with an existing daily-character preference.
3. Release Android, macOS and Rust desktop through the existing native process.
   Platforms can roll out independently; merging does not publish a release.
   After approval, use the Discord release button or run:
   ```sh
   MERGED_SHA=REPLACE_WITH_FULL_40_CHARACTER_MERGE_SHA
   gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$MERGED_SHA"
   ```
   The workflow packages all native targets; unchanged iOS needs no icon-specific
   distribution. Install/update each affected client through its normal channel,
   quit/reopen, and verify the plain green icon while in-app characters still
   rotate. Shell caches may require refreshing the shortcut or signing out/in.
4. Roll back only the affected native release using the same approved workflow
   with the prior known-good revision and a newer build number. No service,
   database or configuration rollback is needed. Test Android enabled aliases
   and pinned shortcuts across rollback; returning to a rotating build restores
   its old icon behavior. No deployment or release command was run for this revert.

### Saved default avatars

An avatar is the user's profile picture, not a separate cosmetic identity. The
mascot is the default picture everywhere: account, members, chat and voice.
`users.avatar_id` is a persisted smallint (0–799), assigned by PostgreSQL once on
account creation. Migration `202609300001_user_avatars.sql` also assigns existing
accounts a random default. Login, username/display-name edits and voice rejoining
do not reroll it. Assignments are not exclusive: people can share an avatar.
The field identifies an actual bundled image, not a hash of a name/account ID.

The collection contains **100 distinct designs × eight hue treatments**. The v3
artwork uses real SVG paths, not embedded bitmap images. It repairs 60 of the
original v2 traces with restored hat outlines, accessory details and contrast;
the other 40 designs retain their exact v2 artwork. The ID mapping stays
character = ID % 100 and colorway = floor(ID / 100), so existing assignments retain
their character/color.
The old v1 atlases and v2 SVGs remain unchanged for older web clients.

Repaired vector masters live in `assets/avatars/vector-v3`, with unchanged designs
falling back to `assets/avatars/vector-v2`. Hand-authored details use filled paths
so Apple, Android and Rust receive the same geometry. The original offline tracing
command `uv run scripts/vectorize-avatars.py` requires ImageMagick 7 and pinned
VTracer; do not rerun it against a published version. To reproduce client exports,
run `node scripts/generate-avatar-vectors.mjs`; `--check` detects drift and runs in
web tests. Web and Rust desktop use `public/images/avatars/v3/{id}.svg`, Apple uses
a preserved-vector asset catalog, and Android uses native VectorDrawables. Native
resources are bundled for offline use. There is no runtime generator, image
service, R2, new secret, or database change for the vector upgrade.

Web requests only the visible SVGs. Versioned v2 and v3 paths get
`Cache-Control: public, max-age=31536000, immutable`; publish changed artwork at a
new versioned path, never overwrite cached URLs or reroll IDs. R2 is unnecessary
for this fixed collection. Future uploaded photos/GIFs should use object storage
with validated media references and separate upload/access/cache policies; storing
an object in R2 alone is not a CDN caching configuration.

Account/member/chat author responses carry `avatarId`. HTTP history, both gateway
replay protocols and outbox publication enrich old messages from the current user
record without rewriting the message log. Voice snapshots carry the authenticated
account's saved avatar, not a client-supplied ID. Existing shared voice state is
backward-compatible: absent IDs render initials until the participant rejoins.
Web, Android, Apple (iOS/macOS) and Rust desktop use the same saved design. Missing or
invalid IDs render initials; presence dots and speaking rings remain separate.
Apple also resolves the platform image from the framework/package resource bundle
explicitly, preserves original colors and shows initials if asset lookup fails.
The reported blank iPhone avatars have not been reproduced on a device; the new
native resource/pixel tests must run before treating that report as resolved.

People can replace the mascot with their own picture, including animated GIFs;
see [Custom profile pictures and space icons](#custom-profile-pictures-and-space-icons-october-14-2026).
A custom picture replaces the displayed mascot everywhere rather than adding a
second picture, and never changes `users.avatar_id`: removing it restores that same
saved mascot in account, member, chat and voice views, never a new one or initials.
The fixed mascot collection stays bundled; there is still no gallery or public
collection page, and arbitrary image URLs are never accepted.

#### Saved-avatar validation

Validation: migration/backfill/profile stability and repeat email-login tests run
against disposable Postgres. Chat tests cover missing-avatar historical payloads,
outbox and gateway replay; voice tests check trusted assignments and old Valkey
state. `node scripts/test-avatars.mjs http://localhost:30701 .amp/in/artifacts`
checks real browser components with a labelled API/gateway mock, including desktop
and narrow layouts, SVG selection, presence and profile rename. Normal screenshots
give every account a saved picture, including existing accounts. A separate
incomplete-response test checks initials; it does not represent migrated accounts.
This browser fixture is not live signup, SFU, physical phone or native evidence.
The vector upgrade additionally decodes all 800 SVGs in Chromium, checks alpha
outside the circular crop, and provides enlarged/100-design review captures.
The v3 tests check exactly 60 repaired designs across all eight hues, preserve
byte-identical artwork for the other 40, and sample rendered beanie/cap/visor
pixels to catch hidden or misplaced paths rather than only checking SVG markup.
Rust desktop's SVG rendering and voice-stack interaction tests pass.
The desktop suite runs with `--test-threads=2`; device/integration tests remain
ignored. Leaving/deleting a space clears private conversation
state immediately, without a retired public-General fallback.
Desktop application clippy passes with `--no-deps`; unrestricted clippy fails
on existing warnings in vendored `webrtc-sys`.
Android and Apple source/tests/resource packaging were updated but require
their platform build/device checks before release; Java and
Swift/Xcode are unavailable in this orb. Docker daemon is unavailable, so API/web
build stages were validated directly rather than building container images.

#### Vector artwork v3 deployment order

1. Prerequisite: the saved-avatar API/migration rollout below is already complete.
   No infrastructure, R2, secrets/configuration, database migrations, API or gateway
   deployment is needed for the vector upgrade. Do not alter saved assignments.
2. Once the merged commit's web image is available, deploy web explicitly (merging
   alone does not deploy). Wait for the workflow to finish, then verify rollout:
   ```sh
   MERGED_SHA=REPLACE_WITH_FULL_40_CHARACTER_MERGE_SHA
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   curl -I https://caper.chat/images/avatars/v3/0.svg
   ```
   Expect SVG content and a one-year immutable cache header. Check existing
   account pictures in chat/member/voice views and at browser zoom; IDs must not
   change. The old v1/v2 image URLs remain available to previously loaded clients.
3. Release Android, iOS/macOS and Rust desktop independently through their normal
   client workflows after platform builds and visual/device checks. Clients bundle
   their resources; they do not depend on web deploying first.
4. Roll back only the affected web image/client release if necessary; leave the
   database untouched. No shared deployment or database writes were run here.

#### Original saved-avatar deployment order

1. No infrastructure, R2, new secrets or configuration changes. Wait for immutable
   API/web images for the merged commit. Merging does not deploy. Keep existing
   database/migration credentials and shared Valkey configuration; do not clear it.
2. Deploy API first. Normal startup applies the additive migration and runtime
   grants. Its volatile random default backfills existing rows under a table lock;
   allow a maintenance window if the account table is large. Wait for the workflow
   to succeed before checking that the new image rolled out:
   ```sh
   MERGED_SHA=REPLACE_WITH_FULL_40_CHARACTER_MERGE_SHA
   gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-api --timeout=15m
   ```
3. Deploy gateway after the migration; its replay queries require the new column.
   Wait for this workflow to succeed and confirm the new image:
   ```sh
   gh workflow run deploy-caper-gateway.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-chat-gateway --timeout=15m
   ```
4. Deploy web, then release native clients through their normal release workflows.
   Client releases are independent after server rollout; old clients ignore the
   additive field and new clients retain initials against older servers.
   ```sh
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```
5. Verify an existing account and a new signup have saved `avatarId` values in
   `/api/account/me`. Check matching chat/member/voice pictures, including old
   history, reload/login, profile rename and voice rejoin. Verify native packaging
   includes the atlas and run device UI checks before publishing those releases.
   Rollback images independently if necessary; **leave the additive column and its
   assignments intact**. Old code ignores it. Do not drop/recreate the column or
   regenerate assignments. No shared migration or deployment was run during development.

### Login email abuse protection

The API rejects reserved `example.com`, `example.net`, `example.org`, `.example`,
`.test`, `.invalid`, and `.localhost` email domains, including subdomains, before
creating challenges or calling SES. This is not a mailbox-existence check: fake
inboxes on real domains can still bounce. Login copy and web/native sign-in stay
unchanged; no CAPTCHA or client exemption is required.

Caper enforces durable email/IP/global limits before sending: defaults are three
per email in 15 minutes, five per email per day, ten per source IP per hour, and
500 globally per hour. These are application limits, not SES quotas. SES sending
quotas limit capacity; they do not validate recipients or prevent every bounce.
SES account-level suppression, not a Caper database list, handles suppressed
recipients. A bounce alarm alone does not mean the SES account is under review.

#### Deployment order for login abuse protection

1. Merge and wait for the merged commit's immutable API image to publish. No new
   infrastructure, secrets/configuration, or database migration is required.
   Existing API settings and rate limits remain unchanged. Merging does not deploy.
2. From an authenticated operator machine, deploy only the API. List runs and
   select the exact run dispatched for this SHA, then wait for its success before
   checking the Kubernetes rollout:
   ```sh
   MERGED_SHA=REPLACE_WITH_FULL_40_CHARACTER_MERGE_SHA
   gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   gh run list --repo joswayski/infrastructure --workflow deploy-caper-api.yml --event workflow_dispatch --limit 5
   gh run watch REPLACE_WITH_DISPATCHED_RUN_ID --repo joswayski/infrastructure --exit-status
   kubectl -n default rollout status deployment/caper-api --timeout=15m
   ```
   No gateway/web deployment or native release is required. The Android smoke
   assertion changes affect CI only, not the shipped app.
3. Verify a reserved-domain request returns 400, then verify ordinary sign-in to a
   controlled inbox on web and native clients. Monitor SES bounce/reputation
   metrics separately. No production email or infrastructure change was tested
   during development.
4. Rollback by redeploying the previous immutable API image through the same
   workflow, waiting for that exact run and rollout as above:
   ```sh
   gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha=REPLACE_WITH_PREVIOUS_API_IMAGE_SHA
   ```
   No configuration or database rollback is needed; the previous image restores
   the previous email validation.

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

## Account-global direct messages and deferred mobile push

One-to-one DMs belong to two accounts, not to a space. Personal notes use the same
private stream with your own account as both participants. Start by exact username
(the field suggests people you share a space or DM with, as described under
spaces); the canonical pair (including a self pair) has one conversation even when started
concurrently.
The list appears below channels in every space, including accounts with no spaces.
Web pins it above the account controls; narrow web and mobile clients expose it in
Browse navigation. Leaving or deleting a space does not delete DMs. Space owners
have no special DM access. This is server-authorized privacy, not end-to-end
encryption. Group DMs, attachments, message deletion, and DM voice are not
implemented; message requests and blocking are described below. Existing text length, send limits, typing, history pagination,
idempotent sends, outbox and gateway replay rules apply unchanged.

The sidebar pins your real account name with a `you` label. Opening it lazily
creates or reopens your notes; displaying the row does not write to the database.
Other accounts cannot read, send, mark read, or subscribe to your notes. Space
owners have an **Invite people** row that opens the existing space-management
invitation form for exact usernames; members and accounts with no space get
**New message** instead. The heading's plus still starts a DM; desktop web, Rust
desktop and macOS reveal it on heading hover or keyboard focus. Non-hover web,
iOS and Android keep it visible. Neither action adds invite links or grants DM
access to owners. Self notes never send push notifications; see
[mobile push](#mobile-push-phase-1-direct-apns-and-fcm).

API contracts (account authentication required):

- `GET /api/dms` → `{conversations:[{id,peer:{id,username,displayName,avatarId},lastSeq,readSeq,status,blocked}]}`.
  `status` and `blocked` are explained under message requests below.
- `POST /api/dms` with `{username}` returns that pair's conversation. Your own
  username returns personal notes with `peer.id` equal to your account ID.
- `POST /api/dms/{id}/read` with `{seq}` monotonically advances the caller's read
  cursor, bounded to the durable channel head. Sequences are decimal strings.
- Message history/send/typing and gateway chat subscriptions use the existing
  channel endpoints with the DM ID. History sets `channel.direct=true` and an
  empty `space.id`. Only the pair can access it; media endpoints reject DMs.

Lists refresh every 15 seconds; the open conversation receives live gateway
messages. Unread means the durable head is beyond the account's read cursor.
Read state is account-wide, not device-wide. New conversations are limited to
20/minute and 1,000 per initiating account; requests other people start with you
don't count. Starting one requires a completed profile (403 `complete profile
required`). Exact usernames are discoverable by
starting a conversation, so each account may start at most 60 an hour by
username, counting lookups that find no one or a person not accepting DMs
(429 `too many conversations; try again later`); membership in a shared space is not required, but a
stranger's first DM is a message request.

### Message requests, blocking and DM privacy

Anyone can start a DM by exact username. People who share an active space get a
normal conversation. Anyone else's first DM is a **message request**: the sender
sees "Waiting for @name to accept" above an enabled composer and can keep
writing; the recipient sees one **Message requests** row with a count of
requests at the top of their DM list. A request never adds to unread dots,
badges or message sounds. Opening one shows its history read-only with
**Accept**, **Decline** and **Block** in place of the composer. Opening it does
not accept it. Choosing to message the sender (for example with **New message**)
accepts it, and so does replying from a client that predates requests. Declining
hides the request from the recipient only; the sender is not told, and later
messages do not bring it back.

Blocking is available from a DM's header, from message actions on another signed-in
account's message, and from a request. Clients confirm first. A block:

- stops DMs both ways. The blocked person can't start a DM or a request, and
  their sends in an existing DM fail with the same refusal a privacy setting gives,
  so they can't tell which it was. You see "You blocked @name" with **Unblock**
  instead of the composer.
- stops every other interaction in that DM too: reactions, pins, edits and typing
  get the same 403 refusal (`dm_not_accepted` for the blocked person, `dm_blocked`
  for you), and none of them accepts a pending request. Every client shows the
  refusal on the message and keeps the conversation open, treats a refused send as
  final (Edit or Dismiss), and hides reactions, pins and edits in a DM you blocked.
- declines any pending request from them.
- stops their space and private-channel invitations to you: inviting you returns
  the same 404 as an unknown username, and invitations sent before the block are
  hidden while it lasts.
- hides their messages wherever you share a conversation. Each run of consecutive
  messages becomes one row, "⊘ N blocked messages — Show", in channels, threads and
  DMs. **Show** reveals that run until you hide it or leave. The notice and revealed
  messages share a neutral background and leading border, ending before the next
  normal message. Their messages don't play sounds, and their typing isn't shown.
  Pins, who-reacted lists and thread participant avatars still list them.
- isn't visible to the blocked person, whose DM list doesn't change.

Unblocking keeps the record (`deleted_at`). The web Edit profile dialog and
`/profile`, and the native clients' user settings ("Privacy and blocked accounts"
on Android and Apple, Settings on Rust desktop), list **Blocked accounts** with
**Unblock** and hold the setting **Who can start a DM with you**:

- **Anyone** (default): people outside your spaces send a request.
- **People in my spaces**: only people who share an active space with you.
- **No one new**: nobody can start a new conversation. Existing ones stay open.

The same settings end with **Delete account → How to delete your account**,
which opens `https://caper.chat/privacy#delete-account` (the web uses the
same-origin `/privacy#delete-account`). Google Play requires this in-app path;
deletion itself is an emailed request handled by hand, as that page describes.

API contracts (account authentication required):

- `GET /api/dms` adds `status`: `accepted`, `outgoing` (you asked) or `incoming`
  (a request for you), and `blocked` (you blocked the peer). Declined incoming
  requests are omitted. Clients treat a missing `status` as `accepted`.
- `POST /api/dms` may now return 403 with `code` `dm_not_accepted` (their setting,
  or they blocked you) or `dm_blocked` (you blocked them), or 429 after ten new
  requests in an hour.
- `POST /api/dms/{id}/accept` returns the accepted conversation. It is idempotent
  for a participant.
- `POST /api/dms/{id}/decline` → 204. It returns 404 unless the conversation is
  a request for you.
- `GET /api/blocks` → `{blocks:[{id,username,displayName,avatarId}]}`, newest first.
- `PUT /api/blocks/{accountId}` and `DELETE /api/blocks/{accountId}` → 204,
  idempotent. You can't block yourself, and at most 1,000 accounts at once.
  `accountId` is the account ID clients already see as `author.id`, `peer.id`
  and member `id`.
- `GET`/`PUT /api/account/privacy` with `{directMessages:"anyone"|"spaces"|"nobody"}`.
- Sending in a DM returns 403 `dm_blocked` or `dm_not_accepted` while either
  person blocks the other.

The parity fixture models the same contract. `POST /__fixture/control
{"messageRequest":{}}` adds a request from Jordan (`jordan`, who shares no
space), and `PUT /api/blocks/member000001` blocks Maya, whose two seeded
`#general` messages then collapse.

### Deployment order for self notes and sidebar dividers

1. Obtain the immutable merged revision as `MERGED_SHA`; wait for CI, API/web
   images, and native artifacts. No new infrastructure, secret, configuration,
   signing setup, permission grant, or provider setup is required. Keep existing
   database, Valkey, and SFU configuration. The initial DM rollout below, including
   its migrations/runtime grants and DM-capable gateway, must already be deployed;
   otherwise complete it first. Merge does not deploy components. Back up the
   database before rollout; never reset it.
2. Deploy API first, including `202610040001_self_direct_messages.sql`. Normal API
   startup applies the migration through the existing direct `MIGRATION_DATABASE_URL`;
   do not edit an applied migration or run manual production SQL. It changes only
   the pair check from `<` to `<=`; ordering, uniqueness, and existing DMs remain.

   ```bash
   gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # Wait for that exact workflow run to succeed, then:
   kubectl -n default rollout status deployment/caper-api --timeout=15m
   ```

   Complete API rollout before exposing the new clients. Gateway needs no rollout:
   its existing membership and replay queries already accept a self pair. Keep
   mobile push deferred. Do not reset Valkey/SFU state or restart healthy voice tracks.
3. After API readiness, web and native can release independently:

   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # Wait for the web run to succeed, then:
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   # After native CI and macOS/Windows/Android/iOS acceptance:
   gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$MERGED_SHA"
   ```

   The native workflow publishes every platform using existing signing credentials.
   Reload web tabs and update native apps. The separate cursor/resize-highlight PR
   can roll out independently; these changes do not depend on it.
4. Verify edge-to-edge DM/voice dividers, the `you` row, owner invitations versus
   member **New message**, notes send/reload/reopen across spaces and with no
   spaces, existing peer DMs, cross-account denial, and no push enqueue. Browser
   fixtures and Linux rendering do not prove Apple/Android or Windows execution;
   physical native acceptance remains required before release.
5. Roll back web to a known-good client revision with the same web deployment
   command. Roll back native through a reverted main revision and a new release
   with a higher build number. Keep the migrated schema and notes data; do not
   restore `<` or delete notes. API rollback requires a reviewed build retaining
   the applied migration and keeping push deferred; older images fail SQLx ledger
   validation or reject self creation. Prefer a forward fix after notes exist.
   No production migration, deploy, or release is performed by this PR.

### Mobile push (phase 1): direct APNs and FCM

Phone push with sender avatars for DMs and channel messages is built on the server and sends
directly to APNs (iPhone) and FCM HTTP v1 (Android); there is no SNS or push
vendor. It stays off until `NOTIFICATIONS_ENABLED=true`, and each platform is
advertised by `GET /api/push/config` only when it is listed in `PUSH_PLATFORMS`
and its credentials load. With it off the endpoint returns `{"platforms":[]}`,
which keeps the native permission prompts hidden; notification levels and mutes
still save. [notifications.md](notifications.md#phase-1-as-built) holds the
contract: who is notified, the phone hold while you are active elsewhere,
payloads, routes, retries and configuration.

- Each committed account-authored message (sends and forwards, never edits)
  writes one `notification_jobs` row in its send transaction. Workers in the API
  role expand it into per-recipient notifications whenever chat is enabled.
  `NOTIFICATIONS_ENABLED` gates phone delivery, not this expansion or local OS
  notifications. Provider sends hold no database connection.
- Registrations belong to the account session: logout revokes them, delivery
  rechecks the session, and dead tokens reported by Apple or Google are revoked.
- Provider keys live only in `production/apps/caper` (and `staging/apps/caper`),
  written by `scripts/store-push-credentials.sh`; never in client builds or source
  control. Logs never contain tokens, keys or message text.
- Provider acceptance does not prove device receipt, and sent notifications
  cannot be recalled. Server tests use mock APNs/FCM servers; validate on physical
  Android and signed iOS devices before adding a platform to `PUSH_PLATFORMS`.
- Open browser tabs and running macOS/Windows/Linux apps implement local OS
  notifications from an account-wide gateway feed, independently of the current
  channel. They include sender artwork, focused-conversation suppression,
  reconnect deduplication, click navigation and logout cleanup. macOS uses an
  upload or sender-avatar attachment rather than replacing the app icon.
  Closed-browser Web Push, macOS APNs and keep-running-in-tray behavior remain
  unimplemented.
- Phone and local desktop/browser notifications can preview one image or video
  through a 15-minute signed URL: images use their existing original (up to
  50,000,000 bytes); videos use only their existing chat poster (at most 512 KiB),
  even when the video exceeds 50 MB. No video bytes are downloaded. The receiving
  device creates its own temporary PNG; notifications never upload or store
  another R2 rendition. Captions survive; missing posters, documents,
  oversized/expired media and decoder/download failures fall back to descriptive
  text. URL expiry does not erase an already-downloaded preview. Multiple message
  attachments remain supported. See [limits and client behavior](notifications.md#uploaded-file-previews)
  and [the CDN/server/client deployment order](notifications.md#uploaded-file-preview-deployment-order).

The additive `202610090002_live_notifications.sql` index supports account
notification cursors. Deploy API/migration, then gateway, then independent web
and native releases. iOS requires Communication Notifications on the parent
App ID, refreshed profiles and an embedded `.notifications` extension profile.
See [the exact operator commands and rollback](notifications.md#sender-avatars-and-desktop-deployment-order).
Merging does not deploy these components.

Sender-avatar/local-notification validation (October 9, 2026):

| Platform | Validation boundary |
| --- | --- |
| API/gateway | Rust workspace tests; 45 disposable Postgres/Valkey tests and two account integration tests pass, including real gateway reconnect, private-grant/session revocation, privacy/settings filtering and current avatar delivery to mock APNs/FCM. Provider mocks are not real device receipt. |
| Web | `npm run check` and `npm test -- --maxWorkers=1` pass (491 tests). Chromium desktop/390px renders of default/granted/denied browser controls inspected; permission states and notification construction explicitly mocked. Fixture delivery includes sender icon/body and click routing. No service worker, Safari or physical-browser acceptance. |
| Rust desktop | 327 full libwebrtc-linked Linux tests pass (9 existing opt-in tests ignored), including sparse notification replay without a UI loop; application-package Clippy passes. A temporary probe rendered the real Linux backend against Dunst and verified its Open callback; sender-avatar screenshot inspected. Dependency-inclusive Clippy hits existing vendored WebRTC safety-documentation warnings. Minimized-window activation and Windows runtime remain unverified. |
| Android | Avatar parser regression added; no Android SDK/build/device available in this orb. FCM physical receipt and rendered MessagingStyle remain unverified. |
| iOS/macOS | Extension/bundle/entitlement changes and macOS local delivery implemented; no Swift/Xcode build, XCTest, signed provisioning or OS banner inspection available in this orb. Signed iOS communication banners, extension fallback, macOS attachments and minimized activation require native acceptance. |
| Containers | No Docker daemon in this orb. Web production build and Rust compilation checked directly; images not built or executed. |

Uploaded-file preview validation (October 11, 2026; 50 MB / poster-only follow-up):

| Platform | Validation boundary |
| --- | --- |
| API/gateway/CDN | Workspace tests and Clippy pass; all 46 disposable API Postgres/Valkey tests and two account integration tests pass. Mock APNs/FCM and the gateway receive the existing PNG poster for a 120 MB video whose stored snapshot has `preview:{}`; missing posters and deletion remove media fields. Unit cases cover the 50,000,000-byte boundary, documents, multiple files and forwarded snapshots. CDN tests retain signature/expiry protection and assert CORS. No live R2 or provider sends. |
| Web | `npm run check` and `npm test -- --maxWorkers=1` pass (565 tests). Chromium downloads a local HTTPS PNG poster into a 1024 × 576 blob and a 12,588,863-byte image into a 1024 × 1024 blob. Malformed/expired/mismatched/oversized inputs fall back; request logs confirm no original-video fetch, including videos below the cap. Gateway tests check text fallback and logout during a download. Browser OS presentation, Safari and physical mobile browsers remain unverified. |
| Rust desktop | 403 libwebrtc-linked Linux tests pass (12 existing opt-in tests ignored); application Clippy and format checks pass. The initial original-only implementation's real Linux/Dunst photo, video-still and document fallback renders and Open callbacks passed. The follow-up removes video decoding and reuses the image renderer for posters; FFmpeg remains needed only for AVIF notification originals. Windows runtime, packaged FFmpeg and minimized-window activation remain unverified. |
| Android | Image/poster parser regression and private FileProvider added; original-video MIME is rejected. XML parses, but no Android SDK build/JUnit run, device or real FCM MessagingStyle preview validation in this orb. |
| iOS/macOS | Shared bounded image/poster downloader included directly in the service extension and CaperCore; sources stream to disk rather than keeping a 50 MB body in extension memory. No Swift/Xcode build, XCTest, signed iOS push, extension expiry or macOS banner validation. |
| Containers | No Docker daemon; application builds/compilation checked directly. No image builds or deployment performed. |

`202610080002_notifications.sql` drops the unused SNS-era `push_devices`,
`push_notifications` and `push_deliveries` tables; `202610030002_push.sql` stays
as applied. API images from before that migration grant access to those tables at
startup and therefore fail against the migrated database: roll the API forward,
not back.

### Deployment order

The steps below cover an initial DM rollout. If DMs are already deployed, this
push-deferral update needs only the API image rollout in step 3 and verification
in step 6; existing gateway and client releases remain compatible.

1. **Prerequisites/infrastructure:** existing account auth, Postgres, Valkey and
   compatible shared-mode API/gateway must be healthy. No DNS/SFU/TURN changes are
   needed for DMs or push deferral. Keep the existing API replica policy. No mobile
   push provider provisioning or new IAM permissions are needed.
2. **Secrets/configuration:** no new settings or secret changes are required.
   Legacy mobile-push settings are ignored by this API. Removing unused settings,
   permissions or provider resources from shared infrastructure is a separate
   reviewed operator action; do not replace or remove the other application keys.
3. **Database/API:** after image builds, deploy the exact merged SHA. API startup
   applies any pending published migrations, including
   `202610030001_direct_messages.sql` and the legacy `202610030002_push.sql`, through
   the migration role and grants runtime table/sequence access. Push deferral adds
   no migration; already-applied versions and checksums remain unchanged. No
   separate SQL job or manual migration is required. Back up the database first;
   never reset it.
   These versions follow the already-published channel-joining
   migration; never rename or edit migrations already recorded in a shared ledger.
   Inspect the ledger before rollout. If a non-disposable database ran the earlier
   feature branch's conflicting DM versions, stop for a migration-history
   compatibility plan rather than renaming or deleting applied records.
   ```sh
   psql "$MIGRATION_DATABASE_URL" -X -v ON_ERROR_STOP=1 \
     -c 'SELECT version, description, success FROM public._sqlx_migrations ORDER BY version;'
   MERGED_SHA=<reviewed-merge-sha>
   gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-api --timeout=15m
   ```
4. **Gateway before clients:** the new participant access query must be on every
   gateway pod before exposing DMs. Older gateway images deny DM subscriptions.
   ```sh
   gh workflow run deploy-caper-gateway.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-chat-gateway --timeout=15m
   ```
5. **Web/native clients:** deploy web after both backend roles are healthy.
   Android, iOS/macOS and Rust desktop releases can then ship independently.
   Do not release uncompiled native changes: run their build/test scripts on
   Android/Apple toolchains and inspect real affected layouts first.
   ```sh
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ./apps/native/android/build.sh
   ./apps/native/apple/build.sh macos
   ./apps/native/apple/build.sh ios
   ```
6. **Verify/rollback:** with two owned accounts, create in both directions, send,
   reconnect, page history, switch spaces and check unread/read state; a third
   account must get 404 for history/send/socket. Confirm authenticated push config
   returns an empty platform list, device registration returns 404, and new DM
   sends do not add rows to the legacy push tables. Native notification controls
   must remain hidden. Rolling back web/native removes the DM entry points.
   An older push-capable API may resume delivery of legacy queued rows if its old
   provider configuration remains; do not roll back to it without a reviewed plan
   to keep push disabled. Keep the API on an image containing the applied migrations:
   SQLx validates the database's migration history, so a pre-migration API image
   is not a safe rollback. An API rollback requires a reviewed build retaining
   those migrations and disabling the feature, not deleting migration records.
   Gateway rollback can disable DM subscriptions while preserving durable data.
   Leave the additive migrations in place; do not drop DM/message/push data or
   restore a database without a separate recovery review. Merging does not deploy.

### Validation boundary

Run the disposable DB/gateway tests in addition to the ordinary
Rust suite (never point these at shared databases):

```sh
DATABASE_URL='postgres://user@127.0.0.1:55432/postgres' \
CHAT_TEST_DATABASE_URL='postgres://user@127.0.0.1:55432/postgres' \
CHAT_TEST_VALKEY_URL='redis://127.0.0.1:6379' \
TEST_VALKEY_URL='redis://127.0.0.1:6379' \
  cargo test --locked -p caper-api -- --ignored --skip tests::shared
```

The loopback UI fixture provides `fixture_alex` / `TEST FIXTURE Alex`, not a real
account or push provider. For self notes, the disposable Postgres/Valkey tests
cover concurrent canonical creation, send/history, cross-account denial, read
cursors, authorized gateway replay, and no push enqueue. The browser DM suite
covers lazy creation, failure/retry, send/reload/reopen, peer-history isolation,
late-response navigation, invitation action, and desktop/narrow layout. Linux
Rust desktop tests cover self selection, owner/member/no-space actions, DM-plus
hover/focus and voice-dock padding/icon geometry; its explicit voice-connected
fixture was rendered and inspected for divider/row layout. The Linux client was
also signed into the disposable HTTP fixture:
opening self notes, sending a reminder, switching to a channel and reopening
notes retained one conversation and the message; **Invite people** opened the
existing management form. These are fixture interactions, not live account/SFU
validation. Swift model coverage was added but not run in the Linux orb. No local
Apple/Android build, Windows runtime, or Docker-image validation is claimed.
Browser checks do not prove native rendering or device
push. Android/Apple compilation and physical device layouts remain native release
acceptance requirements. APNs/FCM credential validation, physical-device delivery,
offline behavior and registration lifecycle/pruning must be covered when direct
mobile push is implemented, not treated as current supported functionality.

### Mobile Browse navigation and channel menu (October 6, 2026)

On narrow layouts, **Back to Browse** replaces the conversation's Browse label.
Tap the channel name for **Pins** and **Members**; neither has a dedicated mobile
header button. (Both are header buttons now: Pins since #363, and Members since
[Sliding member list on phones](#sliding-member-list-on-phones-october-10-2026).) Pins opens the existing pinned-message view; web/Android offer
**Messages** in the dropdown to return, while Apple retains its sheet's Messages
control. Members uses the existing member panel, including Close and outside-tap
dismissal. Direct messages and unjoined previews keep Pins but omit Members;
preview pins remain read-only. No new settings are implied. Wide layouts retain
their existing Pins/member controls.

Swiping right opens the existing space/channel browser; swiping left returns to
the selected conversation without selecting a different channel. Web keeps the
chat mounted, Apple keeps the draft in its chat model, and Android saves the
conversation state while Browse replaces it. Gestures originally required at
least 64 CSS pixels/native points of mostly horizontal travel and reject vertical
drags; see [Sliding Browse on phones](#sliding-browse-on-phones-october-8-2026)
for the current finger-tracked behavior.

| Platform | Intentional behavior and validation boundary |
| --- | --- |
| Web ≤760px | Touch swipes on content and channel rows; inputs, sliders, buttons other than channel selection, text selection, multi-touch, and open menus/dialogs/member overlays do not navigate. Build/type checks and 369 web tests pass. Disposable Chromium touch regression covers both directions, threshold/direction/cancellation/vertical/multi-touch guards, composer and overlay exclusion, channel-row release-click protection, draft/history retention, Back and menu keyboard dismissal. Pin regression covers Pins/Messages/Members menu placement, empty/populated pins, count updates, DMs, read-only previews and unchanged wide controls. Desktop, 390px and 320px captures were inspected. This is not physical-device or Safari validation. |
| Android narrow | Swipes are scoped to the timeline and channel sidebar; composer/account audio controls keep native gestures. Child-consumed drags and member overlays do not trigger navigation. Draft and lazy-list state survive Browse via saved conversation state. No Android build or device run in this orb: Java/Android toolchain is unavailable. Native CI and physical-device acceptance remain required. |
| Apple narrow | Right swipe starts within 24 points of the timeline's left edge; return swipe starts within 24 points of the joined-channel browser's right edge. Search/Browse-channels mode retains Close navigation instead of swipes, to avoid its text field. Composer/account controls and member/reaction overlays are excluded. Updated iOS UI test covers both edge directions, vertical scrolling, draft retention, Members and Pins menu/sheet dismissal. No Xcode/Swift build or UI test run in this Linux orb; macOS/iOS CI and devices remain required. |
| Rust desktop | No navigation behavior change or client release required. The shared icon exporter also bundles the new back glyph here; existing desktop icon packaging tests pass. |
| Containers/live voice | Docker daemon unavailable; the web build stage passed directly. No production deployment, shared data writes or live SFU validation performed. |

#### Deployment order

1. No infrastructure, secret/configuration, database migration, API, gateway or
   Valkey change is necessary. Wait for the merged revision's immutable web image
   and native build checks. Set `MERGED_SHA` to that full merged commit SHA;
   merging publishes build artifacts but does not deploy services or release apps.
2. Deploy web through the existing operator workflow from an authenticated
   checkout with the production Kubernetes context:

   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # Wait for this SHA's deployment workflow to succeed, then:
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```

3. Build/release Android and Apple through their existing native workflows after
   native checks and device acceptance. Web and native clients can deploy
   independently; no server-first compatibility requirement is introduced.
   Native build commands on suitable machines are
   `bash apps/native/android/build.sh`, `bash apps/native/apple/build.sh ios`, and
   `bash apps/native/apple/build.sh macos`. Build output is not an app-store release.
4. Verify swipe out/back with an unsent draft, vertical message/channel scrolling,
   Members menu/Close, Pins/Messages in channels/DMs/previews, keyboard dismissal,
   long-press message actions, and audio slider drags on real Android/iPhone
   devices and mobile Safari. Check wide desktop controls remain unchanged.
   Roll back web by running the deployment
   command with the previous known-good web image SHA; stop native distribution
   and release a corrected higher-build-number client if needed. No data rollback
   or migration reversal is required.

### Optimistic message feedback (October 8, 2026)

Pin/unpin actions update the initiating client's attribution, count and Pins
collection before HTTP confirmation. Edits preview their text immediately while
the editor retains the draft until confirmation or conflict/error recovery.
Sends and reactions retain their existing optimistic behavior. Other readers
receive server-confirmed events; no speculative revision, pin sequence or replay
cursor enters history. Failed or abandoned pin/edit intents reveal the latest
confirmed state, including updates that arrived while the request was pending.
Pending edit previews leave mentions as plain text until the server resolves
their metadata for the new text; confirmed mention metadata remains in history.

macOS custom buttons and menu triggers register AppKit pointing-hand cursor
rectangles instead of setting the cursor only when the pointer enters. Disabled
controls do not register a pointing-hand region.

| Platform | Validation boundary for this change |
| --- | --- |
| Web desktop/narrow | `npm test`: 420 tests across 37 web/shared-native-support files; `npm run check` passes. Disposable Chromium pin/edit regressions hold requests before the server receives them, then cover acknowledgement, two-tab pin fanout, rollback/retry, revision conflicts, thread/pin projections and touch actions. Desktop and 390px touch-emulated captures inspected; not physical-device or Safari acceptance. |
| Rust desktop | 256 tests pass, 9 existing tests ignored; fmt, full native Clippy and executable build pass. Linux 2x Xvfb renders with a labelled static fixture and delayed rejection stub show local pin attribution/count/list, retry and rollback. Not macOS/Windows or live-server/SFU acceptance. |
| Android | Projection/rollback coverage added. Gradle unit-test invocation blocked by missing Android SDK; no Android build or device test in this orb. |
| iOS/macOS | Projection/rollback coverage added. No Swift/Xcode build, device run or AppKit cursor runtime check in this Linux orb; native build and enabled/disabled hover acceptance remain required. |
| Containers/services | Web build stage passes directly; Docker daemon unavailable. No API/gateway change, infrastructure/configuration change, migration, deployment or live SFU validation. Clients can release independently. |

### Thread loading stability (October 8, 2026)

Selecting the already open thread does not reset its state or start another
request, including during pagination. Successful thread pages retain only their
pagination metadata; message content stays in the live channel timeline. Closing
and reopening a loaded thread, or switching back to it, reuses those rows and
preserves older-page boundaries. Channel/history resets and access loss discard
the metadata cache. This is not persistent/offline storage.

Web thread controls prefetch on hover and keyboard focus. Clicks share an
in-flight prefetch; a background response cannot open a closed panel or replace
the selected thread. Speculative failures stay silent and do not prevent a later
load. Cold loads show neutral message-shaped skeletons. No loading paragraph or
skeleton is inserted above existing replies on any client. Retry remains an
explicit action.

| Platform | Validation boundary for this change |
| --- | --- |
| Web desktop/narrow | `npm run check` and `npm test` pass: 461 tests across 40 web/shared-native-support files. `scripts/test-message-threads.mjs` checks zero additional requests and unchanged reply coordinates on repeat clicks, hover/focus prefetch, cached reopening, cold-load skeletons, error/retry, drafts and mobile Back/inert background. Inspected 2x Chromium desktop and 390px touch-emulated captures; `(pointer:coarse)` verified. Not Safari or physical-device acceptance. |
| Rust desktop | 279 tests pass, 9 existing tests ignored, including loopback request/pagination/cache-reset and wide/narrow egui skeleton/unchanged-reply-position regressions. Fmt, application-package Clippy (`--no-deps`) and executable build pass. Inspected 2x Linux Xvfb wide/narrow loaded/cold-thread fixtures. Dependency-inclusive Clippy is blocked by 34 existing vendored `webrtc-sys` missing-safety-doc errors. Not macOS/Windows or live-account acceptance. |
| Android | Equivalent repeat-open/cache and cold-load skeleton behavior implemented. `./gradlew testDebugUnitTest --no-daemon` reaches configuration but is blocked by the missing Android SDK. No Android compilation, rendering or physical-device run in this orb. |
| iOS/macOS | Equivalent behavior and a deferred-request/cache/pagination/reset regression implemented. Swift/Xcode unavailable in this Linux orb; test execution, compilation and native rendering remain required. |
| Containers/services | Web build stage passes directly; Docker daemon unavailable. No API/gateway, infrastructure, secrets/configuration, database migration, deployment or live SFU change. |

Web and native clients can release independently; merging does not deploy.
Before distribution, check repeated thread selection with a scrolled history and
an unsent draft, reopening after live replies/edits/reactions, loading older
replies, retry after failure, and reconnect/access-loss recovery on real native
clients. Roll back web using the previous known-good web image; stop native
distribution and ship a corrected higher-build-number client if needed. No data
rollback is required.

### Returning from Pins (October 8, 2026)

Web keeps the virtualized conversation mounted and measurable behind Pins, but
visibly dimmed and inert to pointer, keyboard and accessibility navigation. Returning
reveals the existing viewport without repeating initial measurement or showing
an empty history. Live arrivals follow the bottom only for readers already there;
readers in older history retain their position. Android retains its channel's
lazy-list state across the Pins branch. Rust desktop uses a separate Pins scroll
area without changing conversation offsets or pagination anchors. Apple already
presents Pins as a sheet over the mounted conversation and is unchanged.

| Platform | Validation boundary |
| --- | --- |
| Web | See the Pin validation boundary above for current combined results and the known narrow live-arrival failure. The timeline remains visibly dimmed and unfocusable behind the modal; screenshots inspected. Earlier return-frame coverage included short/empty conversations. Narrow viewport checks are not physical-device or Safari acceptance. |
| Rust desktop | 276 tests pass, 9 existing tests ignored, including rendered scroll-anchor/paging preservation across empty Pins. No macOS/Windows or physical-device acceptance. |
| Android | Scroll state is hoisted above Pins navigation. Gradle compile attempt is blocked by the missing Android SDK; no native build or device verification. |
| Deployment | Web, Android and Rust desktop can release independently. No API/gateway, infrastructure, secret/configuration, database or Apple release is required. Docker daemon unavailable; web build stages validated directly. No deployment performed. |

### Sliding Browse on phones (October 8, 2026)

On narrow layouts, Browse works like Discord's mobile drawer instead of the two
views swapping in place:

- **Still Browse:** Browse (spaces, channels and DMs) stays still underneath.
  The conversation slides over it, following the finger during a swipe.
- **Resting edge:** with Browse shown, the conversation's left edge stays in
  view on the right, about 20% of the width (56–96 CSS px/dp/pt). Browse's
  channel list is laid out beside it.
- **Account bar:** keeps the full width at the bottom, and the edge view stops
  above it. During the slide, the conversation's bottom edge lifts off the bar.
  (Superseded October 10: the conversation keeps its full height and the bar
  rises over it; see "Full-height Browse slide" below.)
- **Edge view:** inert. Tapping it, or dragging it left, returns to the
  conversation.
- **Release:** past halfway, or a flick of at least 300 CSS px/points per second
  after 40 of travel, completes the move; anything else springs back. A flick
  back toward the start cancels.
- **Buttons:** Back to Browse, Close navigation, and choosing a channel or DM
  animate the same slide.
- **Swipe starts:** unchanged from the October 6 rules above, plus the edge
  view. Swipes never select a channel. Drafts and history are kept.

| Platform | Behavior and validation boundary |
| --- | --- |
| Web ≤760px | Same-document View Transitions snapshot the room. The conversation's snapshot is moved (and its bottom clipped) by the drag, then settled with WAAPI. The rest of the page stays live. Browse's live layout keeps the conversation laid out at the edge: inert, at its usual offset, and clipped above the bar. That keeps its scroll position, which previously reset to the latest message on every Browse. Without View Transitions, or with `prefers-reduced-motion`, Browse switches at once, keeping the edge view and the original 64px/600ms quick-swipe rule. `npm run check` passes, and `npm test` passes 471 tests on Node 24. `scripts/test-desktop-navigation.mjs` passes against disposable mocks in Chromium touch emulation, with timestamped touches. It covers mid-drag tracking, slow snap-back, halfway and fling commits both ways, the resting edge (position, inert, Browse not covered), tap and drag back from the edge, cancel/vertical/wrong-direction/multi-touch guards, reduced motion, Back/Close, overlays, drafts and history reads. The test's stale members-default and Pins-header expectations (#363/#366) were updated. 320px and 390px captures (resting edge, mid-swipe, button slide) were inspected. Not Safari, Firefox or physical-device acceptance. |
| Android narrow | Compose `Animatable` with a critically damped spring that carries the release velocity. The conversation stays composed. Its layer is translated and clipped above the account bar; the clip also limits touches, so the bar's controls stay reachable. The finger is tracked on the unmoving container. Follows the system animator duration scale. `./gradlew :app:compileDebugKotlin` and `:app:lintDebug` pass (Android SDK 36). No emulator or device run: there is no KVM in this environment. |
| Apple narrow | SwiftUI offset and clip shape inside a scoped transaction: no animation while dragged, then an interpolating spring carrying the release velocity. Model-driven changes use the default spring, and Reduce Motion switches without animation. Only a small modifier reads the drag, so the chat itself doesn't re-render per frame. The conversation stays in the hierarchy. Beside Browse it is not hit-testable or exposed to accessibility, and a "Back to conversation" target covers its edge. Switching dismisses the keyboard. Timeline and channel-list edge-start rules (24pt) are unchanged. Swift/Xcode are unavailable in this Linux environment, so this is not compiled, UI-tested or device-checked. The existing iOS UI test's swipe/Back expectations are expected to hold. |
| Rust desktop narrow | No touch gestures. Toggling Browse slides the conversation over a still Browse to the same resting edge (0.3s cubic ease-out). The account bar moves to a full-width bottom bar in narrow Browse, and the conversation's rect stops above it (egui panels set their own clip). Clicking the edge returns. Fmt, application-package Clippy (`--no-deps`) and the debug build pass, and 288 tests pass (9 existing ignored). 60 fps Xvfb recordings of the `parity-narrow` fixture were inspected frame by frame. Not macOS or Windows acceptance. |
| Containers/services | No API, gateway, infrastructure, secret/configuration or database change. No deployment performed. |

#### Deployment order

1. No infrastructure, secret/configuration, database migration, API, gateway or
   Valkey change is necessary. Web, Android, Apple and Rust desktop can deploy
   independently. Wait for the merged revision's immutable web image and native
   build checks, and set `MERGED_SHA` to that full merged commit SHA. Merging
   does not deploy services or release apps.
2. Deploy web through the existing operator workflow from an authenticated
   checkout with the production Kubernetes context:

   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # Wait for this SHA's deployment workflow to succeed, then:
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```

3. Build Android, Apple and Rust desktop through their existing native
   workflows after native checks pass: `bash apps/native/android/build.sh`,
   `bash apps/native/apple/build.sh ios`, `bash apps/native/apple/build.sh macos`
   and `bash apps/native/desktop/build.sh`. Build output is not a store release.
4. On a real iPhone, an Android phone and mobile Safari/Chrome, verify:
   - slow drags snapping back, half-way drags, and flicks in both directions;
   - tapping and dragging the conversation's edge beside Browse;
   - the account bar's controls beside that edge;
   - catching a slide mid-animation;
   - vertical scrolling while horizontally still;
   - Back to Browse and Close navigation;
   - choosing a channel and a DM;
   - draft and scroll retention, and keyboard dismissal;
   - Reduce Motion or Remove animations;
   - member overlay and audio slider drags.

   Roll back web by deploying the previous known-good web image SHA with the
   command above. For native clients, stop distribution and ship a corrected
   higher-build-number build. No data rollback is required.

### Browse hides the conversation on phones (October 10, 2026)

Supersedes the October 8 **Resting edge** and **Edge view**: Browse and the
conversation no longer share a narrow screen.

- **Full slide:** Back to Browse (and a swipe) moves the conversation all the
  way off, past the width so its edge shadow leaves too. Browse's channel list
  takes the full width. There is no conversation edge or "Back to
  conversation" target; Close navigation, choosing a channel/DM, and the
  existing swipe from Browse return.
- **Account bar, release, buttons, swipe starts:** unchanged from October 8,
  minus the edge view as a start.

| Platform            | Behavior and validation boundary                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Web ≤760px          | Travel is the room's width plus 48px. With Browse shown, the conversation stays laid out (scroll kept) but is `visibility: hidden` and inert. `npm run check` passes. `npm test`: 491/492; the one failure (`noise-assets.test.ts`, compile-cache count) fails identically on unmodified `main`. A focused Chromium run at 390px confirmed Back hides the conversation, Browse fills the width (8px gutter) and choosing the channel returns. `scripts/test-desktop-navigation.mjs` was updated for the full slide but stops at its pre-existing desktop presence step (the mock records no presence subscription on `main` either), so its mobile section did not run here. |
| Android narrow      | Translation is the full layer width; the edge target and Browse's end padding are removed. Not compiled here (no Android SDK in this environment); no emulator or device.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Apple narrow        | Offset is the width plus 48pt, past the edge shadow from #431. Back, Close and swipe commits mark their transaction so nothing inside the conversation animates on its own: only the layer moves, fixing the Back arrow that lagged behind the slide and then slid back in on iPhone. Not compiled or device-checked (no Xcode on Linux).                                                                                                                                                                                                                                                                                                                                                                    |
| Rust desktop narrow | The slide travels the width plus 16px and the clickable edge is removed. `cargo fmt --check` and application-package Clippy (`--no-deps -D warnings`) pass; 328 tests pass (9 existing ignored). Not visually inspected under Xvfb this round; not macOS or Windows acceptance.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |

### Cross-client parity pass (October 8, 2026)

An audit of web, Android, Apple and Rust desktop closed these capability gaps.
Presentation stays platform-native.

- **Thread composers** on web, Android and Rust desktop now share the channel
  composer's `:` emoji / `@` mention suggestions (Apple already did). Android's
  thread composer also gains the 3,000+ character counter.
- **Message actions:** macOS and Rust desktop gain web's quick reactions (👍 ❤️ 😂
  🎉 👀, toggling your own), View reactions with the full who-reacted list, Copy
  text and Copy message ID; iOS gains View reactions. iOS and macOS offer Unblock
  on a shown message from a blocked author. iPhone can forward from a thread.
- **Threads:** Rust desktop broadcast replies show "Replied to a thread · View
  thread". Escape closes the thread on macOS and Rust desktop after suggestions,
  pickers, menus, dialogs and Pins have handled it. Escape typed in the macOS
  composer stays with the composer.
- **Android timeline** opens at the latest message, follows new messages only
  while at the bottom, reveals your own sends and keeps its place when older
  pages load; Go to message and historical context windows keep the viewport.
  Hardware Enter sends and Shift+Enter adds a line, as on web/desktop/macOS.
- **Navigation:** Android reopens the last channel viewed in each space and keeps
  the current space/channel when it is chosen again; "Channels N" counts joined
  channels, as on web and Apple.
- **Membership refresh:** Android, Apple and Rust desktop reconcile `GET
  /api/spaces` and the open space every 15 seconds and on returning to the app,
  like web, so invitations and channels changed elsewhere appear without a
  restart. A change made locally meanwhile wins; a failed read keeps what is
  shown. Removal shows "This space is no longer available."
- **Invites:** native space/channel member forms pre-check members and pending
  invitations, show "Invitation sent. They must accept before joining.", explain
  failures in web's words and keep the typed username. Every client's channel
  pre-check, web's included, now says "This person already has access to this
  channel."
- **Rust desktop** always shows private-channel invitations, which previously
  never arrived because `SpaceDetail` read `channel_invitations` instead of the
  API's `channelInvitations`. Owners can manage unjoined channels from Browse on
  Apple and Rust desktop. Ctrl+, opens Settings.
- **Apple:** an automatic voice reconnect keeps per-person volume and Mute for
  me; iPhone clears delivered pushes for an opened conversation (all of them when
  push stops); Audio diagnostics is offered to debug accounts on iPhone.

| Platform | Validation boundary for this change |
| --- | --- |
| Web desktop/narrow | `npm run check` passes; `npm test` passes 471 tests across 40 files under Node 24 (Node 22 fails the unrelated `noise-assets` WebAssembly mock). Chromium against the disposable fixture checked thread `:`/`@` suggestions, Enter/Tab insertion, Escape dismissing suggestions before the thread, and sending, at 1280px and 390px touch; captures inspected. Not Safari or physical-device acceptance. |
| Rust desktop | Fmt and application-package Clippy (`--no-deps`) pass. 256 tests pass with libwebrtc symbols stubbed at link time locally; 53 media/voice tests need real libwebrtc and were not run. No rendered-layout inspection, macOS/Windows, live-server or hardware IME check. |
| Android | With a locally installed SDK: `compileDebugKotlin`, `testDebugUnitTest` (209 tests, 0 failures) and `lintDebug` pass. No emulator or device run: rendering, real scrolling, hardware keys and live polling are unverified. |
| iOS/macOS | Swift 6.1 on Linux parses all Sources/Tests; non-UI CaperCore files type-checked against local stubs. No Xcode build, XCTest run, simulator or device. Check menu `Toggle` rendering, `.onExitCommand`, sheet sequencing and push clearing in Xcode. |
| Containers/services | No API, gateway, infrastructure, secret/configuration or migration change. No Docker build needed. |

Gaps found but not changed here: Rust desktop has no automatic voice reconnect
and Android only retries ICE once; Android and iPhone have no microphone choice;
iPhone has no lock-screen call controls (CallKit); Android has no in-app update notice; Android and Apple
load older history only from the button; Apple scrolls to every new message even
while reading history. Product gaps on every client (no change): message deletion,
links, unread markers, jump to latest and shortcuts. (Per-conversation drafts
were added later on every client.)

**Deployment order.** No backend step. Web, Android, Apple and Rust desktop can
release independently, in any order; merging does not deploy. Roll back web with
the previous web image and native clients with a corrected higher-build-number
release. No data rollback is required.

### Sliding member list on phones (October 10, 2026)

On narrow layouts the member list now behaves like Browse:

- **Header button:** Members (Show/Hide member list) is an icon button right of
  Pins on every width. A channel's title no longer opens a dropdown on phones.
  A 1:1 DM's title keeps its menu for Block/Unblock on Apple and Android; web's
  DM Block stays a header button.
- **Slide:** the list slides in from the trailing edge over the conversation,
  below its header, with Browse's timing, and slides out before it is removed.
  The 25% backdrop fades with it. Header button, Close, backdrop and Android
  Back all animate it.
- **Finger:** on touch clients a left swipe on the conversation opens it and a
  right swipe on the panel closes it, following the finger with Browse's
  release rules (past halfway, or a 300/s flick after 40 of travel). Web and
  Android start the opening swipe anywhere on the timeline; Apple starts it
  within 24pt of the trailing edge, mirroring Browse's leading edge. Opening
  Browse with the list open carries it with the conversation.
- **Look:** rounded 16px corners outlined on every side (the leading-only line
  used to vanish into the top corner on Apple), and the edge shadow the
  conversation casts over Browse (web `-12px 0 32px rgb(0 0 0 / 0.45)`, Apple
  radius 16 / x −12 / 45% as in #431, Android 16dp elevation, Rust desktop's
  egui edge shadow).
- **Reduced motion:** opens and closes at once.

| Platform | Behavior and validation boundary |
| --- | --- |
| Web ≤760px | `membersDrawer.ts` poses the panel with WAAPI (Browse's 320ms easing; a release keeps the finger's speed) and keeps it mounted, inert, while it leaves. Wide layouts toggle at once, as before. `npm run check` passes and `npm test` passes 492 tests on Node 24. In Chromium touch emulation against the disposable fixture, `scripts/test-desktop-navigation.mjs` passes its new header-order, slide-in, backdrop, snap-back, flick-close, mid-drag tracking, flick-open and 320px checks, and all Browse checks. Its first desktop step waits for mocked presence that never arrives (also on `main`: the mocked gateway keeps reconnecting), so that run skipped that one wait. Other agent-browser scripts now wait for the slide-out but were not run. 390px and 320px captures were inspected. Not Safari, Firefox or physical-device acceptance. |
| Android narrow | A second `BrowseDrawer` with the same spring; the drag runs on the unmoving conversation layer in the opposite direction. `compileDebugKotlin`, `testDebugUnitTest` (235 tests) and `lintDebug` (0 errors) pass with SDK 36. `smoke.py` covers the header order and both swipes (ruff passes) but was not run. No emulator or device: there is no KVM here. |
| Apple narrow | `DrawerSwipe` generalizes Browse's swipe to either edge; a `MembersOverlay` poses the panel in a scoped transaction like `BrowseLayer`. Swift 6.1 on Linux parses `CaperView.swift` and the UI tests, and the generic drawer pattern (protocol, `@Observable`, generic `@Environment`) type-checks against a stub with SwiftUI's signature. No Xcode build, UI test or device run. `testNarrowConversationAndBrowse` now covers the header button and both swipes. |
| Rust desktop narrow | No touch gestures, as with Browse. The header button slides the list (0.3s cubic ease-out); clicking the backdrop closes it. Fmt, application-package Clippy (`--no-deps`) and 329 tests pass (9 existing ignored), including a new one for the button order and the slide in and out. Xvfb renders of `parity-narrow` (closed, mid-slide, open, closed by the backdrop) were inspected. Not macOS or Windows acceptance. |
| Containers/services | No API, gateway, infrastructure, secret/configuration or database change. No deployment performed. |

**Deployment order.**

1. No infrastructure, secret/configuration, database migration, API, gateway or
   Valkey change. Web, Android, Apple and Rust desktop deploy independently, in
   any order; merging does not deploy. Wait for the merged revision's immutable
   web image and set `MERGED_SHA` to that full commit SHA.
2. Deploy web from an authenticated checkout with the production Kubernetes
   context:

   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # Wait for this SHA's deployment workflow to succeed, then:
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```

3. Build native clients through their existing workflows:
   `bash apps/native/android/build.sh`, `bash apps/native/apple/build.sh ios`,
   `bash apps/native/apple/build.sh macos` and
   `bash apps/native/desktop/build.sh`. Build output is not a store release.
4. On a real iPhone, an Android phone and mobile Safari/Chrome, check the
   Members button beside Pins, the slide in and out from the button, Close and
   backdrop, slow drags snapping back and flicks both ways, vertical scrolling
   in a long list, opening Browse with the list open, and Reduce Motion.
   Roll back web by deploying the previous known-good image SHA with the command
   above; for native clients ship a corrected higher-build-number build. No data
   rollback is required.

### Full-height Browse slide (October 10, 2026)

On phones, sliding between the conversation and Browse showed the conversation
as a cut-down card: on iPhone it stopped below the status bar and above the home
indicator, its bottom edge (and composer) was cut away as Browse was uncovered,
and the timeline or channel list kept scrolling under a sideways drag.

- **Full height:** the conversation keeps its full height while it slides and at
  rest. On iPhone its background now reaches under the status bar and home
  indicator; the slide clips only sideways.
- **Account bar:** rises from the bottom edge over the conversation as Browse is
  uncovered, on a blackout band reaching the screen edges (and, on iPhone, under
  the home indicator). It keeps the full width. Settled Browse looks as before.
- **No scrolling mid-slide:** once a drag is following a slide, the timeline or
  channel list it started on stops scrolling until the finger lifts (iPhone and
  Android). This also covers the member list's slide, which shares the swipe. Web already cancels the slide when the browser takes a vertical pan,
  and its slide moves snapshots, so nothing scrolls under it.

| Platform | Behavior and validation boundary |
| --- | --- |
| Web ≤760px | The conversation's snapshot is no longer clipped at the top (the channel-row strip from #436) or the bottom. The voice panel in Browse gets its own view-transition name (`browse-bar`) and is raised from the bottom with the same WAAPI scrub/settle as the conversation; its box moves to `::after` so a blackout `::before` band beneath it is part of its snapshot. `npm run check` passes; `npm test` passes except the known Node 22 `noise-assets` failure, which also fails on the base commit. Chromium touch emulation against the `test-desktop-navigation.mjs` fixture at 390px: opening and closing drags captured mid-slide and settled, and inspected. Not Safari or physical-device acceptance. |
| Android narrow | The conversation layer only translates (no clip); the account bar is drawn above it and translated up from below the bottom edge by its own height. Once a drag is sliding, the container reads moves in the Initial pass and consumes them, so the timeline and channel list never start scrolling. System bars stay blackout outside the app's content, as before. `./gradlew :app:compileDebugKotlin` passes (Android SDK 36) with no new warnings. No emulator or device run. |
| Apple narrow | The conversation layer is offset without a clip shape; the workspace clips only sideways (`SideClip`), so its background extends under the status bar and home indicator. The account bar rises over it, hidden once covered (it would show through a translucent keyboard). `BrowseSwipe` sets `scrollDisabled` while it tracks a slide. Swift 6.1 on Linux parses `CaperView.swift`; no Xcode build, type-check, simulator or device run. |
| Rust desktop narrow | No touch gestures. The stage keeps the full content height; the account bar is drawn after it, rising from the bottom edge, with a click-absorbing backdrop so clicks on the bar's background don't reach the conversation. Fmt and application-package Clippy (`--no-deps`) pass; 329 tests pass (9 existing ignored). No rendered-frame inspection, macOS or Windows acceptance. |
| Containers/services | No API, gateway, infrastructure, secret/configuration or database change. |

**Deployment order.** No backend, infrastructure, secret/configuration or
database step. Web, Android, Apple and Rust desktop can release independently,
in any order; merging does not deploy. Deploy web with the existing workflow
(`gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main
-f git_sha="$MERGED_SHA"`, then `kubectl -n default rollout status
deployment/caper-web --timeout=15m`) and build native clients with their existing
scripts. On a real iPhone and Android phone, check slow, halfway and flick slides
both ways, the status bar and home indicator areas mid-slide, the account bar's
controls, and that the timeline doesn't move during a slide. Roll
back web with the previous web image and native clients with a corrected
higher-build-number release. No data rollback is required.

### UI polish pass (October 10, 2026)

A sweep of things a user would notice as odd: highlights that stick after a tap,
hidden or clipped text and menus, jumps while scrolling, mismatched buttons and
missing hover/pressed states. Web shipped in #437; the native clients follow with
the same fixes where the problem existed, plus their own audit fixes.

- **Timelines:** loading older messages keeps the reader's place (web, Apple,
  Android, Rust desktop), and a growing draft, suggestions or the keyboard no
  longer push the newest message out of view (web, Apple, Rust desktop; Android
  already handled it).
- **Hover actions:** compact follow-up messages wrap clear of the hover actions,
  and the action bar stays while its menu or picker is open (web, macOS, Rust
  desktop). Web touch layouts keep text clear of the ⋯ button.
- **Sidebars:** the space header stays pinned above long channel lists, the
  space-unavailable notice sits at the top instead of over the audio controls or
  composer, the narrowest sidebar shows the account avatar alone instead of a
  one-letter name, and the "N in voice" count is never cut off.
- **Dialogs and buttons:** dialog headings stay fixed above their scrolling body;
  web, Apple and Android dialogs fit their content (Rust desktop shells stay
  fixed-size, as `modal_shells_do_not_recenter_for_errors_or_pending_content`
  requires). Sign-in and profile primary actions are compact and right-aligned
  with a spinner while pending, and pending labels no longer resize buttons.
- **Controls:** disclosure chevrons rotate instead of swapping icons, Members
  matches Pins, timers and percentages use tabular digits, and touch targets are
  44px/44pt/48dp. Keyboard focus returns to User Settings after its dialogs close
  (web, macOS, Android hardware keyboards, Rust desktop).
- **Platform fixes:** Android runs edge-to-edge on every API level with `adjustResize`;
  the root consumes the Scaffold's system-bar insets before IME padding, so the
  composer sits directly above the keyboard, and every Material color role comes
  from the brand tokens. Rust desktop's voice error no longer pushes the account
  bar past the sidebar, the space rail scrolls so every space is reachable, and
  filled buttons gain hover, pressed and focus states.

| Platform | Validation boundary |
| --- | --- |
| Web | See #437: `npm run check` passes; `npm test` passes except the known Node 22 `noise-assets` failure. Chromium against the disposable fixture at 1280×800 and 390×844 touch, before/after captures inspected; the older-history hold was measured frame by frame. The `scripts/test-*.mjs` suites were not run (`agent-browser` unavailable). Not Safari, Firefox or physical-device acceptance. |
| Rust desktop | Linux: fmt, application-package Clippy (`--no-deps`) and build pass; 331 tests pass (9 existing ignored), including new regressions for the older-history frame and same-frame composer growth. Before/after Xvfb renders at 1440×900, 1000×560, 420×844 and 390×844, with frame captures for motion. The Forward window was not audited. No Windows or macOS run. |
| Android | `compileDebugKotlin`, `testDebugUnitTest` (236 tests, 0 failures), `lintDebug` and `compileDebugAndroidTestKotlin` pass with Android SDK 36. No emulator or device: rendering, edge-to-edge on Android 8–14 versus 15+, fitted dialogs and chevron motion are unverified. |
| iOS/macOS | Swift 6.1.2 on Linux parses all Apple Sources/Tests (`swiftc -parse`); no Xcode build, type-check, XCTest, simulator or device run. Check dialog sizing, older-history anchoring, keyboard/composer pinning and macOS focus return in Xcode. |
| Containers/services | No API, gateway, infrastructure, secret/configuration or database change. |

**Deployment order.** No backend, infrastructure, secret/configuration or
database step. Web already shipped with #437. Android, Apple and Rust desktop can
release independently, in any order, with their existing build scripts; merging
does not deploy. Before distributing, build Apple in Xcode and check the items
above, and check Android's keyboard and system bars on a phone. Roll back native
clients with a corrected higher-build-number release; no data rollback is
required.

### Resizable thread panes (October 11, 2026)

Web, wide Android and Apple layouts allow dragging the thread's leading divider.
Dragging left widens the thread; Left/Right adjust by 10, Home/End select the
bounds, and double-click/tap resets to 340px/pt/dp. The preferred width persists
locally, including across closing/reopening and application/browser restarts.
Preferred widths range from 300 to 600. The actual pane reserves 300 for the main
conversation, or splits the available conversation area equally when it is less
than 600; shrinking the window does not overwrite the preference. Phone layouts
keep their full-screen threads without a resize handle. Rust desktop already
uses a resizable egui thread SidePanel (300–480) and is unchanged.

| Platform | Validation boundary |
| --- | --- |
| Web | `npm run check` passes; `npm test` passes all 558 tests. `MESSAGE_TEST_ARTIFACTS=/tmp/thread-screenshots node scripts/test-message-threads.mjs` passes against the disposable loopback fixture: exact 83px drag, pointer capture, keyboard/bounds/reset, retained draft, preferred width after reopen/reload/mobile and temporary constraints, and existing reply/navigation checks. Inspected default, resized, constrained desktop and full-screen 390px Chromium touch-emulated captures. Not Safari, Firefox or physical-device acceptance. |
| Android | Wide layouts implement dragging, hardware keys, reset, TalkBack adjustment and SharedPreferences persistence; narrow dialogs remain unchanged. `./gradlew --no-daemon --max-workers=2 compileDebugKotlin testDebugUnitTest lintDebug` cannot configure the project because this orb has no Android SDK. No Kotlin compilation, emulator/device or native render validation. |
| iOS/macOS | Wide layouts implement dragging, keyboard/reset, accessibility adjustment and AppStorage persistence. A macOS parity UI regression covers drag direction/geometry, keyboard bounds/reset and saved width. No Swift compiler or Xcode in this Linux orb: no parse/typecheck, XCTest, simulator/device or native rendering validation. |
| Containers/services | No Docker daemon for an image build; Vite production build/typecheck pass. No API, gateway, infrastructure, secret/configuration or database change. No deployment performed. |

**Deployment order.**

1. No infrastructure, secrets/configuration, database migration or backend service
   rollout is required. Web, Android and Apple can deploy independently. Rust
   desktop needs no release for this change. Merging does not deploy components.
2. After merge, wait for the immutable web image for that revision. From an
   authenticated infrastructure checkout with the production Kubernetes context,
   set `MERGED_SHA` to the full merged SHA and run:

   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # Wait for the deployment workflow for this SHA to succeed, then:
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```

3. Before native distribution, build/test on supported hosts using
   `bash apps/native/android/build.sh`, `bash apps/native/apple/build.sh ios` and
   `bash apps/native/apple/build.sh macos`. Check wide dragging, bounds, keyboard
   and accessibility adjustment, reopen/restart persistence, and unchanged phone
   threads on real clients. Build output is not a signed/store release; use the
   existing release process after validation.
4. Verify resizing and reply/draft/navigation behavior in production web, including
   browser reload and desktop/mobile transitions. Roll back web by deploying the
   previous known-good image SHA with the command above. For native clients, ship
   a corrected higher-build-number release. No data rollback is required.

### Profile-card actions (October 11, 2026)

Web (desktop and narrow), Android, iOS/macOS and Rust desktop offer **Block**
with the existing confirmation and **Unblock** on other people's profile cards.
Blocked cards hide Message and Invite; your own card says You with **Edit profile**,
which opens the existing profile editor on every client. Historical
authors with an account ID but no known username can still be blocked, but cannot
be messaged or invited by an invented username.

**Invite to space** lists non-demo spaces owned by the account, using the existing
owner-only invitation API. Choosing one sends a seven-day pending invitation;
the person must accept before joining. It does not switch the current space/DM
or insert a member locally. Failures remain inline and retryable; success names
the destination and disables it for this card session. Existing members, pending
invitations, recipient privacy/blocks and server limits retain their API rules.

Blocking retains the existing semantics described under Message requests,
blocking and privacy: it prevents DMs/requests and invitations from the blocked
person and collapses their text. It does not remove membership or mute voice
in shared spaces. No API, database or permission change is included.

Validation: `npm run check` and `npm test` (561 tests) pass. The Chromium
`scripts/test-message-pins.mjs` fixture run checks invitation refusal/retry,
pending-not-member state, unchanged navigation, Block cancel/confirm, collapsed
text, Unblock failure/retry and restored actions, plus the existing profile/Pins
flows. Desktop and 390px sheet default/picker/error/success/blocked captures were
inspected; narrow Chromium is not a native phone test. Rust workspace tests,
fmt and the debug build cover Linux (403 application tests pass, 12 existing
tests ignored); loopback profile regressions check ownership/demo filtering,
off-current invitation isolation and Block/Unblock. Linux 2x Xvfb default-card
and expanded-picker renders were inspected.
Application-package Clippy (`--no-deps -D warnings`) passes; dependency-inclusive
workspace Clippy fails on 34 existing vendored `webrtc-sys` missing-safety-doc
errors. Those dependencies are not changed here.
Android Gradle compilation/tests are blocked by the missing Android SDK; Apple
has no Swift/Xcode toolchain here. Neither has native rendering or device
validation. Browser/loopback fixtures are not deployed-API, physical-device or
live-SFU acceptance. No Docker daemon is available; the web build stage passed
directly, but no container image was built in this orb.

**Deployment order.**

1. Land the preceding profile-entry-point fix first. No infrastructure,
   secrets/configuration, database migration, API or gateway rollout is needed.
   After both changes land, web and all native clients can release independently;
   merging alone does not deploy them.
2. After merge, wait for the immutable web image for the merged revision. From an
   authenticated infrastructure checkout with the production Kubernetes context,
   set `MERGED_SHA` to the full merged SHA and run:

   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # Wait for that deployment workflow to succeed, then:
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```

3. Build/test on supported hosts with `bash apps/native/android/build.sh`,
   `bash apps/native/apple/build.sh ios`, `bash apps/native/apple/build.sh macos`
   and `bash apps/native/desktop/build.sh`. Verify profile entry points, Block
   cancel/confirm/Unblock, errors and invitation acceptance on real clients
   before the existing signed/store release process. Check that shared-space
   membership and voice remain unchanged. Build output is not a store release.
4. Verify those same cases on deployed web at desktop/mobile widths. Roll back
   web with the previous known-good image SHA using step 2; ship a corrected
   higher-build-number native release. Existing invitations/blocks remain valid;
   no data rollback is required.

### Workspace loading and stable attachment controls (October 11, 2026)

The client-only web workspace now renders a server-safe branded skeleton during
account/list loading and the first conversation, including direct visits and
reloads. Opening the landing channel no longer waits for the independent DM
list; DM deep links still wait for their access list. Initial message loading
reuses the neutral thread skeleton on web, Android and Apple. Rust desktop
already has an initial history skeleton. No new skeleton animation was added.

Each client's signed-in main conversation composer reserves its attachment
control while upload availability/session checks finish. The control stays
disabled until uploads are allowed, and remains disabled if availability fails;
text composition does not depend on that check. This is not a guarantee that the
upload service is healthy.

| Platform | Validation boundary |
| --- | --- |
| Web | `npm run check` and `npm test` pass (54 files, 565 tests). Three new regression cases fail on the unchanged baseline and pass after the fix. Inspected 1440px and 390px Chromium renders using explicitly labelled local fixture data and held API responses: Join/reload skeleton, delayed DM list and DM deep link, disabled/enabled paperclip, and upload-availability failure. The textarea's measured bounds change by 0px when uploads become ready at both widths. This is not Safari, touch or physical-device validation. |
| Rust desktop | `cargo test --manifest-path apps/native/desktop/Cargo.toml -p caper-desktop --locked` passes (401 passed, 12 ignored). The accessibility regression asserts disabled/enabled/disabled attachment states and unchanged editor bounds. Linux fixture renders cover disabled and enabled controls at 1440px and 390px; a narrow Linux window is not a mobile client. Ignored device/audio tests were not run. |
| Android | Implementation updated, but `./gradlew --no-daemon --max-workers=1 compileDebugKotlin testDebugUnitTest lintDebug` stops at project configuration: Android SDK location not found. No Kotlin compilation, native rendering or device validation. |
| iOS/macOS | Implementation updated; no Swift compiler or Xcode in this orb, so no parse/typecheck, XCTest, native rendering or device validation. |
| Containers/services | No Docker daemon for an image build; the web production build runs directly. No API, gateway, infrastructure, secret/configuration or database change. No deployment performed. |

**Deployment order.**

1. No infrastructure, secrets/configuration, database migration or backend rollout
   is needed. Web and native clients can deploy independently; merging does not
   deploy them.
2. After merge, wait for the immutable web image for that revision. With access to
   the infrastructure repository and the production Kubernetes context, set
   `MERGED_SHA` to the full merged SHA and run:

   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # After the deployment workflow for this SHA succeeds:
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```

3. Build/check Apple and Android on supported hosts before distributing. Verify
   cold launch, delayed history/upload availability, failed availability and
   unchanged composer placement on real clients. After native acceptance, the
   existing signed-release workflow can distribute this exact merged revision:

   ```bash
   gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$MERGED_SHA"
   ```

4. Verify web Join and reload at desktop/phone widths and attachment readiness in
   production. Roll back web with the previous known-good image SHA using the same
   deployment command; native rollback needs a corrected higher-build-number
   release. No data rollback is required.

### Full-surface screen swipes (October 11, 2026)

Narrow touch layouts accept horizontal navigation from the middle of the content,
not just its edges. Right from chat opens Browse; left from Browse returns to
chat. Left from a joined channel opens Members, and right from its panel or
backdrop closes it. Headers, channel rows and space rails also accept navigation
swipes. Vertical scrolling, text editors, audio controls, open dialogs/menus and
text selections keep their own gestures. Web and Rust desktop restrict navigation
to touch, preserving mouse selection. Drafts and conversation state survive
navigation. Wide layouts are unchanged.

This supersedes the edge/target restrictions in the historical navigation
records above. Apple channel-search mode excludes only the search field's
measured bounds, so its surrounding content can still swipe back to chat.
Existing sliding media-viewer paging already starts across the viewer and is
unchanged; zoom/pan and modal/thread interactions remain separate.

| Platform | Validation boundary |
| --- | --- |
| Web | `npm run check` passes; `npm test -- --maxWorkers=2` passes all 565 tests. `scripts/test-desktop-navigation.mjs` passes in Chromium touch emulation against labelled disposable fixtures: center-start Browse/Members, header-button and backdrop swipes, finger tracking, fling/halfway/snap-back, vertical/cancel/multitouch guards, reduced motion, release-click suppression and draft/history retention. Inspected 390px default/Browse/Members and mid-drag captures, plus 320px Members. Two fixture timeouts in the initial unbounded Vitest run passed on the bounded-worker retry. Not Safari or physical-device acceptance. |
| Android | SDK 36 `:app:compileDebugKotlin :app:testDebugUnitTest` passes (300 tests, 0 failures). An initial JVM compilation was killed under memory contention; the one-worker, bounded-heap retry passes. `smoke.py` now covers center-start navigation and Members backdrop dismissal, but was not run: no device/KVM in this orb. No native gesture/rendering acceptance. |
| iOS/macOS | The shared drawer no longer requires a 24pt edge start. The iOS parity test covers center-start navigation, vertical scrolling, retained draft, search-editor exclusion with swipeable search-mode content, and Members backdrop dismissal. No Swift/Xcode toolchain here: no compile, XCTest, simulator or device validation. |
| Rust desktop | Linux application tests pass (406 passed, 12 existing ignored); fmt, application-package Clippy (`--all-targets --no-deps -D warnings`) and debug build pass. Linked egui touch/render/accessibility regression covers center-start Browse/Members, finger-position tracking, vertical/cancel/paused-drag guards, panel/backdrop closing and retained draft/composer exclusion. Inspected 2x Xvfb chat/Browse/Members button-driven renders; this is not physical touch input. Touch-only in narrow windows; wide layouts and mouse drags are unchanged. No physical touchscreen, Windows or macOS acceptance. |
| Containers/services | No Docker daemon for an image build; Vite production build passes directly. No backend, infrastructure, secrets/configuration or database change. No deployment performed. |

**Deployment order.**

1. No infrastructure, secrets/configuration, database migration, API or gateway
   rollout is required. Web, Android, Apple and Rust desktop can release
   independently; merging does not deploy them.
2. After merge, wait for the immutable web image for that revision. From an
   authenticated infrastructure checkout with the production Kubernetes context,
   set `MERGED_SHA` to the full merged SHA and run:

   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # After the deployment workflow for this SHA succeeds:
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```

3. Before native distribution, build/test on supported hosts with
   `bash apps/native/android/build.sh`, `bash apps/native/apple/build.sh ios`,
   `bash apps/native/apple/build.sh macos` and `bash apps/native/desktop/build.sh`.
   Run the Android smoke and Apple parity tests on supported devices/simulators;
   check center/header/rail/backdrop starts, vertical scrolling, text selection,
   audio controls, drafts and reduced motion on real clients. For Rust desktop,
   use a narrow touch window and check unchanged wide/mouse behavior. Build output
   is not a signed/store release; use the existing release process after native
   acceptance.
4. Verify the same cases on deployed mobile web. Roll back web with the previous
   known-good image SHA using step 2; native rollback needs a corrected
   higher-build-number release. No data rollback is required.

### Quicker Browse button slides (October 11, 2026)

Back, Close and channel selection use the same conversation-over-Browse slide,
with a 220ms button duration. Apple supplies the animation explicitly with the
navigation state change, rather than depending only on the layer's fallback.
Finger tracking, swipe release velocity and cancellation, the rising account
bar, and reduced-motion behavior remain unchanged. The member drawer's timing
is unchanged. Rust desktop uses the shorter slide in narrow windows.

| Platform | Validation boundary |
| --- | --- |
| Web | `npm run check` and all 565 `npm test` tests pass. The button-frame regression from `scripts/test-desktop-navigation.mjs` passes independently against the labelled local API fixture at 390px and 320px: 220ms intermediate positions in both directions, hidden/inert chat while browsing, retained draft and instant reduced-motion navigation. Chromium touch-emulated settled and test-frozen intermediate renders were inspected; the recorder dropped transition frames, so the recording is not evidence of smooth motion. Linux WebKit 27.2 with an iPhone profile passes button timing, intermediate pose, return tap, draft retention and reduced motion; the frozen intermediate render was inspected. These are browser emulations, not physical phones. The complete navigation script remains blocked by its existing presence mock; a scratch run skipping that wait also stops at a member-drawer gesture check. |
| Android | Button timing updated; the latest swipe-release curves and member-drawer button spring retained. `:app:compileDebugKotlin` cannot configure because the Android SDK is absent. No Kotlin compilation, native rendering or physical-device validation. |
| iOS/macOS | Explicit button animation and shorter fallback implemented; the latest swipe-release curves and Reduce Motion retained. No Swift compiler or Xcode in this orb, so no native build, UI test or device validation. The reported snap is not reproduced on a physical device here. |
| Rust desktop | Fmt, application-package Clippy (`--all-targets --no-deps -- -D warnings`) and the debug build pass; 403 application tests pass with 12 existing device/audio tests ignored. The test and app executables were linked with LLVM (`cargo rustc --tests -- -Clink-arg=-fuse-ld=lld` and `cargo rustc --bin caper-desktop -- -Clink-arg=-fuse-ld=lld`) after the default linker exceeded the orb's memory limit. Inspected a 390px Linux/Xvfb window: chat, Browse, intermediate return slide and settled chat. This is not macOS/Windows or mobile-device acceptance. |
| Services | No API, gateway, infrastructure, secrets/configuration or database changes. No deployment performed. |

Web and native clients can release independently. After merge, use the web
deployment and native release commands in the preceding deployment order,
substituting this change's merged revision. On real iPhone/Android clients and
mobile Safari/Chrome, verify Back and channel return animate, swipes still track
and settle, drafts survive, and Reduce Motion switches instantly. Roll back web
to the previous image; native rollback requires a corrected higher-build-number
release. No data rollback is needed.

## Swipe release finish (October 11, 2026)

Browse/conversation and phone member-drawer swipes now settle from the held pose
with a velocity-matched, non-overshooting curve on web, Android and Apple. A held
or slow release keeps a 320ms finish instead of shortening it with the remaining
distance; a flick keeps a bounded 160–320ms finish. The initial speed cannot exceed
three times the remaining distance divided by the finish duration. Halfway/fling
commit rules, reduced motion, and button-triggered animations are unchanged.
Rust desktop has no equivalent touch-swipe release path and is unchanged.

In the unchanged Chromium implementation, a held drag released about 71% across
the travel moved 68% of its remaining distance in its first 33ms. With the new
curve it moved 8% in the same interval and landed at rest after 320ms.

Run the focused regression beside the disposable fixture and Vite:

```bash
node scripts/test-navigation-release.mjs
# Override NAVIGATION_TEST_WEB_URL / NAVIGATION_TEST_API_URL for nondefault ports.
# Optional NAVIGATION_TEST_ARTIFACTS captures 2x images and release recordings.
```

| Client | Validation |
| --- | --- |
| Web | `npm run check` and `npm test` pass (565 tests). Chromium with real CDP touch input and verified coarse-pointer media covers held commits both ways, below-halfway cancellation, flicks both ways, member-drawer releases, retained composer draft and reduced motion. Rendered 390px fixture states inspected. This is not Safari or physical-device acceptance. |
| Android | Release calculation updated. `./gradlew :app:compileDebugKotlin` stops at configuration because the Android SDK is absent; no Kotlin build or device rendering was verified. |
| Apple iOS/macOS | Shared swipe implementation updated. No Swift compiler or Xcode in this orb; no native build, simulator or physical-device validation. |
| Containers/live | No Docker daemon; production web build checked directly. No backend, database, infrastructure or secret/configuration changes. No deployment performed. |

Web and native releases can roll out independently after merge. Before native
distribution, verify slow/held/fast drags at 51%, 70% and near the edge in both
directions, below-halfway cancellation, Members and Reduce Motion on supported
devices. Roll back web to its previous image; native rollback requires a corrected
higher-build-number release. No data rollback is needed.

## Space and channel unread indicators (October 11, 2026)

App-icon badges use a separate `badgeCount` field on the same unread endpoint.
See [the app-icon badge contract and platform limits](notifications.md#app-icon-badges):
Apple/web/desktop consume an authoritative message count, Android follows
active notifications, and Windows taskbar rendering remains unverified.
This does not change the navigation-dot rules below.

Web (desktop/mobile), Android, iOS/macOS and Rust desktop show a light dot on
spaces with unread messages or direct mentions. A selected space keeps its dot
until its other unread channels are viewed. Joined channel rows also show dots.
The top-right unread dot and bottom-right mute badge can coexist: notification
levels, pauses, OS permission and mutes do not hide space/channel unread activity.
DM behavior is unchanged. This is message activity, not a new invitation alert,
mention counter or notification inbox.

`GET /api/spaces/unread` returns only unread spaces/channels, with decimal-string
`lastSeq` and `readSeq` values. Joined channels count other unblocked, active authors' main
messages and broadcast replies. Accessible unjoined channels and non-broadcast
thread replies count direct personal mentions only; forwarded mention metadata
does not notify. Space membership, private grants and soft removals constrain
every result. Edits, reactions and pins alone do not create unread messages.

Channel visits capture this snapshot before acknowledging history. Web,
Android, iOS/macOS and Rust desktop place a terracotta **New messages** divider
before the first incoming, nonblocked channel message after that saved read
position. The divider breaks author grouping and remains for the visit even
after badges clear; switching away and returning captures a new boundary.
Live arrivals on an already-read visit do not introduce a divider. DMs, thread
panels and feedback visits are unchanged. If older unloaded history may contain
the boundary, the divider waits for that history rather than labeling the start
of the latest page; this does not change initial scroll position. Old servers
without `readSeq` show no divider.

Divider validation (October 18, 2026): web build/typecheck/lint and 659 JS tests
pass; labelled Chromium desktop/narrow fixtures exercise the read boundary and
grouping break, including returning after new arrivals and revisiting after
reading. API workspace tests, Clippy and the unread/read route regression against
disposable loopback Postgres pass; no migration is needed. Rust desktop passes
557 tests (12 device/audio tests ignored) and Clippy, with inspected
desktop/narrow fixture renders. Android SDK and Apple toolchains are unavailable
here, so their compile, native render
and device checks remain outstanding. These fixtures are not live-account or
cross-device acceptance.

`POST /api/spaces/{space}/channels/{channel}/read` accepts `{"seq":"42"}`.
Read positions are saved per account/channel, advance monotonically, and are
capped to the channel's head. Only a loaded foreground conversation acknowledges
its committed cursor; prefetch, background views, mobile Browse and older search
windows do not read newer messages. Reading one channel never clears another.
Snapshots refresh on launch/foreground and every 15 seconds; every client also
refreshes about a second after live notification activity (one request per
burst). Mentions in unjoined channels show a dot in Browse channels lists. Browse
only stops reads on narrow layouts, for channels and DMs alike. Failed reads retry;
account/request revisions fence late responses. Reads from another client appear
on the next refresh. Historical messages start unread until viewed: there is no
backfill asserting that existing users have seen them.

The list only inspects channels whose stream moved past the reader's position,
takes each joined channel's newest qualifying message by index, and finds
mentions through the partial GIN index `messages_mentions` (migration
`202610110002`, built `CONCURRENTLY` outside a transaction). On 400k seeded
messages in unjoined, never-read channels this took the poll from about 660ms to
under 1ms.

| Platform | Validation boundary |
| --- | --- |
| API/Postgres | Workspace tests, formatting and Clippy pass. The disposable loopback Postgres test exercises the actual unread/read routes: own/blocked authors, private grants, unjoined and thread mentions, broadcast replies, forwarded mention exclusion, channel-specific clearing, monotonic reads, future-cursor clamping and revoked membership. |
| Web | `npm run check` and `npm test -- --maxWorkers=1` pass. Inspected labelled local-fixture desktop/mobile Browse and all-read renders. Browser checks cover prefetch, stale snapshots, newer messages during pending reads, denied OS permission, hidden mobile chat, failed-read retry and older search windows. Fixtures are not multi-device or production acceptance. |
| Rust desktop | 405 tests pass (12 device/audio tests ignored), formatting/Clippy and debug build pass. Inspected the labelled Linux unread fixture: selected space, muted space and muted channel dots remain separate from selection/mute badges. This is not Windows, live account or physical-device acceptance. |
| Android | Implementation updated; Gradle compile/tests/lint stop at project configuration because Android SDK location is missing. No Kotlin compilation, native render or device acceptance. |
| iOS/macOS | Implementation updated; no Swift compiler or Xcode in this orb. No Swift typecheck, XCTest, native render or device acceptance. |
| Containers/services | Docker daemon unavailable; web production build validated directly. No shared database writes, deployment, release or live SFU/device validation. Initial overlapping checks exhausted orb memory and timed out; isolated reruns passed. |

**Deployment order.**

1. No infrastructure apply, new secret/configuration, Valkey or SFU state change
   is needed. Keep existing database/migration credentials and the established
   single API replica. Merging does not deploy anything. Wait for immutable
   images for the merged revision and set `MERGED_SHA` to that full revision.
2. Deploy API first. Normal startup applies
   `202610110001_channel_reads.sql` with `MIGRATION_DATABASE_URL` and grants
   SELECT/INSERT/UPDATE on the retained `channel_reads` table to the runtime
   role. No manual shared SQL, backfill or migration job is required:

   ```bash
   gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # After the deployment workflow for this SHA succeeds:
   kubectl -n default rollout status deployment/caper-api --timeout=15m
   ```

3. The gateway protocol and queries are unchanged; no gateway rollout is needed.
   Old clients remain compatible. After API readiness, web and native releases
   can deploy independently:

   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # After the deployment workflow for this SHA succeeds:
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```

4. Compile/test Apple and Android on supported hosts before distributing the
   clients. Verify multi-account and multi-device reads, desktop/phone navigation,
   muted spaces, denied OS permission, offline/reconnect, private access removal
   and another unread channel in the selected space. After native acceptance:

   ```bash
   gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$MERGED_SHA"
   ```

5. Verify new messages add a dot and opening the last unread channel removes it
   on another client within 15 seconds. Roll back web to the previous known-good
   image using the same deployment command; native rollback requires a corrected
   higher-build-number release. Prefer a forward API fix. Any API rollback build
   must retain the applied migration ledger; do not roll SQLx images back across
   this migration or delete read positions. No shared migration, deployment,
   provider change or release was executed during development.

## Swipe follow-up fixes (October 11, 2026)

Follow-ups to full-surface swipes and swipe release timing:

- iOS: the closed phone member list no longer takes touches over the
  conversation. Its backdrop and panel each carry their own close swipe, so
  Back, Pins, Members, the timeline, the composer and timeline swipes work while
  the list is hidden; swiping either closes it when shown.
- iOS/macOS: a released Browse swipe keeps its speed-matched curve instead of the
  220ms button slide. Buttons, Back and channel choices still use the button slide.
- Android: drags that start on the Browse "Search channels" field stay in the
  field, as on web, Apple and Rust desktop.
- Rust desktop: swipe releases settle with the same 160–320ms speed-matched
  `cubic-bezier(0.2, 0.2·slope, 0.35, 1)` as the other clients, rather than a
  fixed 0.3s ease. Release speed is measured against an earlier frame, so a
  flick whose last move and release share a frame still commits. Undecided
  touches no longer expire after 0.5s.
- Web: after a touch drag that ends without a click, the next keyboard or
  assistive activation (click `detail` 0) is no longer swallowed by Browse or
  the member list. Member-list drags no longer read layout on every move.

| Platform | Validation boundary |
| --- | --- |
| Web | `npm run typecheck`, Oxlint and Oxfmt on the changed files. No browser regression run; no Safari or physical-device acceptance. |
| Android | Implementation only; no Android SDK here, so no Kotlin compilation, tests or device acceptance. |
| iOS/macOS | Implementation only; no Swift compiler or Xcode, so no build, XCTest, simulator or device acceptance. |
| Rust desktop | `cargo test` (all 409 tests, including new release-curve and same-frame-flick tests) and `cargo clippy --all-targets -D warnings` pass on Linux. No physical touchscreen acceptance. |
| Services | No API, gateway, infrastructure, secrets/configuration or database change. |

**Deployment order.**

1. No infrastructure, secrets/configuration, database migration, API or gateway
   rollout is required. Merging deploys nothing.
2. Web, Android, Apple and Rust desktop can release independently with their
   usual release commands for the merged revision. Roll back web to the previous
   image; native rollback requires a corrected higher-build-number release. No
   data rollback is needed.

## Discord-style message replies (October 11, 2026)

**Reply** sends an ordinary recent message with a compact source author/excerpt.
The original stays in place; quoting it does not change its sequence or thread
count. A reply can itself be replied to, edited, reacted to, pinned, forwarded,
or have its own thread. **Reply in thread** remains a separate action. Channel
and thread composers support the same attachments. Replying does not implicitly
mention or ping the source author.

While composing, all clients show **Replying to [author]** followed by up to two
lines of the original text above the input, including narrow/mobile layouts.
The preview uses the complete local snapshot when available so a wide composer
does not stop arbitrarily at 200 characters. Attachment-only messages show
**Attachment**; blocked sources expose neither identity nor text. The X removes
the reply target without clearing the text being written. This October 13
composer refinement does not change the sent quote, source navigation or API.

The quoted original sits in a separate neutral card, with clear spacing before
the current reply and a visible **View original** affordance. Small shaded
pewter cup sockets touch the left edges of the original and replying avatars,
joined by one thin curved string in the gutter. Web and Rust desktop share the
SVG artwork; Android and Apple draw matching vectors. Collapsed quotes show the first 100
Unicode scalars, wrap to at most three lines and fade into the dark card.
The fade spans the lower 60% of the preview. **Show more** floats
at the midpoint of that fade, without a separate button row; clipped previews
retain enough height to keep it clear of the author heading. **Show less** stays
below expanded text so nothing is covered. Both controls are centered white
pills with black text, hover and pressed feedback, and at least 44px touch targets
on mobile. Expansion reveals the complete saved text separately from source
navigation. Short quotes have
neither a fade nor an expansion control unless
line breaks overflow the preview. Scrolling flexes the string by a few pixels
and slightly rotates the cups around their fixed avatar attachment points.
A damped spring settles back to rest; there is no repeating glow or idle jiggling.
Each reply derives its spring tension, damping, scroll response and travel limit
from its own client message ID, not the quoted original. Replies to the same
source therefore move and settle differently, with stable settings on remount
and across pending/delivered rows. All clients use the same UTF-8 FNV-1a seed
and tuning ranges: stiffness 140–240, damping 11–18, scroll impulse 1.0–1.4,
travel 5–7 logical pixels and speed capped at 16 times that travel. This is a
modest increase over the previous uniform 0.8 impulse and 4px travel. Web uses
actual scroll events, including touch/inertial scrolling; native clients track
the row's position as their scroll view moves, without intercepting swipes.
Both string ends follow the rotating cup backs. Animation work stops at rest
and on disposal; reduced motion keeps the connection static. Blocked-source
context hides the decorative connection as well as the quoted identity/text.
Rust desktop reads the Windows, macOS or GNOME animation preference once when
first rendering a connection; restart after changing that preference. Other
Linux desktops without the GNOME setting retain motion. Web, Android and Apple
observe their motion preference changes while mounted. Incoming replies to your
messages use a 12% terracotta wash and accent edge; ordinary and self replies
retain a faint neutral background. Pins, selected threads and source-jump
highlights retain precedence. Quote cards show up to three 44px square attachment
previews and an overflow count. Spoilers and unavailable/unfinished files remain
generic tiles; video originals never load just to draw a thumbnail. Clicking a
preview opens the original message, not the media viewer. Desktop
message toolbars expose **Reply** beside **Reply in thread**, including Reply
inside existing threads. Mobile long-press message actions offer both where
applicable, and source/expansion controls keep a 44px touch target. Long URLs and
line breaks wrap within the quote, including on narrow layouts.

`POST /api/chat/channels/:id/messages` accepts optional `replyToId`, independent
of `threadRootId` and `broadcast`. The API resolves it within the authorized,
locked channel. Channel replies can quote top-level/broadcast messages; thread
replies can quote that thread's root or messages. Missing, cross-channel or
out-of-thread targets return 404. Retries bind the target ID as well as the
existing command identity; changing it returns 409. A definitive attachment
refusal permits reuploading with a new command ID while retaining the original
reply target and snapshot; unknown-outcome retries never replace them.

The optional `replyTo` message field contains `{id, author, text, fullText?,
hasAttachments, threadRootId?, deleted?, attachments?}`. Its text is a send-time
snapshot with no recursive quotes or resolved mentions. Optional `attachments`
are projected from the current, same-channel source at HTTP/outbox/replay delivery
and signed like message attachments; files and expiring URLs are not persisted
in the quote. This also gives legacy references previews, while deleted sources
expose none. The legacy `text` preview remains
bounded to 200 Unicode scalars, so older reply-capable clients still accept the
whole message. New snapshots add `fullText` only when needed, up to the normal
4,000-scalar message limit; updated clients display `fullText ?? text`. Existing
snapshots are not backfilled or rewritten. Source edits do not rewrite existing
quotes. Clicking a quote loads the source through
the existing authorized history path. Clients hide the quoted name/excerpt for
blocked authors. Forward projections remove the quote to avoid sharing source
conversation context with another audience. Normal outbox/gateway delivery and
push rules are unchanged.

| Platform | Validation boundary |
| --- | --- |
| API/Postgres | `cargo fmt --all -- --check`, `cargo test --workspace` (159 passed, 60 integration tests ignored), all 60 ignored integration tests on disposable loopback Postgres/Valkey, and workspace Clippy pass. Coverage includes reply-to-reply, independent threads, unchanged original sequence/count, reference scope, target-bound retries, 200/4,000-scalar Unicode quote boundaries and transactional full-text outbox snapshots. Low-memory builds use one job and zero debug information. |
| Web | `npm run check` and `npm test -- --maxWorkers=1` pass: 579 tests across 55 files. Browser checks cover the 100-scalar faded preview (including emoji), full immutable expansion, both avatar sockets and their string attachments, a separate quote card, long URLs, source jumps and blocked-source privacy. Real timeline scrolling flexes the string and rotates the cups while their endpoints stay attached, then returns exactly to rest with no idle animation. Reduced-motion scrolling stays static. Inspected 2x desktop/narrow collapsed and expanded frames and a fresh scroll/settling recording. The narrow capture verifies coarse-pointer touch emulation, not a native phone. The forwarding browser suite also verifies original reaction counts and file attachments in the picker, delivered snapshot and shared conversation, including subsequent edits. Its synthetic file URL is not a storage/download acceptance test. Existing thread/reaction/resize/retry/long-press checks and attachment recovery remain applicable. Not Safari or physical-device acceptance. |
| Rust desktop | 417 tests pass; 12 existing device/audio tests remain ignored. Formatting, application-package Clippy (`--no-deps`) and the full libwebrtc-linked Linux debug build pass with Clang 21. The separate quote card, shaded SVG sockets and scroll-driven damped string are implemented. Quote tests cover legacy compatibility, full Unicode snapshots and frozen retries. Inspected 2x Linux sockets and quote expansion after real Secret Service sign-in with a disposable fixture keyring. Software rendering ran around 6 FPS: the tiny flex was hard to judge visually; a temporary live geometry trace confirmed real scroll impulses, nonzero deformation and exact settling to zero. Stable integration substeps preserve elapsed time on slow frames; the trace was removed. Initial-reply validation also exercised quick Reply while retaining the draft. Orb setup installs GNOME Keyring and passes twice. Dependency-inclusive Clippy is blocked by 34 existing vendored `webrtc-sys` missing-safety-doc errors. No Windows or physical-device acceptance. |
| Android | Implemented, including delivery-model regression assertions. Gradle tests/compilation stop at configuration because the Android SDK location is missing; no Kotlin compilation or native/device render acceptance. |
| iOS/macOS | Implemented, including delivery-model regression assertions. No Swift compiler/Xcode in this orb; no Swift typecheck, XCTest or native/device render acceptance. |
| Containers/services | Docker daemon unavailable. Web build stages validated directly. No shared database write, deployment, native release or live-media acceptance performed. |

**Direct-reply highlighting and attachment previews (October 19, 2026).**
Web checks and 695 tests pass; inspected desktop and 390px Chromium fixtures
show incoming-only emphasis, square previews, spoiler concealment and overflow.
A recorded thumbnail click navigates to the original; narrow Chromium is not
physical-device acceptance. API workspace tests/Clippy and the disposable
Postgres reply test pass, including current-source media projection, signed
preview URLs, replay and deletion. Other ignored integration tests were not
rerun for this change. Android has 497 passing unit tests and clean lint; APK
assembly still needs prepared native audio/AVIF inputs, with no device render
check. Rust desktop's linked Linux build, six reply-focused tests and application
Clippy pass; its local preview fixture was rendered and inspected. Apple changes
and regression assertions are implemented but remain uncompiled/unrendered
without Swift/Xcode. No Docker daemon, production deploy or native release was
used for this validation. API and gateway delivery must precede updated clients
to supply previews for previously sent replies; no migration is required.

The pill-overlay refinement reruns the web checks (579 tests), browser
thread suite, and Linux desktop tests (417), Clippy and linked build. Browser
checks verify the fade midpoint, no collapsed footer row, clear author headings,
centered expanded controls and 32px desktop/44px touch targets. Inspected actual
mouse-down, keyboard-focus and touch expansion states retain readable black
labels on white pills. Fresh 2x desktop/narrow and Linux collapsed and expanded
captures use disposable local demo conversation data; Linux also exercises
actual pointer click-to-expand. Android compilation
still stops at the missing SDK, and Apple remains uncompiled/unrendered here.

**Independent cone-motion follow-up (October 13, 2026).** Web checks and all
604 unit tests pass. The quote checks in `scripts/test-message-threads.mjs`, run
independently of the earlier thread checks, cover two equal-size replies to one
source on desktop and Chromium coarse-pointer mobile: different motion, attached
endpoints, exact settling and static reduced motion. Restoring the old uniform
physics makes the independence assertion fail. Separate mouse-wheel and CDP
touch-swipe checks also pass; 2x desktop/mobile frames were inspected. The full
thread suite stops earlier at its width assertion (527px versus 600px), so it is
not a passing acceptance result. Recording failed because the encoder fell
behind; frame-by-frame SVG measurements, not a recording, verify the motion.
Linux desktop's linked debug build, application Clippy and 485 tests pass
(12 device/audio tests ignored); its reply fixture was rendered, scrolled and inspected. Android still
stops at the missing SDK; Apple has no Swift/Xcode here. Neither is native-device
acceptance. This tuning needs only web/native client releases, independently;
no API/gateway deployment, migration, infrastructure or configuration change.

**Composer preview follow-up (October 13, 2026).** Web checks and all 607 unit
tests pass. Focused Chromium checks at 1280px and 390px (coarse pointer verified)
cover the author/excerpt above channel and thread inputs, two-line long text,
URLs and emoji, attachment-only fallback, target switching/cancellation without
losing draft text, sending the correct quote, source navigation and blocked
identity/text masking. The separate full thread script still stops at the
527px-versus-600px resize assertion, also reproduced on the unchanged base.
Linux desktop previews were inspected at 2x in wide and narrow windows, including
cancellation retaining the draft. Android compilation cannot configure without
an Android SDK; Apple has no Swift/Xcode toolchain here. These are local fixture
checks, not physical-device or live-account acceptance. No Docker daemon is
available; the web image's application build stages were checked directly.

**Deployment order.**

1. No infrastructure apply, new secrets/configuration, database migration or
   backfill is required. Quotes persist in the existing message JSON payload.
   Keep the established API replica/Valkey configuration. Merging deploys
   nothing. Set `MERGED_SHA` to the full merged revision and wait for that
   revision's immutable API/web images.
2. Deploy the API before releasing clients that send `replyToId` or expect full
   saved quote text:

   ```bash
   gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # After the workflow for this SHA succeeds:
   kubectl -n default rollout status deployment/caper-api --timeout=15m
   ```

3. No gateway, SFU or TURN rollout is required; existing gateway delivery passes
   the optional fields unchanged. Pre-reply clients ignore the context; older
   reply-capable clients display the 200-scalar preview. After API readiness,
   web and native releases can deploy independently:

   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # After the workflow for this SHA succeeds:
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```

4. Compile/test Android and Apple on supported hosts before distributing native
   clients. Verify multi-account replies in spaces and DMs, replies to replies,
   own threads, attachments, offline retry, long/expanded quotes, source
   navigation, reduced motion, blocking and private access removal. After native
   acceptance:

   ```bash
   gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$MERGED_SHA"
   ```

5. Verify HTTP/gateway replay preserve one reply and its quote without moving
   the original or changing its thread count. Roll back web with the same
   deployment command using the previous image SHA. Native corrections need a
   higher-build-number release. Keep a reply-capable API while updated clients
   remain installed: older APIs ignore `replyToId` and lose quote context on new
   sends. Prefer a forward API fix. Do not delete persisted quotes or messages;
   no data rollback is required.

## Own profile refresh (October 11, 2026)

A successful display-name/username save updates retained self identities by
account ID in the editing client: members, invitations, self-DMs, chat authors,
pins, thread participants, forwarded authors, reaction lists and profile cards.
The local voice roster uses the saved display name without leaving the call.
Conversation selection, drafts, rejected sends, historical text and replay
cursors stay intact. Other open sessions still use their existing refresh
behavior; this change adds no realtime profile event.

| Platform | Validation boundary |
| --- | --- |
| Web | `npm run check` and all 578 `npm test` tests pass after rebasing onto current main. `node scripts/test-profile-refresh.mjs` passes against labelled loopback API mocks at 1280px and 390px: space/private-channel settings, stale snapshots, failed save, same-name peer, cached navigation and retained draft/history. Inspected 2x Chromium settings captures; narrow Chromium is not a physical phone. No live SFU/profile-change acceptance. |
| Android | Implementation and regression test added. Gradle `testDebugUnitTest --tests '*ProfileValidationTest'` stops at missing Android SDK; no Kotlin compilation or device acceptance. |
| iOS/macOS | Implementation and regression test added. No Swift compiler or Xcode here; no build, XCTest, simulator or device acceptance. |
| Rust desktop | Formatting passes. All 45 actual `model.rs` tests and Clippy pass through a disposable model-only Cargo harness, including the profile/history/cursor regression. Full desktop application build and UI/device acceptance were not run. |
| Services | Root workspace formatting, tests (159 passed, 60 existing ignored) and Clippy pass. No API/gateway changes; ignored database/Valkey tests were not run. |

**Deployment order.**

1. No infrastructure, secrets/configuration, database migration, API or gateway
   rollout is needed. Merging deploys nothing.
2. Release web from the merged revision through the usual web deployment. Verify
   profile save followed by space/private-channel settings and cached navigation.
   Set `MERGED_SHA` to this PR's merge commit and confirm its web image exists:

   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # After the deployment workflow for this SHA succeeds:
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```

3. Build/test Android, Apple and desktop on supported hosts before distributing
   their independent client releases. Verify the same identity update during an
   active call without losing conversation state. Roll back web to its previous
   image; native rollback requires a corrected higher-build-number release. No
   data rollback is needed.

## Eggshell primary actions and entrance motion (October 11, 2026)

Primary actions (sign-in, dialog confirmations, Join channel, Accept, Send/Save,
Forward, the thread Send and Test microphone) now use an eggshell fill
(`--action` `#F1EDE4`) with blackout labels on web, Android, iOS/macOS and Rust
desktop, matching the homepage download button. Hover brightens and press
deepens the fill; disabled primaries step back to neutral chrome (raised
surface, border, muted label) instead of a faded fill. Terracotta remains for
selection, focus, mentions and unread state. Destructive buttons are unchanged.
Sign-in fields use the dialog fields' focus treatment: a terracotta border with
a soft 2px ring instead of a 4px-offset outline.

Things that appear now arrive instead of teleporting, using shared tokens
(`--ease-out` `cubic-bezier(0.16, 1, 0.3, 1)`, `--motion-fast` 140ms,
`--motion-base` 200ms, `--motion-slow` 260ms):

- Messages you send, and live messages from others, fade in with a 6px settle in
  channels, DMs and threads. Only rows that arrive while the reader is at the
  live end animate (`Arrivals` on every client): not first loads, channel
  switches, older pages, jumps, or newer pages loaded while catching up. A
  pending send animates once, and recycled rows do not replay after one second.
- Dialogs rise 8px from 98% while their scrim fades; anchored menus, cards and
  tooltips rise 4px; touch sheets rise from the bottom; the thread panel slides
  in 12px.
- Attached files pop into the composer, and upload progress glides between
  updates.
- Closing stays instant. Reduced-motion settings (web `prefers-reduced-motion`,
  Android animator scale 0, Apple Reduce Motion) turn entrances off. Rust desktop
  has no OS reduced-motion signal; its headless tests draw final states.

No API, gateway, database, infrastructure, secret or configuration change is
needed. Web and native clients can release independently.

| Client | Validation |
| --- | --- |
| Web | `npm run check` and `npm test` pass (584 tests, including `chat/arrivals.test.ts`). Chromium against the disposable fixture at 1440px (fine pointer) and 390px: sign-in, disabled Continue, dialogs and Manage space inspected; DOM checks confirm no arrival animation on load or channel switch, one for a sent message (not replayed on confirmation) and one for a live incoming message; dialog/backdrop, message actions, mention card, thread panel and space menu entrances ran. Draft-tile entrance is CSS-only and unexercised (the fixture disables uploads). Not Safari or a physical phone. |
| Android | `compileDebugKotlin`, `testDebugUnitTest` (317 tests, including `ArrivalsTest`) and `lintDebug` pass with SDK 36/NDK 27 installed in the orb. No emulator or device rendering. |
| Apple iOS/macOS | Primary-button styles, arrivals (`MessageArrivals` with tests), entrances and Reduce Motion handling implemented. No Swift compiler or Xcode in this orb: not built, tests not run, no simulator or device rendering. |
| Rust desktop | `cargo fmt --check`, scoped clippy (`--package caper-desktop --no-deps -D warnings`) and `cargo test` pass (425 passed, 12 ignored, including arrival and entrance-timing tests). Linux Xvfb renders of `login`, `parity-requests` and `parity-attachments` inspected. Not Windows; motion itself not recorded. |

Before native distribution, check on devices: send and receive at the live end
(one entrance each, none on channel switch or history), dialogs/menus/thread
opening, attaching files, upload progress, primary/disabled/pressed/focus button
states, and Reduce Motion / Remove animations.

## Continuous message scrolling to the composer (October 12, 2026)

Writable conversations and threads no longer reserve a fixed blank strip above
the input. Typing status appears below the conversation input and takes no space
when idle. Message-row spacing remains inside the scrolling surface; keyboard
and home-indicator clearance below the composer is preserved.

| Client | Validation |
| --- | --- |
| Web | `npm run check` and `npm test` pass (585 tests). Labelled Chromium fixtures at 1280px and 390px verify a 0px timeline-to-input gap (previously 33px), image clipping at the input edge, active/idle typing, multiline drafts and filled threads. Isolated `quietTimeline` checks pass after positioning at latest, including composer growth, focus, PageUp/End and history anchoring. The full `scripts/test-chat-history.mjs` still fails in large-history live-follow/reaction scenarios reproduced on unchanged code; it is not a passing acceptance result. Not Safari or a physical phone. |
| Android | Main and thread spacing changed; idle typing row removed. `compileDebugKotlin` stops at missing Android SDK, before Kotlin compilation. No device rendering or acceptance. |
| Apple iOS/macOS | Main and thread spacing changed; typing moved below input. Existing composer UI test now asserts the timeline meets the input with closed/open keyboard and multiline drafts. No Swift compiler or Xcode in this orb: not built, tests not run, no simulator or device acceptance. |
| Rust desktop | Application tests pass (437 passed, 12 existing ignored), including clipping-edge and growing-draft checks. Formatting, application-package Clippy (`--all-targets --no-deps -D warnings`) and debug build pass. Inspected 2x Linux Xvfb attachment and thread renders. No Windows or macOS run. |

No infrastructure, secrets/configuration, database migration, API or gateway
rollout is needed. Web and native clients can release independently; merging
does not deploy them. Before native distribution, verify images, files and text
scroll to the input edge with the keyboard open/closed, typing on/off, multiline
drafts and thread replies on real iPhone/Android devices. Roll back web to its
previous image; native rollback requires a corrected higher-build-number release.

## Staged files and thread replies survive conversation switches (October 12, 2026)

Switching away from a channel or DM no longer gives up its staged files or
thread reply drafts. Channel to DM, DM to channel, channel to channel and DM to
DM all keep each conversation's text, files and thread replies until you come
back, send them, remove them, or sign out. Uploads keep running while their
conversation is closed and are not re-uploaded on return; files whose upload
failed, or is older than the 23-hour renewal point, are renewed on return
(Apple restarts an upload that was still in flight when you switched). Only
explicit **Remove**, sign-out, or loss of access to that conversation cancels a
file's reservation. Drafts live in memory: closing or reloading the app still
discards them.

| Platform | Validation boundary |
| --- | --- |
| Web | `npm run check` and all 594 `npm test` tests pass, including new store tests. In Chromium against the local fixture: #general text, a file and a thread reply, #design text plus a file, then the self-DM and back: each conversation returned with its text, files and thread reply, with no re-upload or cancellation; sending the restored #general draft delivered the file; Remove still cancelled. |
| Android | `:app:compileDebugKotlin`, `:app:testDebugUnitTest` (329 JVM tests) and `:app:compileDebugAndroidTestKotlin` pass with platform/build-tools 36. No emulator here: no device switch check. |
| iOS/macOS | Implementation only; Swift sources parse with Linux Swift 6.1. No Apple SDK build, XCTest or device check. |
| Rust desktop | `cargo test -p caper-desktop`: 449 passed, 12 device/audio ignored, including a new switch test for text, files, background progress and thread replies; fmt and Clippy pass. The one failure, `compress::tests::usage_compression_settings_decode_with_defaults`, comes from #550 on main (it still expects the old 1080 video cap) and is untouched here. |
| Services | No API, gateway, schema or configuration change. |

**Deployment order.**

1. No infrastructure, secrets/configuration, database migration, API or gateway
   rollout is required. Merging deploys nothing.
2. Deploy web from the merged revision with the usual workflow; Android, Apple
   and Rust desktop release independently. Roll back web to the previous image;
   native rollback requires a corrected higher-build-number release. No data
   rollback is needed.

## Swipes no longer tap where they end on iOS (October 12, 2026)

On iPhone, a Browse/conversation or member-list swipe also triggered the
control under the finger when it lifted. Browse rows span the full width, so
swiping left along **New message** (or **Invite people**) returned to chat and
then opened the New direct message dialog. In the conversation the content moves
with the finger, so a link, chip or slow long-press under it could fire too.
`DrawerSwipe` now stops hit-testing its content once a slide follows the finger,
so the controls under it drop their press; the swipe keeps its touch through the
outer content shape. Web (room-level release-click suppression), Android
(consumed moves cancel the child click) and Rust desktop (the swipe owns the
drag, so egui sees no click) already behaved this way and are unchanged.

The iOS New direct message dialog's **Start conversation** now uses the primary
eggshell action, neutral while the username is empty, as on the other clients.
It previously used the terracotta voice style, which looked enabled while empty.

| Platform | Validation boundary |
| --- | --- |
| iOS/macOS | `testNarrowConversationAndBrowse` now swipes along the New message/Invite people row and asserts no dialog opens. Changed sources parse with Linux Swift 6.1. No Apple SDK, Xcode, XCTest, simulator or device run here; needs a physical iPhone check of swipes from Browse rows, timeline links/chips and the member list. |
| Web, Android, Rust desktop | Unchanged; confirmed by code reading only. |
| Services | No API, gateway, schema or configuration change. |

**Deployment order.**

1. No infrastructure, secrets/configuration, database migration, API, gateway,
   web, Android or Rust desktop rollout is required. Merging deploys nothing.
2. Release the Apple clients from the merged revision with the usual TestFlight
   workflow. Rollback requires a corrected higher-build-number release; no data
   rollback is needed.

### Inline profile DMs (October 11, 2026)

Other people's profile cards have an independent, per-person composer on web,
Android, iOS/macOS and Rust desktop. It supports text, emoji and up to 10 files,
including file-only messages. File selection stages locally; Send resolves the
DM and uses the existing compression, reservation, PUT and completion pipeline.
Confirmed delivery clears that person's draft and opens the conversation.
Open conversation navigates without sending; known incoming requests remain
read-only until accepted. Block and DM privacy refusals keep the draft inline.

Layout, the same on every client: the header has the avatar, name and
`@username` with a neutral More profile actions (ellipsis) menu, Open
conversation (speech bubble) and Close icons at the end. Block/Unblock is
inside that secondary menu and blocking still requires confirmation;
Invite to space stays below the header. A full-width divider separates the
composer. The composer reuses the channel composer's shell (Attach/plus at the
start, then the field, Emoji and an eggshell primary Send) with staged files above it
as the channel's draft chips, including image thumbnails and sizes. Delivery
uncertainty is one line ("Delivery isn't confirmed"); web adds Retry and Edit
there. Keyboard hints stay in accessible descriptions, not visible text.

Outside clicks/taps close cards even when they contain draft text or files;
active sends and file/emoji pickers still prevent accidental dismissal. iPhone
uses a backdrop-only tap recognizer on the native sheet's presentation container,
leaving detents, dragging, keyboard handling and child dialogs native.
Pickers preserve the draft and saved insertion point; Close also leaves it saved. Drafts survive
close/reopen while signed in, not logout or app/page restart. Profile navigation
retains the underlying conversation's text and, with #544, its staged files and
thread replies, as for any conversation switch. An unconfirmed POST
freezes its original text, attachment IDs and client message ID; Retry sends that
same command and reuses completed uploads. Web's Edit action warns that the
previous copy may have arrived before allowing changes.

Clipboard files: web accepts files carried by a browser paste event; Apple uses
its native Paste action for file URLs and image bytes. Android and Rust desktop
put **Paste from clipboard** in the Attach menu (beside Photos/Files or Choose
files): Android reads clipboard content URIs, and egui delivers no event for an
image-only Ctrl+V. Native clipboard formats and provider permissions vary.

Emoji buttons: channel, thread and profile composers on every client have an
Emoji button beside the existing `:name:` shortcut. It opens the reaction
picker (a popover above the button on web desktop, Rust desktop and macOS; a
bottom drawer or sheet on touch layouts) and inserts at the caret or replaces
the selection, within the 4,000-character limit. Native pickers now list
smileys first and flags last, approximating web's category order.

| Platform | Validation and remaining acceptance |
| --- | --- |
| Web | `npm run check` and 594 `npm test` tests pass. `PROFILE_TEST_WEB_URL=… node scripts/test-profile-composer.mjs` against the local parity fixture passes (text/emoji/files, file-only send, chooser cancellation/caret, outside clicks, per-person drafts, lost acknowledgment with identical retry and one persisted message, rejection, retained channel text/files (no re-upload), incoming-request isolation, wide/narrow layouts). Channel and thread emoji insertion at the caret checked in Chromium. Rendered desktop/narrow and picker states inspected. File paste uses a synthetic browser event, not the OS clipboard. |
| Android | `:app:compileDebugKotlin`, `:app:testDebugUnitTest` (350 JVM tests) and `:app:compileDebugAndroidTestKotlin` (including the updated profile composer UI tests) pass with platform/build-tools 36. Instrumentation was compiled, not run; no emulator here (no KVM), so no rendered, keyboard, picker, clipboard or device acceptance. |
| iOS/macOS | Changed Swift files parse with Linux Swift 6.1; the picker-order logic was compiled and run standalone. No Apple SDK typecheck, XCTest, render or device/picker/clipboard acceptance. |
| Rust desktop | 458 tests pass (12 device/audio ignored), including new channel emoji insertion, image thumbnail and picker-order tests; fmt and Clippy pass. The one failure, `compress::tests::usage_compression_settings_decode_with_defaults`, is on main from #550 and untouched here. Linux X11 renders inspected: card layout, real clipboard image pasted through the Attach menu with its thumbnail, profile and channel pickers anchored at their buttons. Windows, Wayland, native file picker and live storage acceptance remain separate. |
| API/containers | No API changes; no Docker image changes. No live storage, multi-account production or physical-device checks. |

**Profile actions follow-up (October 12, 2026).** The table above records the
earlier composer validation, not acceptance of the new profile menu and dismissal:

- Web: `npm run check`, 598 `npm test` tests and
  `node scripts/test-profile-composer.mjs` pass. The regression checks the hidden
  Block action, menu/confirmation, paperclip, outside dismissal with retained
  text/files, and active-send protection. Desktop/narrow renders were inspected;
  narrow Chromium is not an iPhone/device check.
- Rust desktop: 476 tests pass (12 device/audio tests ignored); fmt, build and
  Clippy pass. Linux X11 default/menu renders were inspected, and real outside
  clicks followed by reopening retain the typed draft.
- Android: implementation and instrumentation updated, but this follow-up's
  Gradle check stops at missing Android SDK configuration. No compilation,
  instrumentation run, render or device acceptance for these changes.
- iOS/macOS: implementation and parity UI regression updated. This orb has no
  Swift compiler or Apple SDK; typechecking, XCTest, native menu rendering and
  iPhone backdrop taps still require Apple validation before release.

**Deployment order.**

1. Existing account, DM, chat and asset APIs must be available. No new
   infrastructure apply, secret/configuration, database migration, API or gateway
   rollout is required for this feature. File sending needs existing storage
   configuration. Web and native clients can release independently.
   Merging does not deploy them; set `MERGED_SHA` to the full merged revision and
   wait for its immutable web image before rollout.
2. Deploy web with the existing operator workflow, then wait for readiness:

   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$MERGED_SHA"
   # After that deployment workflow succeeds:
   kubectl -n default rollout status deployment/caper-web --timeout=15m
   ```

3. Before native distribution, compile/test with Xcode and the Android SDK and
   complete real picker, clipboard, keyboard/focus and dismissal acceptance on
   each platform. Then publish signed native clients through the existing release:

   ```bash
   gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$MERGED_SHA"
   ```

4. With separate test accounts, verify text, the Emoji buttons, attached/pasted files,
   file-only send, draft close/reopen, preserved underlying drafts, offline retry
   without duplication, blocked/private recipients and incoming-request preview.
   Roll back web using the same workflow with the previous known-good SHA; native
   rollback requires a corrected higher-build-number release. No database or
   infrastructure rollback is needed. No deployment or release was performed.

## Swipe release taps, caught slides and smoother drawers (October 12, 2026)

On iPhone a sideways swipe that started on an image opened the media viewer when
the finger lifted: the conversation moves with the finger, so it lifts on the
control it started on, and only Browse's rows skipped that tap. Drawers also
jumped when a finger caught one mid-slide (it restarted from where it was headed),
jumped by the touch slop as a swipe began, and the phone member list emptied the
moment it started closing.

- **Release taps:** every control on a swiping surface now ignores the tap a
  swipe's lift delivers: images, videos, animations, audio and file cards, links,
  mention pills, avatars and names, reactions (and their hold), thread and edit
  links, header voice controls, Back, member rows and the space rail. Apple uses a
  shared `SwipeTapGuard`: a time window renewed by every move (0.35s) and by the
  lift (0.12s), so a lost gesture can never keep controls off. Each drawer swipe's
  state belongs to one gesture: it resets when the gesture ends or a new one
  starts elsewhere, and the list stops scrolling only while it lasts. Android consumes
  the move once a drag claims it, and a window-level guard cancels presses after
  twice the touch slop when nothing claimed it. Web already suppressed release
  clicks.
- **Caught slides:** a finger that lands on a settling Browse or member slide picks
  it up where it is drawn, on web, Android, Apple and the Rust desktop. Swipes
  follow the finger from the slop with no jump.
- **Media viewer:** swipe-down closing locks its direction on the first move,
  follows the finger 1:1, and springs back (or carries off) smoothly instead of
  snapping. Android's backdrop fades as it leaves and double-tap zoom animates.
- Reduced motion and system animation scale still switch at once.

| Client | Validation |
| --- | --- |
| Web | `npm run check` and `npm test` pass (604 tests). A focused Chromium touch check against the native-parity fixture at 390px: horizontal swipes starting on avatars and author names open Browse with no click, swiping back along a channel row selects nothing, and a plain tap still clicks. `scripts/test-navigation-release.mjs` fails its intermediate-paint assertion here on unchanged `main` too, and `scripts/test-desktop-navigation.mjs` is stale against the current header; neither is evidence about this change. Not Safari or physical-device acceptance. |
| Android | With SDK 36 installed locally: `:app:compileDebugKotlin`, `testDebugUnitTest` (379 tests, including the new `SlideMotionTest`) and `lintDebug` pass. No emulator or device run. |
| Apple iOS/macOS | Swift 6.1 on Linux parses the changed sources and tests, and the new drawer timing and tap-guard code type-checks against a SwiftUI stub. No Xcode build, XCTest, simulator or device run here. New `DrawerMotionTests` and the iOS parity test `testSwipeFromMessageControlsDoesNotTapThem` run on `main`'s native builds. |
| Rust desktop | `cargo fmt`, `cargo test -p caper-desktop` (483 tests) and `cargo clippy --all-targets -D warnings` pass on Linux with clang 21. Windows built only by CI. |

Before native distribution, verify on physical devices: a horizontal swipe from an
image, link, avatar and reaction opens Browse or the member list without opening
it; grabbing a slide mid-flight continues from where it is; vertical and slightly
diagonal timeline scrolls still scroll; and swipe-down closes the media viewer.

## Drawer releases no longer bounce off the far side on iOS (October 12, 2026)

On iPhone, letting go of a Browse or member-list swipe could briefly draw the
drawer past where it rests, on the opposite side, before it eased back. For
example, closing Browse moved the conversation left of the screen edge, showing
Browse on its right. SwiftUI animated the drawer's pose from separate state
changes (the finger's drag ending, the navigation change, the recorded slide).
Since #570 those use bezier curves, which add up where they overlap instead of
merging like the earlier springs.

- **Apple:** a `DrawerPose` modifier draws each drawer layer (the conversation,
  the account bar, the member panel and its backdrop) from one value per frame:
  under the finger, along the recorded `DrawerSettle` curve with a
  `TimelineView`, or at rest. No SwiftUI animation touches the pose, so a release
  continues from the finger to its rest and can't pass either end. A model change
  that records no slide of its own (a channel that finished opening) slides from
  where the drawer was drawn. Reduce Motion still switches at once.
- **Rust desktop:** a drag that takes a drawer no longer jumps it by the 12pt touch
  slop. It follows from where the finger took hold, as on web, Android and Apple.
- Web and Android keep one position per drawer and were reviewed. They have no
  counterpart to this bug.

| Client | Validation |
| --- | --- |
| Apple iOS/macOS | Swift 6.1 on Linux parses `CaperView.swift` and `DrawerMotionTests.swift`. The drawer position logic and all of `DrawerMotionTests` (including new release, model-change, mid-slide reversal and Reduce Motion cases) compile and pass on Linux against a bezier `UnitCurve` stand-in. `DrawerPose`/`BrowseLayer` type-check against a stub with SwiftUI's signatures. No Xcode build, simulator or device run here. |
| Rust desktop | `cargo fmt --check`, `cargo test` (485 passed) and `cargo clippy --package caper-desktop --all-targets --no-deps -D warnings` pass on Linux with clang 21. The narrow swipe tests now check that taking a drawer past the slop doesn't move it, and that it then follows the finger point for point. |

Before native distribution, verify on an iPhone: a slow and a fast release in each
direction for Browse and the member list continue smoothly to their rest with no
bounce past it; Back and channel choices still slide; and a finger still catches
a drawer mid-slide.

## Emoji names inside composers (October 13, 2026)

Channel/DM, thread and profile composers show the picker's catalog name when an
inline emoji is hovered or held. Joined emoji, flags and keycaps stay one target;
ordinary text and unnamed catalog variants do not show a tooltip. This does not
change messages, reactions, picker insertion or native text selection.

- Web, Android and iOS use a 400ms hover and 500ms hold. A completed hold remains
  readable for 1.5 seconds after release. Movement, editing, scrolling and leaving
  the composer dismiss it; entrance motion honors reduced motion.
- macOS uses text-range AppKit tooltips. Rust desktop uses egui's hover/long-touch
  timing and keeps a completed long-touch tooltip visible throughout the hold.
- Web reuses the mention mirror without changing text metrics and associates the
  tooltip with the textarea's existing accessible description. Its name lookup
  normalizes padded keycap code points to match the catalog.

| Client | Validation |
| --- | --- |
| Web desktop/touch | `npm run check` and `npm test` pass. `scripts/test-emoji-tooltips.mjs` mounts the actual composer in a labelled local fixture and checks hover, real Chromium touch input with `(pointer: coarse)`, joined emoji/flags/hearts/keycaps, wrapped lines, Escape/edit/tap/movement/release dismissal, unchanged drafts and no sends. Desktop and 390px tooltip captures inspected. Not Safari or physical-device acceptance. |
| Rust desktop | Desktop tests and application Clippy pass on Linux with clang 21; a rendered-editor regression checks names across newlines/joined emoji and persistent touch holds. An actual Linux window's composer hover tooltip was captured and inspected against the labelled local fixture. No physical touchscreen or Windows/macOS run here. |
| Android | Implementation present; Gradle configuration stops because this orb has no Android SDK. No compilation, emulator or device validation. |
| Apple iOS/macOS | Implementation present; no Swift compiler/Xcode SDK in this orb. No platform build, XCTest, simulator or device validation. |

Before native distribution, compile and inspect the native composers with their
platform toolchains. Verify that long presses still select text and reveal names,
scrolling dismisses names, and closing/covering a composer leaves no tooltip.
No API/gateway, infrastructure, secret or database change is required. Web and
native releases can deploy independently; reverting a client removes only these
tooltips and needs no data rollback.

## Release highlights (What’s new)

Web, Android, iOS, macOS and Rust desktop implement an after-update highlights
view and a User Settings entry for retained history. The first successful fetch
on a new installation quietly records its baseline. Later builds show unread
highlighted releases newest-first in one view; maintenance-only releases advance
the baseline without opening anything. Every dismissal acknowledges the displayed
snapshot. Failed fetches do not acknowledge it, and rollbacks never lower the
saved sequence. Preferences are installation/API-origin/platform-local, not
account-synced. Automatic presentation waits for the signed-in workspace and
defers during calls or another modal. Missing publication retries at most once
per minute while foregrounded; Settings offers immediate retry.

History groups releases under one UTC publication date. Each native release has
one full version link above its highlights; web releases and legacy native entries
without recorded versions use a short commit link instead. Links open that exact
source revision on GitHub. Every client's header matches the
desktop update review: the Caper face, the title, then the installed version right
under it (`Version 0.1.<build>`; web, which has no build number, shows `Web build
<commit>`), linked to its commit, with no intro line. Version links are muted and
underlined and brighten on hover or focus. Release days read like "October 18, 2026"
everywhere.
The native release workflow supplies its actual `0.1.<build>` as optional `version`
metadata in schema-1 snapshots; the API preserves it and older clients ignore it.
Previously published snapshots remain immutable: missing versions are never guessed
from feed sequences or dates.

Mac/Windows/Linux expose **Check for updates** in the account menu and Settings.
Automatic checks advertise a known update through a persistent **Update available**
button above the profile/audio controls; they do not take focus. The button has
its own row, outside the fixed-height account controls, and never animates away.
A thin rainbow edge stays still while a short light sweep passes every eight
seconds. Reduce Motion, disabled controls and background windows stop the sweep;
the decoration never moves the label or intercepts pointer/keyboard input.
Clicking it or completing a manual check with an available release opens a centered
modal with **What’s new expanded by default**. Only the notes scroll; restart/Later
and installer actions stay visible. Later, Escape or the backdrop close the review
without forgetting the update. Covered conversation typing, sending and drops are
blocked. Confirmed restart still leaves a call; non-writable copies use the installer.

Desktop clients pass their source revision to the bundled updater, which reads
current/target exact snapshots and shows the same user-facing highlights for skipped
releases. When the installed build's own snapshot is missing (its notes failed to
publish), the updater places it in the target feed by revision, else by the build
number in each entry's `version`, else shows only the target release, and reports
the history as incomplete instead of showing no highlights. The updater returns them as one stacked list (newest first, repeated
titles dropped) with the newest release's date, so the review shows a date
next to the version (linked to its commit on GitHub), bold titles over short
descriptions and a close button. Installing from the review marks that build's
highlights read, so the restarted app does not open the same What's new again. After a failed update, the error's "use the installer" links straight to
the installer; there is no separate footer link. Signed in, **Send this to the
Caper team** posts the error, installed build and target version as `@feedback`
in your personal notes (no navigation); the team reads and replies there. Missing editorial metadata never blocks installing the signed update.
Older clients keep their signed commit-title changelog until updated once. Linux
fixture rendering and modal regressions cover narrow screens, errors, history gaps,
call warnings and installer fallback; physical Mac/Windows focus, scrolling, motion
and signed update/relaunch acceptance are still required.

### Generation and storage

After Ship rolls out Caper web, infrastructure dispatches `whats-new.yml` on Caper
`main` with the exact confirmed image SHA, authenticating as the Godis deploy
GitHub App (no separate token). It holds the web deployment queue until
generation finishes so successive shipped versions cannot overwrite pending
dispatches. The native release calls the same workflow twice: a draft while the
apps build (`--save-answer`, publishes nothing, keeps Claude's answer as a run
artifact), then a publish after the Apple Silicon and desktop builds succeed and
just before the self-update manifest, so an update never goes live before its
notes. Publishing reuses the draft (`--answer`) only when its evidence hash is
unchanged and otherwise asks Claude again; a failure never holds back the
release. Snapshots describe a revision only to clients running it, so iOS and
Android entries may publish before TestFlight or Play distribution completes. The existing Godis Discord buttons need no code or deployment change.
Committing, building an image, or a failed deployment does not generate notes.

`scripts/whats_new.py` reads the previous per-platform revision, first-parent commit
messages, relevant runtime diffs, and recent highlights. One bounded Claude Messages
API request selects/groups at most five highlights. Tests, infrastructure, tooling,
dependency maintenance and server-only changes cannot independently announce a
client feature. Output must cite commits in each platform's release range, have
changed client/shared runtime evidence, and pass local schema/text/claim checks.
An empty list is valid. These checks constrain errors, but do not prove the model's
prose is factually correct; review generated assets when checking a release.
No model call occurs on a user's device or app open, and no user/account/message
data is sent to Claude. Repository source evidence is sent to the configured API.

Public GitHub prereleases `whats-new-web`, `whats-new-android`, `whats-new-ios`,
`whats-new-macos`, `whats-new-windows` and `whats-new-linux` hold immutable
`<source-sha>.json` snapshots and rolling `latest.json` heads. No database, bucket,
Terraform, API secret or new service is needed. All platform snapshots validate
before publication; immutable assets precede rolling heads. Exact reruns reuse
published bytes without calling Claude. Partial uploads reserve their sequence
and recover on a retry or newer release. Known rollback revisions reuse their
old snapshots without lowering rolling heads; an unrecorded historical rebuild
gets a quiet, new sequence. Do not overwrite/delete immutable assets.

The homepage's Latest changes shows the newest eight highlights from
`whats-new-web/latest.json`. Each web pod reads it when rendering the homepage,
keeps one copy for 60 seconds (a stale copy is served while it refreshes, and
a cold fetch waits at most 3 seconds), and otherwise uses the list captured at
build time: those highlights, or recent `main` commit titles when the feed was
unavailable or empty. A web rollback does not lower the rolling feed, so the
homepage may still list a rolled-back highlight until the next release.

The public API `/api/whats-new?platform=<platform>&revision=<40-lowercase-hex-sha>`
only reads the requested immutable asset from the fixed Caper repository. Clients
never read rolling `latest.json`, so an old build cannot announce future features.
The API has bounded GitHub fetches, per-revision coalescing and a 60-second
success/failure cache. Revisions that loaded (up to 32) are counted apart from
in-flight and failed lookups (up to 32), so unknown revisions can't evict a
working one. Once the failure side is full, a new revision replaces the oldest
settled failure, so made-up revisions can't keep a new build's notes from
loading; only when all 32 are still in flight does a new revision get `502`. A slow fetch never holds up other
revisions.
Each snapshot is at most 64 KiB UTF-8, retains at most 30 chronological releases,
and has positive JavaScript-safe integer sequences. Titles/bodies are plain text,
at most 100/500 Unicode code points. Missing metadata fails closed.

### Deployment order

1. Merge the Caper change, then the infrastructure dispatch change. No Godis
   release, infrastructure apply, database migration, gateway rollout or runtime
   secret change is needed. Merging does not deploy apps or activate generation.
2. Claim the plan's Claude Platform credits by linking the Console organization
   in Claude billing. Subscription sign-in credentials are not API credentials.
   With authenticated `gh`, save the Console API key; it prompts privately, so do
   not paste keys into scripts or commit them. Then, from the infrastructure
   checkout with AWS signed in, copy the existing Godis deploy App's identity
   into infrastructure so Ship can start this workflow:

   ```bash
   gh auth status
   gh secret set ANTHROPIC_API_KEY --repo joswayski/caper
   ./scripts/store-ship-dispatch-secrets.sh --profile production
   gh secret list --repo joswayski/caper
   gh secret list --repo joswayski/infrastructure
   ```

   The generator defaults to `claude-opus-5-5` at `high` effort with
   `fallbacks: "default"`, so a classifier decline retries on Anthropic's
   recommended model. Optional Caper Actions variables `CAPER_RELEASE_NOTES_MODEL`
   and `CAPER_RELEASE_NOTES_EFFORT` (`low`-`max`) override them. A key that is
   not scoped to one workspace is rejected (HTTP 400 `invalid_request_error`)
   unless the `ANTHROPIC_WORKSPACE_ID` Actions variable names the workspace;
   a workspace-scoped key needs no variable. One request per
   shipped release sends at most 400,000 characters (about 100K tokens, under
   $0.50 at Opus 5.5 rates); a typical release is far smaller. When a long gap
   between releases exceeds that, server-only diffs are dropped and client
   diffs shortened evenly (`diffTruncated`); every commit's subject, body and
   paths are always kept. The native workflow uses its own `contents:write` token.
   No AWS/Cloudflare secret, OpenTofu initialization/plan/apply or new capacity
   is required. No shared credentials were configured during development.
3. Choose the merged Caper SHA and wait for its immutable web/API images to exist.
   Deploy the API first, preserving the normal one-replica/shared-state policy:

   ```bash
   CAPER_SHA="$(gh api repos/joswayski/caper/commits/main --jq .sha)"
   gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$CAPER_SHA"
   ```

   Wait for that exact rollout to succeed. Web and native clients can then deploy
   independently through their existing Discord buttons, or from the repo root:

   ```bash
   gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$CAPER_SHA"
   gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$CAPER_SHA"
   ```

4. Verify a first release quietly establishes each feed; a later user-facing
   release creates the exact SHA asset and presents once on an already-baselined
   installation. Check skipped releases, maintenance-only updates, Settings,
   offline/error/retry, other modals, calls and restart. Inspect generated text
   and native scrolling on supported devices. Repair a generation failure by
   rerunning only for a revision that actually shipped:

   ```bash
   gh workflow run whats-new.yml --repo joswayski/caper --ref main -f audience=web -f git_sha="$CAPER_SHA"
   gh run list --repo joswayski/caper --workflow whats-new.yml --limit 5
   gh release download whats-new-web --repo joswayski/caper --pattern "$CAPER_SHA.json" --dir /tmp/caper-highlights-review
   curl --fail --silent --show-error "https://caper.chat/api/whats-new?platform=web&revision=$CAPER_SHA" | jq .
   ```

   Use `audience=native` and the corresponding platform tag for native repair.
   Never generate notes for a PR branch or an unshipped revision. No workflow
   dispatch, production publication or live Claude request was executed here.
5. Roll back clients using the existing exact-image or higher-build-number native
   release process. Do not delete history or lower acknowledgements. Removing
   `ANTHROPIC_API_KEY` stops future generation without affecting deployment;
   existing immutable notes remain available. Prefer a forward API fix; missing
   notes do not affect conversations or signed update verification.

### Validation boundary

| Component | Executed checks and limitations |
| --- | --- |
| Web | `npm run check` and `npm test`: 609 tests pass after rebasing. Inspected final wide/narrow, retained-history, loading, error and empty Chromium renders with eggshell primary actions; exercised quiet baseline, dismissal, visible desktop/narrow focus return, scrolling to the final paragraph above the fixed footer, enabled retry/recovery and no-repeat after reload. Loopback-only native parity fixture, not live Claude output or a physical phone. |
| API | Workspace fmt, tests and Clippy pass: 162 tests pass, 67 existing database/network tests ignored. Public HTTP regression covers pinned revisions, cache/backoff and bounded arbitrary-SHA traffic. The final run used one build job with browser/dev services stopped after a concurrent-build OOM. |
| Rust desktop | 500 tests pass on the integrated base, 12 existing device/audio tests ignored; fmt and application-package Clippy pass. The final test/app binaries use `cargo rustc --tests -- -Clink-arg=-fuse-ld=lld` and `cargo rustc --bin caper-desktop -- -Clink-arg=-fuse-ld=lld` after a default-linker OOM. Inspected final 2x real Linux/Xvfb wide/narrow, loading, error, empty and Settings fixtures. Windows build/runtime remains unverified. |
| Bundled updater | 18 tests, fmt and Clippy pass; real local HTTP regression checks current/target snapshots, skipped/silent releases, retained-history limits and mismatched revisions. Signed production updates were not executed. |
| Android | Isolated native implementation: 305 JVM tests, lint, instrumentation compilation and release revision configuration pass. No device/emulator UI execution or full native-media APK packaging; the integrated latest-main native app was not rebuilt in the parent orb. |
| Apple | Isolated implementation: four actual Foundation contract tests pass under Swift 6 on Linux and changed Swift parses for macOS/iOS targets. Plists/shell scripts and revision override/fallback/rejection checks pass. Final rebased window/Settings and button-style changes were not compiled or run on Apple; no Xcode/SDK typecheck, SwiftUI/AppKit execution, Apple API/updater tests or physical device checks. |
| Generator/release tooling | Ruff, 53 Python tests (one existing macOS-only skip), four font tests and Actionlint pass. TestFlight regression rejects mismatched source metadata before upload. No live provider/GitHub calls. |
| Infrastructure | Fake-GitHub highlights dispatch, disposable-Git deployment and cluster-verification regressions pass, as does Actionlint. No shared state was changed. |

Live Claude schema acceptance, GitHub publication, production dispatch, physical
phone/Mac testing and Mac scroll frame timing remain separate rollout checks.
No Docker daemon was available; web/API build stages were checked directly.

## Cross-client UI consistency pass (October 13, 2026)

A second audit of every client for dismissal, focus, hover, clipping, parity and
performance problems, on top of #616. Behaviour changes ship on every client they
apply to; the rest are one-client defects.

- **Layout and clipping:**
  - Web: between 761 and 1099px, Members no longer stack under the composer. They
    sit beside the conversation while it keeps 320px, as on desktop, and otherwise
    float over its right edge below the header, as on desktop and Android.
  - Web: resizing the window or a panel keeps the newest message in view when you
    were already reading it.
  - Web: the conversation grid has a column track, so no long header content can
    widen it; an empty composer keeps a long placeholder on one line.
  - Desktop: long (up to 80-character) channel and DM names truncate in the wide
    header instead of running under Join voice, Pins and Members. The member list
    scrolls (a full page and its pager stay reachable in short windows) and long
    names truncate. The sidebar space name truncates with a tooltip. Channel
    settings scrolls to its bottom edge instead of clipping above an empty band.
- **Dismissal and focus:**
  - Web: Block and New direct message opened inside the hidden phone sidebar,
    leaving an invisible modal that swallowed every tap. They now open at the top
    level. Phones also get Block in a DM's name menu, where the header button is hidden.
  - Web and desktop: a backdrop click no longer discards unsaved Channel settings
    or a renamed space. Escape and the close button still close them.
  - Web: Forward can't be closed while forwarding; row menus re-place themselves
    when Notifications or Mute expand inside them; the pinned message ⋯ toggles.
  - Desktop: changing channels or closing a thread closes its composer emoji
    picker, which could otherwise insert into the next conversation's draft. A
    dialog opened over a profile card gets Escape first. Files dropped over a
    dialog or modal no longer go to the hidden composer. Browse channels closes on
    a space switch. Choosing System default closes the device menu; long device
    lists scroll.
  - Android: opening Browse or Members on a phone hides the keyboard. Back is only
    caught when Members actually shows. The message actions sheet opens fully and
    scrolls. The thread's "Also send to" label toggles its checkbox.
  - Apple: a dialog's disabled composer stops taking keystrokes. The timeline only
    follows new messages when you are at the bottom (or sent them). A pending
    private-channel invitation can't be dismissed, and finishing it no longer
    closes another dialog. Browse closes on a space switch and ignores a leading
    `#`. Reaction and pin errors can be dismissed. A failed join no longer carries
    over to the next channel.
- **Labels and actions:** every client hides **Copy text** for attachment-only
  messages and says **Message history**. Desktop's menu adds **Reply in thread**
  and says **Pin message**/**Unpin message** (**Saving…** while pending), the hover
  bar's ⋯ says **More actions**, **New direct message** replaces "Start direct
  message", and **Leave channel** is neutral. Android says **New direct message**;
  its DM rows say **Blocked** or **Request sent** like web. The macOS menu adds
  **Reply in thread** and **Add reaction…**. Apple's thread composer shows its
  placeholder.
- **Hover, touch and tokens:**
  - Desktop: hover fills use the shared 6% white `--surface-hover` wash instead of
    RAISED, which matched the sidebar and conversation backgrounds. Channel rows
    brighten on hover and keep the selection wash while focused. Menus and windows
    use BORDER and 8px corners.
  - Web: three hover styles no longer stick after a tap.
  - Android: file-card ripples keep their corners; account-bar volume sliders tick
    like the Audio test's; composers capitalize sentences.
  - Apple: muted/deafened account controls use web's red (#ED5265); iOS sidebar
    icon buttons show press feedback; macOS DM rows get one hover highlight
    instead of two; a few more error labels use `CaperTheme.error`.
- **Performance:**
  - Web: call statistics re-render the call each second only while Connection
    details shows them; the Join glow animates opacity instead of repainting a
    shadow; emoji search normalizes the catalog once; emoji preloading waits for idle time.
  - Android: typing indicators are throttled like web (at most one start per
    500ms, a stop after 500ms idle) instead of one gateway command per keystroke.

No API, database, gateway, infrastructure or configuration changes. Web and native
clients deploy independently; merging alone deploys nothing.

| Client | Validation |
| --- | --- |
| Web | `npm run check` and `npm test` (610 tests) pass. Chromium against the disposable native parity fixture at 1440, 1300, 1024, 900, 820 and 390px (DPR 2, fine pointer): the medium Members layout beside and floating, latest message kept in view through resizes, phone Block dialog visible and closable, an 80-character channel name in desktop and phone headers and a one-line phone placeholder. Not Safari, Firefox or a physical phone. |
| Rust desktop | `cargo fmt --check`, `cargo test -p caper-desktop` (501 passed, 12 ignored device tests) and `cargo clippy --all-targets --no-deps -D warnings` pass on Linux with clang 21. Linux Xvfb fixture renders inspected: Channel settings scrolling to its bottom edge, row and member hover washes. Windows built only by CI. |
| Android | Edited without an Android SDK: not compiled or run here. Imports and API signatures were checked against the compose-bom 2025.09.01 APIs already used. CI must compile it. |
| Apple iOS/macOS | Edited without Xcode or a Swift toolchain: not compiled, tested or run here. CI must build it. |

Before native distribution, verify on devices: Android Browse/Members hiding the
keyboard and Back with a hidden member list; the full-height actions sheet; Apple
typing with a dialog open, timeline follow while scrolled up and at the bottom,
and the invitation dialog while pending.

## Conversation slides over all of Browse (October 13, 2026)

On phones the conversation is now one full-height layer above the whole of Browse,
including the account and voice bar, as in Discord. Browse (space rail, channel
list and account bar) stays still beneath it; the bar no longer rises over the
conversation from the bottom edge. The conversation slides past the screen's edge
(width + 48 px/dp/pt, so its shadow leaves too) and follows a finger 1:1.

Buttons, Back and channel choices slide over 300 ms × the distance left (at least
160 ms) with `cubic-bezier(0.33, 1, 0.68, 1)`: it leaves at once and slows
steadily. The previous 220 ms `cubic-bezier(0.32, 0.72, 0, 1)` covered about half
the way in two frames, then crawled for six. The member list's button slides use
the same timing. Released swipes keep their speed-matched curve.

| Client | Change and validation boundary |
| --- | --- |
| Web ≤760px | The slide no longer uses View Transitions. Browse is always laid out in the room (hidden under the conversation, keeping its scroll); the `.stage` is an absolutely positioned card above it, moved by WAAPI on the element itself, while `data-browse-slide` on the room shows both. The room no longer clips horizontally, so the conversation leaves the screen rather than being cut at the room's inset edge, and nothing is snapshotted, measured or re-laid out mid-slide (`useBrowseLayout`, `--browse-stage-top`, `--browse-bar` and the voice panel's band are removed). Settled conversation and Browse look as before. `npm run check` and `npm test` pass. Chromium at 390px with CDP touch input against `native-parity-fixture.mjs`: button slides recorded frame by frame both ways; drags follow the finger 1:1; slow past-half, held-short cancel, short flicks both ways, vertical scroll, and catching a tapped slide mid-flight all behave; the member drawer, channel choice from Browse and the 1280px layout were inspected. Browser scripts that waited on `data-browse-transition` now wait on the room's `data-browse-slide`. Not Safari or physical-device acceptance. |
| Android narrow | The account bar is part of Browse's column beneath the rail and channels; `browseBarLayer` is removed. Browse's semantics are cleared while it is covered. `conversationLayer` and `browseDrag` share `browseTravel` (width + 48dp). `drawerTapMillis`/`TapEasing` replace the settle timing for button slides; `SlideMotionTest` covers it. `./gradlew :app:compileDebugKotlin` (SDK 36) and `SlideMotionTest` (11) pass. No emulator or device run. |
| Apple narrow | The account bar is the last item in Browse's VStack; its rise offset, height state and `hidesCovered` are removed. `DrawerSettle.Timing.tap` is 0.30 s on the new curve; the member list's button slides use it. `DrawerMotionTests` adds a tap-curve test. No Swift toolchain in this orb: no build, XCTest, simulator or device run. |
| Rust desktop narrow | Browse, account bar included (a bottom panel), is drawn first and the conversation after it, with a backdrop so presses on the conversation never reach Browse. `browse_travel` (width + 48) is shared by pose and touch. `NavigationSettle::toggle` uses the tap timing scaled by distance. With clang 21 on Linux, `cargo fmt --check`, application Clippy (`--no-deps`, `-D warnings`) and `cargo test` (495 passed, 12 existing ignored) pass; the drawn-pose catch test now allows for the tap curve still moving when a finger takes hold, and reopens from rest. No rendered-window, macOS or Windows acceptance. |
| Containers/services | No API, gateway, infrastructure, secret/configuration or database change. |

Before native distribution, check on a real iPhone and Android phone: slow,
halfway and flick slides both ways; that the account bar never shows over the
conversation mid-slide; Back and channel-choice slides; and the account bar's
controls once Browse settles.

## Custom profile pictures and space icons (October 14, 2026)

People can replace their saved mascot with their own picture, and space owners can
give a space an icon so spaces are easy to tell apart in the rail. Both may be still
images or animated GIFs, and GIFs animate on everyone's screen: members, chat
authors, replies, threads, DMs, reactions, voice tiles, invites and the rail.
Reduced-motion settings show a still first frame. Removing a picture restores the
same saved mascot (`users.avatar_id` never changes) or the space's initial.

### Storage, delivery and retention

Pictures reuse the [upload pipeline](#uploads-and-attachments) unchanged: an
`assets` row (`purpose` `avatar` or `space_icon`), bytes `PUT` straight to the
private R2 bucket at `original/{id}`, and delivery through the signed CDN Worker
(`/assets/{id}?exp=&sig=`). Migration `202610140001_pictures.sql` adds the
purposes, `assets.space_id`, and the current picture as `users.avatar_asset` and
`spaces.icon_asset` (asset external IDs). Nothing stores a URL: responses carry
`avatarAsset`/`iconAsset`, and the API's JSON responses and every gateway frame add
a signed `avatarUrl`/`iconUrl` beside them as they leave (24–48 h, re-signed daily,
like attachments). The Worker, bucket, CORS rule and secrets are unchanged.

Message payloads never store a picture. Wherever messages leave (history, threads,
pins, edits, forwards, send responses, the outbox and gateway replay) one batched
lookup sets each person's current picture by account ID, including nested people
(thread participants, reply and forward authors), so a changed or removed picture
is never shown from an old snapshot.

### Live changes

A saved picture, display name or username, and a space's icon or name, reach
other open clients at once (`profiles.rs`). The API publishes each change once to
the broker topic `caper:profiles:v1` with its audience: for a person, everyone who
shares an active space or a DM with them plus their own other sessions; for a
space, its members. Each gateway delivers it to the sockets of those accounts that
opened a `profiles` subscription (`{"type":"subscribe","kind":"profiles"}`), signing
picture URLs on the way out:

```json
{"type":"profile.updated","person":{"id","username","displayName","avatarId","avatarAsset"?,"avatarUrl"?}}
{"type":"space.updated","space":{"id","name","ownerId","iconAsset"?,"iconUrl"?}}
```

Absent picture fields mean the picture or icon was removed. Clients apply a
`profile.updated` to every retained copy of that account (members, invitations, DM
peers, suggestions, message authors including nested ones, reactors, cards), the
same way the [own profile refresh](#own-profile-refresh-october-11-2026) treats the
signed-in account, and a `space.updated` to the rail, lists and open space.
Delivery is best effort with no replay: what clients reload after reconnecting
repairs anything missed. Older gateways answer the unknown subscription with an
error, which clients treat as "not available".

Calls in progress update too: the same change rewrites the name and picture of
that account's participants in every active voice room, so the room's next
snapshot (a new revision) carries it to everyone watching the call.

**Old pictures are kept.** Replacing or removing a picture leaves its row and R2
object in place (rows were already retained; objects are small and changes are
rare). They do not count toward the 10 GB file allowance, keep moderation history
possible, and appear in the **Previous pictures** gallery in profile settings.
Abandoned, cancelled, rejected or orphaned (space deleted or transferred
mid-upload) picture uploads are purged like any other unsent upload. Removing a
user's pictures from storage on account deletion belongs with that future flow.

The gallery lists retained, completed uploads newest first, including the current
picture, in pages of 40. It does not reconstruct editing steps or recover cropped
pixels; each thumbnail is a saved image. Selecting a thumbnail changes only the
preview; **Save picture** applies that exact image without uploading or re-encoding.
Cancel leaves the account alone. The gallery also works while showing the mascot.
Migration `202610190001_avatar_restore.sql` adds nullable `avatar_selected_at`;
uploads reserved before a selection or removal cannot overwrite it on completion.

### API

| Call | Behavior |
| --- | --- |
| `POST /api/assets` `{"purpose":"avatar"\|"spaceIcon", "spaceId"?, filename, contentType, byteSize, width, height}` | Reserves a picture (no `channelId`/`preview`). `spaceIcon` requires owning the space (`404` otherwise). PNG, JPEG, WebP or GIF only (`400 picture_type`); GIFs ≤ 5 MiB and other pictures ≤ 1 MiB (`413 picture_too_large`); 1–1024 px each side. 50 picture uploads per person per day (`429 picture_rate_limited`), plus the usual 30/minute and 20 pending. Returns `{id, kind, upload}`; picture IDs are 20 characters. |
| `POST /api/assets/{id}/complete` | Checks size and magic bytes as for attachments, then the real pixel size read from the first 64 KiB (GIF screen, PNG IHDR, WebP VP8/VP8L/VP8X, JPEG frame header behind EXIF), ≤ 1024 px (`422 picture_invalid`). The first successful completion **applies** the picture and returns `account` (avatar) or `space` (icon); a retry returns the current state without re-applying, so a late retry never undoes a newer picture. Pictures are ordered by reservation: one that completes after a later-reserved picture already completed is kept but not applied, and the response shows the newer one. |
| `POST /api/assets/{id}/cancel` | Cancels a picture before completion; `409` afterwards. |
| `DELETE /api/account/avatar` | Back to the saved mascot; returns the account. |
| `GET /api/account/avatar/history?cursor=<asset ID>` | Returns `{pictures:[{avatarAsset,avatarUrl}],nextCursor}` for your own retained, completed avatar uploads only. Stable newest-first pages of 40; `nextCursor: null` at the end. Includes the current picture. |
| `POST /api/account/avatar/select` `{"avatarAsset":"chosen asset ID","expectedAvatarAsset":"current asset ID" \| null}` | Locks the account, compares the expected current image, and applies the chosen owned/completed/retained avatar. Returns the account and broadcasts its profile change. `409` if the current image changed; `404` for an invalid target (including another user's picture). |
| `DELETE /api/spaces/{space}/icon` | Owner only; back to the initial; returns the space. |

`avatarAsset`/`avatarUrl` join every person that has `avatarId`: the account,
message/typing authors, members and invitations, DM peers, suggestions and blocks,
reactors, voice participants, invite-link inviters and feedback reporters.
`iconAsset`/`iconUrl` join spaces in the spaces list, space detail, invitations,
create/rename/accept responses, the invite preview and feedback reports. Old
clients ignore them and keep showing mascots and initials. Push notifications
still show the sender's mascot (a custom picture there needs notification-service
image downloads on each platform; follow-up).

### Clients

Every client applies the same preparation: a GIF (detected by its bytes) keeps
its animation. Within 5 MB and 1024 × 1024 it uploads unchanged; any bigger GIF
is shrunk on the device instead of refused: every composited frame is
center-cropped square and scaled to 512 px with its timing, then, while the
result is still over 5 MB, to 384, 256, 192 and 128 px, then to every other
frame (each lasting as long as the ones it replaces). Web
(`account/gif-shrink.ts`, in a worker) and Android (`GifShrink.kt`) share one
pure decoder/encoder that gives each frame a palette of only the pixels that
changed and leaves unchanged pixels showing; Apple uses ImageIO and the Rust
desktop the `image` crate's GIF codec. The server's 5 MiB GIF ceiling stays: it
is also what older clients will download. Still pictures open in the attachment
editor with a square crop: reposition, resize, rotate, draw or add text before
confirming **Done**. The selected region is scaled to at most 512 px and
re-encoded as WebP (PNG or JPEG where WebP
encoding is unavailable), which also drops location metadata. Pictures show
aspect-filled in a rounded square (corner radius 30% of the side, the
`--radius-avatar` token; squares show more of a picture than circles), and
speaking rings around them follow that shape. Space icons fill their existing
tile. The mascot or initial underneath stays visible while loading or if a URL
fails. Decoded pictures are cached by asset
ID, since URLs re-sign daily. Edit profile and **Manage space** (owners
only) show the picture or icon in a frame with an edit badge on its corner: the
profile frame opens a menu to choose a new picture or browse previous pictures,
and, while a custom one is set, edit or remove it. A space icon without a custom
image opens the file picker directly. Editing the current picture downloads its saved
pixels; it cannot recover parts cropped out of an earlier upload (choose the
original file again for those). Dropping a file onto the frame opens the same
editor. Cancel leaves the saved picture unchanged and makes no upload reservation.
**Previous pictures** opens a thumbnail grid with a current marker and larger
preview. Save is disabled until a different picture is selected, and while saving.
The saved account refreshes on success; failures remain inline without applying
the preview. Loading, empty history, retry and Load more are supported. Space
icons have no history picker.
GIFs offer **Keep original animation** or an explicitly acknowledged still-frame
edit; cropping animation is not supported. Animated PNG/WebP require confirmation
before becoming stills. The frame highlights the drop target;
only the first file is used, and busy or unavailable editors refuse drops.
This is implemented in the web, Android, Apple and Rust desktop editors.
Without a custom profile picture, the saved mascot holds
up a "change me!" sign (`assets/avatars/change-me.svg`, one vector drawn over
any mascot); a successful change updates the signed-in client everywhere through the
existing own-profile refresh path.

### Previous-picture gallery validation (October 19, 2026)

- API: workspace tests, formatting and Clippy pass. The existing picture
  integration test passes with disposable local PostgreSQL and fake R2: pagination,
  nonadjacent selection, owner/purpose/completion/retention isolation, stale
  requests and late upload completion after selection/removal.
- Web: `npm run check` and 683 Vitest tests pass. Desktop and 390px touch Chromium
  checks exercise preview-only selection, pagination, Cancel/Escape/mobile Back,
  explicit save, no extra upload, pending/disabled state, focus return, conflict
  errors and empty/loading/error/retry states. These use mock
  API/CDN/storage (`PICTURE_GALLERY_ONLY=1 node scripts/test-pictures.mjs`). The
  full script still stops at its existing live-update assertion (also reproduced
  with the unchanged harness during the initial one-step implementation). This is not a
  passing full browser suite or live-storage validation. Desktop/mobile
  screenshots and a chooser pointer recording were inspected.
- Android: Kotlin/Java and test compilation pass, with 488 unit tests passing.
  Full APK assembly requires the missing prepared native-audio inputs; no device run.
- Rust desktop: build, formatting and application Clippy (`--no-deps`, `-D warnings`)
  pass. The full run passed 629 tests (15 ignored); the gallery test initially failed
  because its mock account omitted profile names. After correcting the fixture,
  that test passes, including preview/cancel/explicit-save and conflict/reopen with
  three distinct pictures. The 2x Linux/Xvfb chooser was inspected. No Windows/macOS run.
- Apple: iOS/macOS implementation and history/selection/identity tests are included;
  compilation, tests and visual validation still require Swift/Xcode on an Apple
  runner. Docker is unavailable here; application builds were run directly.

### Picture editing validation (October 18, 2026)

- Web: `npm run check` and all 678 Vitest tests pass. `scripts/test-pictures.mjs`
  passes with explicitly mocked API/storage/CDN at 1280px and 390px touch Chromium:
  selected crop verified in encoded upload pixels, square zoom/rotation, existing
  picture editing, cancel without reservation, drops, original GIF animation and
  explicitly flattened GIFs. Screenshots and pointer recording inspected. One
  existing live-reply assertion failed under concurrent native builds; the full
  browser run passed on rerun. No Safari, physical phone or live-storage test.
- Android: `assembleDebug testDebugUnitTest lintDebug` passes; 479 unit tests.
  One existing timing-sensitive WebSocket test failed during the first concurrent build; the
  full rerun passed unchanged. APK built, but no device/emulator rendering or
  live upload was exercised.
- Rust desktop: build, formatting, application Clippy (`--no-deps`, `-D warnings`),
  and 615 tests pass (15 existing ignored). Decoded output tests distinguish
  left/right crops from automatic centering. The `parity-pictures-editor` fixture
  was rendered under Xvfb and its crop resized/repositioned; the regression test
  checks settings do not cover the editor and cancellation does not upload.
  No Windows/macOS or live-storage test. Dependency Clippy remains blocked by
  existing `webrtc-sys` missing-safety-documentation errors.
- Apple: implementation and two pixel-output regression tests added for iOS/macOS,
  but not compiled or executed: no Swift/Xcode or connected Apple runner in this
  orb. SwiftUI rendering, lifecycle, picker and upload verification remain required.
- Docker daemon unavailable; the web image's build stage was validated directly
  with `npm run check`, not an image build. No API or storage schema changes.

### Profile-picture viewing (October 19, 2026)

Click a custom picture inside a person's profile card to open the original in
the media viewer. Chat avatars still open the profile first. Web images keep the
browser's native right-click Copy image/Open image actions; native profile cards
offer equivalent context-menu or long-press actions. Closing the viewer preserves
the profile composer. Reduced motion keeps animated originals still. Mascot-only
profiles do not offer a custom-picture viewer.

Validation for this change:

- Web: `npm run check` and all 696 Vitest tests pass. Run
  `PICTURE_VIEWER_ONLY=1 node scripts/test-pictures.mjs <loopback-web-url>` with
  Playwright/Chromium configured as documented in that script. Desktop, 390px
  touch emulation and reduced-motion fixtures pass: original proportions,
  native context-menu hit target, PNG clipboard data, external tab, keyboard,
  Back/close and draft preservation. Screenshots and interaction recording
  inspected. These use mocked accounts/CDN, not live accounts or Safari.
- The full picture script still times out at its live-profile-refresh assertion
  (line 981), also reproduced against the unchanged checkout.
- Android: `compileDebugKotlin`, `testDebugUnitTest` (498 tests) and `lintDebug`
  pass. Clipboard interoperability and rendered/device interactions remain unverified.
- Rust desktop: formatting, application Clippy (`--no-deps`) and 650 tests pass
  with `--no-default-features`; 16 tests remain ignored. Clippy including vendored
  WebRTC fails on existing missing safety documentation. Native rendering,
  clipboard and browser-launch integration remain unverified for this change.
- Apple implementation is not compiled or device-verified: no Xcode in this orb.
  Docker daemon unavailable; web build stages checked directly, not as an image.

### Validation (October 14, 2026)

| Platform | Validation boundary |
| --- | --- |
| API/Postgres | `cargo fmt --check`, Clippy (`-D warnings`) and all 238 workspace tests pass, including every ignored database/broker test against disposable Postgres 16 and Redis 7 (standing in for Valkey). New: picture header parsing (GIF, PNG, the three WebP forms, JPEG frames behind 20 KB of EXIF), reservation rules, signing that never signs a 16-character attachment ID, nested people, and one end-to-end test with the fake S3: limits, apply-on-complete, a late retry not undoing a newer picture, cancel rules, real-size rejection, owner-only icons and ownership lost mid-upload, kept pictures outside quota through purge, the daily limit, removal, history authors, and signed URLs in live `/api/spaces` responses. Not live R2/CDN. |
| Web | `npm run check` passes; `npm test` 626 of 629 pass. `downloads.test.ts` and `noise-assets.test.ts` also fail on an untouched checkout in this orb, and `deepfilter.test.ts` timed out under load but passes alone. `scripts/test-pictures.mjs` (Playwright with in-page API mocks and a fake CDN, labelled fixtures) passes at 1280px and 390px: animated, still, broken and absent pictures in chat, members, voice and profile cards; 800×600 PNG → 512 px WebP and unchanged GIF uploads; local and server errors; removal; space icons in the rail; reduced motion. Screenshots inspected. Not Safari, live storage or a phone. |
| Android | Gradle stops at the missing SDK. With a standalone Kotlin 2.2.20 compiler, models, data, `PicturePreparer.kt` and `PictureViews.kt` compile against the real libraries, and 201 data/model unit tests pass, including 14 new picture tests. `MainActivity.kt`, `CaperViewModel.kt` and the sheets were not compiled. No emulator or device run. |
| iOS/macOS | No Xcode. A Linux Swift 6.0.3 toolchain parses every source and test file and type-checks the Foundation-only code with stubs; 10 new picture tests and the existing mention-card tests run and pass there. SwiftUI views, ImageIO/libwebp code and AppModel tests were not compiled. The macOS parity UI tests now scroll until Manage space's Remove and Delete buttons are reachable. No simulator or device run. |
| Rust desktop | `cargo fmt --check`, application Clippy (`--no-deps`, `-D warnings`) and `cargo test` (527 passed, 12 existing ignored; 18 new, including three UI render tests and a loopback upload) pass on Linux with clang 21. The `parity-pictures` fixtures were captured under Xvfb: the GIF avatar changes between frames, plus the round crop, mascot fallback, rail icon and both settings dialogs. Not Windows, a 2x display or the real file pickers. |

The framed editor and "change me!" sign (same day): web `npm run check` and
`scripts/test-pictures.mjs` pass at 1280px and 390px (frame opens the picker
without a picture and a menu with one; Escape closes the menu, not the dialog);
Rust desktop `cargo test` (536 passed), application Clippy and Xvfb renders of
both states. Android and Apple were not compiled (no SDK or Xcode here).

Picture-frame file drops (October 18, 2026): web `npm run check`, all 661
Vitest tests, and `scripts/test-pictures.mjs` pass. The browser regression covers
first-file selection, replacing a custom icon without the menu, refusing a
second drop while busy, and preserving the icon on a non-image error. Chromium
also accepted a native file-drop event at 390px. Desktop/narrow browser screenshots and
an interaction recording were inspected with labelled API/storage mocks; no
live upload was made. Rust desktop builds and passes formatting, application
Clippy and 561 tests (13 ignored); the test harness was built with `cargo rustc
--tests -- -C linker=clang-21 -C link-arg=-fuse-ld=lld` after the default build
ran out of memory. Its Manage space fixture renders under Xvfb; native OS file
drags were not exercised. Apple and Android changes are not compiled or
device-verified in this orb (no Xcode or Android SDK).

Shrinking oversized GIFs (October 15, 2026): web Vitest decodes, shrinks and
re-decodes generated GIFs (12 frames over 5 MB → 512 px under 100 KB with every
delay; tight budgets step down then halve frames keeping total length;
transparency; truncated files), and `scripts/test-pictures.mjs` uploads a 6 MB,
1200 px GIF that Chrome's `ImageDecoder` then plays as 12 frames at 512 × 512.
A 1100 × 800 photo-like GIF shrinks in about 0.5 s to 34 dB PSNR against its
own frames. `GifShrink.kt` and its JUnit tests compile and pass with a
standalone Kotlin 2.2.20 compiler on the JVM, with the same output quality; the
Android app itself was not built (no SDK). Rust desktop `pictures::` tests and
application Clippy pass. Apple's ImageIO path was not compiled (no Xcode).

Run the browser check against `vite dev` with Playwright installed somewhere
(the repository has no Playwright dependency):

```sh
PLAYWRIGHT_MODULE=/path/to/node_modules/playwright/index.mjs \
  node scripts/test-pictures.mjs http://127.0.0.1:5174 /tmp/picture-shots
```

### Deployment order

1. No infrastructure, R2, CORS, CDN Worker or secret change: pictures use the
   existing bucket, `original/` keys and signed `/assets/` URLs.
2. Deploy the API: startup applies the additive migration (new purposes, nullable
   columns; no backfill). Then deploy the gateway so socket frames carry signed
   picture URLs; until then new messages carry `avatarAsset` without a URL and
   clients keep showing mascots.
3. Deploy web, then release Android, iOS/macOS and desktop after their platform
   builds and device checks. Clients are independent once the servers are out.
4. Verify on staging/production: upload a still and an animated GIF avatar and a
   space icon; check members, chat, DMs, voice and the rail on a second account;
   reduced motion; remove both and confirm the same mascot and the initial return.
5. Rollback: prefer a forward fix. A pre-pictures API image can't start against
   the migrated database: startup migration validates the applied history and
   stops on the unknown `202610140001` version, so the existing pods keep
   serving (only the API runs the asset purger; the gateway never does). Don't
   get around that by deleting migration records or setting SQLx's
   `ignore_missing`: the old purge marks every completed upload with no
   message as deleted after 24 hours and removes its R2 objects, which would
   take every applied and kept picture. A reviewed rollback build must keep the
   migration and the current purge, which skips completed pictures. Leave the
   migration in place.

## iOS drawer releases still bounced off the far side (October 14, 2026)

On an iPhone build that has `DrawerPose`, releasing a swipe from Browse toward
the conversation past halfway still drew the conversation left of the screen's
edge, with Browse showing on its right, before it eased back. The October 12
fix assumed the scoped `transaction` kept every SwiftUI animation off the pose;
it was never checked on a device. Positions computed per frame stay between 0
and 1, so a pose past the edge can only come from SwiftUI interpolating the
layer's offset.

- **Apple:** drawer layers move by `DrawnOffset`, a geometry effect with no
  animatable data. Whatever transaction reaches the layer, SwiftUI draws it
  exactly where `DrawerPose` puts it and can't interpolate past either end.
- **Apple:** a released swipe's recorded slide `leads` its view switch. If the
  layer is drawn before the switch reaches it, it keeps drawing the slide
  instead of the rest it is leaving. Before, that drew Browse's pose for a
  frame, a jump an animation could then carry past the conversation's rest.
- Web, Android and the Rust desktop keep one position per drawer animated by a
  single curve. They were reviewed again and have no counterpart.

| Client | Validation |
| --- | --- |
| Apple iOS/macOS | Swift 6.1.2 on Linux: the drawer logic from `CaperView.swift` and all 17 `DrawerMotionTests` (2 new) compile and pass against a bezier `UnitCurve` stand-in. The new release test fails without the `leads` guard. `DrawnOffset` and its SwiftUI use were not compiled (no Xcode). No simulator or device run. |

Verify on an iPhone: drag from Browse past the middle toward the conversation,
slowly and with a flick, and let go. The conversation carries on to the left edge
and stops there, never passing it. Do the same back to Browse and for the member
list, and check that a finger still catches a moving slide.

## Space-rail voice activity (October 18, 2026)

Web, Android, Apple and Rust desktop show a small speaker on a space's icon:
white on neutral chrome when voice is occupied, white on green when this
client is connected there. Browsing a different space does not move the green
badge. Joining/reconnecting alone never turns it green. Unread and mute marks
remain separate.

Authenticated `GET /api/spaces/voice-activity` returns `{ "spaceIds": [...] }`.
It includes only current member spaces with non-monitor participants in
channels the caller has joined and can access; private grants and revocations
use the same authorization as joining voice. Reservations and monitor-only
rooms do not count. No participant identities or channel IDs are returned.
Clients refresh every 15 seconds (web skips hidden tabs), clear polled activity
on failure/logout, and derive their own green badge immediately from the local
connected state. This is a polling hint, not an instantaneous presence feed.

Validation: API HTTP authorization cases pass against disposable Postgres and
Valkey and the in-memory room backend; SFU responses are mocked. Web checks and
tests pass; `VOICE_TEST_RAIL=1 node scripts/test-voice-controls.mjs` exercises
idle/joining/connected, cross-space navigation, outage cleanup and leaving with
mocked media. Chromium 1280px/390px captures were inspected (not physical-device
or Safari acceptance). Rust desktop has endpoint, state and rail tests; its
connected and neutral badges were rendered and inspected in Linux/Xvfb fixtures.
Android SDK and Apple/Xcode toolchains are unavailable in this Linux orb;
their new API tests and native rendering still need platform validation.

## Desktop window restoration (October 19, 2026)

The Rust desktop client retains its normal window size/position and its
maximized/fullscreen mode across normal exits and update restarts. Minimized
geometry never replaces the last usable bounds; maximized/fullscreen bounds
never replace the normal restore rectangle. Invalid legacy tiny saves fall back
to the default window. Windows still starts hidden and applies the saved mode
after the first painted frame rather than maximizing before paint. Fixtures do
not save this state. The record uses eframe 0.33's `window` format so its existing
monitor/DPI placement and off-screen clamping remain in effect.

This starts remembering geometry with the fixed build; it cannot recover a
layout the previous build never saved. macOS's separate SwiftUI client retains
its system-managed window restoration; browser and mobile windows remain owned
by their browser/OS, not the Rust updater.

Validation: desktop `cargo test` passes (640 passed, 16 existing ignored),
`cargo build`, package formatting and application Clippy pass on Linux.
Regression coverage includes minimized and maximized bounds, saved fullscreen,
asynchronous startup mode changes, scaled coordinates and invalid legacy sizes.
Real Xvfb/Openbox relaunches preserve a 910×710 content area at outer position
(170, 115), maximized state and normal bounds after closing while minimized.
An explicitly seeded app-managed fullscreen record also reopens fullscreen and
keeps its normal rectangle on exit.
No Windows runtime, Windows snapping/multi-monitor/DPI-change acceptance, macOS
runtime or signed installer update was exercised. A Linux window-manager-forced
fullscreen check did not round-trip as fullscreen; app-managed fullscreen
restoration does not establish support for that external window-manager mode.

## Voice-start notifications (October 19, 2026)

See [the notification contract](notifications.md#voice-start-alerts-october-19-2026).
The additive `202610190021_voice_notifications.sql` migration adds account/space
preferences, a durable space cooldown and a shared eligibility function/view.
API startup grants the runtime role access. Deploy the API before the gateway,
web and native releases; old clients omit the new fields safely but may not show
this new alert kind. No provider secrets or infrastructure changes are required.

Validation uses real disposable Postgres and Valkey, with the SFU and APNs/FCM
explicitly mocked. Join-route tests cover simultaneous starts, cross-channel
occupancy, empty/rejoin cooldown, private access and expired leases in both memory
and shared media mode. Notification tests cover account opt-out, space overrides
and inheritance, mutes, blocks, revoked membership, private access at delivery,
freshness, mobile holds and APNs/FCM payloads without message IDs.

Web build/typecheck/lint and Vitest pass. Desktop Rust build/tests/Clippy and
Android unit tests/lint pass. Rendered web desktop/narrow controls and Linux
desktop fixtures were inspected; the web pointer interaction was recorded with
a labeled API mock. Apple code and tests are implemented but require Xcode to
compile/run. No physical mobile device, live SFU call, real push-provider delivery
or Windows/macOS notification activation was exercised. Docker is unavailable in
the orb; build stages are validated directly rather than as images.
