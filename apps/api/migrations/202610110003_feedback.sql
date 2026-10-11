-- @feedback: mentioning the Caper team in a channel, thread or DM reports that
-- message. Feedback team members can then open that conversation without
-- joining it, read it for context and reply in place as themselves, so
-- everyone in it sees the answer. Nothing is copied or moved. Kept.

-- Accounts that receive feedback. Removing someone sets deleted_at; adding
-- them again adds a row.
CREATE TABLE feedback_team (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id bigint NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz
);
CREATE UNIQUE INDEX feedback_team_active ON feedback_team (user_id) WHERE deleted_at IS NULL;

-- One row per account-authored message that mentions the team, written in the
-- send transaction. `thread_root_id` is the thread replies belong in: the
-- message itself when it was sent to the channel timeline.
CREATE TABLE feedback_reports (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    message_id bigint NOT NULL UNIQUE REFERENCES messages (id),
    channel_id bigint NOT NULL REFERENCES channels (id),
    thread_root_id bigint NOT NULL REFERENCES messages (id),
    reporter_id bigint NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX feedback_reports_channel ON feedback_reports (channel_id);
CREATE INDEX feedback_reports_recent ON feedback_reports (id DESC);

-- Whether `account` may read and reply in `channel` as the Caper team: an
-- active team member, in a conversation where someone asked for the team.
CREATE FUNCTION feedback_access(channel bigint, account bigint) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT account IS NOT NULL
       AND EXISTS (SELECT 1 FROM public.feedback_team t WHERE t.user_id = account AND t.deleted_at IS NULL)
       AND EXISTS (SELECT 1 FROM public.feedback_reports r WHERE r.channel_id = channel)
$$;
