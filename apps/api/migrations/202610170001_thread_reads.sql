-- Personalized cursors are independent of channel reads and shared payloads.
CREATE TABLE thread_reads (
    thread_root_id bigint NOT NULL REFERENCES messages (id),
    user_id bigint NOT NULL REFERENCES users (id),
    seq bigint NOT NULL DEFAULT 0 CHECK (seq >= 0),
    PRIMARY KEY (thread_root_id, user_id)
);
