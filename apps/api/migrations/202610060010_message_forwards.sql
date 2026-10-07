-- A forward is a destination message referencing one original conversation.
-- References are flattened on creation; destination replies remain independent.
-- Retain both messages: removing an original must never cascade to its forwards.
ALTER TABLE messages ADD COLUMN forward_source_id bigint REFERENCES messages(id);
ALTER TABLE messages ADD CONSTRAINT messages_forward_not_self
    CHECK (forward_source_id IS NULL OR forward_source_id <> id);
CREATE INDEX messages_forward_source ON messages(forward_source_id)
    WHERE forward_source_id IS NOT NULL;
