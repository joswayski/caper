# Notifications

Caper notifies people about DMs, channel messages, mentions, feedback, new
message requests, space/private-channel invitations and voice starting in a quiet
space. Three transports carry the same decisions:

- **Phone push**: direct APNs (iPhone/iPad) and FCM HTTP v1 (Android). Gated by
  `NOTIFICATIONS_ENABLED` and loaded provider credentials.
- **Account gateway feed**: open web tabs and running macOS/Windows/Linux apps
  show local OS notifications. Needs no provider credentials.
- **Notifications inbox**: an account-wide, paginated history on every client,
  derived from accessible activity rather than from the push queue.

Caper talks to Apple and Google directly. There is no SNS or push vendor: the
Postgres outbox and `SKIP LOCKED` workers already provide queueing and retries,
and a relay would add a second copy of device state. Closed-browser Web Push,
macOS APNs and notifications after a desktop app quits are not implemented.

Space/channel unread dots are independent of notification levels, OS permission
and mutes; see [media.md](media.md#space-and-channel-unread-indicators).

## Who gets a notification

Per committed message (edits never notify; demo spaces and guests never do):

1. Never the author.
2. **DMs** (`direct.message`): the other participant only. Personal notes, a DM
   that is still a message request (not accepted), and a DM where either person
   blocked the other send no message alerts. Creating a new request sends a
   separate `direct.request` consent alert, not a preview of its messages.
3. **Space channels**: everyone who can read the channel (active members; for a
   private channel, the owner and people with a grant), minus anyone who blocked
   the author. Joining a channel is consent to hear from it, so readers who
   haven't joined, and thread replies, notify only the people they @mention.
   The kind is `mention.user` for people in `content.mentions` (never a
   forward's note, whose mentions are display-only),
   `mention.everyone` for `@everyone` (every joined reader) or `@here` (joined
   readers whose presence is `online`), otherwise `channel.message`.
4. For each candidate, in order: account level `nothing` or `paused_until` in the
   future → none. Muted channel, DM or space → none, except `mention.user`.
   Effective level = channel override → space override → account level → `all`;
   a DM override of `nothing` silences the DM, otherwise DMs notify under `all`
   and `mentions`. `channel.message` needs `all`; `mention.*` needs `all` or
   `mentions`.
5. One `notifications` row per recipient, and one delivery per active device on
   an advertised platform whose session is still valid.
6. **`@feedback`, `@caper`, `@admin` or any team name** (`feedback`): every feedback team member except the
   author, wherever it was written, including channels they aren't in, DMs and
   thread replies. Like `mention.user`, it ignores mutes. A team member who only
   visits the conversation through feedback access hears about nothing else
   there, except being @named, which also arrives as `feedback` so that it
   opens the visit view. A new message in the Caper DM is `feedback` too. When a
   team member posts in a DM they visit, both participants (or the owner of
   personal notes) get `direct.message`. See [Feedback](media.md#feedback-in-place).

Message kinds are `direct.message`, `channel.message`, `mention.user`,
`mention.everyone` and `feedback`. Non-message kinds are `direct.request`,
`space.invitation`, `channel.invitation` and `voice.started` (below). Clients
treat unknown kinds as a generic message, so new kinds never force an update.
Reactions appear in the inbox and badge but never send an OS push.

## Preferences

| Setting | Scope | Values |
| --- | --- | --- |
| `level` | account | `all` (default: every channel message plus DMs), `mentions` (*Mentions & DMs*), `nothing` (*Off*) |
| `level` override | space, channel | `all`, `mentions`, `nothing`, or `null` (inherit) |
| `level` override | DM | `nothing` or `null` |
| `mutedUntil` | space, channel, DM | `"forever"`, a time at most 365 days ahead, or `null` |
| `mobile` | account | `whenInactive` (default) or `always` |
| `voiceStarted` | account (default `true`); space override `true`/`false`/`null` | voice-start alerts only |
| `paused_until` | account | stored and honoured by delivery; no client UI |

A muted space mutes all its channels; a channel override cannot unmute it. Only
direct @mentions and `feedback` break through a mute. Clients treat an unknown
future level as `mentions`. Every client offers Notifications and Mute in the
space, channel and DM menus, plus the account settings and the per-space
voice-start Default/On/Off choice.

### Phone hold (`mobile = whenInactive`)

Gateway connection IDs are `{tag}:{uuid}`, where `tag` is the first 8 bytes of
the connection's account-session token hash in lowercase hex (`guest` without an
account). A connection is *active* while its presence lease is live and its
activity is within `PRESENCE_IDLE_TIMEOUT_SECONDS` (default 600), the same rule
that makes a person `online`. Activity is input: every client reports keys,
clicks, pointer movement and (on phones) touches in its own windows. If a
different session than the device's (an untagged ID counts as different) saw
input more recently than any connection of the device's own session, that
device's delivery is held for 60 s. When it comes due it is dropped if that is
still true; a phone used since then gets it. `always` never holds.

At send time every delivery, including provider retries, rechecks that the
device and its session are still valid, that the conversation still exists, and
that the recipient still has access and permits this notification (membership,
private-channel grants, channel joins except direct @mentions, blocks, levels,
mutes, pauses). The message must be less than 24 h old and, for DMs, the
recipient must not have read up to it. Changes after the claim cannot recall an
in-flight provider send.

### Turning push on for a phone

Push is on by default on iPhone and Android. Each time the app opens with an
account, including a restored session, and the server offers the phone's
platform, the app turns push on, unless the account turned "Notifications on
this phone" off on that phone:

- **iPhone:** asks iOS for permission. iOS shows its prompt once; later launches
  get the saved decision, so allowing Caper later in iOS Settings turns push on
  at the next launch.
- **Android:** turns push on when notifications are already allowed (below
  Android 13, or the permission was granted). On Android 13+ it otherwise shows
  the `POST_NOTIFICATIONS` prompt once per account; after a denial, app open
  doesn't ask again, and allowing it later in Android settings turns push on.
- Turning the switch off is remembered per account on that phone; turning it on
  clears that. Logging out doesn't count as turning it off.
- Opening a channel or DM in the foreground removes its delivered notifications
  (Android by tag, iPhone by `thread-id`); turning push off on the phone or
  logging out removes them all.

## Content and payloads

- Title: DM → sender display name; channels and mentions → `Sender · #channel (Space)`;
  `feedback` → `Sender · Feedback`.
- Body: the message text, trimmed, cut to 180 characters including a final `…`;
  empty text with attachments → `Sent a photo`, `Sent a video`, `Sent an audio
  file`, `Sent a file: filename`, or `Sent N files`. Empty text without files
  (including a forward without text or files) → `Sent a message`.
- Thread replies also carry `threadRootId`.
- Sender artwork is the current profile's bundled avatar (0–799), not the
  message's historical avatar or a remote image download. Missing/invalid
  artwork keeps the OS/app fallback. Android sets both the `MessagingStyle`
  sender icon and large icon; iPhone's embedded notification service extension
  (`<bundle id>.notifications`, needs the **Communication Notifications**
  capability) donates an `INSendMessageIntent` to produce a communication
  notification. Extension failure or timeout keeps the original text notification.
- APNs: `apns-push-type: alert`, `apns-priority: 10`, topic `APNS_TOPIC` (default
  `chat.caper.ios`), `apns-expiration` now + 24 h (5 min for `voice.started`),
  message alerts carry `apns-collapse-id: <messageId>`. Body
  `{"aps":{"alert":{"title","body"},"sound":"default","thread-id","mutable-content":1,"badge"},"kind","messageId","sender","senderId","senderAvatarId","spaceId","channelId","conversationTitle","recipientCount"}`;
  DMs carry `conversationId` instead of `spaceId`/`channelId`. Avatar is a
  numeric ID or null; channel payloads also carry the conversation title and
  reader count for Apple's group metadata.
- FCM HTTP v1, data only (the app builds the notification): `android.priority: HIGH`,
  `ttl: "86400s"` (`"300s"` for `voice.started`), `collapse_key` = the grouping
  key. String data: `kind`, `messageId`, `conversationId` (DMs) or `spaceId` +
  `channelId`, `title`, `body`, `sender`, `senderId`, optional decimal
  `senderAvatarId`, `conversationTitle` (`#channel (Space)`, channels only) and
  optional `threadRootId`.
- Non-message kinds carry a positive decimal-string `notificationId` instead of
  `messageId`.
- **Grouping** (APNs `thread-id`, FCM `collapse_key`): the DM or channel ID for
  messages; `request:<notificationId>` for message requests;
  `invitation:<notificationId>` for invitations. Requests and invitations never
  group with conversation messages.
- **Deleted messages** take back delivered notifications: Android gets a
  `message.deleted` data message, iPhone a silent `passive` replacement with the
  same collapse ID, running web/desktop clients `notification.retracted`. See
  [media.md](media.md) (message deletion) for the contract.
- Android OS channels: Direct messages, Channel messages, Mentions, Invitations,
  Message requests.

### Uploaded-file previews

APNs, FCM and the account gateway feed optionally include `attachmentUrl`,
`attachmentContentType` and `attachmentSize` (decimal string in FCM; number
elsewhere). The server picks one available PNG/JPEG/GIF/WebP/AVIF image
original or a video's existing chat poster, including attachments in forwarded
snapshots, at most one per message. Images over 50,000,000 bytes get no preview;
video posters are already capped at 512 KiB, so any video with a poster can
preview. Video bytes are never downloaded: a video without a poster gets text
only. Captions remain the body. Deleted files are not signed.

The URL is a 15-minute signed CDN link (from signing time) to the existing image
original (`/assets`, `original/` prefix; legacy `/original` links still work) or
video poster (`/preview`), never a new rendition. API and gateway read poster
type/size from the asset row (so messages with only `preview:{}` work) and reuse
`ASSET_CDN_ORIGIN` and `ASSET_CDN_SIGNING_SECRET`. URLs are bearer capabilities:
access is checked before issuing, but an issued URL cannot be recalled before it
expires. The CDN allows credential-free CORS reads only after its signature
check, including cached responses.

Clients download without credentials or redirects, cap bytes, allow about five
seconds, and make a PNG no larger than 1024 px on its long edge **only on the
receiving device**: web in memory; Android in app-private files kept for the
notification; Apple streams to a temporary file, then hands the PNG to
UserNotifications; Rust desktop in a private temporary directory until
dismissal, expiry or logout. iPhone keeps its communication intent and Android
its MessagingStyle; macOS shows the upload instead of the sender image; Windows
adds a hero image. Any failure (no poster, unsupported decoder, expiry, slow
download, OS timeout) keeps the caption/file description and sender artwork.

## Local web and desktop notifications

- The authenticated account subscribes on `/api/chat/events` with
  `{"type":"subscribe","id":"notifications","kind":"notifications","after"?}`.
  No channel, space, account selector or media token is accepted. The first
  subscription starts at the current head without historical banners; reconnects
  resume `after`. `notification.created` contains a decimal-string `seq`,
  `kind`, `messageId` (or `notificationId`), `title`, `body`, `sender`,
  `senderId`, `senderAvatarId`, `createdAt` and the same routing IDs as phone push.
- Notification IDs are sparse per account. `ready` carries a decimal-string
  `cursor`, including suppressed rows. The gateway rechecks session validity,
  access, blocks, consent, preferences, mutes and DM reads, and polls
  watched-account heads every two seconds.
- Web uses the browser `Notification` API. **Edit profile → Notifications → On
  this browser** requests permission only after a click and offers a local
  toggle. Storage/Web Locks coordinate tabs and keep only bounded message IDs,
  never previews; cross-tab deduplication is best effort without them.
- macOS uses `UNUserNotificationCenter` with the sender avatar as an image
  attachment. Windows uses a registered AppUserModelID and sender app-logo
  override; Linux uses the desktop notification service with an Open action.
  Activation focuses/restores the app and opens the DM or channel.
- The device used most recently alerts. When another sign-in session saw input
  more recently than this connection's session (see [Phone hold](#phone-hold-mobile--wheninactive)),
  the event carries `"quiet": true`: clients update unread state but show no
  banner. Without presence, events are never quiet.
- Clients suppress the conversation visible in a focused window, deduplicate
  replay, discard replay older than two minutes (or over 30 s in the future) and
  clear alerts on logout. Rust desktop presents from its account worker,
  independently of rendering; its reading state is a two-second lease.
- The tab or app must be running; minimized windows still notify. There is no
  keep-running-in-tray mode or service worker.

## Message-request and invitation alerts

- **Message requests** (`direct.request`): creating a new pending DM sends one
  recipient-only alert atomically with the request. Reopening the same pair and
  later pending messages send no more. Title `Message request`; body
  `Alice wants to message you.`, never a message preview or attachment. Payload
  carries `notificationId` and `conversationId`. Taps open the read-only request
  review with Accept/Decline/Block.
- **Invitations** (`space.invitation`, `channel.invitation`): owners inviting an
  existing account alert that recipient only. The invite, notification and
  deliveries commit together; no membership, grant or join is created. Title
  `You’re invited!`; body `Alice invited you to Studio.` or
  `Alice invited you to private channel #planning.` Payload carries
  `notificationId`, `spaceId`, and `channelId` for private channels only. Taps
  open the existing Accept/Decline UI (web: `/spaces?space=…`, plus `&invite=…`
  for private channels); they never accept, grant history or enter voice.
- Both: account `nothing` and `paused_until` suppress them; `all`/`mentions`
  allow them; conversation levels and mutes do not apply. iOS shows a regular
  alert (no communication intent) with the app icon; Android and desktop keep
  sender artwork. Phone hold, the 24-hour cutoff and gateway replay apply.
  Delivery rechecks pending consent, the original invitation generation, blocks,
  live users/resources and preferences, so acceptance, decline, cancellation or
  reinvitation never revives a queued alert.

## Voice-start alerts

`voice.started` fires on the first successful non-monitor voice join while
**every channel in the space** has no live, completed voice participant.
Reservations, failed joins, monitor sessions and expired leases don't count.

- A durable, space-wide 30-minute cooldown starts with each qualifying
  activation, even with no eligible recipients. Suppressed starts are not
  delivered later.
- Recipients: other active space members, joined to the channel or not; private
  channels need a grant or ownership. Blocks either way, deleted
  accounts/channels/spaces and demo spaces exclude delivery.
- Governed by `voiceStarted` (account, then space override). Pauses, mutes,
  account `nothing` and effective conversation `nothing` still suppress;
  `mentions` does not.
- Unsent/replayed events expire after five minutes; providers keep an accepted
  push at most five more. Mobile active-elsewhere suppression applies.
- Payload: `notificationId`, `spaceId`, `channelId`, sender and
  "Name joined voice in #channel." No message ID, chat message or unread
  increment. Tapping opens the channel without connecting the microphone.

Joins lock `voice_notification_state` per space before checking occupancy, which
serializes simultaneous starts across channels and API processes; fanout and
cooldown commit together. A database/media-store error can lose an alert, and
notification failure never tears down a working voice connection.

## Notifications inbox

The Notifications entry on every client opens an account-wide inbox with All,
DMs, Mentions, Threads, Reactions and Invitations filters, an Unread only switch
(showing the account-wide count), per-row Mark read, and paginated history.
Opening a row navigates to its message/thread or the existing invitation/request
consent UI. Listing never marks anything read, marking an invitation read never
accepts it, and the chat underneath does not advance read markers while covered.
Rows offer inline Reply (the compact composer, with account-scoped drafts;
thread replies stay in the thread, others quote the original) and React; neither
creates a DM, accepts a request or acknowledges the row. The open inbox refreshes
on activity signals and every 15 s, revalidating through the oldest loaded row.

- `GET /api/notifications/inbox?filter=all&unread=true&cursor=…` →
  `{items,nextCursor,unreadCount}`. Filters: `all`, `dms`, `mentions`, `threads`,
  `reactions`, `invitations`; omitted `unread` includes read history. Up to 40
  rows per page, newest first; the opaque cursor preserves timestamp ties.
  `unreadCount` is account-wide.
- `POST /api/notifications/inbox/read` `{ids:[…]}` (1–100) → `{unreadCount}`.
  Only accessible IDs are acknowledged; it never advances a conversation past
  other messages or grants membership.
- Rows derive from accessible messages, participated threads, active reactions
  to your messages and pending invitations/requests. Levels, OS permission and
  provider setup never hide activity; mute/block/access rules apply. Deleted
  sources, removed reactions and revoked access disappear, including from history.
- A reaction takes the channel sequence it was added at
  (`message_reactions.seq`), and is read once the conversation's read position
  passes it (clients advance it for reaction events while the conversation is
  shown), or when acknowledged here. Reactions recorded without a sequence stay
  unread until acknowledged. Message acknowledgements also clear matching
  channel/thread unread and suppress delayed live/phone alerts.
- Acknowledgements are kept in `notification_inbox_reads` (runtime role has
  SELECT/INSERT, never DELETE).

## App-icon badges

`GET /api/spaces/unread` includes `badgeCount`: unread messages from other
accounts in accessible joined channels and accepted DMs, direct mentions in
unjoined channels, and replies in participated threads, each message counted
once. Thread read markers clear replies; reading the channel alone does not.
Pending message requests, unexpired invitations and unread active reactions to
your messages each add one. Own/deleted messages, blocked authors,
revoked access, personal notes and edits don't count. Muted conversations add
only direct mentions; alert level, DND and OS permission don't matter. The badge
equals the inbox's unread total. Team-only feedback visits use the separate
feedback inbox count.

Counts are snapshots, never per-push increments. Opening the app does not mark
everything read. Reads refresh the count; logout and expired sessions clear it;
transient fetch failures keep it.

| Client | Behavior |
| --- | --- |
| iPhone/iPad | APNs alerts and deletion replacements carry `aps.badge`, computed at send time; the open app reconciles with `setBadgeCount`. Needs Badges permission. Reads elsewhere while suspended reconcile on the next push or foreground (no silent-push sync). |
| macOS | Dock badge from the unread poll while running. |
| Android | Notification-backed launcher dots/counts per conversation (up to 1,000 retained message identities). Reading, dismissing or retracting reduces it; foreground polling clears conversations read elsewhere. The launcher decides number vs dot; not the server-wide total. Voice notifications don't badge. |
| Web | `(N) Caper` title and feature-detected `setAppBadge`/`clearAppBadge` for installed apps. Background tabs poll, subject to throttling. |
| Rust desktop | Background polling, including minimized. Linux emits Unity LauncherEntry counts for `caper.desktop`; Windows attempts WinRT badges with the app identity (unpackaged/pinned-taskbar support uncertain). `(N) Caper` title fallback. |

## HTTP contract

All routes need account authentication (guests get 401), use camelCase JSON and
external IDs, and return errors as `{"error": "..."}`.

- `GET /api/push/config` → `{"platforms": [...]}`: `apns`, `apnsSandbox` and
  `fcm`, each only when notifications are enabled, it is in `PUSH_PLATFORMS` and
  its credentials load.
- `POST /api/push/devices` `{"platform","token","appId"?}` → 204. An unadvertised
  platform → 400 `push platform unavailable`; APNs tokens must be 64–200 hex
  characters (stored lowercase) and FCM tokens 1–4096 visible characters, else
  400 `invalid push token`; a non-string or invisible `appId` → 400
  `invalid app id`. One registration per sign-in session: a new address replaces
  the session's previous one, and an address held by another account or session
  is revoked there.
- `DELETE /api/push/devices` (same body; any known platform) → 204, idempotent;
  revokes only the caller's matching device. Logout revokes the session's devices.
- `GET /api/notifications/settings` →
  `{"level","mobile","voiceStarted","overrides":[{"spaceId","level","mutedUntil"}, {"spaceId","channelId",...}, {"conversationId",...}]}`.
  Overrides with nothing set (or an expired mute) are omitted, as are scopes the
  caller can no longer read.
- `PUT /api/notifications/settings` `{"level"?,"mobile"?,"voiceStarted"?}` → the
  full settings. 400s: `level must be all, mentions or nothing`, `mobile must be
  always or whenInactive`, `invalid notification settings` (unknown keys).
- `PUT /api/spaces/{spaceId}/notifications`,
  `PUT /api/spaces/{spaceId}/channels/{channelId}/notifications` and
  `PUT /api/dms/{conversationId}/notifications` take `{"level"?,"mutedUntil"?}`
  (spaces also `voiceStarted`) and return the scope's IDs with its values. A
  missing key leaves the field unchanged and `null` resets it. `mutedUntil` is
  returned as whole-second UTC (`2026-10-08T05:00:00Z`). 400s: `level must be
  all, mentions, nothing or null`, `level must be nothing or null` (DMs),
  `mutedUntil must be forever or a time within the next year`. No read access →
  404 `space not found`, `channel not found` or `conversation not found`.

`scripts/native-parity-fixture.mjs` serves the same routes in memory for client
tests: `POST /__fixture/control {"pushPlatforms":[...]}` sets the advertised
platforms, `GET /__fixture/push-devices` reads back registrations, and
`{"reset": true}` clears both and all settings. The owner fixture can subscribe
to notifications; `{"notification":{"channelId","text"}}` emits a labelled test
alert, never a live provider send.

## Pipeline

- `chat::persist` and the forward path insert one `notification_jobs` row in the
  send transaction (invitations and requests enqueue the same way). The
  expansion worker claims jobs with `FOR UPDATE SKIP LOCKED` under a 2-minute
  `locked_at` lease, resolves recipients, reads presence for `@here` and the
  hold, and writes `notifications` and `notification_deliveries` in one
  transaction. A job whose message is more than a day old notifies nobody.
- The delivery worker claims due deliveries the same way, calls the provider
  with no database connection held, then records the result: delivered; retry
  with exponential backoff (15 s doubling to 30 min, or a longer `Retry-After`)
  after 429, 5xx, an expired provider token or a network error; abandoned once
  the next try would be more than 24 h after the message, or after any other
  rejection; or the device revoked (APNs 410, `BadDeviceToken`; FCM
  `UNREGISTERED`, or `INVALID_ARGUMENT` on the token field). APNs
  `DeviceTokenNotForTopic` and FCM `SENDER_ID_MISMATCH` usually mean a wrong
  `APNS_TOPIC` or Firebase project, so they abandon the delivery and log a
  configuration error instead of revoking every device.
- APNs uses HTTP/2 and an ES256 JWT reused for 40 minutes; production and
  sandbox use separate keys. FCM uses an OAuth token from the service account's
  RS256 assertion, cached until 5 minutes before expiry.
- Finished deliveries, retractions and expanded jobs are pruned after 7 days, in
  batches every 10 minutes, without delaying delivery. Devices are revoked,
  never deleted; settings and overrides are updated in place.
- Workers run in every API replica when `CHAT_ENABLED` supplies Postgres and
  Valkey. `NOTIFICATIONS_ENABLED` gates only phone delivery and platform
  advertisement. Bad credentials disable that platform with a
  `push_platform_unavailable` log instead of stopping the API.

### Data model

| Table | Holds |
| --- | --- |
| `notification_devices` | One row per signed-in session's push address: transport (`apns`, `apnsSandbox`, `fcm`), `app_id`, raw token, `revoked_at`/reason |
| `notification_settings` | Account `default_level` (NULL = `all`), `mobile`, `voice_started`, `paused_until` |
| `notification_overrides` | Space or channel/DM `level`, `muted_until` (`infinity` = forever) and space `voice_started` |
| `notification_jobs` | Outbox, one row per message/invitation/request |
| `notifications` | Per-recipient notification rows; also the gateway feed's sequence |
| `notification_deliveries` | Per-device attempts (operational, pruned) |
| `notification_retractions` | Per-device takebacks for deleted messages (operational, pruned) |
| `voice_notification_state` | Per-space voice-start cooldown lock |
| `notification_inbox_reads` | Inbox acknowledgements (retained) |

New tables need runtime grants in `db::grant_runtime_access`.

## Configuration

| Setting | Meaning |
| --- | --- |
| `NOTIFICATIONS_ENABLED` | Enables phone provider delivery and advertisement, not expansion or local desktop alerts. Default off. |
| `PUSH_PLATFORMS` | Comma-separated `apns`, `apnsSandbox`, `fcm`. List only platforms validated on real devices. |
| `APNS_TEAM_ID` | Apple team ID (10 characters). |
| `APNS_KEY_ID`, `APNS_PRIVATE_KEY` | Production key ID and `.p8` PEM (`apns`). |
| `APNS_SANDBOX_KEY_ID`, `APNS_SANDBOX_PRIVATE_KEY` | Sandbox key ID and `.p8` PEM (`apnsSandbox`). |
| `APNS_TOPIC` | Bundle ID; default `chat.caper.ios`. |
| `FCM_SERVICE_ACCOUNT_JSON` | The Firebase service-account JSON key, as one string (`fcm`). |
| `PRESENCE_IDLE_TIMEOUT_SECONDS` | Shared with the gateway; decides `online` and the hold. Default 600. |

PEM values work with real newlines or literal `\n` escapes. All are read from the
API's Secrets Manager secret (`APP_SECRET_ID`) or the environment and are never
logged, nor are tokens or message text.

### Storing credentials

Keep downloaded `.p8` files and the Firebase JSON in a password manager; each
can be downloaded only once. Store them in `production/apps/caper`
(`staging/apps/caper` with `--staging`) with:

```bash
scripts/store-push-credentials.sh \
  --team-id ABCDE12345 \
  --apns-key-id KEY1234567 --apns-key ~/Downloads/AuthKey_KEY1234567.p8 \
  --apns-sandbox-key-id KEY7654321 --apns-sandbox-key ~/Downloads/AuthKey_KEY7654321.p8 \
  --fcm-service-account ~/Downloads/caper-push.json \
  --platforms apns,fcm --enabled true \
  --dry-run   # then run again without it
```

The script merges fields, keeps every other field, never prints values, and
`--dry-run` lists only changed field names. Every option is optional, so one key
can be added or rotated alone. The API receives every secret property as
environment variables through the ExternalSecret `caper-api-account`
(refreshed every 5 minutes) and restarts on change via Reloader, so new values
take effect within about 5 minutes with no deploy. Turning a platform off is the
same command with a shorter `--platforms`; `--enabled false` turns off phone
delivery, not local desktop notifications. The gateway does not need these.

### Apple (APNs)

Needs the Account Holder or Admin role at
[developer.apple.com/account](https://developer.apple.com/account).

1. In Certificates, IDs & Profiles → **Identifiers**, confirm `chat.caper.ios`
   has **Push Notifications** and **Communication Notifications**, and that the
   `<bundle id>.notifications` extension is provisioned.
2. Under **Keys → +**, create `Caper APNs Production`: tick **Apple Push
   Notification service** → **Configure**, Environment *Production*, Key type
   *Team Scoped*; download it and note its 10-character Key ID.
3. Repeat with `Caper APNs Sandbox`, Environment *Sandbox* (Xcode builds and
   staging).
4. Pass the release-signing Team ID, each Key ID and `.p8` to the script.

These keys are separate from the App Store Connect API key used for TestFlight
uploads; release builds already carry `aps-environment=production`.

### Firebase (FCM, Android)

1. At [console.firebase.google.com](https://console.firebase.google.com), create
   a project `Caper` (Analytics off).
2. In **Project settings → General → Your apps**, add Android apps
   `chat.caper.android` and `chat.caper.android.debug`.
3. Download `google-services.json` after adding both. CI writes it into Android
   builds from the base64 GitHub secret `GOOGLE_SERVICES_JSON` (client
   configuration, not a server secret); without it Firebase is compiled in but
   disabled.
4. In **Project settings → Service accounts → Manage service account
   permissions**, create `caper-push` with only **Firebase Cloud Messaging API
   Admin**, add a JSON key, and store it with `--fcm-service-account`.
5. **Project settings → Cloud Messaging** must show *Firebase Cloud Messaging
   API (V1): Enabled*.

## Privacy

- Payloads contain the sender and a message preview. Apple and Google can read
  APNs/FCM payloads in transit; there is no preview-hiding setting yet (iOS's
  own *Show Previews* still applies).
- Message requests never include message content. Sender artwork is a bundled
  avatar ID, never a remote download.
- Attachment previews use short-lived signed URLs to existing files and are
  rendered only on the receiving device; no notification-specific files are
  stored server-side.
- Raw push tokens are stored because direct delivery needs them; they are bound
  to the sign-in session and revoked on logout. Tokens, credentials and message
  text are never logged.
- What was already shown on a lock screen cannot be taken back.

## Platform support

- **iPhone/iPad**: APNs push with communication notifications, previews, badges
  and retraction; local alerts are not used.
- **Android**: FCM data push with MessagingStyle, OS channels per category,
  previews, launcher badges and retraction. Stops after a force-stop.
- **Web**: local notifications while a tab is open; no closed-browser Web Push or
  service worker. Mobile browsers rely on the native apps.
- **macOS**: local notifications and Dock badge while running; no APNs.
- **Windows/Linux (Rust desktop)**: local notifications while running or
  minimized; no tray mode, nothing after quit.
- Each phone platform stays out of `PUSH_PLATFORMS` until it works on real
  signed devices.
