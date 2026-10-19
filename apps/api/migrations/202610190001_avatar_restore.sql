ALTER TABLE users ADD COLUMN previous_avatar_asset text REFERENCES assets (external_id);
ALTER TABLE users ADD COLUMN avatar_history_initialized boolean NOT NULL DEFAULT false;
ALTER TABLE users ADD COLUMN avatar_restored_at timestamptz;
