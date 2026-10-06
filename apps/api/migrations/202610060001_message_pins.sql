-- The message payload is the pin snapshot, just as it carries reactions.
-- This partial index keeps channel-wide pin reads independent of history size.
CREATE INDEX messages_channel_pins ON messages (channel_id)
    WHERE jsonb_typeof(payload->'pin') = 'object';

-- Count actual mutations, not idempotent retries, across API replicas.
CREATE TABLE message_pin_activity (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    message_id bigint NOT NULL REFERENCES messages (id),
    user_id bigint NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX message_pin_activity_recent
    ON message_pin_activity (user_id, created_at);
