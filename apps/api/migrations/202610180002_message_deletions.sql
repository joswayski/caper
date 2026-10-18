-- Deleting a message keeps its row and retained versions. The payload becomes
-- a tombstone (no text, files, mentions or pin), the stored replay log stops
-- carrying its content, and its files are retired. Additive: older API and
-- gateway builds never read the column.
ALTER TABLE messages ADD COLUMN deleted_at timestamptz;
CREATE INDEX messages_recent_deletions ON messages (channel_id, deleted_at)
    WHERE deleted_at IS NOT NULL;
