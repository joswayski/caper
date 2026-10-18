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
    // Only channels whose stream moved past the read position are checked:
    // joined channels take their newest qualifying message by index.
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
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
          SELECT v.id,max(m.channel_seq) AS seq
          FROM (SELECT channel_id,channel_seq,session_id FROM public.messages
                WHERE forward_source_id IS NULL
                  AND payload->'content'->'mentions' @> jsonb_build_array(jsonb_build_object('type','user','id',$2::text))
                OFFSET 0) m
          JOIN visible v ON v.id=m.channel_id AND m.channel_seq>v.read_seq
          JOIN public.chat_sessions cs ON cs.id=m.session_id
          JOIN public.users author ON author.id=cs.user_id AND author.deleted_at IS NULL
          WHERE cs.user_id<>$1
            AND NOT EXISTS(SELECT 1 FROM public.user_blocks b
                 WHERE b.blocker_id=$1 AND b.blocked_id=cs.user_id AND b.deleted_at IS NULL)
          GROUP BY v.id
        )
        SELECT v.space,v.channel,GREATEST(latest.seq,mention.seq),v.read_seq
        FROM visible v
        LEFT JOIN LATERAL (
          SELECT m.channel_seq AS seq FROM public.messages m
          JOIN public.chat_sessions cs ON cs.id=m.session_id
          JOIN public.users author ON author.id=cs.user_id AND author.deleted_at IS NULL
          WHERE v.joined AND m.channel_id=v.id AND m.channel_seq>v.read_seq
            AND (m.thread_root_id IS NULL OR m.broadcast) AND cs.user_id<>$1
            AND NOT EXISTS(SELECT 1 FROM public.user_blocks b
                 WHERE b.blocker_id=$1 AND b.blocked_id=cs.user_id AND b.deleted_at IS NULL)
          ORDER BY m.channel_seq DESC LIMIT 1
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
    for (space, channel, seq, read_seq) in rows {
        spaces
            .entry(space)
            .or_default()
            .push(json!({"id":channel,"lastSeq":seq.to_string(),"readSeq":read_seq.to_string()}));
    }
    let feedback_waiting = crate::feedback::waiting_count(pool(&state)?, principal.user.id).await?;
    Ok(Json(
        json!({"spaces":spaces.into_iter().map(|(id,channels)|json!({"id":id,"channels":channels})).collect::<Vec<_>>(),
            "feedbackWaiting": feedback_waiting}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadInput {
    seq: String,
}

pub(super) async fn read(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, channel)): Path<(String, String)>,
    Json(input): Json<ReadInput>,
) -> Result<StatusCode, ApiError> {
    let seq = crate::chat::cursor(&input.seq)?;
    let result = sqlx::query(
        "INSERT INTO public.channel_reads(channel_id,user_id,seq)
         SELECT c.id,$3,LEAST($4,c.last_seq) FROM public.channels c JOIN public.spaces s ON s.id=c.space_id
         WHERE s.external_id=$1 AND c.external_id=$2 AND s.deleted_at IS NULL AND NOT s.demo AND c.deleted_at IS NULL
           AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$3 AND sm.deleted_at IS NULL)
           AND (NOT c.private OR s.owner_id=$3 OR EXISTS(SELECT 1 FROM public.channel_members cm
                WHERE cm.channel_id=c.id AND cm.user_id=$3 AND cm.deleted_at IS NULL))
         ON CONFLICT(channel_id,user_id) DO UPDATE SET seq=GREATEST(channel_reads.seq,EXCLUDED.seq)",
    )
    .bind(space)
    .bind(channel)
    .bind(principal.user.id)
    .bind(seq)
    .execute(pool(&state)?)
    .await
    .map_err(database_error)?;
    if result.rows_affected() == 0 {
        return Err(not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}
