-- Participation is separate from private-channel authorization.
CREATE TABLE channel_joins (
    channel_id bigint NOT NULL REFERENCES channels (id),
    user_id bigint NOT NULL REFERENCES users (id),
    PRIMARY KEY (channel_id, user_id)
);
CREATE INDEX channel_joins_user_id ON channel_joins (user_id);

CREATE TABLE channel_invitations (
    channel_id bigint NOT NULL REFERENCES channels (id),
    user_id bigint NOT NULL REFERENCES users (id),
    status text NOT NULL CHECK (status IN ('pending', 'accepted', 'declined', 'revoked')),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, user_id)
);
CREATE INDEX channel_invitations_user_id ON channel_invitations (user_id);

-- Preserve existing members' conversations during rollout. New members join
-- only one starter channel; newly created channels do not subscribe everyone.
INSERT INTO channel_joins (channel_id, user_id)
SELECT c.id, sm.user_id
FROM channels c JOIN spaces s ON s.id = c.space_id
JOIN space_members sm ON sm.space_id = s.id
WHERE c.deleted_at IS NULL AND s.deleted_at IS NULL AND NOT s.demo
  AND (NOT c.private OR s.owner_id = sm.user_id OR EXISTS (
      SELECT 1 FROM channel_members cm WHERE cm.channel_id = c.id AND cm.user_id = sm.user_id
  ));
