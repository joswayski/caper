-- Uploaded files. One row per logical file. Clients upload the original to the
-- S3 incoming bucket (`incoming/{external_id}`); the media worker stores the
-- compressed result at `original/{external_id}` in R2, plus an optional
-- `preview/{external_id}`. Rows are never deleted: `deleted_at` hides an asset
-- and `purged_at` records that its R2 objects were removed. Quota counts every
-- row until it is purged.
CREATE TABLE assets (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    external_id text NOT NULL UNIQUE,
    owner_id bigint NOT NULL REFERENCES users (id),
    -- Later purposes (avatars, space icons) can opt out of the storage quota.
    purpose text NOT NULL CHECK (purpose IN ('attachment')),
    channel_id bigint REFERENCES channels (id),
    message_id bigint REFERENCES messages (id),
    position smallint,
    status text NOT NULL DEFAULT 'uploading'
        CHECK (status IN ('uploading', 'processing', 'ready', 'failed')),
    -- Compression settings profile the worker applies (room for paid tiers).
    profile text NOT NULL DEFAULT 'standard',
    -- What the client said it uploaded. Display only until processed.
    declared_content_type text NOT NULL,
    -- Exact size of the original the client may upload.
    upload_byte_size bigint NOT NULL CHECK (upload_byte_size > 0),
    -- The stored result. Until processed these describe the upload.
    kind text NOT NULL CHECK (kind IN ('image', 'video', 'audio', 'file')),
    content_type text NOT NULL,
    filename text NOT NULL,
    -- Stored bytes, which quota counts: the reservation until processed.
    byte_size bigint NOT NULL CHECK (byte_size > 0),
    content_encoding text CHECK (content_encoding IN ('gzip')),
    animated boolean NOT NULL DEFAULT false,
    width integer CHECK (width > 0),
    height integer CHECK (height > 0),
    duration_ms integer CHECK (duration_ms >= 0),
    preview_content_type text,
    preview_byte_size bigint CHECK (preview_byte_size > 0),
    failure text,
    created_at timestamptz NOT NULL DEFAULT now(),
    -- The original arrived (client confirmation or worker pickup).
    uploaded_at timestamptz,
    -- Last contact from the media worker; the sweeper fails silent jobs.
    worker_seen_at timestamptz,
    processed_at timestamptz,
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
CREATE INDEX assets_unfinished ON assets (status, created_at)
    WHERE status IN ('uploading', 'processing') AND deleted_at IS NULL;
CREATE INDEX assets_purge ON assets (deleted_at)
    WHERE deleted_at IS NOT NULL AND purged_at IS NULL;
