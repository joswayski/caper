-- A single short-lived registry snapshot lets a replacement API process resume
-- active Cloudflare sessions after a graceful, non-overlapping deployment.
CREATE TABLE media_deployment_handoff (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    payload jsonb NOT NULL,
    saved_at timestamptz NOT NULL DEFAULT now()
);
