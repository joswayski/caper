-- Voice messages: recorded audio attachments with a sender-drawn waveform and
-- an automatic transcript. transcript_status is NULL when no transcript was
-- requested (ordinary audio files, or AI not configured).
ALTER TABLE assets ADD COLUMN voice boolean NOT NULL DEFAULT false;
ALTER TABLE assets ADD COLUMN waveform bytea CHECK (octet_length(waveform) BETWEEN 1 AND 256);
ALTER TABLE assets ADD COLUMN transcript_status text
    CHECK (transcript_status IN ('pending', 'ready', 'failed'));
ALTER TABLE assets ADD COLUMN transcript text;
ALTER TABLE assets ADD CONSTRAINT assets_voice_audio CHECK (NOT voice OR kind = 'audio');

-- Background AI work, one row per task and subject, claimed under a
-- locked_at lease like notification_jobs. Operational: finished rows may be
-- pruned.
CREATE TABLE ai_jobs (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    task text NOT NULL,
    asset_id bigint REFERENCES assets (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    attempts smallint NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    available_at timestamptz NOT NULL DEFAULT now(),
    locked_at timestamptz,
    finished_at timestamptz
);
CREATE UNIQUE INDEX ai_jobs_task_asset ON ai_jobs (task, asset_id);
CREATE INDEX ai_jobs_pending ON ai_jobs (available_at, id) WHERE finished_at IS NULL;

-- Every AI provider call with its full request (inline media replaced by a
-- reference) and response, to compare models on cost, speed and quality.
-- Kept.
CREATE TABLE ai_calls (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    job_id bigint REFERENCES ai_jobs (id),
    task text NOT NULL,
    provider text NOT NULL,
    model text NOT NULL,
    succeeded boolean NOT NULL,
    http_status smallint,
    latency_ms integer NOT NULL CHECK (latency_ms >= 0),
    input jsonb NOT NULL,
    output jsonb,
    error text,
    cost_usd numeric(14, 8),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ai_calls_task_model ON ai_calls (task, model, created_at);
