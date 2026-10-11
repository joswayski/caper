-- no-transaction
-- Unread navigation finds @mentions through this small index instead of
-- scanning every unread message in channels the reader hasn't joined.
-- CONCURRENTLY keeps sends writable while it builds; it must be the only
-- statement in a migration that runs outside a transaction.
CREATE INDEX CONCURRENTLY IF NOT EXISTS messages_mentions ON messages
    USING gin ((payload -> 'content' -> 'mentions') jsonb_path_ops)
    WHERE forward_source_id IS NULL;
