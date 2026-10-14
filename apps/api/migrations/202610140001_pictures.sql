-- Custom profile pictures and space icons, uploaded through the same assets
-- flow as attachments (R2 bytes, signed CDN delivery). Completing a picture
-- upload applies it. Replaced or removed pictures keep their rows and objects:
-- they are small, rarely change, and stay out of the file storage quota.
ALTER TABLE assets DROP CONSTRAINT assets_purpose_check;
ALTER TABLE assets ADD CONSTRAINT assets_purpose_check
    CHECK (purpose IN ('attachment', 'avatar', 'space_icon'));
ALTER TABLE assets ADD COLUMN space_id bigint REFERENCES spaces (id);
ALTER TABLE assets ADD CONSTRAINT assets_space_icon_space
    CHECK (purpose <> 'space_icon' OR space_id IS NOT NULL);
-- Daily picture-upload limit per person.
CREATE INDEX assets_pictures ON assets (owner_id, created_at) WHERE purpose <> 'attachment';

-- The current custom picture. NULL shows the saved mascot (`avatar_id`, which
-- never changes) or the space's initial.
ALTER TABLE users ADD COLUMN avatar_asset text REFERENCES assets (external_id);
ALTER TABLE spaces ADD COLUMN icon_asset text REFERENCES assets (external_id);
