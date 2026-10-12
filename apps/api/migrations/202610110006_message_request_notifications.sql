-- New DM requests notify once when created, not once per pending message.
ALTER TABLE notifications DROP CONSTRAINT notifications_source;
ALTER TABLE notifications ADD CONSTRAINT notifications_source CHECK (
    (message_id IS NOT NULL AND channel_id IS NOT NULL
        AND kind NOT IN ('space.invitation', 'channel.invitation', 'direct.request'))
    OR (message_id IS NULL AND actor_id IS NOT NULL
        AND ((kind = 'direct.request' AND space_id IS NULL AND channel_id IS NOT NULL)
            OR (space_id IS NOT NULL
                AND ((kind = 'space.invitation' AND channel_id IS NULL)
                    OR (kind = 'channel.invitation' AND channel_id IS NOT NULL)))))
);

-- Privacy is enforced when creating a request; existing conversations are
-- unaffected by later privacy changes. Acceptance, decline and either block
-- suppress unsent request alerts. Shared by phone delivery and gateway replay.
CREATE VIEW notification_request_sources AS
SELECT n.id,
       n.dismissed_at IS NULL AND ch.deleted_at IS NULL
       AND actor.deleted_at IS NULL AND recipient.deleted_at IS NULL
       AND d.accepted_at IS NULL AND d.declined_at IS NULL
       AND d.requested_by = n.actor_id AND d.created_at = n.created_at
       AND n.user_id <> n.actor_id
       AND n.user_id IN (d.low_user_id, d.high_user_id)
       AND NOT EXISTS (SELECT 1 FROM user_blocks b WHERE b.deleted_at IS NULL
           AND ((b.blocker_id = n.user_id AND b.blocked_id = n.actor_id)
               OR (b.blocker_id = n.actor_id AND b.blocked_id = n.user_id))) AS eligible
FROM notifications n
JOIN direct_conversations d ON d.channel_id = n.channel_id
JOIN channels ch ON ch.id = d.channel_id AND ch.space_id IS NULL
JOIN users actor ON actor.id = n.actor_id
JOIN users recipient ON recipient.id = n.user_id
WHERE n.kind = 'direct.request';
