-- Mobile registrations belong to the account session that opted in. Delivery
-- always rechecks that session, so logout/revocation immediately suppresses it.
CREATE TABLE push_devices (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    account_session_hash bytea NOT NULL REFERENCES account_sessions (token_hash) ON DELETE CASCADE,
    platform text NOT NULL CHECK (platform IN ('fcm', 'apns', 'apnsSandbox')),
    token_hash bytea NOT NULL,
    endpoint_arn text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (platform, token_hash)
);
CREATE INDEX push_devices_user_idx ON push_devices (user_id);
CREATE INDEX push_devices_session_idx ON push_devices (account_session_hash);

-- One logical notification per committed DM. Individual endpoint attempts are
-- materialized separately to make retry/claim behavior replica-safe.
CREATE TABLE push_notifications (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    channel_id bigint NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    recipient_user_id bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    message_id bigint NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    expanded_at timestamptz,
    UNIQUE (message_id, recipient_user_id)
);
CREATE INDEX push_notifications_pending_idx ON push_notifications (created_at, id)
    WHERE expanded_at IS NULL;

CREATE TABLE push_deliveries (
    notification_id bigint NOT NULL REFERENCES push_notifications (id) ON DELETE CASCADE,
    device_id bigint NOT NULL REFERENCES push_devices (id) ON DELETE CASCADE,
    attempts smallint NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    available_at timestamptz NOT NULL DEFAULT now(),
    locked_at timestamptz,
    delivered_at timestamptz,
    abandoned_at timestamptz,
    PRIMARY KEY (notification_id, device_id)
);
CREATE INDEX push_deliveries_pending_idx ON push_deliveries (available_at, notification_id, device_id)
    WHERE delivered_at IS NULL AND abandoned_at IS NULL;
