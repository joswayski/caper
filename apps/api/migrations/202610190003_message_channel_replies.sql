-- no-transaction
-- Unread threads read one channel's replies instead of every reply in every
-- channel (messages_thread_timeline) or the channel's whole history.
-- CONCURRENTLY keeps sends writable while it builds; it must be the only
-- statement in a migration that runs outside a transaction.
CREATE INDEX CONCURRENTLY IF NOT EXISTS messages_channel_replies ON messages
    (channel_id, thread_root_id)
    WHERE thread_root_id IS NOT NULL;
