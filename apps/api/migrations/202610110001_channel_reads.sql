-- Account-wide, durable read positions for space channels. DMs keep their
-- existing direct_reads contract. Reads are retained, never deleted on leave.
CREATE TABLE channel_reads (
    channel_id bigint NOT NULL REFERENCES channels (id),
    user_id bigint NOT NULL REFERENCES users (id),
    seq bigint NOT NULL DEFAULT 0 CHECK (seq >= 0),
    PRIMARY KEY (channel_id, user_id)
);
