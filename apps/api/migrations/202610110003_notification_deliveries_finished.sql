-- no-transaction
-- Pruning finds week-old finished deliveries by index instead of scanning the
-- table. LEAST ignores NULL, so this is the earlier of the two finish times.
-- CONCURRENTLY keeps deliveries writable; it must be the migration's only
-- statement.
CREATE INDEX CONCURRENTLY IF NOT EXISTS notification_deliveries_finished
    ON notification_deliveries ((LEAST(delivered_at, abandoned_at)))
    WHERE delivered_at IS NOT NULL OR abandoned_at IS NOT NULL;
