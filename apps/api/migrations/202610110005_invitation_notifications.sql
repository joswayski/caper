-- Invitations are notifications, not messages or membership grants. Existing
-- message notifications and their uniqueness constraint remain unchanged.
ALTER TABLE notifications ALTER COLUMN message_id DROP NOT NULL;
ALTER TABLE notifications ALTER COLUMN channel_id DROP NOT NULL;
ALTER TABLE notifications ADD CONSTRAINT notifications_source CHECK (
    (message_id IS NOT NULL AND channel_id IS NOT NULL
        AND kind NOT IN ('space.invitation', 'channel.invitation'))
    OR (message_id IS NULL AND space_id IS NOT NULL AND actor_id IS NOT NULL
        AND ((kind = 'space.invitation' AND channel_id IS NULL)
            OR (kind = 'channel.invitation' AND channel_id IS NOT NULL)))
);

-- Shared by gateway replay and phone delivery, including retries. The invite
-- and notification use the same transaction timestamp: a later re-invitation
-- cannot resurrect an old queued alert. No historical invites are backfilled.
CREATE VIEW notification_invitation_sources AS
SELECT n.id,
       COALESCE(
           n.dismissed_at IS NULL AND s.deleted_at IS NULL AND NOT s.demo
           AND s.owner_id = n.actor_id AND owner.deleted_at IS NULL
           AND recipient.deleted_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM user_blocks b
               WHERE b.blocker_id = n.user_id AND b.blocked_id = n.actor_id AND b.deleted_at IS NULL)
           AND CASE WHEN n.kind = 'space.invitation' THEN
               si.status = 'pending' AND si.updated_at = n.created_at
               AND si.updated_at > now() - interval '7 days'
               AND NOT EXISTS (SELECT 1 FROM space_members sm
                   WHERE sm.space_id = s.id AND sm.user_id = n.user_id AND sm.deleted_at IS NULL)
           ELSE
               ci.status = 'pending' AND ci.updated_at = n.created_at
               AND ci.updated_at > now() - interval '7 days'
               AND ch.deleted_at IS NULL AND ch.private AND ch.space_id = s.id
               AND EXISTS (SELECT 1 FROM space_members sm
                   WHERE sm.space_id = s.id AND sm.user_id = n.user_id AND sm.deleted_at IS NULL)
               AND NOT EXISTS (SELECT 1 FROM channel_members cm
                   WHERE cm.channel_id = ch.id AND cm.user_id = n.user_id AND cm.deleted_at IS NULL)
           END, false) AS eligible
FROM notifications n
JOIN spaces s ON s.id = n.space_id
JOIN users owner ON owner.id = n.actor_id
JOIN users recipient ON recipient.id = n.user_id
LEFT JOIN channels ch ON ch.id = n.channel_id
LEFT JOIN space_invitations si ON n.kind = 'space.invitation'
    AND si.space_id = n.space_id AND si.user_id = n.user_id
LEFT JOIN channel_invitations ci ON n.kind = 'channel.invitation'
    AND ci.channel_id = n.channel_id AND ci.user_id = n.user_id
WHERE n.message_id IS NULL;
