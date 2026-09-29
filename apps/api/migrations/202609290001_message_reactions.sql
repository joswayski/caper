-- Reaction membership is independent of a particular chat session. Account
-- external IDs therefore collapse all of an account's chat sessions, while a
-- guest's chat-session external ID remains its stable actor ID.
CREATE TABLE message_reactions (
    message_id bigint NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    emoji text NOT NULL,
    author_external_id text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (message_id, emoji, author_external_id)
);
CREATE INDEX message_reactions_message ON message_reactions (message_id, emoji);

-- Only mutations are recorded. This makes retries and already-satisfied PUTs
-- free while bounding deliberate add/remove toggling across API replicas.
CREATE TABLE message_reaction_activity (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    message_id bigint NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    author_external_id text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX message_reaction_activity_recent
    ON message_reaction_activity (author_external_id, created_at);
