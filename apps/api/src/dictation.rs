//! Dictation: speech the composer turns into text. A signed-in client posts a
//! short recording (`POST /api/dictation`, the raw audio as the body) and gets
//! `{text}` back to insert into the draft. Nothing is kept: not the audio, and
//! not the text, which only lives in the person's draft. `ai_calls` records
//! each provider attempt with the transcript replaced by its length.
use crate::{ApiError, AppState, ai::Ai, assets, auth::Principal, chat};
use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::post,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower_http::limit::RequestBodyLimitLayer;

/// Ten minutes (the voice message limit) at a phone recorder's 64 kbps is
/// about 5 MB.
const BODY_LIMIT: usize = 8 * 1024 * 1024;
/// The recording limit plus a little for the recorder stopping late.
const MAX_SECONDS: u64 = 610;
const TASK: &str = "dictate";
/// Dictations an account may make in an hour, shared across API replicas.
const PER_HOUR: i32 = 120;
const MAX_TEXT_CHARS: usize = 4_000;

/// Its own body limit: the app's 256 KiB is far too small for audio. Merge
/// after the app-wide limit so that limit doesn't wrap this route as well.
pub(crate) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/dictation", post(dictate))
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::account_auth,
        ))
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT))
}

async fn dictate(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    audio: Bytes,
) -> Result<Json<Value>, ApiError> {
    let (Some(ai), Some(pool)) = (state.ai.as_ref(), state.database.as_ref()) else {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "dictation unavailable",
        ));
    };
    if audio.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "empty recording"));
    }
    rate_limit(pool, principal.user.id).await?;
    Ok(Json(json!({ "text": transcribe(pool, ai, &audio).await? })))
}

async fn transcribe(pool: &PgPool, ai: &Ai, audio: &[u8]) -> Result<String, ApiError> {
    let unreadable = || ApiError::new(StatusCode::BAD_REQUEST, "unreadable recording");
    let speech = assets::speech_audio(audio)
        .await
        .map_err(|()| unreadable())?;
    if flac_seconds(&speech).is_none_or(|seconds| seconds > MAX_SECONDS) {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "dictation is limited to 10 minutes",
        ));
    }
    let (text, mut calls) = ai
        .transcribe(&speech, "flac", json!({"dictation": true}))
        .await;
    for call in &mut calls {
        if let Some(said) = call
            .output
            .as_mut()
            .and_then(|output| output.get_mut("text"))
        {
            let chars = said.as_str().map_or(0, |text| text.chars().count());
            *said = json!({ "chars": chars });
        }
    }
    assets::record_calls(pool, None, TASK, &calls).await;
    let text = text
        .ok_or_else(|| ApiError::new(StatusCode::BAD_GATEWAY, "transcription failed; try again"))?;
    Ok(text.trim().chars().take(MAX_TEXT_CHARS).collect())
}

/// Whole seconds of a FLAC stream, from its STREAMINFO block (ffmpeg fills in
/// the sample count when it writes a file).
fn flac_seconds(flac: &[u8]) -> Option<u64> {
    let info = flac.strip_prefix(b"fLaC")?.get(4..4 + 34)?;
    let packed = u64::from_be_bytes(info[10..18].try_into().ok()?);
    let rate = packed >> 44;
    let samples = packed & 0xF_FFFF_FFFF;
    (rate > 0 && samples > 0).then(|| samples.div_ceil(rate))
}

async fn rate_limit(pool: &PgPool, user: i64) -> Result<(), ApiError> {
    let attempts: i32 = sqlx::query_scalar(
        "INSERT INTO public.space_invite_limits(user_id,attempts,dictation_attempts) VALUES($1,0,1)
         ON CONFLICT(user_id) DO UPDATE SET
           dictation_attempts=CASE WHEN space_invite_limits.dictation_window_start <= now()-interval '1 hour' THEN 1
                                   ELSE LEAST(space_invite_limits.dictation_attempts+1,$2+1) END,
           dictation_window_start=CASE WHEN space_invite_limits.dictation_window_start <= now()-interval '1 hour' THEN now()
                                       ELSE space_invite_limits.dictation_window_start END
         RETURNING dictation_attempts",
    )
    .bind(user)
    .bind(PER_HOUR)
    .fetch_one(pool)
    .await
    .map_err(|_| chat::unavailable())?;
    if attempts > PER_HOUR {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too many dictations; try again later",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cloudflare, Config};
    use axum::{body::Body, http::Request};
    use sqlx::{Connection, Executor, postgres::PgConnectOptions};
    use std::{str::FromStr, sync::Arc};
    use tower::ServiceExt;

    async fn send(state: AppState, path: &str, body: Vec<u8>) -> (StatusCode, Value) {
        let response = crate::app(state)
            .oneshot(
                Request::post(path)
                    .header("content-type", "audio/webm")
                    .header("content-length", body.len())
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn speech(seconds: f32) -> Vec<u8> {
        tokio::process::Command::new("ffmpeg")
            .args(["-nostdin", "-loglevel", "error", "-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=330:duration={seconds}"))
            .args(["-c:a", "libopus", "-f", "webm", "-"])
            .output()
            .await
            .unwrap()
            .stdout
    }

    #[tokio::test]
    async fn recordings_get_their_own_body_limit() {
        let state = AppState::new(Config::test(true), Arc::new(Cloudflare::new()));
        // Well past the app-wide 256 KiB, it reaches the handler.
        let (status, body) = send(state.clone(), "/api/dictation", vec![0; 1 << 20]).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"], "dictation unavailable");
        let (status, _) = send(state.clone(), "/api/dictation", vec![0; BODY_LIMIT + 1]).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        // Other routes keep the small limit.
        let (status, _) = send(state, "/api/account/profile", vec![b' '; 300 * 1024]).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    #[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL"]
    async fn dictation_returns_text_and_keeps_none_of_it() {
        let url = std::env::var("CHAT_TEST_DATABASE_URL")
            .expect("CHAT_TEST_DATABASE_URL must point at disposable Postgres");
        let options = PgConnectOptions::from_str(&url).unwrap();
        assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
        let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
        let database = format!("dictation_test_{}", uuid::Uuid::new_v4().simple());
        admin
            .execute(format!("CREATE DATABASE {database}").as_str())
            .await
            .unwrap();
        let pool = PgPool::connect_with(options.database(&database))
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        // The test principal is user 1, which the migrations create.

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let provider = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new().route(
            "/audio/transcriptions",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["input_audio"]["format"], "flac");
                Json(json!({"text": "  hello from the bus  ", "usage": {"cost": 0.0001}}))
            }),
        );
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut state = AppState::with_database(
            Config::test(true),
            Arc::new(Cloudflare::new()),
            Some(pool.clone()),
        );
        state.ai = Some(Ai::new(provider, "key".into(), vec!["good/model".into()]));

        let (status, body) = send(state.clone(), "/api/dictation", speech(1.5).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({"text": "hello from the bus"}));
        let (task, job, output): (String, Option<i64>, Value) =
            sqlx::query_as("SELECT task, job_id, output FROM public.ai_calls")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!((task.as_str(), job), ("dictate", None));
        assert_eq!(output["text"], json!({"chars": 22}));
        assert!(!output.to_string().contains("bus"));

        let (status, _) = send(state.clone(), "/api/dictation", b"not audio".to_vec()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = send(state.clone(), "/api/dictation", speech(611.0).await).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

        sqlx::query("UPDATE public.space_invite_limits SET dictation_attempts=$1")
            .bind(PER_HOUR)
            .execute(&pool)
            .await
            .unwrap();
        let (status, _) = send(state.clone(), "/api/dictation", speech(1.0).await).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        // A new hour starts a new window.
        sqlx::query("UPDATE public.space_invite_limits SET dictation_window_start=now()-interval '61 minutes'")
            .execute(&pool)
            .await
            .unwrap();
        let (status, _) = send(state, "/api/dictation", speech(1.0).await).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn flac_length_comes_from_streaminfo() {
        let flac = assets::speech_audio(&speech(2.5).await).await.unwrap();
        assert_eq!(flac_seconds(&flac), Some(3));
        assert_eq!(flac_seconds(b"fLaC"), None);
        assert_eq!(
            flac_seconds(b"OggS not flac at all, long enough to read"),
            None
        );
    }
}
