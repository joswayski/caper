-- Dictation sends a recording to the AI provider and costs money per minute.
-- Charge dictations per account on the shared limiter row (already granted to
-- the runtime role), in their own window.
ALTER TABLE space_invite_limits
    ADD COLUMN dictation_window_start timestamptz NOT NULL DEFAULT now(),
    ADD COLUMN dictation_attempts integer NOT NULL DEFAULT 0;
