-- Product records are retained: removals set deleted_at instead of deleting.
-- Each period is its own row, so re-adding never overwrites an earlier removal.
-- Operational tables (sign-in, rate limits, push delivery) keep their cascades.
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

ALTER TABLE channel_joins DROP CONSTRAINT channel_joins_pkey;
ALTER TABLE channel_joins
    ADD COLUMN id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    ADD COLUMN deleted_at timestamptz;
CREATE UNIQUE INDEX channel_joins_active
    ON channel_joins (channel_id, user_id) WHERE deleted_at IS NULL;

ALTER TABLE message_reactions DROP CONSTRAINT message_reactions_pkey;
ALTER TABLE message_reactions DROP CONSTRAINT message_reactions_message_id_fkey;
ALTER TABLE message_reactions
    ADD COLUMN id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    ADD COLUMN deleted_at timestamptz,
    ADD CONSTRAINT message_reactions_message_id_fkey
        FOREIGN KEY (message_id) REFERENCES messages (id);
CREATE UNIQUE INDEX message_reactions_active
    ON message_reactions (message_id, emoji, user_id) WHERE deleted_at IS NULL;
