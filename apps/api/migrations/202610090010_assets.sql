-- Uploaded files. One row per logical file; its optional preview is a fixed
-- derived object (`preview/{external_id}`) beside `original/{external_id}`.
-- Rows are never deleted: `deleted_at` hides an asset and `purged_at` records
-- that its R2 objects were removed. Quota counts every row until it is purged.
CREATE TABLE assets (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    external_id text NOT NULL UNIQUE,
    owner_id bigint NOT NULL REFERENCES users (id),
    -- Later purposes (avatars, space icons) can opt out of the storage quota.
    purpose text NOT NULL CHECK (purpose IN ('attachment')),
    channel_id bigint REFERENCES channels (id),
    message_id bigint REFERENCES messages (id),
    position smallint,
    kind text NOT NULL CHECK (kind IN ('image', 'video', 'audio', 'file')),
    content_type text NOT NULL,
    filename text NOT NULL,
    -- Stored bytes: what quota counts and what the upload must exactly match.
    byte_size bigint NOT NULL CHECK (byte_size > 0),
    -- Client-reported size before compression. Display only, never trusted.
    source_byte_size bigint CHECK (source_byte_size > 0),
    width integer CHECK (width > 0),
    height integer CHECK (height > 0),
    duration_ms integer CHECK (duration_ms >= 0),
    preview_content_type text,
    preview_byte_size bigint CHECK (preview_byte_size > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    uploaded_at timestamptz,
    deleted_at timestamptz,
    purged_at timestamptz,
    CHECK (purpose <> 'attachment' OR channel_id IS NOT NULL),
    CHECK ((preview_content_type IS NULL) = (preview_byte_size IS NULL)),
    CHECK (message_id IS NULL OR uploaded_at IS NOT NULL)
);

CREATE INDEX assets_owner ON assets (owner_id, created_at);
CREATE INDEX assets_message ON assets (message_id, position) WHERE message_id IS NOT NULL;
CREATE INDEX assets_unattached ON assets (created_at)
    WHERE message_id IS NULL AND deleted_at IS NULL;
CREATE INDEX assets_purge ON assets (deleted_at)
    WHERE deleted_at IS NOT NULL AND purged_at IS NULL;
