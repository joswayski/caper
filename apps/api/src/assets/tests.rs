use super::*;
use crate::{Cloudflare, Config, accounts::User, chat::Chat};
use axum::{body::Bytes, extract::Path as AxumPath, http::HeaderMap, response::IntoResponse};
use sqlx::{
    Connection, Executor,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    str::FromStr,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[test]
fn presigning_matches_the_aws_s3_documented_example() {
    // https://docs.aws.amazon.com/AmazonS3/latest/API/sigv4-query-string-auth.html
    let url = presign_v4(
        "GET",
        "https://examplebucket.s3.amazonaws.com",
        "/test.txt",
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "us-east-1",
        DateTime::parse_from_rfc3339("2013-05-24T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
        86400,
        &[],
    );
    assert_eq!(
        url,
        "https://examplebucket.s3.amazonaws.com/test.txt?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host&X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
    );
}

#[test]
fn upload_urls_sign_the_exact_type_length_and_name() {
    let r2 = R2::new(
        "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com".into(),
        "production-caper".into(),
        "key".into(),
        "secret".into(),
    );
    let url = r2.presign(
        "PUT",
        "original/abc",
        900,
        &[
            ("content-type", "image/png"),
            ("content-length", "12"),
            ("content-disposition", "attachment"),
        ],
    );
    assert!(url.starts_with(
        "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com/production-caper/original/abc?"
    ));
    assert!(url.contains(
        "X-Amz-SignedHeaders=content-disposition%3Bcontent-length%3Bcontent-type%3Bhost"
    ));
    assert!(url.contains("X-Amz-Expires=900"));
    assert!(url.contains("%2Fauto%2Fs3%2Faws4_request"));
}

#[test]
fn delivery_urls_are_stable_for_a_day_and_expire_within_two() {
    let signer = CdnSigner::new("https://cdn.caper.chat".into(), vec![7; 32]);
    let morning = 1_790_000_000 - 1_790_000_000 % DAY + 60;
    let evening = morning + DAY - 120;
    let url = signer.url("original/abc", morning);
    assert_eq!(url, signer.url("original/abc", evening));
    assert_ne!(url, signer.url("original/abc", morning + DAY));
    assert_ne!(url, signer.url("original/abd", morning));
    let expires: i64 = url
        .split("exp=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(expires - morning > DAY && expires - morning <= 2 * DAY);
    // The Worker verifies HMAC-SHA256("{key}\n{exp}"); keep this vector in sync
    // with apps/cdn/worker.test.mjs.
    let fixed = CdnSigner::new(
        "https://cdn.test".into(),
        b"0123456789abcdef0123456789abcdef".to_vec(),
    );
    assert_eq!(
        fixed.url("original/abc", 0),
        "https://cdn.test/assets/abc?exp=172800&sig=2GDNk1-0tzBzLEfmuQIvaGGFQ3ItbeJMj8gWdbnL8Ck"
    );
}

#[test]
fn messages_gain_urls_only_when_signed_and_available() {
    let signer = CdnSigner::new("https://cdn.test".into(), vec![1; 32]);
    let message = json!({"content":{"version":1,"type":"text","text":"","attachments":[
        {"id":"one","kind":"image","preview":{}},
        {"id":"two","kind":"file"},
        {"id":"three","kind":"image","unavailable":true}
    ]}});
    assert_eq!(sign_attachments(message.clone(), None), message);
    let signed = sign_attachments(message, Some(&signer));
    let attachments = signed["content"]["attachments"].as_array().unwrap();
    assert!(
        attachments[0]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://cdn.test/assets/one?exp=")
    );
    assert!(
        attachments[0]["previewUrl"]
            .as_str()
            .unwrap()
            .starts_with("https://cdn.test/preview/one?exp=")
    );
    assert!(attachments[1]["url"].is_string() && attachments[1].get("previewUrl").is_none());
    assert!(attachments[2].get("url").is_none());
    // A forward carries a snapshot of its source; its files are signed too.
    let forwarded = sign_attachments(
        json!({"type":"message.created","message":{"forward":{"message":{"content":{"attachments":[{"id":"src","kind":"image"}]}}}}}),
        Some(&signer),
    );
    assert!(
        forwarded["message"]["forward"]["message"]["content"]["attachments"][0]["url"]
            .as_str()
            .unwrap()
            .contains("/assets/src?")
    );
    let event = sign_attachments(
        json!({"type":"message.created","message":{"content":{"attachments":[{"id":"x","kind":"file"}]}}}),
        Some(&signer),
    );
    assert!(event["message"]["content"]["attachments"][0]["url"].is_string());
}

#[test]
fn sniffing_rejects_mislabelled_inline_media() {
    assert!(sniff("image/png", b"\x89PNG\r\n\x1a\n...."));
    assert!(!sniff("image/png", b"<svg onload=alert(1)>"));
    assert!(sniff("image/webp", b"RIFF\0\0\0\0WEBPVP8 "));
    assert!(!sniff("image/webp", b"RIFF\0\0\0\0WAVEfmt "));
    assert!(sniff("video/mp4", b"\0\0\0\x18ftypisom"));
    assert!(sniff("image/avif", b"\0\0\0\x1cftypavif"));
    assert!(!sniff("image/avif", b"\0\0\0\x1cftypisom"));
    assert!(sniff("video/webm", b"\x1a\x45\xdf\xa3\x01"));
    assert!(sniff("audio/mpeg", b"ID3\x04"));
    assert!(!sniff("image/gif", b"<html>"));
    assert!(sniff("application/pdf", b"%PDF-1.7\n"));
    assert!(!sniff("application/pdf", b"<html><script>"));
    assert!(opens_in_browser("application/pdf") && opens_in_browser("image/png"));
    assert!(!opens_in_browser("text/html") && !opens_in_browser("image/svg+xml"));
    assert_eq!(kind("image/svg+xml"), "file");
    assert_eq!(kind("text/html"), "file");
    assert_eq!(kind("image/heic"), "file");
    assert_eq!(kind("video/mp4"), "video");
}

#[test]
fn configuration_requires_complete_r2_and_cdn_settings() {
    use RuntimeEnvironment as E;
    const ACCOUNT: (&str, &str) = ("R2_ACCOUNT_ID", "0123456789abcdef0123456789abcdef");
    const BUCKET: (&str, &str) = ("R2_BUCKET", "staging-caper");
    const ID: (&str, &str) = ("R2_ACCESS_KEY_ID", "id");
    const SECRET: (&str, &str) = ("R2_SECRET_ACCESS_KEY", "secret");
    assert!(
        R2::from_env(&E::from_values_for_test([]))
            .unwrap()
            .is_none()
    );
    assert!(R2::from_env(&E::from_values_for_test([ACCOUNT, BUCKET])).is_err());
    let configured = R2::from_env(&E::from_values_for_test([ACCOUNT, BUCKET, ID, SECRET]))
        .unwrap()
        .unwrap();
    assert_eq!(
        configured.endpoint,
        "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com"
    );
    let local = R2::from_env(&E::from_values_for_test([
        ACCOUNT,
        BUCKET,
        ID,
        SECRET,
        ("R2_ENDPOINT", "http://127.0.0.1:9000/"),
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(local.endpoint, "http://127.0.0.1:9000");
    for endpoint in ["https://evil.example", "http://127.0.0.1.evil.example"] {
        assert!(
            R2::from_env(&E::from_values_for_test([
                ACCOUNT,
                BUCKET,
                ID,
                SECRET,
                ("R2_ENDPOINT", endpoint),
            ]))
            .is_err()
        );
    }
    let secret = (
        "ASSET_CDN_SIGNING_SECRET",
        "ssssssssssssssssssssssssssssssss",
    );
    assert!(
        CdnSigner::from_env(&E::from_values_for_test([
            ("ASSET_CDN_ORIGIN", "https://cdn.caper.chat/"),
            secret
        ]))
        .unwrap()
        .is_some()
    );
    for origin in ["http://cdn.caper.chat", "https://cdn.caper.chat?x=1"] {
        assert!(
            CdnSigner::from_env(&E::from_values_for_test([
                ("ASSET_CDN_ORIGIN", origin),
                secret
            ]))
            .is_err()
        );
    }
    assert!(
        CdnSigner::from_env(&E::from_values_for_test([
            ("ASSET_CDN_ORIGIN", "https://cdn.caper.chat"),
            ("ASSET_CDN_SIGNING_SECRET", "short"),
        ]))
        .is_err()
    );
    assert!(
        CdnSigner::from_env(&E::from_values_for_test([(
            "ASSET_CDN_ORIGIN",
            "https://cdn.caper.chat"
        )]))
        .is_err()
    );
}

#[test]
fn compression_settings_default_and_validate_from_configuration() {
    use RuntimeEnvironment as E;
    let defaults = Compression::from_env(&E::from_values_for_test([])).unwrap();
    assert_eq!(defaults, Compression::default());
    assert_eq!(
        serde_json::to_value(&defaults).unwrap(),
        json!({"imageFormat":"avif","avifQuality":85,"imageQuality":92,"imageMaxEdge":4096,"paletteColors":256,"previewEdge":640,
               "videoMaxHeight":1440,"videoBitrateKbps":8000,"videoSourceBitratePercent":200,
               "videoHighFpsBoostPercent":150,"audioBitrateKbps":128})
    );
    let tuned = Compression::from_env(&E::from_values_for_test([
        ("ASSET_IMAGE_QUALITY", "80"),
        ("ASSET_IMAGE_FORMAT", " WebP "),
        ("ASSET_AVIF_QUALITY", "70"),
        ("ASSET_VIDEO_MAX_HEIGHT", "0"),
        ("ASSET_PALETTE_COLORS", "0"),
        ("ASSET_VIDEO_SOURCE_BITRATE_PERCENT", "0"),
        ("ASSET_VIDEO_HIGH_FPS_BOOST_PERCENT", "200"),
    ]))
    .unwrap();
    assert_eq!(
        (
            tuned.video_source_bitrate_percent,
            tuned.video_high_fps_boost_percent
        ),
        (0, 200)
    );
    assert_eq!(
        (
            tuned.image_format,
            tuned.avif_quality,
            tuned.image_quality,
            tuned.video_max_height,
            tuned.palette_colors
        ),
        ("webp", 70, 80, 0, 0)
    );
    for (name, value) in [
        ("ASSET_IMAGE_QUALITY", "0"),
        ("ASSET_IMAGE_QUALITY", "high"),
        ("ASSET_IMAGE_FORMAT", "jxl"),
        ("ASSET_AVIF_QUALITY", "101"),
        ("ASSET_PALETTE_COLORS", "257"),
        ("ASSET_VIDEO_BITRATE_KBPS", "10"),
        ("ASSET_VIDEO_SOURCE_BITRATE_PERCENT", "1001"),
        ("ASSET_VIDEO_HIGH_FPS_BOOST_PERCENT", "99"),
    ] {
        assert!(
            Compression::from_env(&E::from_values_for_test([(name, value)])).is_err(),
            "{name}={value}"
        );
    }
}

#[test]
fn names_and_types_are_normalized() {
    assert_eq!(filename("C:\\Users\\me\\shot.png").unwrap(), "shot.png");
    assert_eq!(filename("../../etc/passwd").unwrap(), "passwd");
    assert!(filename("..").is_err());
    assert!(filename("a\nb").is_err());
    assert!(filename(&"a".repeat(256)).is_err());
    assert_eq!(content_type("").unwrap(), "application/octet-stream");
    assert_eq!(content_type("Image/PNG; charset=x").unwrap(), "image/png");
    assert!(content_type("text/html\r\nx: y").is_err());
    assert!(content_type("nonsense").is_err());
    assert_eq!(
        disposition("résumé \"final\".pdf"),
        "attachment; filename=\"r_sum_ _final_.pdf\"; filename*=UTF-8''r%C3%A9sum%C3%A9%20%22final%22.pdf"
    );
}

type Store = Arc<Mutex<HashMap<String, Vec<u8>>>>;
type DeleteFailures = Arc<Mutex<std::collections::HashSet<String>>>;

/// Minimal path-style S3 stand-in. It records objects but does not verify
/// signatures; live R2 validation is a separate deployment check.
async fn fake_r2() -> (String, Store, DeleteFailures) {
    let store: Store = Arc::default();
    let failures: DeleteFailures = Arc::default();
    let shared = store.clone();
    let app = Router::new().route(
        "/{bucket}/{*key}",
        get({
            let store = shared.clone();
            move |AxumPath((_, key)): AxumPath<(String, String)>, headers: HeaderMap| async move {
                let Some(body) = store.lock().unwrap().get(&key).cloned() else {
                    return StatusCode::NOT_FOUND.into_response();
                };
                // Only the `bytes=0-{end}` prefixes the API asks for.
                let end = headers
                    .get("range")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.strip_prefix("bytes=0-"))
                    .and_then(|end| end.parse::<usize>().ok());
                let body = match end {
                    Some(end) => body.into_iter().take(end + 1).collect(),
                    None => body,
                };
                ([("content-length", body.len().to_string())], body).into_response()
            }
        })
        .put({
            let store = shared.clone();
            move |AxumPath((_, key)): AxumPath<(String, String)>, body: Bytes| async move {
                store.lock().unwrap().insert(key, body.to_vec());
                StatusCode::OK
            }
        })
        .delete({
            let store = shared.clone();
            let failures = failures.clone();
            move |AxumPath((_, key)): AxumPath<(String, String)>| async move {
                if failures.lock().unwrap().remove(&key) {
                    return StatusCode::SERVICE_UNAVAILABLE;
                }
                store.lock().unwrap().remove(&key);
                StatusCode::NO_CONTENT
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), store, failures)
}

fn principal(id: i64, external_id: &str) -> Principal {
    Principal {
        user: User {
            id,
            external_id: external_id.to_owned(),
            avatar_id: 1,
            avatar_asset: None,
            email: None,
            username: Some(external_id.to_lowercase()),
            display_name: Some(external_id.to_owned()),
        },
        token_hash: Vec::new(),
    }
}

async fn put(created: &Value, field: &str, body: &[u8]) {
    let upload = &created[field];
    let mut request = reqwest::Client::new().put(upload["url"].as_str().unwrap());
    for (name, value) in upload["headers"].as_object().unwrap() {
        request = request.header(name, value.as_str().unwrap());
    }
    assert!(
        request
            .body(body.to_vec())
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
}

fn png(size: usize) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.resize(size, 0);
    bytes
}

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL"]
async fn uploads_reserve_quota_verify_bytes_attach_once_and_purge() {
    let url = std::env::var("CHAT_TEST_DATABASE_URL")
        .expect("CHAT_TEST_DATABASE_URL must point at disposable Postgres");
    let options = PgConnectOptions::from_str(&url).unwrap();
    assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
    let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
    let database = format!("assets_test_{}", Uuid::new_v4().simple());
    admin
        .execute(format!("CREATE DATABASE {database}").as_str())
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options.database(&database))
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let mut ids = Vec::new();
    for name in ["Owner", "Outsider"] {
        let external = random_id(12);
        let id: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES($1,lower($2),$2) RETURNING id")
            .bind(&external).bind(name).fetch_one(&pool).await.unwrap();
        ids.push((id, external));
    }
    let owner = principal(ids[0].0, &ids[0].1);
    let outsider = principal(ids[1].0, &ids[1].1);
    let space_id: i64 = sqlx::query_scalar(
        "INSERT INTO public.spaces(external_id,name,owner_id) VALUES($1,'Friends',$2) RETURNING id",
    )
    .bind(random_id(12))
    .bind(owner.user.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
        .bind(space_id)
        .bind(owner.user.id)
        .execute(&pool)
        .await
        .unwrap();
    let channel = random_id(12);
    sqlx::query("INSERT INTO public.channels(external_id,space_id,name) VALUES($1,$2,'general')")
        .bind(&channel)
        .bind(space_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) SELECT id,$2 FROM public.channels WHERE external_id=$1")
        .bind(&channel)
        .bind(owner.user.id)
        .execute(&pool)
        .await
        .unwrap();
    let account_hash = Sha256::digest(b"owner-account").to_vec();
    sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
        .bind(&account_hash).bind(owner.user.id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,user_id,name,account_session_hash) VALUES($1,$2,$3,'Owner',$4)")
        .bind(random_id(12)).bind(Sha256::digest(b"owner-chat").as_slice()).bind(owner.user.id).bind(&account_hash).execute(&pool).await.unwrap();

    let (endpoint, store, failures) = fake_r2().await;
    let assets = Assets::new(
        pool.clone(),
        R2::new(endpoint, "test".into(), "key".into(), "secret".into()),
        1000,
    );
    let signer = Arc::new(CdnSigner::new("https://cdn.test".into(), vec![9; 32]));
    let mut state = AppState::with_database(
        Config::test(false),
        Arc::new(Cloudflare::new()),
        Some(pool.clone()),
    );
    state.assets = Some(assets.clone());
    let mut chat = Chat::new(
        pool.clone(),
        redis::Client::open("redis://127.0.0.1:1").unwrap(),
    );
    chat.cdn = Some(signer.clone());
    state.chat = Some(chat);
    let input = |name: &str, content_type: &str, size: i64, preview: Option<i64>| {
        serde_json::from_value::<CreateInput>(json!({
            "channelId": channel, "filename": name, "contentType": content_type,
            "byteSize": size, "sourceByteSize": size * 3, "width": 40, "height": 30,
            "preview": preview.map(|size| json!({"contentType":"image/webp","byteSize":size})),
        }))
        .unwrap()
    };
    let create_as = |who: &Principal, input: CreateInput| {
        create(State(state.clone()), Extension(who.clone()), Json(input))
    };

    // Outsiders cannot reserve uploads in channels they cannot read.
    assert_eq!(
        create_as(&outsider, input("a.png", "image/png", 10, None))
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    // Quota counts the original and its preview.
    let full = create_as(&owner, input("a.png", "image/png", 990, Some(20)))
        .await
        .unwrap_err();
    assert_eq!(
        (full.status, full.code),
        (StatusCode::PAYLOAD_TOO_LARGE, Some("storage_full"))
    );

    let (status, Json(shot)) = create_as(&owner, input("shot.png", "image/png", 100, Some(20)))
        .await
        .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(shot["kind"], "image");
    assert_eq!(shot["storage"], json!({"used":120,"limit":1000}));
    assert_eq!(shot["upload"]["headers"]["content-type"], "image/png");
    assert!(
        shot["upload"]["headers"]["content-disposition"]
            .as_str()
            .unwrap()
            .contains("shot.png")
    );
    let shot_id = shot["id"].as_str().unwrap().to_owned();
    let complete_as = |who: &Principal, id: &str| {
        complete(
            State(state.clone()),
            Extension(who.clone()),
            Path(id.to_owned()),
        )
    };
    assert_eq!(
        complete_as(&owner, &shot_id).await.unwrap_err().status,
        StatusCode::CONFLICT
    );
    put(&shot, "upload", &png(100)).await;
    put(&shot, "previewUpload", b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0").await;
    assert_eq!(
        complete_as(&outsider, &shot_id).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    let described = complete_as(&owner, &shot_id).await.unwrap().0;
    assert_eq!(described["size"], 100);
    assert_eq!(described["preview"], json!({}));
    assert_eq!(complete_as(&owner, &shot_id).await.unwrap().0, described);

    // A renamed HTML file is refused and its reservation is queued for purge.
    let (_, Json(fake)) = create_as(&owner, input("fake.png", "image/png", 6, None))
        .await
        .unwrap();
    put(&fake, "upload", b"<html>").await;
    let fake_id = fake["id"].as_str().unwrap().to_owned();
    assert_eq!(
        complete_as(&owner, &fake_id).await.unwrap_err().status,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // Attachments link once, keep order, and gain signed URLs on the way out.
    let (_, Json(doc)) = create_as(&owner, input("notes.txt", "text/plain", 5, None))
        .await
        .unwrap();
    put(&doc, "upload", b"hello").await;
    let doc_id = doc["id"].as_str().unwrap().to_owned();
    let _ = complete_as(&owner, &doc_id).await.unwrap();
    let client_message_id = Uuid::new_v4();
    let message = crate::chat::send_message(
        &pool,
        &channel,
        "owner-chat",
        client_message_id,
        "",
        &[shot_id.clone(), doc_id.clone()],
        None,
        false,
        None,
    )
    .await
    .unwrap();
    let attachments = &message["content"]["attachments"];
    assert_eq!(attachments[0]["id"], shot_id.as_str());
    assert_eq!(attachments[1]["kind"], "file");
    assert!(
        attachments[0].get("url").is_none(),
        "stored payloads never contain URLs"
    );
    // An accepted but unacknowledged Send still retries successfully after the
    // unsent lifetime. Expiry must not run before idempotency lookup.
    sqlx::query(
        "UPDATE public.assets SET uploaded_at=now()-interval '25 hours' WHERE external_id=ANY($1)",
    )
    .bind([&shot_id, &doc_id])
    .execute(&pool)
    .await
    .unwrap();
    let retry = crate::chat::send_message(
        &pool,
        &channel,
        "owner-chat",
        client_message_id,
        "",
        &[shot_id.clone(), doc_id.clone()],
        None,
        false,
        None,
    )
    .await
    .unwrap();
    assert_eq!(retry, message);
    assert_eq!(
        crate::chat::send_message(
            &pool,
            &channel,
            "owner-chat",
            Uuid::new_v4(),
            "again",
            std::slice::from_ref(&doc_id),
            None,
            false,
            None
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        crate::chat::send_message(
            &pool,
            &channel,
            "owner-chat",
            Uuid::new_v4(),
            "",
            &[],
            None,
            false,
            None
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::BAD_REQUEST
    );
    let history =
        crate::chat::history_with(&pool, &channel, None, Some(owner.user.id), Some(&signer))
            .await
            .unwrap();
    let listed = &history["messages"][0]["content"]["attachments"];
    assert!(
        listed[0]["previewUrl"]
            .as_str()
            .unwrap()
            .contains("/preview/")
    );
    assert!(
        listed[1]["url"]
            .as_str()
            .unwrap()
            .contains(&format!("/assets/{doc_id}?exp="))
    );
    // Pin and forward responses carry the message with fresh URLs too, like
    // a send's: clients keep them for the pinned list and the new forward.
    let app = crate::app(state.clone());
    let chat_request = |method: &str, path: String, body: Value| {
        let app = app.clone();
        let request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("x-caper-chat-token", "owner-chat")
            .header("authorization", "Bearer owner-account")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        async move {
            let response = tower::ServiceExt::oneshot(app, request).await.unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            (status, serde_json::from_slice::<Value>(&bytes).unwrap())
        }
    };
    let message_id = message["id"].as_str().unwrap();
    let (status, pinned) = chat_request(
        "PUT",
        format!("/api/chat/channels/{channel}/messages/{message_id}/pin"),
        json!({"active": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{pinned}");
    assert!(
        pinned["message"]["content"]["attachments"][1]["url"]
            .as_str()
            .unwrap()
            .contains(&format!("/assets/{doc_id}?exp="))
    );
    let (status, forwarded) = chat_request(
        "POST",
        format!("/api/chat/channels/{channel}/forwards"),
        json!({"sourceChannelId": channel, "sourceMessageId": message_id,
            "clientMessageId": Uuid::new_v4()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{forwarded}");
    assert!(
        forwarded["forward"]["message"]["content"]["attachments"][1]["url"]
            .as_str()
            .unwrap()
            .contains(&format!("/assets/{doc_id}?exp="))
    );
    // So does the forwarded conversation the forward opens.
    let (status, conversation) = chat_request(
        "GET",
        format!(
            "/api/chat/channels/{channel}/forwards/{}/thread",
            forwarded["id"].as_str().unwrap()
        ),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{conversation}");
    for shown in [&conversation["root"], &conversation["source"]["message"]] {
        assert!(
            shown["content"]["attachments"][0]["previewUrl"]
                .as_str()
                .unwrap()
                .contains("/preview/"),
            "{shown}"
        );
    }

    let refreshed = urls(
        State(state.clone()),
        Extension(owner.clone()),
        Json(UrlsInput {
            ids: vec![shot_id.clone(), fake_id.clone()],
        }),
    )
    .await
    .unwrap()
    .0;
    assert!(refreshed["urls"][&shot_id]["previewUrl"].is_string());
    assert!(
        refreshed["urls"][&shot_id]["url"]
            .as_str()
            .unwrap()
            .contains(&format!("/assets/{shot_id}?exp="))
    );
    assert!(refreshed["urls"].get(&fake_id).is_none());
    let hidden = urls(
        State(state.clone()),
        Extension(outsider.clone()),
        Json(UrlsInput {
            ids: vec![shot_id.clone()],
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(hidden["urls"], json!({}));

    // Rejected and abandoned uploads are purged; quota is released only then.
    let before = usage(State(state.clone()), Extension(owner.clone()))
        .await
        .unwrap()
        .0;
    assert_eq!(before["used"], 100 + 20 + 6 + 5);
    let (_, Json(stale)) = create_as(&owner, input("stale.png", "image/png", 8, None))
        .await
        .unwrap();
    put(&stale, "upload", &png(8)).await;
    let _ = complete_as(&owner, stale["id"].as_str().unwrap())
        .await
        .unwrap();
    sqlx::query(
        "UPDATE public.assets SET uploaded_at = now() - interval '25 hours' WHERE external_id=$1",
    )
    .bind(stale["id"].as_str().unwrap())
    .execute(&pool)
    .await
    .unwrap();
    // Expiry is enforced on Send even before the next cleanup tick.
    let expired = crate::chat::send_message(
        &pool,
        &channel,
        "owner-chat",
        Uuid::new_v4(),
        "keep this text",
        &[stale["id"].as_str().unwrap().to_owned()],
        None,
        false,
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(
        (expired.status, expired.code),
        (StatusCode::CONFLICT, Some("attachment_unavailable"))
    );
    assert_eq!(purge(&assets).await.unwrap(), 2);
    assert!(!store.lock().unwrap().contains_key(&original_key(&fake_id)));
    assert!(store.lock().unwrap().contains_key(&original_key(&shot_id)));
    let after = usage(State(state.clone()), Extension(owner.clone()))
        .await
        .unwrap()
        .0;
    assert_eq!(after["used"], 100 + 20 + 5);

    // Deleting a sent file hides it at once and purges after the delay; rows stay.
    sqlx::query("UPDATE public.assets SET deleted_at = now() WHERE external_id=$1")
        .bind(&doc_id)
        .execute(&pool)
        .await
        .unwrap();
    let history =
        crate::chat::history_with(&pool, &channel, None, Some(owner.user.id), Some(&signer))
            .await
            .unwrap();
    let listed = &history["messages"][0]["content"]["attachments"];
    assert_eq!(listed[1]["unavailable"], true);
    assert!(listed[1].get("url").is_none());
    assert_eq!(purge(&assets).await.unwrap(), 0);
    sqlx::query(
        "UPDATE public.assets SET deleted_at = now() - interval '25 hours' WHERE external_id=$1",
    )
    .bind(&doc_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(purge(&assets).await.unwrap(), 1);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM public.assets")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 4);

    let cancel_as = |who: &Principal, id: &str| {
        cancel(
            State(state.clone()),
            Extension(who.clone()),
            Path(id.to_owned()),
        )
    };
    assert_eq!(
        cancel_as(&owner, &shot_id).await.unwrap_err().status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        cancel_as(&outsider, &shot_id).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        cancel_as(&owner, "missing").await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );

    // Both completed and incomplete removals release quota at the next purge,
    // not a day later. Cancellation is idempotent and prevents completion/Send.
    for completed in [false, true] {
        let (_, Json(removed)) = create_as(&owner, input("remove.png", "image/png", 8, Some(20)))
            .await
            .unwrap();
        let id = removed["id"].as_str().unwrap();
        put(&removed, "upload", &png(8)).await;
        put(&removed, "previewUpload", b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0").await;
        if completed {
            let _ = complete_as(&owner, id).await.unwrap();
        }
        assert_eq!(
            cancel_as(&outsider, id).await.unwrap_err().status,
            StatusCode::NOT_FOUND
        );
        for _ in 0..2 {
            assert_eq!(cancel_as(&owner, id).await.unwrap(), StatusCode::NO_CONTENT);
        }
        assert_eq!(
            complete_as(&owner, id).await.unwrap_err().status,
            StatusCode::NOT_FOUND
        );
        let refused = crate::chat::send_message(
            &pool,
            &channel,
            "owner-chat",
            Uuid::new_v4(),
            "unsent",
            &[id.to_owned()],
            None,
            false,
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(refused.code, Some("attachment_unavailable"));
        assert_eq!(
            usage(State(state.clone()), Extension(owner.clone()))
                .await
                .unwrap()
                .0["used"],
            148
        );
        let failed_key = if completed {
            preview_key(id)
        } else {
            original_key(id)
        };
        failures.lock().unwrap().insert(failed_key.clone());
        assert_eq!(purge(&assets).await.unwrap(), 0);
        assert!(store.lock().unwrap().contains_key(&failed_key));
        assert_eq!(
            usage(State(state.clone()), Extension(owner.clone()))
                .await
                .unwrap()
                .0["used"],
            148,
            "both objects must be deleted before quota is released"
        );
        assert_eq!(purge(&assets).await.unwrap(), 1);
        assert!(!store.lock().unwrap().contains_key(&original_key(id)));
        assert!(!store.lock().unwrap().contains_key(&preview_key(id)));
        assert_eq!(
            usage(State(state.clone()), Extension(owner.clone()))
                .await
                .unwrap()
                .0["used"],
            120
        );
        // Even a very late PUT must not recreate a permanently forgotten orphan.
        sqlx::query("UPDATE public.assets SET deleted_at=now()-interval '30 days', purged_at=now()-interval '29 days' WHERE external_id=$1")
            .bind(id).execute(&pool).await.unwrap();
        let purged_at: DateTime<Utc> =
            sqlx::query_scalar("SELECT purged_at FROM public.assets WHERE external_id=$1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        put(&removed, "upload", &png(8)).await;
        put(&removed, "previewUpload", b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0").await;
        PurgeSweep::default().run(&assets).await.unwrap();
        assert!(!store.lock().unwrap().contains_key(&original_key(id)));
        assert!(!store.lock().unwrap().contains_key(&preview_key(id)));
        assert_eq!(
            sqlx::query_scalar::<_, DateTime<Utc>>(
                "SELECT purged_at FROM public.assets WHERE external_id=$1"
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap(),
            purged_at
        );
        assert!(
            store.lock().unwrap().contains_key(&original_key(&shot_id)),
            "sweeps never delete sent objects"
        );
    }

    // Concurrent Send and removal have exactly one winner; cancellation must
    // never make an accepted message's attachment unavailable.
    let (_, Json(racing)) = create_as(&owner, input("race.txt", "text/plain", 5, None))
        .await
        .unwrap();
    let race_id = racing["id"].as_str().unwrap();
    put(&racing, "upload", b"hello").await;
    let _ = complete_as(&owner, race_id).await.unwrap();
    let ids = [race_id.to_owned()];
    let (sent, cancelled) = tokio::join!(
        crate::chat::send_message(
            &pool,
            &channel,
            "owner-chat",
            Uuid::new_v4(),
            "racing",
            &ids,
            None,
            false,
            None
        ),
        cancel_as(&owner, race_id)
    );
    match sent {
        Ok(_) => {
            assert_eq!(cancelled.unwrap_err().status, StatusCode::CONFLICT);
            assert_eq!(purge(&assets).await.unwrap(), 0);
            assert!(store.lock().unwrap().contains_key(&original_key(race_id)));
            // Fixture cleanup follows sent-file retention, not cancellation.
            sqlx::query("UPDATE public.assets SET deleted_at=now()-interval '25 hours' WHERE external_id=$1")
                .bind(race_id).execute(&pool).await.unwrap();
        }
        Err(error) => {
            assert_eq!(error.code, Some("attachment_unavailable"));
            assert_eq!(cancelled.unwrap(), StatusCode::NO_CONTENT);
        }
    }
    assert_eq!(purge(&assets).await.unwrap(), 1);

    // Reconciliation progresses beyond a full batch and wraps to revisit old
    // keys; arrivals during a pass must not extend its fixed ceiling.
    sqlx::query("INSERT INTO public.assets (external_id,owner_id,purpose,channel_id,kind,content_type,filename,byte_size,deleted_at,purged_at)
        SELECT 'sweep-' || n,$1,'attachment',c.id,'file','text/plain','sweep.txt',1,now(),now()
        FROM public.channels c CROSS JOIN generate_series(1,30) n WHERE c.external_id=$2")
        .bind(owner.user.id).bind(&channel).execute(&pool).await.unwrap();
    for n in 1..=30 {
        store
            .lock()
            .unwrap()
            .insert(original_key(&format!("sweep-{n}")), vec![1]);
    }
    let mut sweep = PurgeSweep::default();
    failures.lock().unwrap().insert(original_key("sweep-1"));
    sweep.run(&assets).await.unwrap();
    assert!(
        store
            .lock()
            .unwrap()
            .contains_key(&original_key("sweep-30"))
    );
    sqlx::query("INSERT INTO public.assets(external_id,owner_id,purpose,channel_id,kind,content_type,filename,byte_size,deleted_at,purged_at)
        SELECT 'sweep-new',owner_id,purpose,channel_id,kind,content_type,filename,byte_size,deleted_at,purged_at
        FROM public.assets WHERE external_id='sweep-1'")
        .execute(&pool).await.unwrap();
    store
        .lock()
        .unwrap()
        .insert(original_key("sweep-new"), vec![1]);
    // Recreate an early key after it was already visited in this pass.
    store
        .lock()
        .unwrap()
        .insert(original_key("sweep-1"), vec![2]);
    sweep.run(&assets).await.unwrap();
    assert!(
        !store
            .lock()
            .unwrap()
            .contains_key(&original_key("sweep-30"))
    );
    assert!(store.lock().unwrap().contains_key(&original_key("sweep-1")));
    assert!(
        store
            .lock()
            .unwrap()
            .contains_key(&original_key("sweep-new")),
        "new arrivals wait for the next pass"
    );
    sweep.run(&assets).await.unwrap();
    assert!(!store.lock().unwrap().contains_key(&original_key("sweep-1")));
    sweep.run(&assets).await.unwrap();
    assert!(
        !store
            .lock()
            .unwrap()
            .contains_key(&original_key("sweep-new"))
    );

    // Scripted floods hit the pending and per-minute reservation limits.
    sqlx::query("UPDATE public.assets SET created_at = now() - interval '2 minutes'")
        .execute(&pool)
        .await
        .unwrap();
    let mut accepted = 0;
    let limited = loop {
        match create_as(&owner, input("tiny.txt", "text/plain", 1, None)).await {
            Ok(_) => accepted += 1,
            Err(error) => break error,
        }
    };
    assert_eq!(accepted, MAX_PENDING);
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    let usage = usage(State(state.clone()), Extension(owner.clone()))
        .await
        .unwrap()
        .0;
    assert_eq!(usage["compression"]["imageQuality"], 92);

    pool.close().await;
    admin
        .execute(format!("DROP DATABASE {database} WITH (FORCE)").as_str())
        .await
        .unwrap();
}

fn gif(width: u16, height: u16, size: usize) -> Vec<u8> {
    let mut bytes = b"GIF89a".to_vec();
    bytes.extend(width.to_le_bytes());
    bytes.extend(height.to_le_bytes());
    bytes.resize(size, 0);
    bytes
}

fn sized_png(width: u32, height: u32, size: usize) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    bytes.resize(size, 0);
    bytes
}

/// A camera JPEG: its frame header sits behind a large EXIF segment.
fn camera_jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8, 0xff, 0xe1, 0x4e, 0x22];
    bytes.resize(bytes.len() + 0x4e20, 0);
    bytes.extend([0xff, 0xc0, 0x00, 0x11, 8]);
    bytes.extend(height.to_be_bytes());
    bytes.extend(width.to_be_bytes());
    bytes.resize(bytes.len() + 400, 0);
    bytes
}

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL"]
async fn pictures_apply_on_completion_are_kept_outside_quota_and_sign_on_the_way_out() {
    let url = std::env::var("CHAT_TEST_DATABASE_URL")
        .expect("CHAT_TEST_DATABASE_URL must point at disposable Postgres");
    let options = PgConnectOptions::from_str(&url).unwrap();
    assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
    let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
    let database = format!("pictures_test_{}", Uuid::new_v4().simple());
    admin
        .execute(format!("CREATE DATABASE {database}").as_str())
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options.database(&database))
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let mut people = Vec::new();
    for name in ["Owner", "Outsider"] {
        let external = random_id(12);
        let (id, avatar): (i64, i16) = sqlx::query_as("INSERT INTO public.users(external_id,username,display_name) VALUES($1,lower($2),$2) RETURNING id,avatar_id")
            .bind(&external).bind(name).fetch_one(&pool).await.unwrap();
        people.push((principal(id, &external), avatar));
    }
    let (owner, mascot) = people[0].clone();
    let outsider = people[1].0.clone();
    let space_external = random_id(12);
    let space_id: i64 = sqlx::query_scalar(
        "INSERT INTO public.spaces(external_id,name,owner_id) VALUES($1,'Friends',$2) RETURNING id",
    )
    .bind(&space_external)
    .bind(owner.user.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
        .bind(space_id)
        .bind(owner.user.id)
        .execute(&pool)
        .await
        .unwrap();

    let (endpoint, store, _) = fake_r2().await;
    // A tiny file quota: pictures must not count against it.
    let assets = Assets::new(
        pool.clone(),
        R2::new(endpoint, "test".into(), "key".into(), "secret".into()),
        100,
    );
    let signer = Arc::new(CdnSigner::new("https://cdn.test".into(), vec![9; 32]));
    let mut state = AppState::with_database(
        Config::test(false),
        Arc::new(Cloudflare::new()),
        Some(pool.clone()),
    );
    state.assets = Some(assets.clone());
    let mut chat = Chat::new(
        pool.clone(),
        redis::Client::open("redis://127.0.0.1:1").unwrap(),
    );
    chat.cdn = Some(signer.clone());
    state.chat = Some(chat);

    let picture = |purpose: &str, content_type: &str, size: usize, side: i32| {
        let mut input = json!({"purpose": purpose, "filename": "picture", "contentType": content_type,
            "byteSize": size, "width": side, "height": side});
        if purpose == "spaceIcon" {
            input["spaceId"] = json!(space_external);
        }
        serde_json::from_value::<CreateInput>(input).unwrap()
    };
    let create_as = |who: &Principal, input: CreateInput| {
        create(State(state.clone()), Extension(who.clone()), Json(input))
    };
    let complete_as = |who: &Principal, id: &str| {
        complete(
            State(state.clone()),
            Extension(who.clone()),
            Path(id.to_owned()),
        )
    };
    let code = |error: ApiError| (error.status, error.code);

    // Reservations: type, dimensions and targets are checked first (never bytes).
    assert_eq!(
        code(
            create_as(&owner, picture("avatar", "image/avif", 10, 10))
                .await
                .unwrap_err()
        ),
        (StatusCode::BAD_REQUEST, Some("picture_type"))
    );
    assert_eq!(
        create_as(&outsider, picture("spaceIcon", "image/gif", 100, 100))
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    let with_channel: CreateInput =
        serde_json::from_value(json!({"purpose":"avatar","channelId":"x",
        "filename":"a","contentType":"image/png","byteSize":10,"width":1,"height":1}))
        .unwrap();
    assert_eq!(
        create_as(&owner, with_channel).await.unwrap_err().status,
        StatusCode::BAD_REQUEST
    );

    // A GIF far larger than the file quota uploads unchanged and applies on completion.
    let (status, Json(first)) = create_as(&owner, picture("avatar", "image/gif", 4000, 480))
        .await
        .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    assert!(first.get("storage").is_none());
    let first_id = first["id"].as_str().unwrap().to_owned();
    assert_eq!(first_id.len(), crate::pictures::ID_LENGTH);
    assert_eq!(first["upload"]["headers"]["content-type"], "image/gif");
    put(&first, "upload", &gif(480, 480, 4000)).await;
    let done = complete_as(&owner, &first_id).await.unwrap().0;
    assert_eq!(done["account"]["avatarAsset"], first_id.as_str());
    assert_eq!(done["account"]["avatarId"], mascot);
    let usage_now = usage(State(state.clone()), Extension(owner.clone()))
        .await
        .unwrap()
        .0;
    assert_eq!(usage_now["used"], 0);

    // A newer picture wins; a late retry of the older completion never undoes it.
    let photo = camera_jpeg(640, 480);
    let (_, Json(second)) = create_as(&owner, picture("avatar", "image/jpeg", photo.len(), 640))
        .await
        .unwrap();
    let second_id = second["id"].as_str().unwrap().to_owned();
    put(&second, "upload", &photo).await;
    assert_eq!(
        complete_as(&owner, &second_id).await.unwrap().0["account"]["avatarAsset"],
        second_id.as_str()
    );
    assert_eq!(
        complete_as(&owner, &first_id).await.unwrap().0["account"]["avatarAsset"],
        second_id.as_str()
    );
    // Uploads finishing out of order: the later choice wins even when the
    // earlier one completes last, and the earlier one is kept, not discarded.
    let mut chosen = Vec::new();
    for _ in 0..2 {
        let (_, Json(reserved)) =
            create_as(&owner, picture("avatar", "image/jpeg", photo.len(), 640))
                .await
                .unwrap();
        put(&reserved, "upload", &photo).await;
        chosen.push(reserved["id"].as_str().unwrap().to_owned());
    }
    let [earlier, later] = chosen.as_slice() else {
        unreachable!()
    };
    assert_eq!(
        complete_as(&owner, later).await.unwrap().0["account"]["avatarAsset"],
        later.as_str()
    );
    let done = complete_as(&owner, earlier).await.unwrap().0;
    assert_eq!(done["id"], earlier.as_str());
    assert_eq!(done["account"]["avatarAsset"], later.as_str());
    let (current, kept): (Option<String>, bool) = sqlx::query_as(
        "SELECT u.avatar_asset, a.uploaded_at IS NOT NULL AND a.deleted_at IS NULL
         FROM public.users u, public.assets a WHERE u.id=$1 AND a.external_id=$2",
    )
    .bind(owner.user.id)
    .bind(earlier)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(current.as_deref(), Some(later.as_str()));
    assert!(kept);
    // The rest of this test follows the current picture.
    let second_id = later.clone();
    // Applied pictures cannot be cancelled; pending ones can.
    assert_eq!(
        cancel(
            State(state.clone()),
            Extension(owner.clone()),
            Path(second_id.clone())
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::CONFLICT
    );
    let (_, Json(dropped)) = create_as(&owner, picture("avatar", "image/png", 50, 10))
        .await
        .unwrap();
    assert_eq!(
        cancel(
            State(state.clone()),
            Extension(owner.clone()),
            Path(dropped["id"].as_str().unwrap().to_owned())
        )
        .await
        .unwrap(),
        StatusCode::NO_CONTENT
    );

    // No pixel limit: a 4000 px wide picture applies as it is.
    let (_, Json(wide)) = create_as(&owner, picture("avatar", "image/png", 64, 100))
        .await
        .unwrap();
    put(&wide, "upload", &sized_png(4000, 10, 64)).await;
    let wide_id = wide["id"].as_str().unwrap().to_owned();
    assert_eq!(
        complete_as(&owner, &wide_id).await.unwrap().0["account"]["avatarAsset"],
        wide_id.as_str()
    );
    let (_, Json(fake)) = create_as(&owner, picture("avatar", "image/png", 6, 10))
        .await
        .unwrap();
    put(&fake, "upload", b"<html>").await;
    assert_eq!(
        code(
            complete_as(&owner, fake["id"].as_str().unwrap())
                .await
                .unwrap_err()
        ),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("picture_invalid"))
    );
    let current: Option<String> =
        sqlx::query_scalar("SELECT avatar_asset FROM public.users WHERE id=$1")
            .bind(owner.user.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(current.as_deref(), Some(wide_id.as_str()));

    // Space icons apply to spaces the uploader still owns.
    let (_, Json(icon)) = create_as(&owner, picture("spaceIcon", "image/png", 64, 256))
        .await
        .unwrap();
    put(&icon, "upload", &sized_png(256, 256, 64)).await;
    let icon_id = icon["id"].as_str().unwrap().to_owned();
    let applied = complete_as(&owner, &icon_id).await.unwrap().0;
    assert_eq!(applied["space"]["iconAsset"], icon_id.as_str());
    assert_eq!(applied["space"]["id"], space_external.as_str());
    let (_, Json(late)) = create_as(&owner, picture("spaceIcon", "image/png", 64, 256))
        .await
        .unwrap();
    put(&late, "upload", &sized_png(256, 256, 64)).await;
    sqlx::query("UPDATE public.spaces SET owner_id=$2 WHERE id=$1")
        .bind(space_id)
        .bind(outsider.user.id)
        .execute(&pool)
        .await
        .unwrap();
    let late_id = late["id"].as_str().unwrap().to_owned();
    assert_eq!(
        complete_as(&owner, &late_id).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    sqlx::query("UPDATE public.spaces SET owner_id=$2 WHERE id=$1")
        .bind(space_id)
        .bind(owner.user.id)
        .execute(&pool)
        .await
        .unwrap();
    let icon_now: Option<String> =
        sqlx::query_scalar("SELECT icon_asset FROM public.spaces WHERE id=$1")
            .bind(space_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(icon_now.as_deref(), Some(icon_id.as_str()));

    // Applied pictures outlive the unsent-file lifetime, including replaced
    // ones; abandoned, rejected and orphaned uploads are purged.
    sqlx::query("UPDATE public.assets SET uploaded_at=now()-interval '3 days', created_at=now()-interval '3 days'")
        .execute(&pool)
        .await
        .unwrap();
    let (_, Json(abandoned)) = create_as(&owner, picture("avatar", "image/png", 64, 10))
        .await
        .unwrap();
    put(&abandoned, "upload", &sized_png(10, 10, 64)).await;
    sqlx::query(
        "UPDATE public.assets SET created_at=now()-interval '2 hours' WHERE external_id=$1",
    )
    .bind(abandoned["id"].as_str().unwrap())
    .execute(&pool)
    .await
    .unwrap();
    // One rejected, one cancelled, one orphaned by ownership, one abandoned.
    assert_eq!(purge(&assets).await.unwrap(), 4);
    for kept in [&first_id, &second_id, &wide_id, &icon_id] {
        assert!(store.lock().unwrap().contains_key(&original_key(kept)));
        let deleted: bool = sqlx::query_scalar(
            "SELECT deleted_at IS NOT NULL FROM public.assets WHERE external_id=$1",
        )
        .bind(kept)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!deleted, "applied pictures are kept");
    }
    assert!(!store.lock().unwrap().contains_key(&original_key(&late_id)));

    // Responses sign picture IDs on the way out, in people and spaces alike.
    // The router's test sign-in is account ID 1 (the seeded Caper account).
    let router = crate::app(state.clone());
    sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,1)")
        .bind(space_id)
        .execute(&pool)
        .await
        .unwrap();
    let get = |path: String| {
        let router = router.clone();
        async move {
            use tower::ServiceExt;
            let response = router
                .oneshot(
                    axum::http::Request::get(path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            serde_json::from_slice::<Value>(&bytes).unwrap()
        }
    };
    let now = Utc::now().timestamp();
    let signed = |id: &str| json!(signer.url(&original_key(id), now));
    let listed = get("/api/spaces".into()).await;
    assert_eq!(listed["spaces"][0]["iconAsset"], icon_id.as_str());
    assert_eq!(listed["spaces"][0]["iconUrl"], signed(&icon_id));
    let detail = get(format!("/api/spaces/{space_external}")).await;
    assert_eq!(detail["space"]["iconUrl"], signed(&icon_id));
    let members = detail["members"].as_array().unwrap();
    let owner_entry = members
        .iter()
        .find(|member| member["id"] == owner.user.external_id.as_str())
        .unwrap();
    assert_eq!(owner_entry["avatarAsset"], wide_id.as_str());
    assert_eq!(owner_entry["avatarUrl"], signed(&wide_id));
    let team = members
        .iter()
        .find(|member| member["id"] != owner.user.external_id.as_str())
        .unwrap();
    assert!(team.get("avatarAsset").is_none() && team.get("avatarUrl").is_none());

    // Message authors show their current picture; messages never store one.
    let channel = random_id(12);
    sqlx::query("INSERT INTO public.channels(external_id,space_id,name) VALUES($1,$2,'general')")
        .bind(&channel)
        .bind(space_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) SELECT id,$2 FROM public.channels WHERE external_id=$1")
        .bind(&channel)
        .bind(owner.user.id)
        .execute(&pool)
        .await
        .unwrap();
    let account_hash = Sha256::digest(b"picture-account").to_vec();
    sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
        .bind(&account_hash).bind(owner.user.id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,user_id,name,account_session_hash) VALUES($1,$2,$3,'Owner',$4)")
        .bind(random_id(12)).bind(Sha256::digest(b"picture-chat").as_slice()).bind(owner.user.id).bind(&account_hash).execute(&pool).await.unwrap();
    let sent = crate::chat::send_message(
        &pool,
        &channel,
        "picture-chat",
        Uuid::new_v4(),
        "hello",
        &[],
        None,
        false,
        None,
    )
    .await
    .unwrap();
    assert!(sent["author"].get("avatarAsset").is_none());
    let author = |history: Value| history["messages"][0]["author"].clone();
    let history =
        crate::chat::history_with(&pool, &channel, None, Some(owner.user.id), Some(&signer))
            .await
            .unwrap();
    assert_eq!(author(history)["avatarAsset"], wide_id.as_str());

    // Removing returns to the same saved mascot; only owners clear icons.
    let removed = crate::pictures::remove_avatar(State(state.clone()), Extension(owner.clone()))
        .await
        .unwrap()
        .0;
    assert!(removed.get("avatarAsset").is_none());
    assert_eq!(removed["avatarId"], mascot);
    let history =
        crate::chat::history_with(&pool, &channel, None, Some(owner.user.id), Some(&signer))
            .await
            .unwrap();
    assert!(author(history).get("avatarAsset").is_none());
    assert_eq!(
        crate::pictures::remove_icon(
            State(state.clone()),
            Extension(outsider.clone()),
            Path(space_external.clone())
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::NOT_FOUND
    );
    let cleared = crate::pictures::remove_icon(
        State(state.clone()),
        Extension(owner.clone()),
        Path(space_external.clone()),
    )
    .await
    .unwrap()
    .0;
    assert!(cleared.get("iconAsset").is_none());
    assert_eq!(cleared["name"], "Friends");

    // Picture changes are limited per day, separately from file uploads.
    sqlx::query("UPDATE public.assets SET created_at=now() WHERE owner_id=$1")
        .bind(owner.user.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO public.assets(external_id,owner_id,purpose,kind,content_type,filename,byte_size,uploaded_at,created_at)
         SELECT 'limit' || n, $1, 'avatar', 'image', 'image/png', 'a', 1, now()-interval '2 minutes', now()-interval '2 minutes' FROM generate_series(1, 50) n",
    )
    .bind(owner.user.id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        code(
            create_as(&owner, picture("avatar", "image/png", 50, 10))
                .await
                .unwrap_err()
        ),
        (StatusCode::TOO_MANY_REQUESTS, Some("picture_rate_limited"))
    );

    drop(pool);
    admin
        .execute(format!("DROP DATABASE {database} WITH (FORCE)").as_str())
        .await
        .unwrap();
}
