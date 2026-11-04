-- Acknowledgements are independent of delivery settings and never grant access.
-- Source records and conversation/thread cursors remain authoritative.
CREATE TABLE notification_inbox_reads (
    user_id bigint NOT NULL REFERENCES users(id),
    item_id text NOT NULL,
    read_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, item_id)
);
