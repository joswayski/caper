//! In-place content edits and retained versions. Delivery order, thread summaries,
//! reactions and pins remain independent of a message's content revision.
use super::*;
use attachments::{AttachmentChange, Change, CurrentFile, Plan};
use sqlx::{Postgres, Transaction};

mod attachments;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct EditInput {
    text: String,
    expected_revision: i32,
    /// The message's files after the edit, in their current order. Left out,
    /// the files stay exactly as they are.
    attachments: Option<Vec<AttachmentChange>>,
}

pub(super) async fn edit(
    State(state): State<AppState>,
    Path((channel, message)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<EditInput>,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let result = persist_edit_with(
        &chat.pool,
        &channel,
        &message,
        sender_token(&headers)?,
        &input.text,
        input.expected_revision,
        input.attachments.as_deref(),
    )
    .await?;
    chat.wake.notify_one();
    Ok(Json(assets::sign_attachments(
        crate::chat::with_pictures(&chat.pool, result).await,
        chat.cdn.as_deref(),
    )))
}

/// Membership changes lock space before channel. Check access in a fresh
/// statement after waiting, just like sends/history, so revocation wins.
pub(super) async fn locked_channel(
    tx: &mut Transaction<'_, Postgres>,
    channel: &str,
    user: Option<i64>,
    writing: bool,
) -> Result<(i64, i64, bool), ApiError> {
    let space: Option<i64> =
        sqlx::query_scalar("SELECT space_id FROM public.channels WHERE external_id=$1")
            .bind(channel)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    // Shared for writers too: the channel lock below orders this channel, and
    // membership changes still wait for (and block) every holder.
    if let Some(space) = space {
        sqlx::query("SELECT id FROM public.spaces WHERE id=$1 FOR SHARE")
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
                    WHERE d.channel_id=c.id AND $2 IN (d.low_user_id,d.high_user_id) AND lo.deleted_at IS NULL AND hi.deleted_at IS NULL))
                OR ((s.id IS NULL OR (s.deleted_at IS NULL AND NOT s.demo)) AND public.feedback_access(c.id,$2))))",
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
    // A channel never moves between a space and DMs, so the unlocked read holds.
    Ok((row.0, row.1, space.is_none()))
}

/// A text-only edit, as clients sent before files could be edited.
#[cfg(test)]
async fn persist_edit(
    pool: &PgPool,
    channel: &str,
    message: &str,
    token: &str,
    text: &str,
    expected_revision: i32,
) -> Result<Value, ApiError> {
    persist_edit_with(pool, channel, message, token, text, expected_revision, None).await
}

/// One revision of a message's text and, when `files` is given, its files.
async fn persist_edit_with(
    pool: &PgPool,
    channel: &str,
    message: &str,
    token: &str,
    text: &str,
    expected_revision: i32,
    files: Option<&[AttachmentChange]>,
) -> Result<Value, ApiError> {
    // Text may be empty only for messages with files; checked once loaded.
    prepare_content(text, true)?;
    if expected_revision < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid message revision",
        ));
    }
    if let Some(files) = files {
        attachments::check_request(files)?;
    }
    let mut tx = pool.begin().await.map_err(database_error)?;
    let (_, _, name, user, avatar, _) = authorize_sender(&mut tx, token).await?;
    // The editor is the author, so their session already carries the current name.
    let current_name = user.map(|_| name.as_str());
    let (channel_id, head, direct) = locked_channel(&mut tx, channel, user, true).await?;
    if let (true, Some(user)) = (direct, user) {
        crate::direct::authorize_interaction(&mut tx, channel_id, user).await?;
    }
    let row: Option<(i64, Value, Option<i64>)> = sqlx::query_as(
        "SELECT m.id,m.payload,cs.user_id FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id WHERE m.channel_id=$1 AND m.external_id=$2 AND m.forward_source_id IS NULL AND m.deleted_at IS NULL FOR UPDATE OF m",
    )
    .bind(channel_id)
    .bind(message)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?;
    let (message_id, mut payload, author) =
        row.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "message not found"))?;
    // A text-only edit keeps the message's files as they are. Files can be
    // replaced or spoilered, never removed, so whether there are any holds.
    // A GIF stays too: editing changes only its caption.
    let shown = payload["content"].get("attachments").cloned();
    let gif = payload["content"].get("gif").cloned();
    let mut content = prepare_content(text, shown.is_some() || gif.is_some())?;
    if let Some(attachments) = &shown {
        content["attachments"] = attachments.clone();
    }
    if let Some(gif) = gif {
        content["gif"] = gif;
    }
    let Some(owner) = user.filter(|user| Some(*user) == author) else {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "only the author can edit this message",
        ));
    };
    let revision = payload["revision"].as_i64().unwrap_or(1) as i32;
    // A lost HTTP response may be retried. Accept exactly that result without
    // appending another version/event; never silently overwrite a newer edit.
    // This runs before any upload is linked: a retry's uploads already are.
    if revision != expected_revision {
        let same_files = files.is_none_or(|changes| {
            self::files(&payload["content"]) == attachments::requested(changes)
        });
        return if i64::from(revision) == i64::from(expected_revision) + 1
            && payload["content"]["text"] == content["text"]
            && same_files
        {
            Ok(enrich_author(payload, avatar, current_name))
        } else {
            Err(ApiError::new(
                StatusCode::CONFLICT,
                "message changed; reload it before editing again",
            ))
        };
    }
    // Checked against the files the editor saw at this revision. `None` when
    // the files stay as they are, including a list that changes nothing.
    let plan = match files {
        Some(files) => {
            let current = current_files(&mut tx, message_id, shown.as_ref()).await?;
            Some(attachments::plan(&current, files)?).filter(|plan| plan.changes(&current))
        }
        None => None,
    };
    if payload["content"]["text"] == content["text"] && plan.is_none() {
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
    if let Some(plan) = &plan {
        content["attachments"] = Value::Array(
            edited_files(&mut tx, plan, shown.as_ref(), message_id, channel_id, owner).await?,
        );
    }
    // Edited text is re-resolved like a new message, so added or removed
    // `@mentions` follow the latest revision.
    let mut found = mentions::parse(text, !direct);
    if crate::feedback::reported(&mut tx, message_id).await? {
        // The team was notified when it was sent, so it stays marked that way
        // even when the edit drops the `@feedback` text, or never had it (each
        // top-level message in the Caper DM is reported by itself).
        if !found.contains(&Mention::Feedback) {
            found.push(Mention::Feedback);
        }
    } else {
        found.retain(|mention| *mention != Mention::Feedback);
    }
    let content = with_mentions(&mut tx, content, &found).await?;
    let now = Utc::now();
    payload["content"] = content;
    payload["revision"] = json!(revision + 1);
    payload["editedAt"] = json!(now.to_rfc3339());
    payload["editSeq"] = json!((head + 1).to_string());
    if plan.is_some() {
        // Released clients keep a message's files unless this revision is
        // newer than the files they have.
        payload["attachmentsSeq"] = payload["editSeq"].clone();
    }
    sqlx::query("INSERT INTO public.message_versions(message_id,revision,content,created_at) VALUES($1,$2,$3,$4)")
        .bind(message_id).bind(revision + 1).bind(&payload["content"]).bind(now)
        .execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("UPDATE public.messages SET payload=$2 WHERE id=$1")
        .bind(message_id)
        .bind(&payload)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    if let Some(plan) = &plan {
        record_files(&mut tx, plan, message_id).await?;
    }
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

/// The files a message shows, in order, with what their asset rows say.
async fn current_files(
    tx: &mut Transaction<'_, Postgres>,
    message_id: i64,
    shown: Option<&Value>,
) -> Result<Vec<CurrentFile>, ApiError> {
    let linked = assets::shown_files(tx, message_id).await?;
    Ok(attachments::shown(shown)
        .into_iter()
        .map(|(id, spoiler)| {
            let row = linked.iter().find(|file| file.id == id);
            CurrentFile {
                id: id.to_owned(),
                kind: row.map_or("file", |file| file.kind.as_str()).to_owned(),
                spoiler,
                source: row.and_then(|file| file.source.clone()),
                available: row.is_some_and(|file| file.available),
            }
        })
        .collect())
}

/// The message's files after the edit. New uploads are linked under the same
/// rules as a send's (the author's, this channel's, completed in the last 24
/// hours and not yet sent) and must be the kind of file they replace.
async fn edited_files(
    tx: &mut Transaction<'_, Postgres>,
    plan: &Plan,
    shown: Option<&Value>,
    message_id: i64,
    channel_id: i64,
    owner: i64,
) -> Result<Vec<Value>, ApiError> {
    let mut uploads = assets::attach(tx, &plan.uploads(), owner, channel_id)
        .await?
        .into_iter();
    let mut restores = assets::restorable(tx, &plan.restores(), message_id, owner)
        .await?
        .into_iter();
    let shown = shown
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    plan.files
        .iter()
        .zip(shown)
        .map(|(file, now)| {
            let mut described = match &file.change {
                Change::Keep => now.clone(),
                Change::Restore { .. } => restores.next().ok_or_else(unavailable)?,
                Change::Upload { kind, .. } => {
                    let upload = uploads.next().ok_or_else(unavailable)?;
                    if upload["kind"] != kind.as_str() {
                        return Err(attachments::Refusal::WrongKind.into());
                    }
                    upload
                }
            };
            // A missing spoiler means false, as when sending.
            if file.spoiler {
                described["spoiler"] = Value::Bool(true);
            } else if let Some(object) = described.as_object_mut() {
                object.remove("spoiler");
            }
            Ok(described)
        })
        .collect()
}

/// Retires replaced files and links replacements in their slots.
async fn record_files(
    tx: &mut Transaction<'_, Postgres>,
    plan: &Plan,
    message_id: i64,
) -> Result<(), ApiError> {
    let mut placements = Vec::new();
    for (position, file) in plan.files.iter().enumerate() {
        let position = i16::try_from(position).map_err(|_| unavailable())?;
        match &file.change {
            Change::Keep => {}
            Change::Restore { .. } => placements.push(assets::Placement::Restore {
                id: &file.id,
                position,
            }),
            Change::Upload { source, edit, .. } => placements.push(assets::Placement::Upload {
                id: &file.id,
                position,
                source,
                edit: serde_json::to_value(edit).map_err(|_| unavailable())?,
            }),
        }
    }
    assets::replace_files(tx, message_id, &plan.retired(), &placements).await
}

/// For the author's editor: each photo and video the message shows, the
/// first-sent file an edit starts from (with fresh URLs) and the edit the
/// shown file already has, so re-editing never compounds earlier edits.
pub(super) async fn attachment_sources(
    State(state): State<AppState>,
    Path((channel, message)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let user = request_user(&chat.pool, &headers).await?;
    Ok(Json(
        sources_page(&chat.pool, &channel, &message, user, chat.cdn.as_deref()).await?,
    ))
}

async fn sources_page(
    pool: &PgPool,
    channel: &str,
    message: &str,
    user: Option<i64>,
    cdn: Option<&CdnSigner>,
) -> Result<Value, ApiError> {
    let mut tx = pool.begin().await.map_err(database_error)?;
    let (channel_id, _, _) = locked_channel(&mut tx, channel, user, false).await?;
    let row: Option<(i64, Value, Option<i64>)> = sqlx::query_as(
        "SELECT m.id,m.payload,cs.user_id FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id WHERE m.channel_id=$1 AND m.external_id=$2 AND m.forward_source_id IS NULL AND m.deleted_at IS NULL",
    )
    .bind(channel_id)
    .bind(message)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?;
    let (id, payload, author) =
        row.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "message not found"))?;
    if user.is_none() || user != author {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "only the author can edit this message",
        ));
    }
    drop(tx);
    let shown: Vec<&str> = files(&payload["content"])
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let sources = assets::edit_sources(pool, id, &shown, cdn).await?;
    Ok(json!({
        "messageId": message,
        "revision": payload["revision"].as_i64().unwrap_or(1),
        "sources": sources,
    }))
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
        signed_versions_page(
            &chat.pool,
            &channel,
            &message,
            query.before,
            user,
            chat.cdn.as_deref(),
        )
        .await?,
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
    let (channel_id, _, _) = locked_channel(&mut tx, &channel, user, false).await?;
    let (payload, avatar, name): (Value, Option<i16>, Option<String>) = sqlx::query_as(
        "SELECT m.payload,u.avatar_id,u.display_name FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL WHERE m.channel_id=$1 AND m.external_id=$2",
    ).bind(channel_id).bind(message).fetch_optional(&mut *tx).await.map_err(database_error)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "message not found"))?;
    // Like history, never serve a forward's stored snapshot: its source may
    // since have been deleted.
    let mut messages = [enrich_author(payload, avatar, name.as_deref())];
    forwarding::hydrate(&mut tx, &mut messages).await?;
    drop(tx);
    deliver(&chat.pool, chat.cdn.as_deref(), &mut messages).await?;
    let [message] = messages;
    Ok(Json(message))
}

/// A versions page without URLs, as other readers see it or the author does
/// without a CDN.
#[cfg(test)]
async fn versions_page(
    pool: &PgPool,
    channel: &str,
    message: &str,
    before: Option<i32>,
    user: Option<i64>,
) -> Result<Value, ApiError> {
    signed_versions_page(pool, channel, message, before, user, None).await
}

/// A version's or message content's files, as `(id, spoiler)`.
fn files(content: &Value) -> Vec<(&str, bool)> {
    attachments::shown(content.get("attachments"))
}

/// Retained versions, newest first. `filesChanged` marks a revision whose
/// files (or spoilers) differ from the one before it. Only the author sees
/// files an edit replaced, with fresh URLs; for everyone else they are
/// placeholders and no version carries a URL.
async fn signed_versions_page(
    pool: &PgPool,
    channel: &str,
    message: &str,
    before: Option<i32>,
    user: Option<i64>,
    cdn: Option<&CdnSigner>,
) -> Result<Value, ApiError> {
    if before.is_some_and(|v| v < 1) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid message revision",
        ));
    }
    let mut tx = pool.begin().await.map_err(database_error)?;
    let (channel_id, _, _) = locked_channel(&mut tx, channel, user, false).await?;
    let (id, payload, author): (i64, Value, Option<i64>) = sqlx::query_as(
        "SELECT m.id,m.payload,cs.user_id FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id WHERE m.channel_id=$1 AND m.external_id=$2 AND m.deleted_at IS NULL",
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
    drop(tx);
    let more = rows.len() > PAGE as usize;
    let mut versions: Vec<Value> = rows
        .iter()
        .take(PAGE as usize)
        .enumerate()
        .map(|(i, (revision, content, at))| {
            let mut version =
                json!({"revision":revision,"content":content,"createdAt":at.to_rfc3339()});
            // The extra row fetched for `hasMore` is the last one's predecessor.
            if let Some((previous, earlier, _)) = rows.get(i + 1)
                && *previous == revision - 1
                && files(earlier) != files(content)
            {
                version["filesChanged"] = json!(true);
            }
            version
        })
        .collect();
    if payload["revision"].as_i64().unwrap_or(1) == 1 && before.is_none_or(|v| v > 1) {
        versions.push(
            json!({"revision":1,"content":payload["content"],"createdAt":payload["createdAt"]}),
        );
    }
    if user.is_some() && user == author {
        assets::mark_deleted(pool, &mut versions).await?;
        for version in &mut versions {
            *version = assets::sign_attachments(std::mem::take(version), cdn);
        }
    } else {
        let current: Vec<&str> = files(&payload["content"])
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assets::conceal(&mut versions, |id| !current.contains(&id));
    }
    Ok(json!({"messageId":message,"versions":versions,"hasMore":more}))
}

#[cfg(test)]
mod tests;
