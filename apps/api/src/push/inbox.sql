WITH visible AS (
    SELECT c.id, c.space_id, COALESCE(cr.seq, dr.seq, 0) AS read_seq,
        (c.space_id IS NULL OR EXISTS(SELECT 1 FROM public.channel_joins j
            WHERE j.channel_id=c.id AND j.user_id=$1 AND j.deleted_at IS NULL)) AS joined,
        (COALESCE(co.muted_until>now(),false) OR COALESCE(so.muted_until>now(),false)) AS muted
    FROM public.channels c
    LEFT JOIN public.spaces s ON s.id=c.space_id
    LEFT JOIN public.channel_reads cr ON cr.channel_id=c.id AND cr.user_id=$1
    LEFT JOIN public.direct_reads dr ON dr.channel_id=c.id AND dr.user_id=$1
    LEFT JOIN public.notification_overrides co ON co.channel_id=c.id AND co.user_id=$1
    LEFT JOIN public.notification_overrides so ON so.space_id=c.space_id AND so.user_id=$1
    WHERE c.deleted_at IS NULL AND (
        (c.space_id IS NULL AND EXISTS(SELECT 1 FROM public.direct_conversations d
            JOIN public.users peer ON peer.id=CASE WHEN d.low_user_id=$1 THEN d.high_user_id ELSE d.low_user_id END
            WHERE d.channel_id=c.id AND $1 IN(d.low_user_id,d.high_user_id)
                AND d.accepted_at IS NOT NULL AND peer.deleted_at IS NULL
                AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.deleted_at IS NULL
                    AND ((b.blocker_id=$1 AND b.blocked_id=peer.id) OR (b.blocker_id=peer.id AND b.blocked_id=$1)))))
        OR (s.deleted_at IS NULL AND NOT s.demo
            AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$1 AND sm.deleted_at IS NULL)
            AND (NOT c.private OR s.owner_id=$1 OR EXISTS(SELECT 1 FROM public.channel_members cm
                WHERE cm.channel_id=c.id AND cm.user_id=$1 AND cm.deleted_at IS NULL))))
), message_activity AS (
    SELECT 'm:' || m.id AS id,
        CASE WHEN mention.mentioned THEN 'mention' WHEN m.thread_root_id IS NOT NULL THEN 'reply' ELSE 'message' END AS kind,
        m.created_at, author.user_id AS actor_id, m.id AS message_id, v.id AS channel_id, v.space_id,
        NULL::text AS emoji,
        (m.thread_root_id IS NULL OR m.channel_seq>COALESCE(tr.seq,0))
            AND (m.channel_seq>v.read_seq OR (m.thread_root_id IS NOT NULL AND participation.mine)) AS unread
    FROM visible v
    JOIN public.messages m ON m.channel_id=v.id AND m.deleted_at IS NULL
    JOIN public.chat_sessions author ON author.id=m.session_id
    JOIN public.users u ON u.id=author.user_id AND u.deleted_at IS NULL
    LEFT JOIN public.thread_reads tr ON tr.thread_root_id=m.thread_root_id AND tr.user_id=$1
    CROSS JOIN LATERAL (SELECT COALESCE(m.forward_source_id IS NULL AND m.payload->'content'->'mentions'
        @> jsonb_build_array(jsonb_build_object('type','user','id',(SELECT external_id FROM public.users WHERE id=$1))),false) AS mentioned) mention
    CROSS JOIN LATERAL (SELECT v.joined AND EXISTS(
        SELECT 1 FROM public.messages p JOIN public.chat_sessions ps ON ps.id=p.session_id
        WHERE (p.id=m.thread_root_id OR p.thread_root_id=m.thread_root_id) AND ps.user_id=$1) AS mine) participation
    WHERE author.user_id<>$1
        AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.blocker_id=$1 AND b.blocked_id=author.user_id AND b.deleted_at IS NULL)
        AND (NOT v.muted OR mention.mentioned)
        AND ((v.joined AND (m.thread_root_id IS NULL OR m.broadcast)) OR mention.mentioned
            OR (m.thread_root_id IS NOT NULL AND participation.mine))
), reaction_activity AS (
    SELECT 'r:' || r.id AS id, 'reaction'::text AS kind, r.created_at, r.user_id AS actor_id,
        m.id AS message_id, v.id AS channel_id, v.space_id, r.emoji,
        -- Seen once the conversation's read position passes it, as for messages.
        r.seq IS NULL OR r.seq>v.read_seq AS unread
    FROM visible v
    JOIN public.messages m ON m.channel_id=v.id AND m.deleted_at IS NULL
    JOIN public.chat_sessions author ON author.id=m.session_id AND author.user_id=$1
    JOIN public.message_reactions r ON r.message_id=m.id AND r.deleted_at IS NULL AND r.user_id<>$1
    JOIN public.users actor ON actor.id=r.user_id AND actor.deleted_at IS NULL
    WHERE NOT v.muted
        AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.blocker_id=$1 AND b.blocked_id=r.user_id AND b.deleted_at IS NULL)
), pending AS (
    SELECT 'd:' || d.channel_id || ':' || extract(epoch FROM d.created_at)::text AS id,
        'direct.request'::text AS kind, d.created_at, d.requested_by AS actor_id,
        NULL::bigint AS message_id, d.channel_id, NULL::bigint AS space_id, NULL::text AS emoji, true AS unread
    FROM public.direct_conversations d
    JOIN public.channels c ON c.id=d.channel_id AND c.deleted_at IS NULL
    JOIN public.users actor ON actor.id=d.requested_by AND actor.deleted_at IS NULL
    WHERE $1 IN(d.low_user_id,d.high_user_id) AND d.requested_by<>$1
        AND d.accepted_at IS NULL AND d.declined_at IS NULL
        AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.deleted_at IS NULL
            AND ((b.blocker_id=$1 AND b.blocked_id=d.requested_by) OR (b.blocker_id=d.requested_by AND b.blocked_id=$1)))
    UNION ALL
    SELECT 's:' || si.space_id || ':' || extract(epoch FROM si.updated_at)::text,
        'space.invitation', si.updated_at, s.owner_id, NULL::bigint, NULL::bigint, s.id, NULL::text, true
    FROM public.space_invitations si
    JOIN public.spaces s ON s.id=si.space_id AND s.deleted_at IS NULL AND NOT s.demo
    JOIN public.users owner ON owner.id=s.owner_id AND owner.deleted_at IS NULL
    WHERE si.user_id=$1 AND si.status='pending' AND si.updated_at>now()-interval '7 days'
        AND NOT EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$1 AND sm.deleted_at IS NULL)
        AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.blocker_id=$1 AND b.blocked_id=s.owner_id AND b.deleted_at IS NULL)
    UNION ALL
    SELECT 'c:' || ci.channel_id || ':' || extract(epoch FROM ci.updated_at)::text,
        'channel.invitation', ci.updated_at, s.owner_id, NULL::bigint, c.id, s.id, NULL::text, true
    FROM public.channel_invitations ci
    JOIN public.channels c ON c.id=ci.channel_id AND c.deleted_at IS NULL AND c.private
    JOIN public.spaces s ON s.id=c.space_id AND s.deleted_at IS NULL AND NOT s.demo
    JOIN public.users owner ON owner.id=s.owner_id AND owner.deleted_at IS NULL
    WHERE ci.user_id=$1 AND ci.status='pending' AND ci.updated_at>now()-interval '7 days'
        AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$1 AND sm.deleted_at IS NULL)
        AND NOT EXISTS(SELECT 1 FROM public.channel_members cm WHERE cm.channel_id=c.id AND cm.user_id=$1 AND cm.deleted_at IS NULL)
        AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.blocker_id=$1 AND b.blocked_id=s.owner_id AND b.deleted_at IS NULL)
), sources AS (
    SELECT * FROM message_activity UNION ALL SELECT * FROM reaction_activity UNION ALL SELECT * FROM pending
), activity AS (
    SELECT sources.*, sources.unread AND reads.item_id IS NULL AS is_unread
    FROM sources LEFT JOIN public.notification_inbox_reads reads ON reads.user_id=$1 AND reads.item_id=sources.id
)
