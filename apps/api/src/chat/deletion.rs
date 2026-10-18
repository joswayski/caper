//! Deleting a sent message. The row, its retained versions, reactions and
//! files are kept; readers get a tombstone in its place, so threads, replies
//! and delivery order stay intact around it.
use super::*;
use editing::locked_channel;

/// What every reader sees of a deleted message's content.
fn tombstone_content() -> Value {
    json!({"version":1,"type":"text","text":""})
}

pub(super) async fn delete(
    State(state): State<AppState>,
    Path((channel, message)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let result = persist_delete(&chat.pool, &channel, &message, sender_token(&headers)?).await?;
    chat.wake.notify_one();
    Ok(Json(result))
}

/// Deletes the author's own message. Repeating it returns the same tombstone,
/// so a retry after a lost response never fails.
pub(super) async fn persist_delete(
    pool: &PgPool,
    channel: &str,
    message: &str,
    token: &str,
) -> Result<Value, ApiError> {
    let mut tx = pool.begin().await.map_err(database_error)?;
    let (_, _, name, user, avatar, _) = authorize_sender(&mut tx, token).await?;
    let current_name = user.map(|_| name.as_str());
    // Unlike edits, a block never stops people removing their own messages.
    let (channel_id, head, _) = locked_channel(&mut tx, channel, user, true).await?;
    let row: Option<(i64, Value, Option<i64>, bool, i64)> = sqlx::query_as(
        "SELECT m.id,m.payload,cs.user_id,m.deleted_at IS NOT NULL,m.channel_seq FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id WHERE m.channel_id=$1 AND m.external_id=$2 FOR UPDATE OF m",
    )
    .bind(channel_id)
    .bind(message)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?;
    let (message_id, mut payload, author, deleted, message_seq) =
        row.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "message not found"))?;
    let Some(owner) = user.filter(|user| Some(*user) == author) else {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "only the author can delete this message",
        ));
    };
    if deleted {
        return Ok(enrich_author(payload, avatar, current_name));
    }
    let recent: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id WHERE m.channel_id=$1 AND cs.user_id=$2 AND m.deleted_at>now()-interval '1 minute'",
    )
    .bind(channel_id)
    .bind(owner)
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?;
    if recent >= 60 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "deleting too quickly; try again shortly",
        ));
    }
    // The content is retained as a version, exactly as the first edit keeps
    // the original. Edited messages already have their latest one.
    sqlx::query("INSERT INTO public.message_versions(message_id,revision,content,created_at) VALUES($1,1,$2,$3::text::timestamptz) ON CONFLICT DO NOTHING")
        .bind(message_id).bind(&payload["content"]).bind(payload["createdAt"].as_str())
        .execute(&mut *tx).await.map_err(database_error)?;
    let now = Utc::now();
    let seq = head + 1;
    let revision = payload["revision"].as_i64().unwrap_or(1);
    let pinned = payload.get("pin").is_some_and(Value::is_object);
    // Delivered as an edit so every client, including released ones that
    // predate deletion, replaces the content and drops the files.
    payload["content"] = tombstone_content();
    payload["revision"] = json!(revision + 1);
    payload["editedAt"] = json!(now.to_rfc3339());
    payload["editSeq"] = json!(seq.to_string());
    payload["attachmentsSeq"] = json!(seq.to_string());
    payload["deletedAt"] = json!(now.to_rfc3339());
    sqlx::query("UPDATE public.messages SET payload=$2,deleted_at=$3 WHERE id=$1")
        .bind(message_id)
        .bind(&payload)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    sqlx::query(
        "UPDATE public.assets SET retired_at=now() WHERE message_id=$1 AND retired_at IS NULL",
    )
    .bind(message_id)
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    // Replays and pending outbox rows never hand the deleted content out
    // again. Events about a message are only ever committed after it.
    sqlx::query("UPDATE public.channel_events SET payload=jsonb_set(payload,'{message,content}',$3) WHERE channel_id=$1 AND seq>=$4 AND payload->'message'->>'id'=$2 AND jsonb_typeof(payload->'message'->'content')='object'")
        .bind(channel_id).bind(message).bind(tombstone_content()).bind(message_seq)
        .execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("UPDATE public.channel_events e SET payload=jsonb_set(e.payload,'{message,forward}',jsonb_build_object('message',NULL,'seq','0'))
                 FROM public.messages f WHERE f.forward_source_id=$1 AND e.channel_id=f.channel_id AND e.seq>=f.channel_seq
                   AND e.payload->'message'->>'id'=f.external_id AND jsonb_typeof(e.payload->'message'->'forward')='object'")
        .bind(message_id).execute(&mut *tx).await.map_err(database_error)?;
    // Replies keep their quote's author and link, never the deleted text.
    sqlx::query("UPDATE public.messages SET payload=jsonb_set(payload,'{replyTo}',((payload->'replyTo')-'fullText')||'{\"text\":\"\",\"hasAttachments\":false,\"deleted\":true}') WHERE channel_id=$1 AND channel_seq>$3 AND payload->'replyTo'->>'id'=$2")
        .bind(channel_id).bind(message).bind(message_seq)
        .execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("UPDATE public.channel_events SET payload=jsonb_set(payload,'{message,replyTo}',((payload->'message'->'replyTo')-'fullText')||'{\"text\":\"\",\"hasAttachments\":false,\"deleted\":true}') WHERE channel_id=$1 AND seq>$3 AND payload->'message'->'replyTo'->>'id'=$2")
        .bind(channel_id).bind(message).bind(message_seq)
        .execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("INSERT INTO public.channel_events(channel_id,seq,payload) VALUES($1,$2,$3)")
        .bind(channel_id).bind(seq)
        .bind(json!({"type":"message.edited","schemaVersion":1,"channelId":channel,"seq":seq.to_string(),"message":payload}))
        .execute(&mut *tx).await.map_err(database_error)?;
    let mut last = seq;
    if pinned {
        // Unpinned in its own event, so pin lists drop it like any unpin.
        last += 1;
        payload["pin"] = Value::Null;
        payload["pinSeq"] = json!(last.to_string());
        sqlx::query("UPDATE public.messages SET payload=$2 WHERE id=$1")
            .bind(message_id)
            .bind(&payload)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query("INSERT INTO public.channel_events(channel_id,seq,payload) VALUES($1,$2,$3)")
            .bind(channel_id).bind(last)
            .bind(json!({"type":"message.pin","schemaVersion":1,"channelId":channel,"seq":last.to_string(),"message":enrich_author(payload.clone(),avatar,current_name)}))
            .execute(&mut *tx).await.map_err(database_error)?;
    }
    sqlx::query("UPDATE public.channels SET last_seq=$2 WHERE id=$1")
        .bind(channel_id)
        .bind(last)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok(enrich_author(payload, avatar, current_name))
}

#[cfg(test)]
mod tests;
