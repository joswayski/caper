-- The public demo is a real space/channel, but not a permanent product space.
CREATE TABLE spaces (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    external_id text NOT NULL UNIQUE,
    name text NOT NULL,
    demo boolean NOT NULL DEFAULT false
);
CREATE UNIQUE INDEX spaces_one_demo ON spaces (demo) WHERE demo;

CREATE TABLE channels (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    external_id text NOT NULL UNIQUE,
    space_id bigint NOT NULL REFERENCES spaces (id),
    name text NOT NULL,
    last_seq bigint NOT NULL DEFAULT 0 CHECK (last_seq >= 0),
    UNIQUE (space_id, name)
);

CREATE TABLE chat_sessions (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    external_id text NOT NULL UNIQUE,
    token_hash bytea NOT NULL UNIQUE,
    user_id bigint REFERENCES users (id),
    account_session_hash bytea,
    name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL DEFAULT now() + interval '30 days'
);
CREATE INDEX chat_sessions_created ON chat_sessions (created_at);

CREATE TABLE messages (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    external_id text NOT NULL UNIQUE,
    channel_id bigint NOT NULL REFERENCES channels (id),
    session_id bigint NOT NULL REFERENCES chat_sessions (id),
    client_message_id uuid NOT NULL,
    request_hash bytea NOT NULL,
    channel_seq bigint NOT NULL,
    -- Versioned content, initially only {version:1,type:"text",text:...}.
    payload jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (session_id, channel_id, client_message_id),
    UNIQUE (channel_id, client_message_id),
    UNIQUE (channel_id, channel_seq)
);
CREATE INDEX messages_session_created ON messages (session_id, created_at);
CREATE INDEX messages_channel_created ON messages (channel_id, created_at);

-- Retained replay log and transactional outbox in one table. Publishing does
-- not delete an event: a successful Pub/Sub send is not recipient delivery.
CREATE TABLE channel_events (
    channel_id bigint NOT NULL REFERENCES channels (id),
    seq bigint NOT NULL,
    payload jsonb NOT NULL,
    published_at timestamptz,
    PRIMARY KEY (channel_id, seq)
);
CREATE INDEX channel_events_pending ON channel_events (channel_id, seq)
    WHERE published_at IS NULL;
