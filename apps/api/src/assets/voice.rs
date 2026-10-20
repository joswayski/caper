//! Voice messages: audio recorded in the composer, stored as mono AAC with
//! the sender's waveform, and transcribed in the background. Transcription
//! starts when the upload completes, usually before the message is sent; a
//! transcript that finishes after sending reaches clients as a
//! `message.attachments` update.
use super::{Assets, Row, invalid, original_key, privacy::VOICE_CONTENT_TYPE};
use crate::{ApiError, ai::Ai, pictures::Purpose};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{process::Stdio, time::Duration};
use tokio::{fs, io::AsyncWriteExt, process::Command};

pub(super) const NAME: &str = "Voice message.m4a";
pub(super) const MAX_DURATION_MS: i32 = 10 * 60 * 1000;
const MAX_WAVEFORM_BYTES: usize = 256;
const TASK: &str = "transcribe";
const MAX_ATTEMPTS: i16 = 3;
/// Ten minutes at the stored 64 kbps is about 5 MB; anything far larger is
/// not one of ours.
const MAX_DOWNLOAD_BYTES: usize = 32 * 1024 * 1024;
const MAX_TRANSCRIPT_CHARS: usize = 50_000;
const IDLE: Duration = Duration::from_secs(2);

/// Validates the voice fields of an upload and decodes its waveform.
pub(super) fn check_upload(
    voice: bool,
    waveform: Option<&str>,
    purpose: Purpose,
    kind: &str,
    duration_ms: Option<i32>,
    has_preview: bool,
) -> Result<Option<Vec<u8>>, ApiError> {
    if !voice {
        return match waveform {
            Some(_) => Err(invalid("a waveform is only for voice messages")),
            None => Ok(None),
        };
    }
    if purpose != Purpose::Attachment
        || kind != "audio"
        || has_preview
        || !duration_ms.is_some_and(|ms| (1..=MAX_DURATION_MS).contains(&ms))
    {
        return Err(invalid(
            "voice messages are audio attachments up to 10 minutes",
        ));
    }
    waveform
        .map(|value| {
            STANDARD
                .decode(value)
                .ok()
                .filter(|bytes| (1..=MAX_WAVEFORM_BYTES).contains(&bytes.len()))
                .ok_or_else(|| invalid("invalid waveform"))
        })
        .transpose()
}

/// The transcript as clients see it; absent when none was requested.
pub(super) fn transcript(status: Option<&str>, text: Option<&str>) -> Option<Value> {
    match status? {
        "ready" => Some(json!({"status": "ready", "text": text.unwrap_or_default()})),
        status @ ("pending" | "failed") => Some(json!({ "status": status })),
        _ => None,
    }
}

/// Records a completed voice upload as AAC and queues its transcript.
pub(super) async fn published(
    tx: &mut sqlx::PgConnection,
    asset: &str,
    transcribe: bool,
    row: &mut Row,
) -> Result<(), ApiError> {
    row.content_type = VOICE_CONTENT_TYPE.into();
    row.filename = NAME.into();
    if transcribe && row.transcript_status.is_none() {
        row.transcript_status = Some("pending".into());
    }
    let id: i64 = sqlx::query_scalar(
        "UPDATE public.assets SET content_type=$2, filename=$3, transcript_status=$4
         WHERE external_id=$1 RETURNING id",
    )
    .bind(asset)
    .bind(&row.content_type)
    .bind(&row.filename)
    .bind(&row.transcript_status)
    .fetch_one(&mut *tx)
    .await
    .map_err(super::database_error)?;
    if row.transcript_status.as_deref() == Some("pending") {
        sqlx::query(
            "INSERT INTO public.ai_jobs (task, asset_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(TASK)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(super::database_error)?;
    }
    Ok(())
}

/// One transcription at a time per pod; jobs are leased, so pods share them.
pub(crate) fn spawn_transcriber(assets: Assets, ai: Ai) {
    tokio::spawn(async move {
        loop {
            match next(&assets, &ai).await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(()) => tracing::warn!(
                    event_name = "transcription_retry",
                    "transcription queue will retry"
                ),
            }
            tokio::time::sleep(IDLE).await;
        }
    });
}

pub(super) async fn next(assets: &Assets, ai: &Ai) -> Result<bool, ()> {
    let job: Option<(i64, i64, i16)> = sqlx::query_as(
        "WITH due AS (
             SELECT id FROM public.ai_jobs
             WHERE task=$1 AND finished_at IS NULL AND available_at <= now()
               AND (locked_at IS NULL OR locked_at < now() - interval '10 minutes')
             ORDER BY available_at, id LIMIT 1 FOR UPDATE SKIP LOCKED)
         UPDATE public.ai_jobs j SET locked_at=now(), attempts=LEAST(j.attempts+1, 32767)
         FROM due WHERE j.id=due.id
         RETURNING j.id, j.asset_id, j.attempts",
    )
    .bind(TASK)
    .fetch_optional(&assets.pool)
    .await
    .map_err(|_| ())?;
    let Some((job, asset, attempts)) = job else {
        return Ok(false);
    };
    let outcome = run(assets, ai, job, asset).await;
    let finished = match outcome {
        Ok(()) => true,
        Err(()) if attempts >= MAX_ATTEMPTS => {
            tracing::warn!(
                event_name = "transcription_failed",
                attempts,
                "transcription gave up"
            );
            store(&assets.pool, asset, "failed", None).await?;
            true
        }
        Err(()) => false,
    };
    sqlx::query(if finished {
        "UPDATE public.ai_jobs SET finished_at=now(), locked_at=NULL WHERE id=$1"
    } else {
        "UPDATE public.ai_jobs SET locked_at=NULL, available_at=now()+make_interval(secs => 30 * attempts) WHERE id=$1"
    })
    .bind(job)
    .execute(&assets.pool)
    .await
    .map_err(|_| ())?;
    Ok(true)
}

async fn run(assets: &Assets, ai: &Ai, job: i64, asset: i64) -> Result<(), ()> {
    let row: Option<(String, Option<i32>, bool)> = sqlx::query_as(
        "SELECT external_id, duration_ms,
                deleted_at IS NULL AND purged_at IS NULL AND uploaded_at IS NOT NULL AND voice
         FROM public.assets WHERE id=$1",
    )
    .bind(asset)
    .fetch_optional(&assets.pool)
    .await
    .map_err(|_| ())?;
    let Some((external_id, duration_ms, live)) = row else {
        return Ok(());
    };
    if !live {
        // Discarded before sending, or deleted since: nothing to show.
        return Ok(());
    }
    let audio = download(assets, &external_id).await?;
    let speech = speech_audio(&audio).await?;
    let (text, calls) = ai
        .transcribe(
            &speech,
            "flac",
            json!({"asset": external_id, "durationMs": duration_ms}),
        )
        .await;
    for call in &calls {
        let recorded = sqlx::query(
            "INSERT INTO public.ai_calls (job_id, task, provider, model, succeeded, http_status, latency_ms, input, output, error, cost_usd)
             VALUES ($1, $2, 'openrouter', $3, $4, $5, $6, $7, $8, $9, $10::float8::numeric)",
        )
        .bind(job)
        .bind(TASK)
        .bind(&call.model)
        .bind(call.succeeded)
        .bind(call.http_status.and_then(|v| i16::try_from(v).ok()))
        .bind(call.latency_ms)
        .bind(&call.input)
        .bind(&call.output)
        .bind(&call.error)
        .bind(call.cost_usd)
        .execute(&assets.pool)
        .await;
        if recorded.is_err() {
            tracing::warn!(
                event_name = "ai_call_log_failed",
                "an AI call was not recorded"
            );
        }
        tracing::info!(
            event_name = "ai_call",
            task = TASK,
            model = %call.model,
            succeeded = call.succeeded,
            latency_ms = call.latency_ms,
            cost_usd = call.cost_usd,
            "AI call"
        );
    }
    let text = text.ok_or(())?;
    let text: String = text.trim().chars().take(MAX_TRANSCRIPT_CHARS).collect();
    store(&assets.pool, asset, "ready", Some(&text)).await
}

async fn download(assets: &Assets, external_id: &str) -> Result<Vec<u8>, ()> {
    let mut response = assets
        .r2
        .http
        .get(
            assets
                .r2
                .presign("GET", &original_key(external_id), 180, &[]),
        )
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_DOWNLOAD_BYTES {
            return Err(());
        }
    }
    Ok(bytes)
}

/// 16 kHz mono FLAC: lossless at the rate speech models use, and accepted by
/// every transcription provider.
async fn speech_audio(audio: &[u8]) -> Result<Vec<u8>, ()> {
    let directory = tempfile::tempdir().map_err(|_| ())?;
    let input = directory.path().join("voice.m4a");
    let output = directory.path().join("speech.flac");
    let mut file = fs::File::create(&input).await.map_err(|_| ())?;
    file.write_all(audio).await.map_err(|_| ())?;
    file.flush().await.map_err(|_| ())?;
    drop(file);
    let mut child = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(&input)
        .args([
            "-vn", "-ac", "1", "-ar", "16000", "-c:a", "flac", "-f", "flac",
        ])
        .arg(&output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| ())?;
    let status = tokio::time::timeout(Duration::from_secs(120), child.wait())
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
    if !status.success() {
        return Err(());
    }
    fs::read(&output).await.map_err(|_| ())
}

/// Saves a transcript and, once the voice message has been sent, puts it in
/// the message and tells its channel. An unsent upload only keeps it on the
/// asset; sending copies it into the message from there.
pub(super) async fn store(
    pool: &PgPool,
    asset: i64,
    status: &str,
    text: Option<&str>,
) -> Result<(), ()> {
    // A send can link the asset between the two statements; try again then.
    for _ in 0..3 {
        let linked: Option<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
            "SELECT a.external_id, m.id, m.channel_id FROM public.assets a
             LEFT JOIN public.messages m ON m.id=a.message_id WHERE a.id=$1",
        )
        .bind(asset)
        .fetch_optional(pool)
        .await
        .map_err(|_| ())?;
        let Some((external_id, message, channel)) = linked else {
            return Ok(());
        };
        let (Some(message), Some(channel)) = (message, channel) else {
            let unsent = sqlx::query(
                "UPDATE public.assets SET transcript_status=$2, transcript=$3
                 WHERE id=$1 AND message_id IS NULL",
            )
            .bind(asset)
            .bind(status)
            .bind(text)
            .execute(pool)
            .await
            .map_err(|_| ())?
            .rows_affected()
                == 1;
            if unsent {
                return Ok(());
            }
            continue;
        };
        let mut tx = pool.begin().await.map_err(|_| ())?;
        // The order sends, edits and reactions lock in: space, channel, message.
        sqlx::query(
            "SELECT s.id FROM public.channels c JOIN public.spaces s ON s.id=c.space_id
             WHERE c.id=$1 FOR SHARE OF s",
        )
        .bind(channel)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| ())?;
        let (channel_external, head): (String, i64) = sqlx::query_as(
            "SELECT external_id, last_seq FROM public.channels WHERE id=$1 FOR UPDATE",
        )
        .bind(channel)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| ())?;
        let (message_external, mut payload, deleted): (String, Value, bool) = sqlx::query_as(
            "SELECT external_id, payload, deleted_at IS NOT NULL FROM public.messages WHERE id=$1 FOR UPDATE",
        )
        .bind(message)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| ())?;
        sqlx::query("UPDATE public.assets SET transcript_status=$2, transcript=$3 WHERE id=$1")
            .bind(asset)
            .bind(status)
            .bind(text)
            .execute(&mut *tx)
            .await
            .map_err(|_| ())?;
        if !deleted && set_transcript(&mut payload, &external_id, transcript(Some(status), text)) {
            let seq = head + 1;
            payload["attachmentsSeq"] = json!(seq.to_string());
            let event = json!({
                "type": "message.attachments",
                "schemaVersion": 1,
                "channelId": channel_external,
                "seq": seq.to_string(),
                "messageId": message_external,
                "attachments": payload["content"]["attachments"],
            });
            sqlx::query("UPDATE public.messages SET payload=$2 WHERE id=$1")
                .bind(message)
                .bind(&payload)
                .execute(&mut *tx)
                .await
                .map_err(|_| ())?;
            sqlx::query(
                "INSERT INTO public.channel_events(channel_id,seq,payload) VALUES($1,$2,$3)",
            )
            .bind(channel)
            .bind(seq)
            .bind(&event)
            .execute(&mut *tx)
            .await
            .map_err(|_| ())?;
            sqlx::query("UPDATE public.channels SET last_seq=$2 WHERE id=$1")
                .bind(channel)
                .bind(seq)
                .execute(&mut *tx)
                .await
                .map_err(|_| ())?;
        }
        tx.commit().await.map_err(|_| ())?;
        return Ok(());
    }
    Err(())
}

/// Sets the transcript on the message's copy of the attachment; false when
/// the message no longer shows it (edited away).
fn set_transcript(payload: &mut Value, id: &str, transcript: Option<Value>) -> bool {
    let Some(file) = payload
        .pointer_mut("/content/attachments")
        .and_then(Value::as_array_mut)
        .and_then(|files| files.iter_mut().find(|file| file["id"] == id))
        .and_then(Value::as_object_mut)
    else {
        return false;
    };
    match transcript {
        Some(value) => file.insert("transcript".into(), value),
        None => file.remove("transcript"),
    };
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploads_must_be_short_audio_attachments() {
        fn ok(waveform: Option<&str>) -> Result<Option<Vec<u8>>, ApiError> {
            check_upload(
                true,
                waveform,
                Purpose::Attachment,
                "audio",
                Some(4_200),
                false,
            )
        }
        assert_eq!(ok(None).unwrap(), None);
        assert_eq!(ok(Some("AQID")).unwrap(), Some(vec![1, 2, 3]));
        assert!(ok(Some("not base64!")).is_err());
        assert!(ok(Some("")).is_err());
        assert!(ok(Some(&STANDARD.encode([7; 257]))).is_err());
        assert!(ok(Some(&STANDARD.encode([7; 256]))).is_ok());
        for (purpose, kind, duration, preview) in [
            (Purpose::Avatar, "audio", Some(1_000), false),
            (Purpose::Attachment, "video", Some(1_000), false),
            (Purpose::Attachment, "audio", None, false),
            (Purpose::Attachment, "audio", Some(0), false),
            (
                Purpose::Attachment,
                "audio",
                Some(MAX_DURATION_MS + 1),
                false,
            ),
            (Purpose::Attachment, "audio", Some(1_000), true),
        ] {
            assert!(check_upload(true, None, purpose, kind, duration, preview).is_err());
        }
        assert!(
            check_upload(
                false,
                Some("AQID"),
                Purpose::Attachment,
                "audio",
                None,
                false
            )
            .is_err()
        );
        assert_eq!(
            check_upload(false, None, Purpose::Attachment, "audio", None, false).unwrap(),
            None
        );
    }

    #[test]
    fn transcripts_are_described_by_status() {
        assert_eq!(transcript(None, None), None);
        assert_eq!(
            transcript(Some("pending"), None),
            Some(json!({"status": "pending"}))
        );
        assert_eq!(
            transcript(Some("failed"), None),
            Some(json!({"status": "failed"}))
        );
        assert_eq!(
            transcript(Some("ready"), Some("hi")),
            Some(json!({"status": "ready", "text": "hi"}))
        );
        assert_eq!(transcript(Some("other"), None), None);
    }

    #[test]
    fn only_the_shown_attachment_gets_the_transcript() {
        let mut payload = json!({"content": {"attachments": [{"id": "a"}, {"id": "b", "transcript": {"status": "pending"}}]}});
        assert!(set_transcript(
            &mut payload,
            "b",
            transcript(Some("ready"), Some("yo"))
        ));
        assert_eq!(
            payload["content"]["attachments"][1]["transcript"]["text"],
            "yo"
        );
        assert!(
            payload["content"]["attachments"][0]
                .get("transcript")
                .is_none()
        );
        assert!(!set_transcript(&mut payload, "gone", None));
    }

    #[tokio::test]
    #[ignore = "requires ffmpeg, installed by the Postgres CI job"]
    async fn speech_audio_is_flac_for_any_recorded_container() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("tone.webm");
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                "-c:a",
                "libopus",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(made.success());
        let flac = speech_audio(&std::fs::read(&source).unwrap())
            .await
            .unwrap();
        assert_eq!(&flac[..4], b"fLaC");
    }
}
