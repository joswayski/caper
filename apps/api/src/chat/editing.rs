//! In-place content edits and retained versions. Delivery order, thread summaries,
//! reactions and pins remain independent of a message's content revision.
use super::*;
use sqlx::{Postgres, Transaction};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct EditInput {
    text: String,
    expected_revision: i32,
}

pub(super) async fn edit(
    State(state): State<AppState>,
    Path((channel, message)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<EditInput>,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let result = persist_edit(
        &chat.pool,
        &channel,
        &message,
        sender_token(&headers)?,
        &input.text,
        input.expected_revision,
    )
    .await?;
    chat.wake.notify_one();
    Ok(Json(result))
}

/// Membership changes lock space before channel. Check access in a fresh
/// statement after waiting, just like sends/history, so revocation wins.
async fn locked_channel(
    tx: &mut Transaction<'_, Postgres>,
    channel: &str,
    user: Option<i64>,
    writing: bool,
) -> Result<(i64, i64), ApiError> {
    let space: Option<i64> =
        sqlx::query_scalar("SELECT space_id FROM public.channels WHERE external_id=$1")
            .bind(channel)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    if let Some(space) = space {
        sqlx::query(if writing {
            "SELECT id FROM public.spaces WHERE id=$1 FOR UPDATE"
        } else {
            "SELECT id FROM public.spaces WHERE id=$1 FOR SHARE"
        })
        .bind(space)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)?;
    }
    // Lock first; a subsequent statement gets the post-wait permissions snapshot.
    let row: Option<(i64, i64)> = sqlx::query_as(if writing {
        "SELECT id,last_seq FROM public.channels WHERE external_id=$1 FOR UPDATE"
    } else {
        "SELECT id,last_seq FROM public.channels WHERE external_id=$1 FOR SHARE"
    })
    .bind(channel)
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?;
    let row = row.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM public.channels c LEFT JOIN public.spaces s ON s.id=c.space_id
         WHERE c.id=$1 AND c.deleted_at IS NULL AND $2::bigint IS NOT NULL
           AND ((s.deleted_at IS NULL AND NOT s.demo
                 AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$2 AND sm.deleted_at IS NULL)
                 AND (s.owner_id=$2 OR NOT c.private OR EXISTS(SELECT 1 FROM public.channel_members cm WHERE cm.channel_id=c.id AND cm.user_id=$2 AND cm.deleted_at IS NULL))
                 AND (NOT $3 OR EXISTS(SELECT 1 FROM public.channel_joins cj WHERE cj.channel_id=c.id AND cj.user_id=$2 AND cj.deleted_at IS NULL)))
                OR (c.space_id IS NULL AND EXISTS(SELECT 1 FROM public.direct_conversations d
                    JOIN public.users lo ON lo.id=d.low_user_id JOIN public.users hi ON hi.id=d.high_user_id
                    WHERE d.channel_id=c.id AND $2 IN (d.low_user_id,d.high_user_id) AND lo.deleted_at IS NULL AND hi.deleted_at IS NULL))))",
    )
    .bind(row.0)
    .bind(user)
    .bind(writing)
    .fetch_one(&mut **tx)
    .await
    .map_err(database_error)?;
    if !allowed {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "channel not found"));
    }
    if let (true, None, Some(user)) = (writing, space, user) {
        crate::direct::ensure_not_blocked(tx, row.0, user).await?;
    }
    Ok(row)
}

async fn persist_edit(
    pool: &PgPool,
    channel: &str,
    message: &str,
    token: &str,
    text: &str,
    expected_revision: i32,
) -> Result<Value, ApiError> {
    let content = prepare_text(text)?;
    if expected_revision < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid message revision",
        ));
    }
    let mut tx = pool.begin().await.map_err(database_error)?;
    let (_, _, name, user, avatar) = authorize_sender(&mut tx, token).await?;
    // The editor is the author, so their session already carries the current name.
    let current_name = user.map(|_| name.as_str());
    let (channel_id, head) = locked_channel(&mut tx, channel, user, true).await?;
    let row: Option<(i64, Value, Option<i64>)> = sqlx::query_as(
        "SELECT m.id,m.payload,cs.user_id FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id WHERE m.channel_id=$1 AND m.external_id=$2 AND m.forward_source_id IS NULL FOR UPDATE OF m",
    )
    .bind(channel_id)
    .bind(message)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?;
    let (message_id, mut payload, author) =
        row.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "message not found"))?;
    if user.is_none() || user != author {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "only the author can edit this message",
        ));
    }
    let revision = payload["revision"].as_i64().unwrap_or(1) as i32;
    // A lost HTTP response may be retried. Accept exactly that result without
    // appending another version/event; never silently overwrite a newer edit.
    if revision != expected_revision {
        return if i64::from(revision) == i64::from(expected_revision) + 1
            && payload["content"]["text"] == content["text"]
        {
            Ok(enrich_author(payload, avatar, current_name))
        } else {
            Err(ApiError::new(
                StatusCode::CONFLICT,
                "message changed; reload it before editing again",
            ))
        };
    }
    if payload["content"]["text"] == content["text"] {
        return Ok(enrich_author(payload, avatar, current_name));
    }
    let recent: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.message_versions v JOIN public.messages m ON m.id=v.message_id JOIN public.chat_sessions cs ON cs.id=m.session_id WHERE m.channel_id=$1 AND cs.user_id=$2 AND v.revision>1 AND v.created_at>now()-interval '1 minute'",
    )
    .bind(channel_id)
    .bind(user)
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?;
    if recent >= 30 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "editing too quickly; try again shortly",
        ));
    }
    // Lazy baseline preserves pre-feature messages without rewriting the entire
    // messages table. Original send idempotency/hash and event payload stay intact.
    sqlx::query("INSERT INTO public.message_versions(message_id,revision,content,created_at) VALUES($1,1,$2,$3::text::timestamptz) ON CONFLICT DO NOTHING")
        .bind(message_id).bind(&payload["content"]).bind(payload["createdAt"].as_str())
        .execute(&mut *tx).await.map_err(database_error)?;
    // Edited text is re-resolved like a new message, so added or removed
    // `@mentions` follow the latest revision.
    let in_space: bool =
        sqlx::query_scalar("SELECT space_id IS NOT NULL FROM public.channels WHERE id=$1")
            .bind(channel_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(database_error)?;
    let content = with_mentions(&mut tx, content, &mentions::parse(text, in_space)).await?;
    let now = Utc::now();
    payload["content"] = content;
    payload["revision"] = json!(revision + 1);
    payload["editedAt"] = json!(now.to_rfc3339());
    payload["editSeq"] = json!((head + 1).to_string());
    sqlx::query("INSERT INTO public.message_versions(message_id,revision,content,created_at) VALUES($1,$2,$3,$4)")
        .bind(message_id).bind(revision + 1).bind(&payload["content"]).bind(now)
        .execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("UPDATE public.messages SET payload=$2 WHERE id=$1")
        .bind(message_id)
        .bind(&payload)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    sqlx::query("INSERT INTO public.channel_events(channel_id,seq,payload) VALUES($1,$2,$3)")
        .bind(channel_id).bind(head + 1)
        .bind(json!({"type":"message.edited","schemaVersion":1,"channelId":channel,"seq":(head + 1).to_string(),"message":payload}))
        .execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("UPDATE public.channels SET last_seq=$2 WHERE id=$1")
        .bind(channel_id)
        .bind(head + 1)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok(enrich_author(payload, avatar, current_name))
}

#[derive(Deserialize)]
pub(super) struct VersionsQuery {
    before: Option<i32>,
}

pub(super) async fn versions(
    State(state): State<AppState>,
    Path((channel, message)): Path<(String, String)>,
    Query(query): Query<VersionsQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let user = request_user(&chat.pool, &headers).await?;
    Ok(Json(
        versions_page(&chat.pool, &channel, &message, query.before, user).await?,
    ))
}

pub(super) async fn message(
    State(state): State<AppState>,
    Path((channel, message)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let user = request_user(&chat.pool, &headers).await?;
    let mut tx = chat.pool.begin().await.map_err(database_error)?;
    let (channel_id, _) = locked_channel(&mut tx, &channel, user, false).await?;
    let (payload, avatar, name): (Value, Option<i16>, Option<String>) = sqlx::query_as(
        "SELECT m.payload,u.avatar_id,u.display_name FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL WHERE m.channel_id=$1 AND m.external_id=$2",
    ).bind(channel_id).bind(message).fetch_optional(&mut *tx).await.map_err(database_error)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "message not found"))?;
    // Like history, never serve a forward's stored snapshot: its source may
    // since have been deleted.
    let mut payload = [enrich_author(payload, avatar, name.as_deref())];
    forwarding::hydrate(&mut tx, &mut payload).await?;
    let [payload] = payload;
    Ok(Json(payload))
}

async fn versions_page(
    pool: &PgPool,
    channel: &str,
    message: &str,
    before: Option<i32>,
    user: Option<i64>,
) -> Result<Value, ApiError> {
    if before.is_some_and(|v| v < 1) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid message revision",
        ));
    }
    let mut tx = pool.begin().await.map_err(database_error)?;
    let (channel_id, _) = locked_channel(&mut tx, channel, user, false).await?;
    let (id, payload): (i64, Value) = sqlx::query_as(
        "SELECT id,payload FROM public.messages WHERE channel_id=$1 AND external_id=$2",
    )
    .bind(channel_id)
    .bind(message)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "message not found"))?;
    let rows: Vec<(i32, Value, chrono::DateTime<Utc>)> = sqlx::query_as(
        "SELECT revision,content,created_at FROM public.message_versions WHERE message_id=$1 AND ($2::integer IS NULL OR revision<$2) ORDER BY revision DESC LIMIT $3",
    ).bind(id).bind(before).bind(PAGE + 1).fetch_all(&mut *tx).await.map_err(database_error)?;
    let more = rows.len() > PAGE as usize;
    let mut versions: Vec<Value> = rows.into_iter().take(PAGE as usize)
        .map(|(revision, content, at)| json!({"revision":revision,"content":content,"createdAt":at.to_rfc3339()})).collect();
    if payload["revision"].as_i64().unwrap_or(1) == 1 && before.is_none_or(|v| v > 1) {
        versions.push(
            json!({"revision":1,"content":payload["content"],"createdAt":payload["createdAt"]}),
        );
    }
    Ok(json!({"messageId":message,"versions":versions,"hasMore":more}))
}

#[cfg(test)]
mod tests;
