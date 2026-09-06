-- Baseline schema. Feature tables belong in later numbered migrations.
CREATE TABLE IF NOT EXISTS schema_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT INTO schema_meta (key, value)
VALUES ('app', 'caper')
ON CONFLICT (key) DO NOTHING;
