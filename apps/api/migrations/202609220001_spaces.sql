ALTER TABLE spaces
    ADD COLUMN owner_id bigint REFERENCES users (id),
    ADD COLUMN deleted_at timestamptz;

ALTER TABLE channels
    ADD COLUMN private boolean NOT NULL DEFAULT false,
    ADD COLUMN deleted_at timestamptz;

ALTER TABLE channels DROP CONSTRAINT channels_space_id_name_key;
CREATE UNIQUE INDEX channels_active_name
    ON channels (space_id, name) WHERE deleted_at IS NULL;

CREATE TABLE space_members (
    space_id bigint NOT NULL REFERENCES spaces (id),
    user_id bigint NOT NULL REFERENCES users (id),
    PRIMARY KEY (space_id, user_id)
);
CREATE INDEX space_members_user_id ON space_members (user_id);

CREATE TABLE channel_members (
    channel_id bigint NOT NULL REFERENCES channels (id),
    user_id bigint NOT NULL REFERENCES users (id),
    PRIMARY KEY (channel_id, user_id)
);
CREATE INDEX channel_members_user_id ON channel_members (user_id);

ALTER TABLE spaces ADD CONSTRAINT spaces_owner_required
    CHECK ((demo AND owner_id IS NULL) OR (NOT demo AND owner_id IS NOT NULL));
