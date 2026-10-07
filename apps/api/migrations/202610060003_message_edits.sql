-- Retained content versions, never an operational rate-limit log. Original
-- content is captured lazily on first edit; unedited messages need no backfill.
CREATE TABLE message_versions (
    message_id bigint NOT NULL REFERENCES messages (id),
    revision integer NOT NULL CHECK (revision >= 1),
    content jsonb NOT NULL,
    created_at timestamptz NOT NULL,
    PRIMARY KEY (message_id, revision)
);
CREATE INDEX message_versions_recent_edits ON message_versions (created_at)
    WHERE revision > 1;
