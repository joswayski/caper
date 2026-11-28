-- The unsent composer text of each conversation (and each thread reply), per
-- account, so every signed-in device shows the same draft. One row per
-- draft, updated in place: sending or clearing stores empty text, so no row
-- is ever deleted and no draft history is kept.
CREATE TABLE message_drafts (
    user_id bigint NOT NULL REFERENCES users (id),
    channel_id bigint NOT NULL REFERENCES channels (id),
    -- NULL for the conversation's own composer; a reply's thread root otherwise.
    thread_root_id bigint REFERENCES messages (id),
    text text NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE NULLS NOT DISTINCT (user_id, channel_id, thread_root_id)
);
CREATE INDEX message_drafts_recent ON message_drafts (user_id, updated_at DESC);

-- A read on one device asks each of the account's phones to show the new
-- app-icon count. One row per phone, reused: `pending` while a refresh is
-- due, and `badge_count` is the count the phone was last given (by an alert,
-- a retraction, a refresh or its own unread poll), so an unchanged count is
-- not sent again. Operational state, bounded by the number of devices.
CREATE TABLE notification_badge_syncs (
    device_id bigint PRIMARY KEY REFERENCES notification_devices (id),
    pending boolean NOT NULL DEFAULT false,
    requested_at timestamptz NOT NULL DEFAULT now(),
    available_at timestamptz NOT NULL DEFAULT now(),
    locked_at timestamptz,
    attempts smallint NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    badge_count bigint,
    last_error text
);
CREATE INDEX notification_badge_syncs_due ON notification_badge_syncs (available_at) WHERE pending;
