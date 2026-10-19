-- Files larger than one part upload in parts. The row keeps R2's upload ID
-- (so a cancelled or abandoned upload can be aborted), the part size the
-- client was given, and when the client last asked for part URLs: a long
-- upload that is still making progress is not abandoned after an hour.
ALTER TABLE assets
    ADD COLUMN multipart_upload_id text,
    ADD COLUMN part_bytes bigint,
    ADD COLUMN upload_active_at timestamptz;
