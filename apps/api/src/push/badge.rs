//! App-icon attention count, independent of OS permission, alert level and DND.
//! Count messages, not stream events or deliveries (one user can have many devices).
use sqlx::PgPool;

pub(crate) async fn count(pool: &PgPool, user: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "WITH visible AS (
          SELECT c.id,c.space_id,COALESCE(cr.seq,dr.seq,0) AS read_seq,
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
        ), unread AS (
          SELECT m.id FROM visible v
          JOIN public.messages m ON m.channel_id=v.id AND m.deleted_at IS NULL
          JOIN public.chat_sessions author ON author.id=m.session_id
          JOIN public.users u ON u.id=author.user_id AND u.deleted_at IS NULL
          LEFT JOIN public.thread_reads tr ON tr.thread_root_id=m.thread_root_id AND tr.user_id=$1
          CROSS JOIN LATERAL (SELECT m.forward_source_id IS NULL AND m.payload->'content'->'mentions'
            @> jsonb_build_array(jsonb_build_object('type','user','id',(SELECT external_id FROM public.users WHERE id=$1))) AS mentioned) mention
          WHERE author.user_id<>$1
            AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.blocker_id=$1 AND b.blocked_id=author.user_id AND b.deleted_at IS NULL)
            AND (NOT v.muted OR mention.mentioned)
            AND (m.thread_root_id IS NULL OR m.channel_seq>COALESCE(tr.seq,0))
            AND ((m.channel_seq>v.read_seq AND
                   ((v.joined AND (m.thread_root_id IS NULL OR m.broadcast)) OR mention.mentioned))
              OR (m.thread_root_id IS NOT NULL AND v.joined AND EXISTS(
                SELECT 1 FROM public.messages participation
                JOIN public.chat_sessions participant ON participant.id=participation.session_id
                WHERE (participation.id=m.thread_root_id OR participation.thread_root_id=m.thread_root_id)
                  AND participant.user_id=$1)))
        ), pending AS (
          SELECT d.channel_id FROM public.direct_conversations d
          JOIN public.channels c ON c.id=d.channel_id AND c.deleted_at IS NULL
          JOIN public.users actor ON actor.id=d.requested_by AND actor.deleted_at IS NULL
          WHERE $1 IN(d.low_user_id,d.high_user_id) AND d.requested_by<>$1
            AND d.accepted_at IS NULL AND d.declined_at IS NULL
            AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.deleted_at IS NULL
              AND ((b.blocker_id=$1 AND b.blocked_id=d.requested_by) OR (b.blocker_id=d.requested_by AND b.blocked_id=$1)))
          UNION ALL
          SELECT si.space_id FROM public.space_invitations si
          JOIN public.spaces s ON s.id=si.space_id AND s.deleted_at IS NULL AND NOT s.demo
          JOIN public.users owner ON owner.id=s.owner_id AND owner.deleted_at IS NULL
          WHERE si.user_id=$1 AND si.status='pending' AND si.updated_at>now()-interval '7 days'
            AND NOT EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$1 AND sm.deleted_at IS NULL)
            AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.blocker_id=$1 AND b.blocked_id=s.owner_id AND b.deleted_at IS NULL)
          UNION ALL
          SELECT ci.channel_id FROM public.channel_invitations ci
          JOIN public.channels c ON c.id=ci.channel_id AND c.deleted_at IS NULL AND c.private
          JOIN public.spaces s ON s.id=c.space_id AND s.deleted_at IS NULL AND NOT s.demo
          JOIN public.users owner ON owner.id=s.owner_id AND owner.deleted_at IS NULL
          WHERE ci.user_id=$1 AND ci.status='pending' AND ci.updated_at>now()-interval '7 days'
            AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$1 AND sm.deleted_at IS NULL)
            AND NOT EXISTS(SELECT 1 FROM public.channel_members cm WHERE cm.channel_id=c.id AND cm.user_id=$1 AND cm.deleted_at IS NULL)
            AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.blocker_id=$1 AND b.blocked_id=s.owner_id AND b.deleted_at IS NULL)
        ) SELECT (SELECT count(*) FROM unread)+(SELECT count(*) FROM pending)",
    )
    .bind(user)
    .fetch_one(pool)
    .await
}
