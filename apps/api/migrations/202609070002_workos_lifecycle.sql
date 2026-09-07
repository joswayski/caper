-- Keep provider lifecycle ordering and durable receipts separate from Caper profile data.
ALTER TABLE users
    ALTER COLUMN email DROP NOT NULL,
    ALTER COLUMN email_verified_at DROP NOT NULL,
    ADD COLUMN workos_updated_at timestamptz,
    ADD COLUMN deleted_at timestamptz;

CREATE TABLE workos_events (
    event_id text PRIMARY KEY,
    event_type text NOT NULL,
    processed_at timestamptz NOT NULL DEFAULT now()
);
