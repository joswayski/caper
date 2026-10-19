//! Participation comes from authorship, never from viewing or summary avatars.
use super::*;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/chat/channels/{channel}/threads/unread", get(list))
        .route(
            "/api/chat/channels/{channel}/messages/{root}/thread/read",
            put(read),
        )
}

async fn user(pool: &PgPool, headers: &HeaderMap) -> Result<i64, ApiError> {
    request_user(pool, headers)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "sign in required"))
}

async fn list(
    State(state): State<AppState>,
    Path(channel): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let pool = &enabled(&state)?.pool;
    let user = user(pool, &headers).await?;
    let mut tx = pool.begin().await.map_err(database_error)?;
    let (channel, _, _) = editing::locked_channel(&mut tx, &channel, Some(user), false).await?;
    // One pass over this channel's replies (replies always share their root's
    // channel) finds each thread's newest reply from someone else and whether
    // the reader replied; only those threads are then checked against the
    // root and the read position. Deleted replies are tombstones, not news.
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "WITH replies AS (
           SELECT reply.thread_root_id AS root_id,reply.channel_seq,author.user_id=$2 AS mine,
                  reply.deleted_at IS NULL AND author.user_id<>$2
                    AND EXISTS(SELECT 1 FROM public.users u WHERE u.id=author.user_id AND u.deleted_at IS NULL)
                    AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.blocker_id=$2
                                   AND b.blocked_id=author.user_id AND b.deleted_at IS NULL) AS counts
           FROM public.messages reply JOIN public.chat_sessions author ON author.id=reply.session_id
           WHERE reply.channel_id=$1 AND reply.thread_root_id IS NOT NULL
         ), threads AS (
           SELECT root_id,max(channel_seq) FILTER (WHERE counts) AS seq,bool_or(mine) AS replied
           FROM replies GROUP BY root_id
           HAVING bool_or(counts)
         )
         SELECT root.external_id,t.seq FROM threads t
         JOIN public.messages root ON root.id=t.root_id
         JOIN public.chat_sessions root_author ON root_author.id=root.session_id
         LEFT JOIN public.thread_reads r ON r.thread_root_id=root.id AND r.user_id=$2
         WHERE root.channel_id=$1 AND root.thread_root_id IS NULL AND t.seq>COALESCE(r.seq,0)
           AND (t.replied OR root_author.user_id=$2)
         ORDER BY root.external_id",
    )
    .bind(channel)
    .bind(user)
    .fetch_all(&mut *tx)
    .await
    .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok(Json(json!({"threads":rows.into_iter().map(|(root,seq)|
        json!({"rootId":root,"seq":seq.to_string()})).collect::<Vec<_>>()})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadInput {
    seq: String,
}

async fn read(
    State(state): State<AppState>,
    Path((channel, root)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<ReadInput>,
) -> Result<StatusCode, ApiError> {
    let pool = &enabled(&state)?.pool;
    let user = user(pool, &headers).await?;
    let seq = cursor(&input.seq)?;
    let mut tx = pool.begin().await.map_err(database_error)?;
    let (channel, _, _) = editing::locked_channel(&mut tx, &channel, Some(user), false).await?;
    let result = sqlx::query(
        "INSERT INTO public.thread_reads(thread_root_id,user_id,seq)
         SELECT root.id,$3,LEAST($4,COALESCE((SELECT max(reply.channel_seq)
                FROM public.messages reply WHERE reply.thread_root_id=root.id),0))
         FROM public.messages root WHERE root.channel_id=$1 AND root.external_id=$2
           AND root.thread_root_id IS NULL
         ON CONFLICT(thread_root_id,user_id) DO UPDATE SET seq=GREATEST(thread_reads.seq,EXCLUDED.seq)",
    )
    .bind(channel)
    .bind(root)
    .bind(user)
    .bind(seq)
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "thread not found"));
    }
    tx.commit().await.map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
}
