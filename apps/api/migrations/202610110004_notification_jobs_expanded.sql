-- no-transaction
-- Pruning finds week-old expanded jobs by index instead of scanning the table.
CREATE INDEX CONCURRENTLY IF NOT EXISTS notification_jobs_expanded
    ON notification_jobs (expanded_at)
    WHERE expanded_at IS NOT NULL;
