ALTER TABLE users
    ADD COLUMN avatar_id smallint NOT NULL DEFAULT floor(random() * 800)::smallint,
    ADD CONSTRAINT users_avatar_id_range CHECK (avatar_id >= 0 AND avatar_id < 800);
