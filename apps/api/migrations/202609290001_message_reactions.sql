-- Reaction membership belongs to an account, not a particular chat session.
-- Public author IDs are projected from users.external_id at the API boundary.
CREATE TABLE message_reactions (
    message_id bigint NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    emoji text NOT NULL,
    user_id bigint NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (message_id, emoji, user_id)
);
CREATE INDEX message_reactions_message ON message_reactions (message_id, emoji);

-- Only mutations are recorded. This makes retries and already-satisfied PUTs
-- free while bounding deliberate add/remove toggling across API replicas.
CREATE TABLE message_reaction_activity (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    message_id bigint NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    user_id bigint NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX message_reaction_activity_recent
    ON message_reaction_activity (user_id, created_at);
