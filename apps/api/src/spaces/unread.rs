//! Unread navigation is independent of OS notification permission and mutes.
use super::*;
use std::collections::BTreeMap;

pub(super) async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Value>, ApiError> {
    // Use message sequences, not channel heads: reactions, edits and pins
    // advance the stream but must not invent unread messages. Unjoined
    // channels and non-broadcast thread replies count only direct @mentions.
    // Only channels whose stream moved past the read position are checked.
    // Count messages, not stream events; the two sets below are disjoint.
    // `mentionCount` is the subset that @mentions or replies to the reader.
    let rows: Vec<(String, String, i64, i64, i64, i64)> = sqlx::query_as(
        "WITH visible AS (
          SELECT s.external_id AS space,c.id,c.external_id AS channel,COALESCE(r.seq,0) AS read_seq,
                 EXISTS(SELECT 1 FROM public.channel_joins cj
                        WHERE cj.channel_id=c.id AND cj.user_id=$1 AND cj.deleted_at IS NULL) AS joined
          FROM public.space_members sm JOIN public.spaces s ON s.id=sm.space_id
          JOIN public.channels c ON c.space_id=s.id
          LEFT JOIN public.channel_reads r ON r.channel_id=c.id AND r.user_id=$1
          WHERE sm.user_id=$1 AND sm.deleted_at IS NULL AND s.deleted_at IS NULL AND NOT s.demo
            AND c.deleted_at IS NULL AND c.last_seq>COALESCE(r.seq,0)
            AND (NOT c.private OR s.owner_id=$1 OR EXISTS(SELECT 1 FROM public.channel_members cm
                 WHERE cm.channel_id=c.id AND cm.user_id=$1 AND cm.deleted_at IS NULL))
        ), mentioned AS (
          -- Few messages mention any one person: find them once through
          -- messages_mentions instead of walking each unread history. OFFSET 0
          -- keeps the planner from pushing the per-channel join into this scan.
          SELECT v.id,max(m.channel_seq) AS seq,count(*) AS unread_count
          FROM (SELECT channel_id,channel_seq,session_id,thread_root_id,broadcast FROM public.messages
                WHERE forward_source_id IS NULL AND deleted_at IS NULL
                  AND NOT EXISTS(SELECT 1 FROM public.notification_inbox_reads ir WHERE ir.user_id=$1 AND ir.item_id='m:' || messages.id AND ir.revoked_at IS NULL)
                  AND payload->'content'->'mentions' @> jsonb_build_array(jsonb_build_object('type','user','id',$2::text))
                OFFSET 0) m
          JOIN visible v ON v.id=m.channel_id AND m.channel_seq>v.read_seq
          JOIN public.chat_sessions cs ON cs.id=m.session_id
          JOIN public.users author ON author.id=cs.user_id AND author.deleted_at IS NULL
          WHERE cs.user_id<>$1 AND NOT (v.joined AND (m.thread_root_id IS NULL OR m.broadcast))
            AND NOT EXISTS(SELECT 1 FROM public.user_blocks b
                 WHERE b.blocker_id=$1 AND b.blocked_id=cs.user_id AND b.deleted_at IS NULL)
          GROUP BY v.id
        )
        SELECT v.space,v.channel,GREATEST(latest.seq,mention.seq),v.read_seq,
               COALESCE(latest.unread_count,0)+COALESCE(mention.unread_count,0),
               COALESCE(latest.mention_count,0)+COALESCE(mention.unread_count,0)
        FROM visible v
        LEFT JOIN LATERAL (
          -- Mentions and replies to the reader drive the rail's count badge.
          SELECT max(m.channel_seq) AS seq,count(*) AS unread_count,
                 count(*) FILTER (WHERE m.forward_source_id IS NULL AND (
                   m.payload->'content'->'mentions' @> jsonb_build_array(jsonb_build_object('type','user','id',$2::text))
                   OR m.payload->'replyTo'->'author'->>'id'=$2::text)) AS mention_count
          FROM public.messages m
          JOIN public.chat_sessions cs ON cs.id=m.session_id
          JOIN public.users author ON author.id=cs.user_id AND author.deleted_at IS NULL
          WHERE v.joined AND m.channel_id=v.id AND m.channel_seq>v.read_seq AND m.deleted_at IS NULL
            AND (m.thread_root_id IS NULL OR m.broadcast) AND cs.user_id<>$1
            AND NOT EXISTS(SELECT 1 FROM public.notification_inbox_reads ir WHERE ir.user_id=$1 AND ir.item_id='m:' || m.id AND ir.revoked_at IS NULL)
            AND NOT EXISTS(SELECT 1 FROM public.user_blocks b
                 WHERE b.blocker_id=$1 AND b.blocked_id=cs.user_id AND b.deleted_at IS NULL)
        ) latest ON true
        LEFT JOIN mentioned mention ON mention.id=v.id
        WHERE latest.seq IS NOT NULL OR mention.seq IS NOT NULL
        ORDER BY v.space,v.channel",
    )
    .bind(principal.user.id)
    .bind(&principal.user.external_id)
    .fetch_all(pool(&state)?)
    .await
    .map_err(database_error)?;
    let mut spaces: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for (space, channel, seq, read_seq, unread_count, mention_count) in rows {
        spaces
            .entry(space)
            .or_default()
            .push(json!({"id":channel,"lastSeq":seq.to_string(),"readSeq":read_seq.to_string(),"unreadCount":unread_count,"mentionCount":mention_count}));
    }
    let feedback_waiting = crate::feedback::waiting_count(pool(&state)?, principal.user.id).await?;
    let badge_count = crate::push::badge::count(pool(&state)?, principal.user.id)
        .await
        .map_err(database_error)?;
    crate::push::badge::seen(pool(&state)?, &principal.token_hash, badge_count).await;
    Ok(Json(
        json!({"spaces":spaces.into_iter().map(|(id,channels)|json!({"id":id,"channels":channels})).collect::<Vec<_>>(),
            "feedbackWaiting": feedback_waiting, "badgeCount": badge_count}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct ReadInput {
    seq: String,
    /// Mark unread: set the position to `seq` even when that moves it back.
    #[serde(default)]
    mark_unread: bool,
}

pub(super) async fn read(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, channel)): Path<(String, String)>,
    Json(input): Json<ReadInput>,
) -> Result<StatusCode, ApiError> {
    let seq = crate::chat::cursor(&input.seq)?;
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    // The channel and the reader's current position, locked so a concurrent
    // read or mark unread applies after this one rather than between.
    let target: Option<(i64, i64, Option<i64>)> = sqlx::query_as(
        "SELECT c.id,LEAST($4,c.last_seq),
                (SELECT r.seq FROM public.channel_reads r WHERE r.channel_id=c.id AND r.user_id=$3 FOR UPDATE)
         FROM public.channels c JOIN public.spaces s ON s.id=c.space_id
         WHERE s.external_id=$1 AND c.external_id=$2 AND s.deleted_at IS NULL AND NOT s.demo AND c.deleted_at IS NULL
           AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$3 AND sm.deleted_at IS NULL)
           AND (NOT c.private OR s.owner_id=$3 OR EXISTS(SELECT 1 FROM public.channel_members cm
                WHERE cm.channel_id=c.id AND cm.user_id=$3 AND cm.deleted_at IS NULL))",
    )
    .bind(space)
    .bind(channel)
    .bind(principal.user.id)
    .bind(seq)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?;
    let (channel, seq, previous) = target.ok_or_else(not_found)?;
    // Ordinary reads only ever advance; Mark unread sets the position exactly.
    sqlx::query(
        "INSERT INTO public.channel_reads(channel_id,user_id,seq) VALUES($1,$2,$3)
         ON CONFLICT(channel_id,user_id) DO UPDATE SET seq=CASE WHEN $4 THEN EXCLUDED.seq
           ELSE GREATEST(channel_reads.seq,EXCLUDED.seq) END",
    )
    .bind(channel)
    .bind(principal.user.id)
    .bind(seq)
    .bind(input.mark_unread)
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    if input.mark_unread {
        crate::push::inbox::marked_unread(
            &mut tx,
            principal.user.id,
            channel,
            None,
            seq,
            previous.unwrap_or(0),
        )
        .await
        .map_err(database_error)?;
    }
    tx.commit().await.map_err(database_error)?;
    crate::push::badge::request(pool(&state)?, principal.user.id).await;
    Ok(StatusCode::NO_CONTENT)
}
