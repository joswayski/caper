-- Records are retained rather than deleted. Removing a user row must not
-- silently take its sign-in history with it.
ALTER TABLE account_sessions DROP CONSTRAINT account_sessions_user_id_fkey;
ALTER TABLE account_sessions ADD CONSTRAINT account_sessions_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id);

-- One row per membership period. Removal sets deleted_at; re-adding someone
-- inserts a new row, so earlier removal times are never overwritten.
ALTER TABLE space_members DROP CONSTRAINT space_members_pkey;
ALTER TABLE space_members
    ADD COLUMN id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    ADD COLUMN deleted_at timestamptz;
CREATE UNIQUE INDEX space_members_active
    ON space_members (space_id, user_id) WHERE deleted_at IS NULL;

ALTER TABLE channel_members DROP CONSTRAINT channel_members_pkey;
ALTER TABLE channel_members
    ADD COLUMN id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    ADD COLUMN deleted_at timestamptz;
CREATE UNIQUE INDEX channel_members_active
    ON channel_members (channel_id, user_id) WHERE deleted_at IS NULL;
