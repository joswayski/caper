//! Live, read-only sharing of a message and its replies. Destination access is
//! sufficient to read this conversation, never the source channel's history.
use super::*;
use crate::spaces::{channel_access, channel_participation};
use sqlx::{Postgres, Transaction};

#[cfg(test)]
mod tests;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/chat/forward-destinations", get(destinations))
        .route("/api/chat/channels/{channel}/forwards", post(send_forward))
        .route(
            "/api/chat/channels/{channel}/forwards/{message}/thread",
            get(thread),
        )
}

fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "message not found")
}

async fn destinations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let user = request_user(&chat.pool, &headers)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "sign in required"))?;
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id',c.external_id,'name',c.name,'spaceId',s.external_id,'spaceName',s.name,'direct',false)
         FROM public.channels c JOIN public.spaces s ON s.id=c.space_id
         WHERE c.deleted_at IS NULL AND s.deleted_at IS NULL AND NOT s.demo
           AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$1 AND sm.deleted_at IS NULL)
           AND EXISTS(SELECT 1 FROM public.channel_joins cj WHERE cj.channel_id=c.id AND cj.user_id=$1 AND cj.deleted_at IS NULL)
           AND (NOT c.private OR s.owner_id=$1 OR EXISTS(SELECT 1 FROM public.channel_members cm WHERE cm.channel_id=c.id AND cm.user_id=$1 AND cm.deleted_at IS NULL))
         UNION ALL
         SELECT jsonb_build_object('id',c.external_id,'name',u.display_name,'spaceName','Direct messages','direct',true)
         FROM public.direct_conversations d JOIN public.channels c ON c.id=d.channel_id
         JOIN public.users u ON u.id=CASE WHEN d.low_user_id=$1 THEN d.high_user_id ELSE d.low_user_id END
         WHERE $1 IN(d.low_user_id,d.high_user_id) AND c.deleted_at IS NULL AND u.deleted_at IS NULL
           AND (d.accepted_at IS NOT NULL OR d.requested_by=$1 OR d.declined_at IS NULL)",
    ).bind(user).fetch_all(&chat.pool).await.map_err(database_error)?;
    Ok(Json(json!({"destinations":rows})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ForwardInput {
    source_channel_id: String,
    source_message_id: String,
    client_message_id: Uuid,
    #[serde(default)]
    text: String,
}

async fn send_forward(
    State(state): State<AppState>,
    Path(channel): Path<String>,
    headers: HeaderMap,
    Json(input): Json<ForwardInput>,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let payload = persist_forward(&chat.pool, &channel, sender_token(&headers)?, input).await?;
    chat.wake.notify_one();
    Ok(Json(payload))
}

// Match membership mutations' space -> channel order. Opposite-direction
// forwards lock the spaces and channels canonically rather than source first.
async fn lock_spaces(tx: &mut Transaction<'_, Postgres>, channels: &[i64]) -> Result<(), ApiError> {
    sqlx::query("SELECT id FROM public.spaces WHERE id IN(SELECT space_id FROM public.channels WHERE id=ANY($1)) ORDER BY id FOR SHARE")
        .bind(channels).fetch_all(&mut **tx).await.map_err(database_error)?;
    Ok(())
}

async fn persist_forward(
    pool: &PgPool,
    channel: &str,
    token: &str,
    input: ForwardInput,
) -> Result<Value, ApiError> {
    let content = if input.text.is_empty() {
        json!({"version":1,"type":"text","text":""})
    } else {
        prepare_text(&input.text)?
    };
    let mut tx = pool.begin().await.map_err(database_error)?;
    let (session, author_id, name, user, avatar_id) = authorize_sender(&mut tx, token).await?;
    // Include the canonical original in the lock set when re-forwarding. This
    // closes the create/update race even if the sender cannot browse its space.
    let source: (i64, i64) = sqlx::query_as("SELECT original.id,original.channel_id FROM public.messages selected JOIN public.channels c ON c.id=selected.channel_id JOIN public.messages original ON original.id=COALESCE(selected.forward_source_id,selected.id) WHERE c.external_id=$1 AND selected.external_id=$2")
        .bind(&input.source_channel_id).bind(&input.source_message_id).fetch_optional(&mut *tx).await.map_err(database_error)?.ok_or_else(not_found)?;
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM public.channels WHERE external_id=ANY($1) OR id=$2 ORDER BY id",
    )
    .bind([channel, input.source_channel_id.as_str()])
    .bind(source.1)
    .fetch_all(&mut *tx)
    .await
    .map_err(database_error)?;
    lock_spaces(&mut tx, &ids).await?;
    sqlx::query("SELECT id FROM public.channels WHERE id=ANY($1) ORDER BY id FOR UPDATE")
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?;
    // Use the locked connection: borrowing another pool connection here can
    // exhaust the pool when concurrent forwards each hold a transaction.
    channel_access(&mut *tx, &input.source_channel_id, user).await?;
    let destination = channel_participation(&mut *tx, channel, user).await?;
    // Forwards are sends: blocks and message requests gate them the same way.
    if let (None, Some(user)) = (destination.space_id, user) {
        crate::direct::authorize_send(&mut tx, destination.id, user).await?;
    }
    // Reading a forward is also permission to forward its original again.
    // No client-provided source payload or source-access bypass is accepted.
    let source = source.0;
    let forward = source_snapshot(&mut tx, source)
        .await?
        .ok_or_else(not_found)?;
    let hash = Sha256::digest(
        serde_json::to_vec(&json!({"source":source,"text":input.text}))
            .map_err(|_| unavailable())?,
    )
    .to_vec();
    let existing: Option<(i64, Vec<u8>, Value)> = sqlx::query_as("SELECT session_id,request_hash,payload FROM public.messages WHERE channel_id=$1 AND client_message_id=$2")
        .bind(destination.id).bind(input.client_message_id).fetch_optional(&mut *tx).await.map_err(database_error)?;
    if let Some((sender, original, mut payload)) = existing {
        if sender != session || original != hash {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "retry key already used for a different forward",
            ));
        }
        payload["forward"] = forward;
        return Ok(enrich_author(
            payload,
            avatar_id,
            user.map(|_| name.as_str()),
        ));
    }
    let (global, personal): (i64, i64) = sqlx::query_as("SELECT count(*),count(*) FILTER(WHERE session_id=$2) FROM public.messages WHERE channel_id=$1 AND created_at>now()-interval '1 minute'")
        .bind(destination.id).bind(session).fetch_one(&mut *tx).await.map_err(database_error)?;
    if global >= 120 || personal >= 30 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "sending too quickly; try again shortly",
        ));
    }
    let head: i64 = sqlx::query_scalar("SELECT last_seq FROM public.channels WHERE id=$1")
        .bind(destination.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
    let seq = head + 1;
    let payload = json!({"id":random_id(15),"channelId":channel,"seq":seq.to_string(),"author":{"id":author_id,"name":name,"isGuest":false,"avatarId":avatar_id},"content":content,"createdAt":Utc::now().to_rfc3339(),"clientMessageId":input.client_message_id,"forward":forward,"forwardSeq":seq.to_string()});
    sqlx::query("INSERT INTO public.messages(external_id,channel_id,session_id,client_message_id,request_hash,channel_seq,payload,forward_source_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(payload["id"].as_str()).bind(destination.id).bind(session).bind(input.client_message_id).bind(hash).bind(seq).bind(&payload).bind(source).execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("INSERT INTO public.channel_events(channel_id,seq,payload) VALUES($1,$2,$3)")
        .bind(destination.id).bind(seq).bind(json!({"type":"message.created","schemaVersion":1,"channelId":channel,"seq":seq.to_string(),"message":payload})).execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("UPDATE public.channels SET last_seq=$2 WHERE id=$1")
        .bind(destination.id)
        .bind(seq)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    if destination.space_id.is_none() {
        sqlx::query("INSERT INTO public.direct_reads(channel_id,user_id,seq) VALUES($1,$2,$3) ON CONFLICT(channel_id,user_id) DO UPDATE SET seq=GREATEST(direct_reads.seq,EXCLUDED.seq)")
            .bind(destination.id).bind(user).bind(seq).execute(&mut *tx).await.map_err(database_error)?;
    }
    tx.commit().await.map_err(database_error)?;
    Ok(payload)
}

// The projection shares content and conversation participants, but never pins,
// parent/sibling access, or another forwarded wrapper.
fn shared_message(mut message: Value) -> Value {
    if let Some(object) = message.as_object_mut() {
        if object.remove("threadRootId").is_some() {
            object.remove("thread");
        }
        for key in ["broadcast", "pin", "pinSeq", "forward", "forwardSeq"] {
            object.remove(key);
        }
    }
    message
}

async fn source_snapshot(
    tx: &mut Transaction<'_, Postgres>,
    source: i64,
) -> Result<Option<Value>, ApiError> {
    let row: Option<(Value, i64, Option<i16>, Option<String>)> = sqlx::query_as("SELECT m.payload,c.last_seq,u.avatar_id,u.display_name FROM public.messages m JOIN public.channels c ON c.id=m.channel_id LEFT JOIN public.spaces s ON s.id=c.space_id JOIN public.chat_sessions cs ON cs.id=m.session_id LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL WHERE m.id=$1 AND c.deleted_at IS NULL AND (c.space_id IS NULL OR s.deleted_at IS NULL)")
        .bind(source).fetch_optional(&mut **tx).await.map_err(database_error)?;
    Ok(row.map(|(message, head, avatar, name)| json!({"message":shared_message(enrich_author(message,avatar,name.as_deref())),"seq":head.to_string()})))
}

pub(super) async fn hydrate(
    tx: &mut Transaction<'_, Postgres>,
    messages: &mut [Value],
) -> Result<(), ApiError> {
    let ids: Vec<&str> = messages.iter().filter_map(|m| m["id"].as_str()).collect();
    let rows: Vec<(String, i64)> = sqlx::query_as("SELECT external_id,forward_source_id FROM public.messages WHERE external_id=ANY($1) AND forward_source_id IS NOT NULL")
        .bind(ids).fetch_all(&mut **tx).await.map_err(database_error)?;
    for (id, source) in rows {
        let snapshot = source_snapshot(tx, source)
            .await?
            .unwrap_or_else(|| json!({"message":null,"seq":"0"}));
        if let Some(message) = messages.iter_mut().find(|m| m["id"].as_str() == Some(&id)) {
            message["forward"] = snapshot;
        }
    }
    Ok(())
}

async fn thread(
    State(state): State<AppState>,
    Path((channel, message)): Path<(String, String)>,
    Query(query): Query<HistoryQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let user = request_user(&chat.pool, &headers).await?;
    let before = query.before.as_deref().map(cursor).transpose()?;
    let mut tx = chat.pool.begin().await.map_err(database_error)?;
    let id: i64 = sqlx::query_scalar("SELECT id FROM public.channels WHERE external_id=$1")
        .bind(&channel)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?
        .ok_or_else(not_found)?;
    lock_spaces(&mut tx, &[id]).await?;
    sqlx::query("SELECT id FROM public.channels WHERE id=$1 FOR SHARE")
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
    channel_access(&mut *tx, &channel, user).await?;
    let source: i64 = sqlx::query_scalar("SELECT forward_source_id FROM public.messages WHERE channel_id=$1 AND external_id=$2 AND forward_source_id IS NOT NULL")
        .bind(id).bind(&message).fetch_optional(&mut *tx).await.map_err(database_error)?.ok_or_else(not_found)?;
    // One repeatable snapshot for the root, replies, and their source cursor.
    // The destination grant authorizes only direct children of this original.
    let rows: Vec<(Value, Option<i16>, i64, Option<String>)> = sqlx::query_as("SELECT m.payload,u.avatar_id,c.last_seq,u.display_name FROM public.messages root JOIN public.channels c ON c.id=root.channel_id LEFT JOIN public.spaces s ON s.id=c.space_id JOIN public.messages m ON m.channel_id=root.channel_id AND (m.id=root.id OR m.thread_root_id=root.id) JOIN public.chat_sessions cs ON cs.id=m.session_id LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL WHERE root.id=$1 AND c.deleted_at IS NULL AND (c.space_id IS NULL OR s.deleted_at IS NULL) AND (m.id=root.id OR $2::bigint IS NULL OR m.channel_seq<$2) ORDER BY (m.id=root.id) DESC,m.channel_seq DESC LIMIT $3")
        .bind(source).bind(before).bind(PAGE+2).fetch_all(&mut *tx).await.map_err(database_error)?;
    let mut root = Value::Null;
    let mut replies = Vec::new();
    let mut head = "0".to_owned();
    for (payload, avatar, cursor, name) in rows {
        head = cursor.to_string();
        let original = shared_message(enrich_author(payload, avatar, name.as_deref()));
        // The first row is always the selected original, not a guessed root.
        if root.is_null() {
            root = original;
        } else {
            replies.push(original);
        }
    }
    let more = replies.len() > PAGE as usize;
    replies.truncate(PAGE as usize);
    replies.reverse();
    hydrate(&mut tx, &mut replies).await?;
    Ok(Json(
        json!({"root":root,"messages":replies,"cursor":head,"hasMore":more}),
    ))
}

/// Project source edits, reactions and replies into destination replay logs.
/// This runs inside the publisher's transaction, so a crash cannot permanently
/// lose an update. Forward events never recurse: every reference is flattened.
pub(super) async fn project_events(
    tx: &mut Transaction<'_, Postgres>,
    events: &[super::PendingEvent],
) -> Result<(), ApiError> {
    let mut targets = Vec::new();
    for (_, _, event, _, _) in events {
        match event["type"].as_str() {
            Some("message.created" | "message.edited") => {
                if let Some(id) = event["message"]["id"].as_str() {
                    targets.push(id.to_owned());
                }
                if let Some(id) = event["message"]["threadRootId"].as_str() {
                    targets.push(id.to_owned());
                }
            }
            Some("message.reactions") => {
                if let Some(id) = event["messageId"].as_str() {
                    targets.push(id.to_owned());
                }
            }
            _ => {}
        }
    }
    if targets.is_empty() {
        return Ok(());
    }
    // Reactions/edits on an original reply also invalidate its parent's view.
    let parents: Vec<String> = sqlx::query_scalar("SELECT root.external_id FROM public.messages m JOIN public.messages root ON root.id=m.thread_root_id WHERE m.external_id=ANY($1)")
        .bind(&targets).fetch_all(&mut **tx).await.map_err(database_error)?;
    targets.extend(parents);
    let destinations: Vec<i64> = sqlx::query_scalar("SELECT DISTINCT f.channel_id FROM public.messages f JOIN public.messages source ON source.id=f.forward_source_id JOIN public.channels c ON c.id=f.channel_id WHERE source.external_id=ANY($1) AND c.deleted_at IS NULL ORDER BY f.channel_id")
        .bind(&targets).fetch_all(&mut **tx).await.map_err(database_error)?;
    lock_spaces(tx, &destinations).await?;
    sqlx::query("SELECT id FROM public.channels WHERE id=ANY($1) ORDER BY id FOR UPDATE")
        .bind(&destinations)
        .fetch_all(&mut **tx)
        .await
        .map_err(database_error)?;
    let forwards: Vec<(i64, i64, i64, String, Value)> = sqlx::query_as("SELECT f.id,f.channel_id,f.forward_source_id,c.external_id,f.payload FROM public.messages f JOIN public.messages source ON source.id=f.forward_source_id JOIN public.channels c ON c.id=f.channel_id LEFT JOIN public.spaces s ON s.id=c.space_id WHERE source.external_id=ANY($1) AND f.channel_id=ANY($2) AND c.deleted_at IS NULL AND (c.space_id IS NULL OR s.deleted_at IS NULL) ORDER BY f.channel_id,f.id")
        .bind(&targets).bind(&destinations).fetch_all(&mut **tx).await.map_err(database_error)?;
    for (id, channel, source, external, mut payload) in forwards {
        let snapshot = source_snapshot(tx, source)
            .await?
            .unwrap_or_else(|| json!({"message":null,"seq":"0"}));
        if payload["forward"] == snapshot {
            continue;
        }
        let seq: i64 = sqlx::query_scalar(
            "UPDATE public.channels SET last_seq=last_seq+1 WHERE id=$1 RETURNING last_seq",
        )
        .bind(channel)
        .fetch_one(&mut **tx)
        .await
        .map_err(database_error)?;
        payload["forward"] = snapshot;
        payload["forwardSeq"] = json!(seq.to_string());
        sqlx::query("UPDATE public.messages SET payload=$2 WHERE id=$1")
            .bind(id)
            .bind(&payload)
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
        sqlx::query("INSERT INTO public.channel_events(channel_id,seq,payload) VALUES($1,$2,$3)")
            .bind(channel).bind(seq).bind(json!({"type":"message.forward","schemaVersion":1,"channelId":external,"seq":seq.to_string(),"message":payload})).execute(&mut **tx).await.map_err(database_error)?;
    }
    Ok(())
}
