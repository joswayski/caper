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
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT root.external_id,max(reply.channel_seq)
         FROM public.messages root
         JOIN public.messages reply ON reply.thread_root_id=root.id
         JOIN public.chat_sessions author ON author.id=reply.session_id
         JOIN public.users u ON u.id=author.user_id AND u.deleted_at IS NULL
         LEFT JOIN public.thread_reads r ON r.thread_root_id=root.id AND r.user_id=$2
         WHERE root.channel_id=$1 AND root.thread_root_id IS NULL
           AND reply.channel_seq>COALESCE(r.seq,0) AND author.user_id<>$2
           AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.blocker_id=$2
                          AND b.blocked_id=author.user_id AND b.deleted_at IS NULL)
           AND EXISTS(SELECT 1 FROM public.messages participation
                      JOIN public.chat_sessions participant ON participant.id=participation.session_id
                      WHERE (participation.id=root.id OR participation.thread_root_id=root.id)
                        AND participant.user_id=$2)
         GROUP BY root.id,root.external_id ORDER BY root.external_id",
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
