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
        "https://cdn.test/original/abc?exp=172800&sig=2GDNk1-0tzBzLEfmuQIvaGGFQ3ItbeJMj8gWdbnL8Ck"
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
    // A forward carries a snapshot of its source; its files are signed too.
    let forwarded = sign_attachments(
        json!({"type":"message.created","message":{"forward":{"message":{"content":{"attachments":[{"id":"src","kind":"image"}]}}}}}),
        Some(&signer),
    );
    assert!(
        forwarded["message"]["forward"]["message"]["content"]["attachments"][0]["url"]
            .as_str()
            .unwrap()
            .contains("/original/src?")
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
        json!({"imageQuality":92,"imageMaxEdge":4096,"paletteColors":256,"previewEdge":640,
               "videoMaxHeight":1080,"videoBitrateKbps":6000,"audioBitrateKbps":128})
    );
    let tuned = Compression::from_env(&E::from_values_for_test([
        ("ASSET_IMAGE_QUALITY", "80"),
        ("ASSET_VIDEO_MAX_HEIGHT", "0"),
        ("ASSET_PALETTE_COLORS", "0"),
    ]))
    .unwrap();
    assert_eq!(
        (
            tuned.image_quality,
            tuned.video_max_height,
            tuned.palette_colors
        ),
        (80, 0, 0)
    );
    for (name, value) in [
        ("ASSET_IMAGE_QUALITY", "0"),
        ("ASSET_IMAGE_QUALITY", "high"),
        ("ASSET_PALETTE_COLORS", "257"),
        ("ASSET_VIDEO_BITRATE_KBPS", "10"),
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

/// Minimal path-style S3 stand-in. It records objects but does not verify
/// signatures; live R2 validation is a separate deployment check.
async fn fake_r2() -> (String, Store) {
    let store: Store = Arc::default();
    let shared = store.clone();
    let app = Router::new().route(
        "/{bucket}/{*key}",
        get({
            let store = shared.clone();
            move |AxumPath((_, key)): AxumPath<(String, String)>, headers: HeaderMap| async move {
                let Some(body) = store.lock().unwrap().get(&key).cloned() else {
                    return StatusCode::NOT_FOUND.into_response();
                };
                let body = if headers.contains_key("range") {
                    body.into_iter().take(64).collect()
                } else {
                    body
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

    let (endpoint, store) = fake_r2().await;
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
    state.chat = Some(Chat {
        pool: pool.clone(),
        broker: redis::Client::open("redis://127.0.0.1:1").unwrap(),
        wake: Arc::new(tokio::sync::Notify::new()),
        cdn: Some(signer.clone()),
    });
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
    let message = crate::chat::send_message(
        &pool,
        &channel,
        "owner-chat",
        Uuid::new_v4(),
        "",
        &[shot_id.clone(), doc_id.clone()],
        None,
        false,
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
    assert_eq!(
        crate::chat::send_message(
            &pool,
            &channel,
            "owner-chat",
            Uuid::new_v4(),
            "again",
            std::slice::from_ref(&doc_id),
            None,
            false
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
            false
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
            .contains(&format!("/original/{doc_id}?exp="))
    );

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
        .execute(format!("DROP DATABASE {database}").as_str())
        .await
        .unwrap();
}
