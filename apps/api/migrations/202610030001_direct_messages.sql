-- Account-global DMs reuse the durable message stream, without spaces or grants.
ALTER TABLE channels ALTER COLUMN space_id DROP NOT NULL;
ALTER TABLE channels ADD CONSTRAINT channels_unscoped_private
    CHECK (space_id IS NOT NULL OR private);

CREATE TABLE direct_conversations (
    channel_id bigint PRIMARY KEY REFERENCES channels (id),
    low_user_id bigint NOT NULL REFERENCES users (id),
    high_user_id bigint NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (low_user_id < high_user_id),
    UNIQUE (low_user_id, high_user_id)
);
CREATE INDEX direct_conversations_high_user ON direct_conversations (high_user_id);

CREATE TABLE direct_reads (
    channel_id bigint NOT NULL REFERENCES direct_conversations (channel_id),
    user_id bigint NOT NULL REFERENCES users (id),
    seq bigint NOT NULL DEFAULT 0 CHECK (seq >= 0),
    PRIMARY KEY (channel_id, user_id)
);
