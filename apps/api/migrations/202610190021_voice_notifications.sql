-- Additive voice-start settings are compatible with clients that omit them.
ALTER TABLE notification_settings ADD COLUMN voice_started boolean NOT NULL DEFAULT true;
ALTER TABLE notification_overrides ADD COLUMN voice_started boolean;
ALTER TABLE notification_overrides ADD CONSTRAINT voice_override_space
    CHECK (voice_started IS NULL OR space_id IS NOT NULL);

-- Durable space-wide cooldown and serialization of successful voice joins.
CREATE TABLE voice_notification_state (
    space_id bigint PRIMARY KEY REFERENCES spaces(id),
    notified_at timestamptz
);

ALTER TABLE notifications DROP CONSTRAINT notifications_source;
ALTER TABLE notifications ADD CONSTRAINT notifications_source CHECK (
    (message_id IS NOT NULL AND channel_id IS NOT NULL
        AND kind NOT IN ('space.invitation', 'channel.invitation', 'direct.request', 'voice.started'))
    OR (message_id IS NULL AND actor_id IS NOT NULL
        AND ((kind = 'direct.request' AND space_id IS NULL AND channel_id IS NOT NULL)
            OR (space_id IS NOT NULL
                AND ((kind = 'space.invitation' AND channel_id IS NULL)
                    OR (kind IN ('channel.invitation', 'voice.started') AND channel_id IS NOT NULL)))))
);

-- Shared by enqueue, gateway replay and phone delivery. Voice is space-wide:
-- channel joins are not required, but private grants, mutes and blocks are.
CREATE FUNCTION voice_notification_allowed(recipient bigint, actor bigint, channel bigint)
RETURNS boolean LANGUAGE sql STABLE SET search_path = public AS $$
    SELECT EXISTS (
        SELECT 1 FROM channels c JOIN spaces s ON s.id=c.space_id
        JOIN space_members sm ON sm.space_id=s.id AND sm.user_id=recipient AND sm.deleted_at IS NULL
        JOIN users u ON u.id=recipient AND u.deleted_at IS NULL
        JOIN users a ON a.id=actor AND a.deleted_at IS NULL
        LEFT JOIN notification_settings ns ON ns.user_id=recipient
        LEFT JOIN notification_overrides so ON so.user_id=recipient AND so.space_id=s.id
        LEFT JOIN notification_overrides co ON co.user_id=recipient AND co.channel_id=c.id
        WHERE c.id=channel AND c.deleted_at IS NULL AND s.deleted_at IS NULL AND NOT s.demo
          AND recipient<>actor
          AND (NOT c.private OR s.owner_id=recipient OR EXISTS (
              SELECT 1 FROM channel_members cm WHERE cm.channel_id=c.id AND cm.user_id=recipient AND cm.deleted_at IS NULL))
          AND NOT EXISTS (SELECT 1 FROM user_blocks b WHERE b.deleted_at IS NULL
              AND ((b.blocker_id=recipient AND b.blocked_id=actor) OR (b.blocker_id=actor AND b.blocked_id=recipient)))
          AND COALESCE(ns.default_level,'all')<>'nothing'
          AND NOT COALESCE(ns.paused_until>now(),false)
          AND NOT COALESCE(so.muted_until>now(),false)
          AND NOT COALESCE(co.muted_until>now(),false)
          AND COALESCE(co.level,so.level,ns.default_level,'all')<>'nothing'
          AND COALESCE(so.voice_started,ns.voice_started,true)
    );
$$;

CREATE VIEW notification_voice_sources AS
SELECT n.id, n.dismissed_at IS NULL
    AND n.created_at > now()-interval '5 minutes'
    AND public.voice_notification_allowed(n.user_id,n.actor_id,n.channel_id) AS eligible
FROM notifications n WHERE n.kind='voice.started';
