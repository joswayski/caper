-- Message requests: a DM between people who share no space waits for the
-- recipient to accept. Existing conversations are already accepted, and a
-- conversation is accepted unless it is created as a request.
ALTER TABLE direct_conversations
    ADD COLUMN requested_by bigint REFERENCES users (id),
    ADD COLUMN accepted_at timestamptz DEFAULT now(),
    ADD COLUMN declined_at timestamptz;
UPDATE direct_conversations SET accepted_at = created_at;
ALTER TABLE direct_conversations ADD CONSTRAINT direct_conversations_request
    CHECK (accepted_at IS NOT NULL OR requested_by IS NOT NULL);
CREATE INDEX direct_conversations_requests
    ON direct_conversations (requested_by, created_at) WHERE accepted_at IS NULL;

-- Who may start a new DM with an account: anyone (as a request), people who
-- share a space, or nobody. Existing conversations are never affected.
ALTER TABLE users ADD COLUMN dm_policy text NOT NULL DEFAULT 'anyone'
    CHECK (dm_policy IN ('anyone', 'spaces', 'nobody'));

-- Blocks are retained: unblocking sets deleted_at, and blocking again adds a row.
CREATE TABLE user_blocks (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    blocker_id bigint NOT NULL REFERENCES users (id),
    blocked_id bigint NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz,
    CHECK (blocker_id <> blocked_id)
);
CREATE UNIQUE INDEX user_blocks_active
    ON user_blocks (blocker_id, blocked_id) WHERE deleted_at IS NULL;
CREATE INDEX user_blocks_blocked ON user_blocks (blocked_id) WHERE deleted_at IS NULL;
