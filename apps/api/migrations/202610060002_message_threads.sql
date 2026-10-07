-- A broadcast reply is one message visible in both its thread and channel.
-- Keep replies and their parent; no cascading deletion of conversation data.
-- Follows the pin migration merged earlier on the same day.
ALTER TABLE messages
    ADD COLUMN thread_root_id bigint REFERENCES messages (id),
    ADD COLUMN broadcast boolean NOT NULL DEFAULT false,
    ADD CONSTRAINT messages_broadcast_reply CHECK (NOT broadcast OR thread_root_id IS NOT NULL);
CREATE INDEX messages_channel_timeline ON messages (channel_id, channel_seq DESC)
    WHERE thread_root_id IS NULL OR broadcast;
CREATE INDEX messages_thread_timeline ON messages (thread_root_id, channel_seq DESC)
    WHERE thread_root_id IS NOT NULL;
