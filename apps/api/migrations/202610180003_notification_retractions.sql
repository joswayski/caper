-- Takes back a phone notification already delivered for a message that was
-- then deleted: one row per delivered notification and device. Operational
-- state like deliveries; finished rows are pruned after a week.
CREATE TABLE notification_retractions (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    notification_id bigint NOT NULL REFERENCES notifications (id),
    device_id bigint NOT NULL REFERENCES notification_devices (id),
    attempts smallint NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    available_at timestamptz NOT NULL DEFAULT now(),
    locked_at timestamptz,
    sent_at timestamptz,
    abandoned_at timestamptz,
    last_error text,
    UNIQUE (notification_id, device_id)
);
CREATE INDEX notification_retractions_due ON notification_retractions (available_at, id)
    WHERE sent_at IS NULL AND abandoned_at IS NULL;
CREATE INDEX notification_retractions_finished ON notification_retractions ((LEAST(sent_at, abandoned_at)))
    WHERE sent_at IS NOT NULL OR abandoned_at IS NOT NULL;
