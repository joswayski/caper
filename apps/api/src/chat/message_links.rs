//! Message links (`https://caper.chat/m/<channel>/<message>`): where a linked
//! message is and what it says, for readers who can see its conversation.
//! Everyone else, signed out or not, gets the same 404 as for a message that
//! never existed, so a link reveals nothing about the original.
use super::*;

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/api/chat/message-links/{channel}/{message}", get(resolve))
}

fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "message not found")
}

/// `{spaceId, message}`: the message as history shows it (a deleted one keeps
/// its `deletedAt` tombstone), and its space (null for a DM).
async fn resolve(
    State(state): State<AppState>,
    Path((channel, message)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let user = request_user(&chat.pool, &headers).await?;
    let mut tx = chat.pool.begin().await.map_err(database_error)?;
    // The same read access as history, under the same locks.
    let (channel_id, _, _) = editing::locked_channel(&mut tx, &channel, user, false)
        .await
        .map_err(|error| match error.status {
            StatusCode::NOT_FOUND | StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED => not_found(),
            _ => error,
        })?;
    let (payload, avatar, name, space): (Value, Option<i16>, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT m.payload,u.avatar_id,u.display_name,s.external_id
             FROM public.messages m JOIN public.channels c ON c.id=m.channel_id
             LEFT JOIN public.spaces s ON s.id=c.space_id
             JOIN public.chat_sessions cs ON cs.id=m.session_id
             LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL
             WHERE m.channel_id=$1 AND m.external_id=$2",
        )
        .bind(channel_id)
        .bind(&message)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?
        .ok_or_else(not_found)?;
    // Like history, a forward's original is read fresh, never from its snapshot.
    let mut messages = [enrich_author(payload, avatar, name.as_deref())];
    forwarding::hydrate(&mut tx, &mut messages).await?;
    drop(tx);
    deliver(&chat.pool, chat.cdn.as_deref(), &mut messages).await?;
    let [message] = messages;
    Ok(Json(json!({"spaceId": space, "message": message})))
}
