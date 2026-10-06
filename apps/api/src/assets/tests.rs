use super::*;
use crate::{Cloudflare, Config, accounts::User, chat::Chat};
use axum::{body::Bytes, extract::Path as AxumPath, http::HeaderValue, response::IntoResponse};
use sqlx::{
    Connection, Executor,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    str::FromStr,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

fn example_signing() -> Signing<'static> {
    Signing {
        access_key: "AKIAIOSFODNN7EXAMPLE",
        secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        token: None,
        region: "us-east-1",
    }
}

fn example_time() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2013-05-24T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn presigning_matches_the_aws_s3_documented_example() {
    // https://docs.aws.amazon.com/AmazonS3/latest/API/sigv4-query-string-auth.html
    let url = presign_v4(
        "GET",
        "https://examplebucket.s3.amazonaws.com",
        "/test.txt",
        &example_signing(),
        example_time(),
        86400,
        &[],
    );
    assert_eq!(
        url,
        "https://examplebucket.s3.amazonaws.com/test.txt?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host&X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
    );
}

#[test]
fn temporary_credentials_sign_their_session_token_and_exact_headers() {
    let signing = Signing {
        token: Some("token/with+chars="),
        ..example_signing()
    };
    let headers = [("content-type", "image/png"), ("content-length", "12")];
    let url = presign_v4(
        "PUT",
        "https://uploads.s3.us-east-1.amazonaws.com",
        "/incoming/abc",
        &signing,
        example_time(),
        900,
        &headers,
    );
    assert!(url.starts_with("https://uploads.s3.us-east-1.amazonaws.com/incoming/abc?"));
    assert!(url.contains("X-Amz-Security-Token=token%2Fwith%2Bchars%3D"));
    assert!(url.contains("X-Amz-SignedHeaders=content-length%3Bcontent-type%3Bhost"));
    assert!(url.contains("X-Amz-Expires=900"));
    // The token is part of the signed query: changing it changes the signature.
    let other = presign_v4(
        "PUT",
        "https://uploads.s3.us-east-1.amazonaws.com",
        "/incoming/abc",
        &Signing {
            token: Some("other"),
            ..example_signing()
        },
        example_time(),
        900,
        &headers,
    );
    let signature = |u: &str| u.split("X-Amz-Signature=").nth(1).unwrap().to_owned();
    assert_ne!(signature(&url), signature(&other));
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
        "https://cdn.test/original/abc?exp=172800&sig=2GDNk1-0tzBzLEfmuQIvaGGFQ3ItbeJMj8gWdbnL8Ck"
    );
}

#[test]
fn only_ready_files_gain_download_urls_and_previews_show_while_processing() {
    let signer = CdnSigner::new("https://cdn.test".into(), vec![1; 32]);
    let message = json!({"content":{"version":1,"type":"text","text":"","attachments":[
        {"id":"one","kind":"image","preview":{}},
        {"id":"two","kind":"file","status":"ready"},
        {"id":"three","kind":"image","unavailable":true},
        {"id":"four","kind":"video","status":"processing","preview":{}},
        {"id":"five","kind":"video","status":"processing"},
        {"id":"six","kind":"image","status":"failed","preview":{}}
    ]}});
    assert_eq!(sign_attachments(message.clone(), None), message);
    let signed = sign_attachments(message, Some(&signer));
    let attachments = signed["content"]["attachments"].as_array().unwrap();
    assert!(
        attachments[0]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://cdn.test/original/one?exp=")
    );
    assert!(
        attachments[0]["previewUrl"]
            .as_str()
            .unwrap()
            .starts_with("https://cdn.test/preview/one?exp=")
    );
    assert!(attachments[1]["url"].is_string() && attachments[1].get("previewUrl").is_none());
    assert!(attachments[2].get("url").is_none());
    assert!(attachments[3].get("url").is_none() && attachments[3]["previewUrl"].is_string());
    assert!(attachments[4].get("url").is_none() && attachments[4].get("previewUrl").is_none());
    assert!(attachments[5].get("url").is_none() && attachments[5].get("previewUrl").is_none());
    let created = sign_attachments(
        json!({"type":"message.created","message":{"content":{"attachments":[{"id":"x","kind":"file"}]}}}),
        Some(&signer),
    );
    assert!(created["message"]["content"]["attachments"][0]["url"].is_string());
    let updated = sign_attachments(
        json!({"type":"message.attachments","attachments":[{"id":"x","kind":"file","status":"ready"}]}),
        Some(&signer),
    );
    assert!(updated["attachments"][0]["url"].is_string());
    assert_eq!(kind("image/svg+xml"), "file");
    assert_eq!(kind("image/heic"), "file");
    assert_eq!(kind("video/mp4"), "video");
}

#[test]
fn configuration_requires_complete_upload_r2_and_cdn_settings() {
    use RuntimeEnvironment as E;
    const ACCOUNT: (&str, &str) = ("R2_ACCOUNT_ID", "0123456789abcdef0123456789abcdef");
    const BUCKET: (&str, &str) = ("R2_BUCKET", "staging-caper");
    const ID: (&str, &str) = ("R2_ACCESS_KEY_ID", "id");
    const SECRET: (&str, &str) = ("R2_SECRET_ACCESS_KEY", "secret");
    const UPLOADS: (&str, &str) = ("MEDIA_UPLOAD_BUCKET", "caper-production-uploads");
    const REGION: (&str, &str) = ("MEDIA_UPLOAD_REGION", "us-east-1");
    const WORKER: (&str, &str) = ("MEDIA_WORKER_SECRET", "wwwwwwwwwwwwwwwwwwwwwwwwwwwwwwww");
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

    // Uploads stay off without the incoming bucket, even with R2 configured.
    assert!(
        Incoming::from_env(&E::from_values_for_test([ACCOUNT, BUCKET, ID, SECRET]))
            .unwrap()
            .is_none()
    );
    let incoming = Incoming::from_env(&E::from_values_for_test([UPLOADS, REGION]))
        .unwrap()
        .unwrap();
    assert_eq!(
        incoming.scheme_host,
        "https://caper-production-uploads.s3.us-east-1.amazonaws.com"
    );
    assert_eq!(incoming.path_prefix, "");
    assert!(matches!(incoming.credentials, Credentials::Chain(_)));
    let local = Incoming::from_env(&E::from_values_for_test([
        UPLOADS,
        REGION,
        ("MEDIA_UPLOAD_ENDPOINT", "http://127.0.0.1:9000/"),
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(
        (local.scheme_host.as_str(), local.path_prefix.as_str()),
        ("http://127.0.0.1:9000", "/caper-production-uploads")
    );
    for bad in [
        [UPLOADS, ("MEDIA_UPLOAD_ENDPOINT", "https://evil.example")],
        [("MEDIA_UPLOAD_BUCKET", "Bad_Bucket"), REGION],
        [UPLOADS, ("MEDIA_UPLOAD_REGION", "us east")],
    ] {
        assert!(Incoming::from_env(&E::from_values_for_test(bad)).is_err());
    }
    assert!(Incoming::from_env(&E::from_values_for_test([UPLOADS])).is_err());

    // The incoming bucket needs R2 (purging) and the worker secret.
    assert!(Assets::from_env(None, &E::from_values_for_test([UPLOADS, REGION, WORKER])).is_err());
    assert!(
        Assets::from_env(
            None,
            &E::from_values_for_test([ACCOUNT, BUCKET, ID, SECRET, UPLOADS, REGION])
        )
        .is_err()
    );
    assert!(
        Assets::from_env(
            None,
            &E::from_values_for_test([ACCOUNT, BUCKET, ID, SECRET])
        )
        .unwrap()
        .is_none()
    );

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
}

#[test]
fn media_settings_default_and_validate_from_configuration() {
    use RuntimeEnvironment as E;
    let defaults = MediaSettings::from_env(&E::from_values_for_test([])).unwrap();
    assert_eq!(defaults, MediaSettings::default());
    // The media worker parses exactly this shape.
    assert_eq!(
        serde_json::to_value(&defaults).unwrap(),
        json!({"imageAvifQuality":90,"imageAvifSpeed":6,"imageLosslessRatio":1.3,
               "imageMinSavingsPercent":15,"imageMaxEdge":0,"previewEdge":640,
               "videoCrf":20,"videoPreset":"slow","videoLongSeconds":300,
               "videoLongPreset":"veryfast","videoMaxHeight":1080,"audioKbps":128,
               "fileMinSavingsPercent":10})
    );
    let tuned = MediaSettings::from_env(&E::from_values_for_test([
        ("MEDIA_IMAGE_AVIF_QUALITY", "85"),
        ("MEDIA_VIDEO_CRF", "23"),
        ("MEDIA_VIDEO_PRESET", "medium"),
        ("MEDIA_IMAGE_LOSSLESS_RATIO", "2"),
    ]))
    .unwrap();
    assert_eq!(
        (
            tuned.image_avif_quality,
            tuned.video_crf,
            tuned.video_preset.as_str(),
            tuned.image_lossless_ratio
        ),
        (85, 23, "medium", 2.0)
    );
    for (name, value) in [
        ("MEDIA_IMAGE_AVIF_QUALITY", "0"),
        ("MEDIA_IMAGE_AVIF_QUALITY", "high"),
        ("MEDIA_VIDEO_CRF", "52"),
        ("MEDIA_VIDEO_PRESET", "placebo; rm -rf /"),
        ("MEDIA_AUDIO_KBPS", "8"),
        ("MEDIA_IMAGE_LOSSLESS_RATIO", "-1"),
    ] {
        assert!(
            MediaSettings::from_env(&E::from_values_for_test([(name, value)])).is_err(),
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
}

type Store = Arc<Mutex<HashMap<String, Vec<u8>>>>;

/// Minimal path-style S3 stand-in for both the incoming bucket and R2. It
/// records objects but does not verify signatures; live S3/R2 validation is a
/// separate deployment check.
async fn fake_s3() -> (String, Store) {
    let store: Store = Arc::default();
    let shared = store.clone();
    let app = Router::new().route(
        "/{bucket}/{*key}",
        get({
            let store = shared.clone();
            move |AxumPath((_, key)): AxumPath<(String, String)>| async move {
                let Some(body) = store.lock().unwrap().get(&key).cloned() else {
                    return StatusCode::NOT_FOUND.into_response();
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
            move |AxumPath((_, key)): AxumPath<(String, String)>| async move {
                store.lock().unwrap().remove(&key);
                StatusCode::NO_CONTENT
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), store)
}

fn principal(id: i64, external_id: &str) -> Principal {
    Principal {
        user: User {
            id,
            external_id: external_id.to_owned(),
            avatar_id: 1,
            email: None,
            username: Some(external_id.to_lowercase()),
            display_name: Some(external_id.to_owned()),
        },
        token_hash: Vec::new(),
    }
}

async fn put(created: &Value, body: &[u8]) {
    let upload = &created["upload"];
    assert_eq!(upload["method"], "PUT");
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

const WORKER_SECRET: &str = "wwwwwwwwwwwwwwwwwwwwwwwwwwwwwwww";

fn bearer(secret: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {secret}")).unwrap(),
    );
    headers
}

async fn attachment_events(pool: &PgPool) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT payload FROM public.channel_events WHERE payload->>'type'='message.attachments' ORDER BY seq",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn message_payload(pool: &PgPool, id: &str) -> Value {
    sqlx::query_scalar("SELECT payload FROM public.messages WHERE external_id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL"]
async fn uploads_process_on_the_server_update_messages_and_purge() {
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

    let (endpoint, store) = fake_s3().await;
    let mut assets = Assets::new(
        pool.clone(),
        R2::new(endpoint.clone(), "r2".into(), "key".into(), "secret".into()),
        Incoming::local(endpoint, "uploads".into(), "key".into(), "secret".into()),
        WORKER_SECRET.into(),
    );
    assets.quota_bytes = 1000;
    assets.max_upload_bytes = 950;
    let signer = Arc::new(CdnSigner::new("https://cdn.test".into(), vec![9; 32]));
    let mut state = AppState::with_database(
        Config::test(false),
        Arc::new(Cloudflare::new()),
        Some(pool.clone()),
    );
    state.assets = Some(assets.clone());
    state.chat = Some(Chat {
        pool: pool.clone(),
        broker: redis::Client::open("redis://127.0.0.1:1").unwrap(),
        wake: Arc::new(tokio::sync::Notify::new()),
        cdn: Some(signer.clone()),
    });
    let input = |name: &str, content_type: &str, size: i64| {
        serde_json::from_value::<CreateInput>(json!({
            "channelId": channel, "filename": name, "contentType": content_type, "byteSize": size,
        }))
        .unwrap()
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
    let worker_headers = bearer(WORKER_SECRET);
    let start_job = |id: &str| {
        worker_start(
            State(state.clone()),
            worker_headers.clone(),
            Path(id.to_owned()),
        )
    };
    let finish_job = |id: &str, body: Value| {
        worker_finish(
            State(state.clone()),
            worker_headers.clone(),
            Path(id.to_owned()),
            Json(serde_json::from_value(body).unwrap()),
        )
    };

    // Clients send only name, type and exact size; previews and compression
    // settings are gone from the request.
    assert!(
        serde_json::from_value::<CreateInput>(json!({
            "channelId": channel, "filename": "a.png", "contentType": "image/png", "byteSize": 1,
            "preview": {"contentType":"image/webp","byteSize":1}
        }))
        .is_err()
    );
    // Outsiders cannot reserve uploads in channels they cannot read.
    assert_eq!(
        create_as(&outsider, input("a.png", "image/png", 10))
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    let large = create_as(&owner, input("huge.mov", "video/quicktime", 951))
        .await
        .unwrap_err();
    assert_eq!(
        (large.status, large.code),
        (StatusCode::PAYLOAD_TOO_LARGE, Some("file_too_large"))
    );

    let (status, Json(shot)) = create_as(&owner, input("shot.png", "image/png", 100))
        .await
        .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(shot["kind"], "image");
    assert_eq!(shot["storage"], json!({"used":100,"limit":1000}));
    assert_eq!(
        shot["upload"]["headers"],
        json!({"content-type":"image/png"})
    );
    let shot_id = shot["id"].as_str().unwrap().to_owned();
    let upload_url = shot["upload"]["url"].as_str().unwrap();
    assert!(upload_url.contains(&format!("/uploads/incoming/{shot_id}?")));
    assert!(upload_url.contains("X-Amz-SignedHeaders=content-length%3Bcontent-type%3Bhost"));
    // The original's size is held against the quota while it processes.
    let full = create_as(&owner, input("full.png", "image/png", 901))
        .await
        .unwrap_err();
    assert_eq!(
        (full.status, full.code),
        (StatusCode::PAYLOAD_TOO_LARGE, Some("storage_full"))
    );

    assert_eq!(
        complete_as(&owner, &shot_id).await.unwrap_err().status,
        StatusCode::CONFLICT
    );
    put(&shot, &[7; 100]).await;
    assert_eq!(
        complete_as(&outsider, &shot_id).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    let described = complete_as(&owner, &shot_id).await.unwrap().0;
    assert_eq!(
        described,
        json!({"id":shot_id,"kind":"image","contentType":"image/png","name":"shot.png","size":100,"status":"processing"})
    );
    assert_eq!(complete_as(&owner, &shot_id).await.unwrap().0, described);

    // An upload of the wrong size is refused and fails.
    let (_, Json(short)) = create_as(&owner, input("short.txt", "text/plain", 6))
        .await
        .unwrap();
    put(&short, b"hello").await;
    let short_id = short["id"].as_str().unwrap().to_owned();
    assert_eq!(
        complete_as(&owner, &short_id).await.unwrap_err().status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        complete_as(&owner, &short_id).await.unwrap_err().status,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // Files go into a message while still processing: no download URL yet.
    let (_, Json(doc)) = create_as(&owner, input("notes.txt", "text/plain", 5))
        .await
        .unwrap();
    put(&doc, b"hello").await;
    let doc_id = doc["id"].as_str().unwrap().to_owned();
    let _ = complete_as(&owner, &doc_id).await.unwrap();
    let message = crate::chat::persist_message(
        &pool,
        &channel,
        "owner-chat",
        Uuid::new_v4(),
        "",
        &[shot_id.clone(), doc_id.clone()],
    )
    .await
    .unwrap();
    let message_id = message["id"].as_str().unwrap().to_owned();
    let attachments = &message["content"]["attachments"];
    assert_eq!(attachments[0]["id"], shot_id.as_str());
    assert_eq!(attachments[0]["status"], "processing");
    assert_eq!(attachments[1]["kind"], "file");
    assert!(
        attachments[0].get("url").is_none(),
        "stored payloads never contain URLs"
    );
    assert_eq!(
        crate::chat::persist_message(
            &pool,
            &channel,
            "owner-chat",
            Uuid::new_v4(),
            "again",
            std::slice::from_ref(&doc_id)
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        crate::chat::persist_message(
            &pool,
            &channel,
            "owner-chat",
            Uuid::new_v4(),
            "",
            std::slice::from_ref(&short_id)
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::CONFLICT,
        "failed files cannot be sent"
    );
    let history =
        crate::chat::history_with(&pool, &channel, None, Some(owner.user.id), Some(&signer))
            .await
            .unwrap();
    let listed = &history["messages"][0]["content"]["attachments"];
    assert!(listed[0].get("url").is_none() && listed[0].get("previewUrl").is_none());

    // Worker routes need the shared secret.
    assert_eq!(
        worker_start(
            State(state.clone()),
            bearer("wrong-wrong-wrong-wrong-wrong-wrong"),
            Path(shot_id.clone())
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        worker_start(
            State(state.clone()),
            HeaderMap::new(),
            Path(shot_id.clone())
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::UNAUTHORIZED
    );
    let job = start_job(&shot_id).await.unwrap().0;
    assert_eq!(job["uploadByteSize"], 100);
    assert_eq!(job["declaredContentType"], "image/png");
    assert_eq!(job["filename"], "shot.png");
    assert_eq!(job["settings"]["imageAvifQuality"], 90);
    // SQS redelivery may claim again until the file finishes.
    assert!(start_job(&shot_id).await.is_ok());
    assert_eq!(
        start_job(&short_id).await.unwrap_err().status,
        StatusCode::CONFLICT
    );

    // A preview (video poster) shows up before processing finishes.
    assert_eq!(
        worker_preview(
            State(state.clone()),
            worker_headers.clone(),
            Path(shot_id.clone()),
            Json(
                serde_json::from_value(
                    json!({"contentType":"image/webp","byteSize":20,"width":40,"height":30})
                )
                .unwrap()
            ),
        )
        .await
        .unwrap(),
        StatusCode::NO_CONTENT
    );
    let events = attachment_events(&pool).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["messageId"], message_id.as_str());
    assert_eq!(events[0]["attachments"][0]["status"], "processing");
    assert_eq!(events[0]["attachments"][0]["preview"], json!({}));
    assert_eq!(events[0]["attachments"][0]["width"], 40);
    let payload = message_payload(&pool, &message_id).await;
    assert_eq!(payload["attachmentsSeq"], events[0]["seq"]);
    let signed = sign_attachments(events[0].clone(), Some(&signer));
    assert!(signed["attachments"][0]["previewUrl"].is_string());
    assert!(signed["attachments"][0].get("url").is_none());

    // Progress is ephemeral and best effort (no broker here).
    assert_eq!(
        worker_progress(
            State(state.clone()),
            worker_headers.clone(),
            Path(shot_id.clone()),
            Json(ProgressInput { percent: 50 }),
        )
        .await
        .unwrap(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(attachment_events(&pool).await.len(), 1);

    // Finishing stores the result, settles quota to the stored size and
    // updates the message.
    assert_eq!(
        finish_job(
            &shot_id,
            json!({"kind":"image","contentType":"image/avif","filename":"shot.avif","byteSize":60,"width":40,"height":30,"animated":false})
        )
        .await
        .unwrap(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        finish_job(
            &shot_id,
            json!({"kind":"image","contentType":"image/avif","filename":"shot.avif","byteSize":60})
        )
        .await
        .unwrap(),
        StatusCode::NO_CONTENT,
        "finish is idempotent"
    );
    assert_eq!(
        start_job(&shot_id).await.unwrap_err().status,
        StatusCode::CONFLICT
    );
    let _ = start_job(&doc_id).await.unwrap();
    assert_eq!(
        finish_job(
            &doc_id,
            json!({"kind":"file","contentType":"text/plain","filename":"notes.txt","byteSize":4,"contentEncoding":"gzip"})
        )
        .await
        .unwrap(),
        StatusCode::NO_CONTENT
    );
    for bad in [
        json!({"kind":"html","contentType":"text/html","filename":"x","byteSize":1}),
        json!({"kind":"file","contentType":"text/plain","filename":"x","byteSize":1,"contentEncoding":"br"}),
        json!({"kind":"file","contentType":"text/plain","filename":"x","byteSize":1,"preview":{"contentType":"image/svg+xml","byteSize":1}}),
    ] {
        assert_eq!(
            finish_job(&doc_id, bad).await.unwrap_err().status,
            StatusCode::BAD_REQUEST
        );
    }
    let events = attachment_events(&pool).await;
    assert_eq!(events.len(), 3);
    let last = &events[2]["attachments"];
    assert_eq!(
        last[0],
        json!({"id":shot_id,"kind":"image","contentType":"image/avif","name":"shot.avif","size":60,"status":"ready","width":40,"height":30,"preview":{}})
    );
    assert_eq!(last[1]["status"], "ready");
    let payload = message_payload(&pool, &message_id).await;
    assert_eq!(&payload["content"]["attachments"], last);
    assert_eq!(payload["attachmentsSeq"], events[2]["seq"]);
    let history =
        crate::chat::history_with(&pool, &channel, None, Some(owner.user.id), Some(&signer))
            .await
            .unwrap();
    let listed = &history["messages"][0]["content"]["attachments"];
    assert!(
        listed[0]["url"]
            .as_str()
            .unwrap()
            .contains(&format!("/original/{shot_id}?exp="))
    );
    assert!(listed[0]["previewUrl"].is_string());
    let used = usage(State(state.clone()), Extension(owner.clone()))
        .await
        .unwrap()
        .0;
    assert_eq!(
        used,
        json!({"used": 60 + 20 + 4 + 6, "limit": 1000, "maxUploadBytes": 950})
    );

    let refreshed = urls(
        State(state.clone()),
        Extension(owner.clone()),
        Json(UrlsInput {
            ids: vec![shot_id.clone(), short_id.clone()],
        }),
    )
    .await
    .unwrap()
    .0;
    assert!(refreshed["urls"][&shot_id]["url"].is_string());
    assert!(refreshed["urls"][&shot_id]["previewUrl"].is_string());
    assert!(refreshed["urls"].get(&short_id).is_none());
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

    // A worker failure marks the file failed in its message and releases quota.
    let (_, Json(clip)) = create_as(&owner, input("clip.mov", "video/quicktime", 50))
        .await
        .unwrap();
    put(&clip, &[1; 50]).await;
    let clip_id = clip["id"].as_str().unwrap().to_owned();
    let _ = complete_as(&owner, &clip_id).await.unwrap();
    let second = crate::chat::persist_message(
        &pool,
        &channel,
        "owner-chat",
        Uuid::new_v4(),
        "clip",
        std::slice::from_ref(&clip_id),
    )
    .await
    .unwrap();
    let _ = start_job(&clip_id).await.unwrap();
    assert_eq!(
        worker_fail(
            State(state.clone()),
            worker_headers.clone(),
            Path(clip_id.clone()),
            Json(FailInput {
                reason: "undecodable\nvideo".into()
            }),
        )
        .await
        .unwrap(),
        StatusCode::NO_CONTENT
    );
    let failed = message_payload(&pool, second["id"].as_str().unwrap()).await;
    assert_eq!(failed["content"]["attachments"][0]["status"], "failed");
    let reason: String =
        sqlx::query_scalar("SELECT failure FROM public.assets WHERE external_id=$1")
            .bind(&clip_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(reason, "undecodablevideo");
    assert_eq!(
        finish_job(
            &clip_id,
            json!({"kind":"video","contentType":"video/mp4","filename":"clip.mp4","byteSize":10})
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::CONFLICT,
        "a failed file cannot finish later"
    );

    // A job the worker went silent on times out through the sweeper.
    let (_, Json(stuck)) = create_as(&owner, input("stuck.png", "image/png", 9))
        .await
        .unwrap();
    put(&stuck, &[2; 9]).await;
    let stuck_id = stuck["id"].as_str().unwrap().to_owned();
    let _ = complete_as(&owner, &stuck_id).await.unwrap();
    let third = crate::chat::persist_message(
        &pool,
        &channel,
        "owner-chat",
        Uuid::new_v4(),
        "",
        std::slice::from_ref(&stuck_id),
    )
    .await
    .unwrap();
    sqlx::query(
        "UPDATE public.assets SET uploaded_at = now() - interval '31 minutes' WHERE external_id=$1",
    )
    .bind(&stuck_id)
    .execute(&pool)
    .await
    .unwrap();

    // The worker's R2 writes, so purging has something to remove.
    for id in [&shot_id, &doc_id, &clip_id, &stuck_id] {
        store
            .lock()
            .unwrap()
            .insert(original_key(id), b"stored".to_vec());
    }
    store
        .lock()
        .unwrap()
        .insert(preview_key(&shot_id), b"preview".to_vec());

    let wake = state.chat.as_ref().unwrap().wake.clone();
    purge(&assets, Some(wake.clone())).await.unwrap();
    let timed_out = message_payload(&pool, third["id"].as_str().unwrap()).await;
    assert_eq!(timed_out["content"]["attachments"][0]["status"], "failed");
    // Failed files lose their objects at once; ready ones stay.
    for gone in [&clip_id, &stuck_id] {
        assert!(!store.lock().unwrap().contains_key(&original_key(gone)));
    }
    assert!(store.lock().unwrap().contains_key(&original_key(&shot_id)));
    assert!(store.lock().unwrap().contains_key(&preview_key(&shot_id)));
    let after = usage(State(state.clone()), Extension(owner.clone()))
        .await
        .unwrap()
        .0;
    assert_eq!(after["used"], 60 + 20 + 4);

    // Abandoned uploads are purged; deleting a sent file hides it at once and
    // purges after the delay; rows stay.
    let (_, Json(stale)) = create_as(&owner, input("stale.png", "image/png", 8))
        .await
        .unwrap();
    sqlx::query(
        "UPDATE public.assets SET created_at = now() - interval '2 hours' WHERE external_id=$1",
    )
    .bind(stale["id"].as_str().unwrap())
    .execute(&pool)
    .await
    .unwrap();
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
    purge(&assets, Some(wake.clone())).await.unwrap();
    assert!(store.lock().unwrap().contains_key(&original_key(&doc_id)));
    let stale_deleted: bool = sqlx::query_scalar(
        "SELECT deleted_at IS NOT NULL AND purged_at IS NOT NULL FROM public.assets WHERE external_id=$1",
    )
    .bind(stale["id"].as_str().unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(stale_deleted);
    sqlx::query(
        "UPDATE public.assets SET deleted_at = now() - interval '25 hours' WHERE external_id=$1",
    )
    .bind(&doc_id)
    .execute(&pool)
    .await
    .unwrap();
    purge(&assets, Some(wake)).await.unwrap();
    assert!(!store.lock().unwrap().contains_key(&original_key(&doc_id)));
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM public.assets")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 6);

    // Scripted floods hit the pending and per-minute reservation limits.
    sqlx::query("UPDATE public.assets SET created_at = now() - interval '2 minutes'")
        .execute(&pool)
        .await
        .unwrap();
    let mut accepted = 0;
    let limited = loop {
        match create_as(&owner, input("tiny.txt", "text/plain", 1)).await {
            Ok(_) => accepted += 1,
            Err(error) => break error,
        }
    };
    assert_eq!(accepted, MAX_PENDING);
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);

    pool.close().await;
    admin
        .execute(format!("DROP DATABASE {database}").as_str())
        .await
        .unwrap();
}
