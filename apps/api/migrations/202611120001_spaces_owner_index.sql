-- Creating a space counts the spaces its owner already has while holding the
-- owner's row lock; without this the count reads every space.
CREATE INDEX spaces_owner_id ON spaces (owner_id) WHERE deleted_at IS NULL;
