-- Editing a sent message's photos and videos. A replaced file keeps its
-- message_id, so cleanup never purges it and it still counts toward its
-- owner's storage, and is retired: no longer shown on its message and readable
-- only by its owner. A replacement records the first-sent file it was rendered
-- from and the edit description, both shown only to the author. Additive:
-- older API and gateway builds list their columns explicitly.
ALTER TABLE assets ADD COLUMN retired_at timestamptz;
ALTER TABLE assets ADD COLUMN edit_source_id bigint REFERENCES assets (id);
ALTER TABLE assets ADD COLUMN edit jsonb;
ALTER TABLE assets ADD CONSTRAINT assets_retired_sent
    CHECK (retired_at IS NULL OR message_id IS NOT NULL);
ALTER TABLE assets ADD CONSTRAINT assets_edit_source
    CHECK ((edit IS NULL) = (edit_source_id IS NULL));
