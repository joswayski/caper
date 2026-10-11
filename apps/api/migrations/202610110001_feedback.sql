-- @feedback: mentioning the Caper team anywhere forwards the message (or its
-- thread) live into the author's own conversation with the team, where every
-- feedback team member can read and reply. Everything here is retained.

-- The team's account is the other participant of every feedback conversation.
-- It has no email, so nobody can sign in as it. Prefer `caper`; an existing
-- account may already hold that reserved name.
ALTER TABLE users ADD COLUMN team_account boolean NOT NULL DEFAULT false;
CREATE UNIQUE INDEX users_one_team_account ON users (team_account) WHERE team_account;
INSERT INTO users (external_id, username, display_name, avatar_id, dm_policy, team_account)
SELECT substr(md5(random()::text || clock_timestamp()::text), 1, 12),
       COALESCE(
           (SELECT v.name FROM (VALUES (1, 'caper'), (2, 'caper_feedback'), (3, 'caper_team')) AS v(rank, name)
            WHERE NOT EXISTS (SELECT 1 FROM users u WHERE u.username = v.name)
            ORDER BY v.rank LIMIT 1),
           'caper_' || substr(md5(random()::text), 1, 8)),
       'Caper', 1, 'nobody', true;
-- Its acknowledgements are messages, which need a chat session. Its token is
-- random and never issued, and without an account session it can never send.
INSERT INTO chat_sessions (external_id, token_hash, user_id, name, expires_at)
SELECT substr(md5(random()::text || clock_timestamp()::text), 1, 12),
       sha256(convert_to(random()::text || clock_timestamp()::text || id::text, 'UTF8')),
       id, 'Caper', 'infinity'
FROM users WHERE team_account;

-- Accounts that read and answer every feedback conversation. Removing someone
-- sets deleted_at; adding them again adds a row.
CREATE TABLE feedback_team (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id bigint NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz
);
CREATE UNIQUE INDEX feedback_team_active ON feedback_team (user_id) WHERE deleted_at IS NULL;

-- A feedback conversation is the DM between `feedback_user_id` and the team
-- account. It is always accepted and never a message request.
ALTER TABLE direct_conversations
    ADD COLUMN feedback_user_id bigint REFERENCES users (id),
    ADD CONSTRAINT direct_conversations_feedback_participant
        CHECK (feedback_user_id IS NULL OR feedback_user_id IN (low_user_id, high_user_id));
CREATE UNIQUE INDEX direct_conversations_feedback
    ON direct_conversations (feedback_user_id) WHERE feedback_user_id IS NOT NULL;

-- One row per account-authored message that mentions the team, written in the
-- send transaction. The chat publisher forwards it into the reporter's
-- feedback conversation. Kept as the record of what was reported.
CREATE TABLE feedback_reports (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    message_id bigint NOT NULL UNIQUE REFERENCES messages (id),
    reporter_id bigint NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    attempts smallint NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    available_at timestamptz NOT NULL DEFAULT now(),
    processed_at timestamptz,
    outcome text CHECK (outcome IN ('forwarded', 'duplicate', 'unavailable', 'limited', 'failed')),
    forward_id bigint REFERENCES messages (id),
    CHECK ((processed_at IS NULL) = (outcome IS NULL)),
    CHECK (forward_id IS NULL OR outcome = 'forwarded')
);
CREATE INDEX feedback_reports_pending ON feedback_reports (available_at, id) WHERE processed_at IS NULL;
CREATE INDEX feedback_reports_reporter ON feedback_reports (reporter_id, created_at);
