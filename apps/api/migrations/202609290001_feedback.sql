ALTER TABLE spaces ADD COLUMN feedback boolean NOT NULL DEFAULT false;
CREATE UNIQUE INDEX spaces_one_feedback ON spaces (feedback) WHERE feedback;
ALTER TABLE spaces ADD CONSTRAINT spaces_feedback_not_demo CHECK (NOT (feedback AND demo));

ALTER TABLE channels ADD COLUMN feedback_user_id bigint REFERENCES users (id);
ALTER TABLE channels ADD CONSTRAINT channels_feedback_private CHECK (feedback_user_id IS NULL OR private);
CREATE UNIQUE INDEX channels_one_feedback_conversation
    ON channels (space_id, feedback_user_id) WHERE feedback_user_id IS NOT NULL;
-- Match the inbox's public-first, newest-private-first keyset order.
CREATE INDEX channels_feedback_inbox
    ON channels (space_id, (feedback_user_id IS NOT NULL),
                 (CASE WHEN feedback_user_id IS NULL THEN id END), id DESC)
    WHERE deleted_at IS NULL;

CREATE TABLE feedback_read_cursors (
    channel_id bigint NOT NULL REFERENCES channels (id),
    user_id bigint NOT NULL REFERENCES users (id),
    seq bigint NOT NULL DEFAULT 0 CHECK (seq >= 0),
    PRIMARY KEY (channel_id, user_id)
);
CREATE INDEX feedback_read_cursors_user_id ON feedback_read_cursors (user_id);
