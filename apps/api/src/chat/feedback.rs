//! `@feedback`: a message that mentions the Caper team (`@feedback`, `@caper`,
//! `@admins` and the other `mentions::TEAM_ALIASES`) is forwarded live into
//! its author's feedback conversation: their DM with the team account, which
//! every feedback team member can also read and answer.
//!
//! Sending only records a `feedback_reports` row in its own transaction. The
//! chat publisher creates the forward afterwards, so reporting can never fail
//! or slow down the message itself. A thread reply reports its whole thread; a
//! message that is nothing but `@feedback` reports the message before it.
use super::forwarding::{MAX_FORWARDS, lock_spaces, source_snapshot};
use super::*;
use sqlx::{Postgres, Transaction};

#[cfg(test)]
mod tests;

/// Forwards per reporter per hour. Later reports are kept but not forwarded,
/// so one account can't flood the team's notifications.
const HOURLY_FORWARDS: i64 = 30;
/// A report that keeps failing is recorded as `failed` after this many tries.
const MAX_ATTEMPTS: i16 = 5;

/// Records a report for an account-authored message that mentions the team.
/// Messages already in a feedback conversation are never reported again.
pub(super) async fn enqueue(
    connection: &mut sqlx::PgConnection,
    message: i64,
    reporter: i64,
    channel: i64,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO public.feedback_reports (message_id,reporter_id)
         SELECT $1,$2 WHERE NOT EXISTS(SELECT 1 FROM public.direct_conversations
             WHERE channel_id=$3 AND feedback_user_id IS NOT NULL)
         ON CONFLICT (message_id) DO NOTHING",
    )
    .bind(message)
    .bind(reporter)
    .bind(channel)
    .execute(connection)
    .await
    .map_err(database_error)?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Forwarded(i64),
    /// The conversation's latest message is a recent forward of the same original.
    Duplicate,
    /// The message, its conversation, the reporter or the team account is gone.
    Unavailable,
    Limited,
}

impl Outcome {
    fn name(&self) -> &'static str {
        match self {
            Self::Forwarded(_) => "forwarded",
            Self::Duplicate => "duplicate",
            Self::Unavailable => "unavailable",
            Self::Limited => "limited",
        }
    }
}

/// Handles one due report in its own transaction. Returns whether there was
/// one, so the publisher keeps going while reports are waiting.
pub(super) async fn forward_pending(chat: &Chat) -> Result<bool, ()> {
    let mut tx = chat.pool.begin().await.map_err(|_| ())?;
    let claimed: Option<(i64, i64, i64, i16)> = sqlx::query_as(
        "SELECT id,message_id,reporter_id,attempts FROM public.feedback_reports
         WHERE processed_at IS NULL AND available_at<=now()
         ORDER BY available_at,id LIMIT 1 FOR UPDATE SKIP LOCKED",
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| ())?;
    let Some((report, message, reporter, attempts)) = claimed else {
        return Ok(false);
    };
    match forward(&mut tx, report, message, reporter).await {
        Ok(outcome) => {
            let forward_id = match outcome {
                Outcome::Forwarded(id) => Some(id),
                _ => None,
            };
            sqlx::query("UPDATE public.feedback_reports SET processed_at=now(),outcome=$2,forward_id=$3,attempts=attempts+1 WHERE id=$1")
                .bind(report).bind(outcome.name()).bind(forward_id).execute(&mut *tx).await.map_err(|_| ())?;
            tx.commit().await.map_err(|_| ())?;
            tracing::info!(
                event_name = "feedback_report_processed",
                outcome = outcome.name(),
                "feedback report processed"
            );
        }
        Err(_) => {
            drop(tx);
            let failed = attempts + 1 >= MAX_ATTEMPTS;
            sqlx::query(
                "UPDATE public.feedback_reports SET attempts=attempts+1,
                     available_at=now()+make_interval(secs => 5*power(2,attempts)),
                     processed_at=CASE WHEN $2 THEN now() END, outcome=CASE WHEN $2 THEN 'failed' END
                 WHERE id=$1 AND processed_at IS NULL",
            )
            .bind(report)
            .bind(failed)
            .execute(&chat.pool)
            .await
            .map_err(|_| ())?;
            tracing::warn!(
                event_name = "feedback_report_retry",
                failed,
                "feedback report will retry"
            );
        }
    }
    Ok(true)
}

async fn forward(
    tx: &mut Transaction<'_, Postgres>,
    report: i64,
    message: i64,
    reporter: i64,
) -> Result<Outcome, ApiError> {
    type Trigger = (i64, Option<i64>, i64, Value, i64);
    let trigger: Option<Trigger> = sqlx::query_as(
        "SELECT m.channel_id,m.thread_root_id,m.channel_seq,m.payload,m.session_id
         FROM public.messages m WHERE m.id=$1",
    )
    .bind(message)
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?;
    let Some((channel, root, seq, payload, session)) = trigger else {
        return Ok(Outcome::Unavailable);
    };
    let text = payload["content"]["text"].as_str().unwrap_or_default();
    let files = payload["content"]["attachments"]
        .as_array()
        .is_some_and(|files| !files.is_empty());
    let bare = !files && payload.get("forward").is_none() && mentions::only_team_mentions(text);
    // A reply reports its thread. A bare `@feedback` points at the message
    // shown just before it (a reply sent to the channel means its thread).
    let selected = match root {
        Some(root) => root,
        None if bare => sqlx::query_scalar(
            "SELECT COALESCE(p.thread_root_id,p.id) FROM public.messages p
             WHERE p.channel_id=$1 AND p.channel_seq<$2 AND (p.thread_root_id IS NULL OR p.broadcast)
             ORDER BY p.channel_seq DESC LIMIT 1",
        )
        .bind(channel)
        .bind(seq)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)?
        .unwrap_or(message),
        None => message,
    };
    // Forwards are flattened: a forwarded message reports its original.
    let (original, source_channel, source_external): (i64, i64, String) = sqlx::query_as(
        "SELECT original.id,original.channel_id,c.external_id FROM public.messages selected
         JOIN public.messages original ON original.id=COALESCE(selected.forward_source_id,selected.id)
         JOIN public.channels c ON c.id=original.channel_id WHERE selected.id=$1",
    )
    .bind(selected)
    .fetch_one(&mut **tx)
    .await
    .map_err(database_error)?;
    // Only what the reporter can still read is shared with the team. Checked
    // before creating their conversation, and again once the channels are locked.
    if !readable(tx, &source_external, reporter).await? {
        return Ok(Outcome::Unavailable);
    }
    let Some((destination, destination_external)) =
        crate::direct::feedback_conversation(tx, reporter).await?
    else {
        return Ok(Outcome::Unavailable);
    };
    if source_channel == destination {
        return Ok(Outcome::Duplicate);
    }
    // The same space -> channel lock order as forwarding and sending.
    let mut channels = vec![source_channel, destination];
    channels.sort_unstable();
    lock_spaces(tx, &channels).await?;
    sqlx::query("SELECT id FROM public.channels WHERE id=ANY($1) ORDER BY id FOR UPDATE")
        .bind(&channels)
        .fetch_all(&mut **tx)
        .await
        .map_err(database_error)?;
    if !readable(tx, &source_external, reporter).await? {
        return Ok(Outcome::Unavailable);
    }
    // Repeating `@feedback` in a thread the team was just sent adds nothing:
    // the forward is live. Later on, the team hears about it again.
    let latest: Option<(Option<i64>, bool)> = sqlx::query_as(
        "SELECT forward_source_id,created_at>now()-interval '10 minutes' FROM public.messages
         WHERE channel_id=$1 ORDER BY channel_seq DESC LIMIT 1",
    )
    .bind(destination)
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?;
    if latest == Some((Some(original), true)) {
        return Ok(Outcome::Duplicate);
    }
    let (forwards, recent): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM public.messages WHERE forward_source_id=$1),
                (SELECT count(*) FROM public.feedback_reports WHERE reporter_id=$2 AND outcome='forwarded'
                 AND processed_at>now()-interval '1 hour')",
    )
    .bind(original)
    .bind(reporter)
    .fetch_one(&mut **tx)
    .await
    .map_err(database_error)?;
    if forwards >= MAX_FORWARDS || recent >= HOURLY_FORWARDS {
        return Ok(Outcome::Limited);
    }
    let Some(snapshot) = source_snapshot(tx, original).await? else {
        return Ok(Outcome::Unavailable);
    };
    let author: Option<(String, String, i16)> = sqlx::query_as(
        "SELECT external_id,display_name,avatar_id FROM public.users
         WHERE id=$1 AND deleted_at IS NULL AND display_name IS NOT NULL",
    )
    .bind(reporter)
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?;
    let Some((author_id, name, avatar_id)) = author else {
        return Ok(Outcome::Unavailable);
    };
    // When the report isn't itself the forwarded message, its words become
    // the note, so the team sees what was said along with what it points at.
    let note = if selected == message || bare {
        ""
    } else {
        text
    };
    let (content, _) = with_mentions(
        tx,
        json!({"version":1,"type":"text","text":note}),
        &mentions::parse(note, false),
    )
    .await?;
    let head: i64 = sqlx::query_scalar("SELECT last_seq FROM public.channels WHERE id=$1")
        .bind(destination)
        .fetch_one(&mut **tx)
        .await
        .map_err(database_error)?;
    let seq = head + 1;
    let client_id = Uuid::new_v4();
    let hash = Sha256::digest(
        serde_json::to_vec(&json!({"feedback":report,"source":original}))
            .map_err(|_| unavailable())?,
    )
    .to_vec();
    let payload = json!({"id":random_id(15),"channelId":destination_external,"seq":seq.to_string(),"author":{"id":author_id,"name":name,"isGuest":false,"avatarId":avatar_id},"content":content,"createdAt":Utc::now().to_rfc3339(),"clientMessageId":client_id,"forward":snapshot,"forwardSeq":seq.to_string()});
    let forward: i64 = sqlx::query_scalar("INSERT INTO public.messages(external_id,channel_id,session_id,client_message_id,request_hash,channel_seq,payload,forward_source_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id")
        .bind(payload["id"].as_str()).bind(destination).bind(session).bind(client_id).bind(hash).bind(seq).bind(&payload).bind(original)
        .fetch_one(&mut **tx).await.map_err(database_error)?;
    // Notifies the feedback team like any message in the conversation.
    crate::push::enqueue(tx, forward)
        .await
        .map_err(database_error)?;
    sqlx::query("INSERT INTO public.channel_events(channel_id,seq,payload) VALUES($1,$2,$3)")
        .bind(destination).bind(seq).bind(json!({"type":"message.created","schemaVersion":1,"channelId":destination_external,"seq":seq.to_string(),"message":payload}))
        .execute(&mut **tx).await.map_err(database_error)?;
    sqlx::query("UPDATE public.channels SET last_seq=$2 WHERE id=$1")
        .bind(destination)
        .bind(seq)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    sqlx::query("INSERT INTO public.direct_reads(channel_id,user_id,seq) VALUES($1,$2,$3) ON CONFLICT(channel_id,user_id) DO UPDATE SET seq=GREATEST(direct_reads.seq,EXCLUDED.seq)")
        .bind(destination).bind(reporter).bind(seq).execute(&mut **tx).await.map_err(database_error)?;
    Ok(Outcome::Forwarded(forward))
}

async fn readable(
    tx: &mut Transaction<'_, Postgres>,
    channel: &str,
    reporter: i64,
) -> Result<bool, ApiError> {
    match crate::spaces::channel_access(&mut **tx, channel, Some(reporter)).await {
        Ok(_) => Ok(true),
        Err(error) if error.status == StatusCode::NOT_FOUND => Ok(false),
        Err(error) => Err(error),
    }
}
