-- Phase 1 push notifications with direct APNs/FCM delivery. The SNS-era
-- push_* tables were never used and hold token hashes and endpoint ARNs that
-- direct delivery cannot reuse. 202610030002_push.sql stays as applied.
DROP TABLE push_deliveries;
DROP TABLE push_notifications;
DROP TABLE push_devices;

-- One row per signed-in device that opted in. Raw tokens are required for
-- direct delivery. Rows are revoked (logout, replacement, a dead token), never
-- deleted, and delivery rechecks the session.
CREATE TABLE notification_devices (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id bigint NOT NULL REFERENCES users (id),
    account_session_hash bytea NOT NULL REFERENCES account_sessions (token_hash),
    transport text NOT NULL CHECK (transport IN ('apns', 'apnsSandbox', 'fcm')),
    -- Bundle ID or package when the client sends one; informational in phase 1.
    app_id text NOT NULL DEFAULT '',
    -- APNs token (lowercase hex) or FCM registration token; opaque otherwise.
    address text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    revoked_reason text
);
CREATE UNIQUE INDEX notification_devices_active_address
    ON notification_devices (transport, address) WHERE revoked_at IS NULL;
CREATE INDEX notification_devices_active_user
    ON notification_devices (user_id) WHERE revoked_at IS NULL;
CREATE INDEX notification_devices_active_session
    ON notification_devices (account_session_hash) WHERE revoked_at IS NULL;

-- Account-level preferences, updated in place. A NULL level means `all`.
CREATE TABLE notification_settings (
    user_id bigint PRIMARY KEY REFERENCES users (id),
    default_level text CHECK (default_level IN ('all', 'mentions', 'nothing')),
    -- Do not disturb. Column only in phase 1.
    paused_until timestamptz,
    mobile text NOT NULL DEFAULT 'whenInactive' CHECK (mobile IN ('always', 'whenInactive')),
    -- Later: `senderOnly` and `none` send less text. Phase 1 always sends `full`.
    preview text NOT NULL DEFAULT 'full' CHECK (preview IN ('full', 'senderOnly', 'none')),
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- Space or channel scope; a DM is its channel. Updated in place: NULL level
-- inherits, and muted_until 'infinity' means forever.
CREATE TABLE notification_overrides (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id bigint NOT NULL REFERENCES users (id),
    space_id bigint REFERENCES spaces (id),
    channel_id bigint REFERENCES channels (id),
    level text CHECK (level IN ('all', 'mentions', 'nothing')),
    muted_until timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((space_id IS NULL) <> (channel_id IS NULL))
);
CREATE UNIQUE INDEX notification_overrides_space
    ON notification_overrides (user_id, space_id) WHERE space_id IS NOT NULL;
CREATE UNIQUE INDEX notification_overrides_channel
    ON notification_overrides (user_id, channel_id) WHERE channel_id IS NOT NULL;

-- Outbox: one row per committed account-authored message, written in the send
-- transaction. Workers expand it asynchronously under a locked_at lease.
-- Operational state: expanded rows may be pruned.
CREATE TABLE notification_jobs (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    message_id bigint NOT NULL UNIQUE REFERENCES messages (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    attempts smallint NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    available_at timestamptz NOT NULL DEFAULT now(),
    locked_at timestamptz,
    expanded_at timestamptz
);
CREATE INDEX notification_jobs_pending
    ON notification_jobs (available_at, id) WHERE expanded_at IS NULL;

-- One per recipient and message; the future activity inbox. Kept.
CREATE TABLE notifications (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id bigint NOT NULL REFERENCES users (id),
    kind text NOT NULL,
    message_id bigint NOT NULL REFERENCES messages (id),
    channel_id bigint NOT NULL REFERENCES channels (id),
    space_id bigint REFERENCES spaces (id),
    actor_id bigint REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    dismissed_at timestamptz,
    UNIQUE (message_id, user_id)
);
CREATE INDEX notifications_user ON notifications (user_id, created_at);

-- One per notification and device. Held deliveries wait while the recipient
-- is active on another session. Operational state: finished rows may be pruned.
CREATE TABLE notification_deliveries (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    notification_id bigint NOT NULL REFERENCES notifications (id),
    device_id bigint NOT NULL REFERENCES notification_devices (id),
    held boolean NOT NULL DEFAULT false,
    attempts smallint NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    available_at timestamptz NOT NULL DEFAULT now(),
    locked_at timestamptz,
    delivered_at timestamptz,
    abandoned_at timestamptz,
    -- A short provider or reason code. Never a token or message text.
    last_error text,
    UNIQUE (notification_id, device_id)
);
CREATE INDEX notification_deliveries_pending
    ON notification_deliveries (available_at, id)
    WHERE delivered_at IS NULL AND abandoned_at IS NULL;
CREATE INDEX notification_deliveries_device
    ON notification_deliveries (device_id)
    WHERE delivered_at IS NULL AND abandoned_at IS NULL;
