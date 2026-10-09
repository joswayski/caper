use super::{credentials::testing, *};
use crate::{Cloudflare, Config, auth::random_id, presence};
use axum::{
    body::{Body, to_bytes},
    http::{Request, Version},
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    future::IntoFuture,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tower::ServiceExt;
use uuid::Uuid;

// ---- Mock APNs and FCM -----------------------------------------------------

struct Recorded {
    path: String,
    version: Version,
    headers: axum::http::HeaderMap,
    body: Value,
}

/// One loopback server plays APNs (HTTP/2 without TLS) or FCM and Google's
/// token endpoint. Replies are scripted in order; the default is 200.
#[derive(Default)]
struct Mock {
    requests: Mutex<Vec<Recorded>>,
    replies: Mutex<VecDeque<(u16, Value, Option<u64>)>>,
    assertions: Mutex<Vec<String>>,
}

impl Mock {
    fn reply(&self, status: u16, body: Value, retry_after: Option<u64>) {
        self.replies
            .lock()
            .unwrap()
            .push_back((status, body, retry_after));
    }

    fn take(&self) -> Vec<Recorded> {
        std::mem::take(&mut self.requests.lock().unwrap())
    }
}

async fn mock() -> (String, Arc<Mock>) {
    let mock = Arc::new(Mock::default());
    let router = Router::new()
        .route("/token", post(token_endpoint))
        .fallback(provider_endpoint)
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, router).into_future());
    (format!("http://{address}"), mock)
}

async fn token_endpoint(State(mock): State<Arc<Mock>>, body: String) -> Json<Value> {
    let mut assertions = mock.assertions.lock().unwrap();
    assert!(body.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer"));
    assertions.push(
        body.split('&')
            .find_map(|pair| pair.strip_prefix("assertion="))
            .unwrap()
            .to_owned(),
    );
    Json(json!({
        "access_token": format!("access-{}", assertions.len()),
        "expires_in": 3599,
        "token_type": "Bearer",
    }))
}

async fn provider_endpoint(
    State(mock): State<Arc<Mock>>,
    request: axum::extract::Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 64 * 1024).await.unwrap();
    mock.requests.lock().unwrap().push(Recorded {
        path: parts.uri.path().to_owned(),
        version: parts.version,
        headers: parts.headers,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    });
    let (status, body, retry_after) =
        mock.replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or((200, json!({}), None));
    let mut response = (StatusCode::from_u16(status).unwrap(), Json(body)).into_response();
    if let Some(seconds) = retry_after {
        response
            .headers_mut()
            .insert("retry-after", seconds.to_string().parse().unwrap());
    }
    response
}

fn service_account(base: &str, private_key: &str) -> String {
    json!({
        "type": "service_account",
        "project_id": "caper-test",
        "private_key_id": "key-1",
        "private_key": private_key,
        "client_email": "caper-push@caper-test.iam.gserviceaccount.com",
        "token_uri": format!("{base}/token"),
    })
    .to_string()
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn channel_alert(text: &str) -> Alert {
    Alert::new(
        "mention.user",
        "message00000001",
        Conversation::Channel {
            space_id: "space0000001".into(),
            channel_id: "chan00000001".into(),
            title: "#general (Studio)".into(),
        },
        "Alice",
        "alice0000001",
        text,
    )
}

const APNS_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

// ---- Content and configuration ---------------------------------------------

#[test]
fn previews_trim_truncate_to_180_characters_and_name_empty_messages() {
    assert_eq!(preview("  hello \n"), "hello");
    assert_eq!(preview(" \n "), "Sent a message");
    let exact = "é".repeat(180);
    assert_eq!(preview(&exact), exact);
    let long = format!("{} tail", "🙂".repeat(200));
    let cut = preview(&long);
    assert_eq!(cut.chars().count(), 180);
    assert!(cut.ends_with("🙂…"));
    // No space before the ellipsis when the cut lands after one.
    let spaced = format!("{} b", "a".repeat(178));
    assert_eq!(
        preview(&format!("{spaced}{}", "c".repeat(10))),
        format!("{}…", "a".repeat(178))
    );
    let alert = channel_alert("hi");
    assert_eq!(alert.title, "Alice · #general (Studio)");
    let direct = Alert::new(
        "direct.message",
        "m",
        Conversation::Direct {
            id: "dm0000000001".into(),
        },
        "Alice",
        "a",
        "",
    );
    assert_eq!(
        (direct.title.as_str(), direct.body.as_str(), direct.thread()),
        ("Alice", "Sent a message", "dm0000000001")
    );
}

#[test]
fn platforms_need_the_flag_the_list_and_credentials_that_load() {
    let (p8, _) = testing::p8();
    let (rsa, _) = testing::rsa();
    let escaped = p8.replace('\n', "\\n");
    let account = service_account("https://oauth2.example", &rsa);
    let names = |push: &Push| {
        push.platforms()
            .iter()
            .map(|p| p.name())
            .collect::<Vec<_>>()
    };
    let listed = RuntimeEnvironment::from_values_for_test([
        ("NOTIFICATIONS_ENABLED", "true"),
        ("PUSH_PLATFORMS", " fcm, apns,apnsSandbox, webpush"),
        ("APNS_TEAM_ID", "TEAM123456"),
        ("APNS_KEY_ID", "KEY1234567"),
        ("APNS_PRIVATE_KEY", &escaped),
        ("FCM_SERVICE_ACCOUNT_JSON", &account),
    ]);
    let push = Push::from_env(&listed);
    assert!(push.enabled());
    // apnsSandbox is listed but has no key of its own.
    assert_eq!(names(&push), ["apns", "fcm"]);
    let sandbox = RuntimeEnvironment::from_values_for_test([
        ("NOTIFICATIONS_ENABLED", "1"),
        ("PUSH_PLATFORMS", "apnsSandbox,fcm"),
        ("APNS_TEAM_ID", "TEAM123456"),
        ("APNS_SANDBOX_KEY_ID", "KEY7654321"),
        ("APNS_SANDBOX_PRIVATE_KEY", &p8),
        ("FCM_SERVICE_ACCOUNT_JSON", "{\"project_id\":\"caper\"}"),
    ]);
    assert_eq!(names(&Push::from_env(&sandbox)), ["apnsSandbox"]);
    let off = RuntimeEnvironment::from_values_for_test([
        ("PUSH_PLATFORMS", "apns,fcm"),
        ("APNS_TEAM_ID", "TEAM123456"),
        ("APNS_KEY_ID", "KEY1234567"),
        ("APNS_PRIVATE_KEY", &p8),
    ]);
    assert!(!Push::from_env(&off).enabled());
    assert!(Push::from_env(&off).platforms().is_empty());
    let unlisted = RuntimeEnvironment::from_values_for_test([
        ("NOTIFICATIONS_ENABLED", "true"),
        ("APNS_TEAM_ID", "TEAM123456"),
        ("APNS_KEY_ID", "KEY1234567"),
        ("APNS_PRIVATE_KEY", &p8),
    ]);
    assert!(Push::from_env(&unlisted).platforms().is_empty());
}

#[test]
fn device_input_checks_platform_token_shape_and_app_id() {
    let all = Platform::ALL;
    let error =
        |body: Value, platforms: &[Platform]| device_input(&body, platforms).unwrap_err().message;
    assert_eq!(
        device_input(&json!({"platform": "apns", "token": APNS_TOKEN.to_uppercase(), "appId": "chat.caper.ios"}), &all).unwrap(),
        Device { platform: Platform::Apns, token: APNS_TOKEN.into(), app_id: "chat.caper.ios".into() }
    );
    assert_eq!(
        error(json!({"platform": "fcm", "token": "t"}), &[Platform::Apns]),
        "push platform unavailable"
    );
    assert_eq!(
        error(json!({"platform": "webpush", "token": "t"}), &all),
        "push platform unavailable"
    );
    assert_eq!(
        error(json!({"token": "t"}), &all),
        "push platform unavailable"
    );
    for token in [
        json!("ab".repeat(31)),
        json!("zz".repeat(32)),
        json!("a".repeat(201)),
        json!(42),
    ] {
        assert_eq!(
            error(json!({"platform": "apnsSandbox", "token": token}), &all),
            "invalid push token"
        );
    }
    assert!(device_input(&json!({"platform": "apns", "token": "a".repeat(200)}), &all).is_ok());
    for token in ["", "has space", "tab\t", "é"] {
        assert_eq!(
            error(json!({"platform": "fcm", "token": token}), &all),
            "invalid push token"
        );
    }
    assert_eq!(
        error(json!({"platform": "fcm", "token": "x".repeat(4097)}), &all),
        "invalid push token"
    );
    assert!(
        device_input(
            &json!({"platform": "fcm", "token": "x:y".repeat(1365), "appId": null}),
            &all
        )
        .is_ok()
    );
    assert_eq!(
        error(
            json!({"platform": "fcm", "token": "t", "appId": "a b"}),
            &all
        ),
        "invalid app id"
    );
}

#[tokio::test]
async fn config_requires_an_account_and_lists_only_available_platforms() {
    let (rsa, _) = testing::rsa();
    let fcm_only = Push::with_providers(Providers {
        fcm: Some(
            fcm::Fcm::new(
                "http://127.0.0.1:9",
                &service_account("http://127.0.0.1:9", &rsa),
            )
            .unwrap(),
        ),
        ..Providers::default()
    });
    for (auth_fixture, push, expected) in [
        (false, Push::default(), (StatusCode::UNAUTHORIZED, None)),
        (
            true,
            Push::default(),
            (StatusCode::OK, Some(json!({"platforms": []}))),
        ),
        (
            true,
            fcm_only,
            (StatusCode::OK, Some(json!({"platforms": ["fcm"]}))),
        ),
    ] {
        let mut config = Config::test(false);
        config.auth_fixture = auth_fixture;
        let mut state = AppState::new(config, Arc::new(Cloudflare::new()));
        state.push = push;
        let response = crate::app(state)
            .oneshot(
                Request::builder()
                    .uri("/api/push/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(
            (
                status,
                expected
                    .1
                    .is_some()
                    .then(|| serde_json::from_slice::<Value>(&body).unwrap())
            ),
            expected
        );
    }
}

// ---- Providers against mock servers ----------------------------------------

#[tokio::test]
async fn apns_sends_http2_alerts_with_a_cached_es256_token_and_classifies_replies() {
    let (base, mock) = mock().await;
    let (p8, public_key) = testing::p8();
    let apns = apns::Apns::new(
        &base,
        "chat.caper.ios",
        "TEAM123456",
        "KEY1234567",
        &p8.replace('\n', "\\n"),
    )
    .unwrap();
    let alert = channel_alert("Hello @bob");
    assert_eq!(apns.send(APNS_TOKEN, &alert).await, Outcome::Delivered);
    assert_eq!(apns.send(APNS_TOKEN, &alert).await, Outcome::Delivered);
    let requests = mock.take();
    assert_eq!(requests.len(), 2);
    let first = &requests[0];
    assert_eq!(first.version, Version::HTTP_2);
    assert_eq!(first.path, format!("/3/device/{APNS_TOKEN}"));
    let header = |name: &str| first.headers[name].to_str().unwrap().to_owned();
    assert_eq!(header("apns-topic"), "chat.caper.ios");
    assert_eq!(header("apns-push-type"), "alert");
    assert_eq!(header("apns-priority"), "10");
    let expiration: i64 = header("apns-expiration").parse().unwrap();
    assert!((expiration - now() - 86_400).abs() <= 5, "{expiration}");
    let bearer = header("authorization");
    let jwt = bearer.strip_prefix("bearer ").unwrap();
    let (jwt_header, claims) = testing::verify(jwt, &public_key, true).unwrap();
    assert_eq!(jwt_header, json!({"alg": "ES256", "kid": "KEY1234567"}));
    assert_eq!(claims["iss"], "TEAM123456");
    assert!((claims["iat"].as_i64().unwrap() - now()).abs() <= 5);
    assert_eq!(
        requests[1].headers["authorization"],
        bearer.as_str(),
        "the provider token is reused"
    );
    assert_eq!(
        first.body,
        json!({
            "aps": {"alert": {"title": "Alice · #general (Studio)", "body": "Hello @bob"}, "sound": "default", "thread-id": "chan00000001"},
            "kind": "mention.user", "messageId": "message00000001", "spaceId": "space0000001", "channelId": "chan00000001",
        })
    );

    let direct = Alert::new(
        "direct.message",
        "message00000002",
        Conversation::Direct {
            id: "dm0000000001".into(),
        },
        "Alice",
        "alice0000001",
        "hi",
    );
    apns.send(APNS_TOKEN, &direct).await;
    let body = &mock.take()[0].body;
    assert_eq!(body["conversationId"], "dm0000000001");
    assert_eq!(body["aps"]["thread-id"], "dm0000000001");
    assert!(body.get("spaceId").is_none() && body.get("channelId").is_none());

    for (status, reason, retry_after, expected) in [
        (
            410,
            "Unregistered",
            None,
            Outcome::Revoke("apns 410 Unregistered".into()),
        ),
        (
            400,
            "BadDeviceToken",
            None,
            Outcome::Revoke("apns 400 BadDeviceToken".into()),
        ),
        (
            429,
            "TooManyRequests",
            Some(7),
            Outcome::Retry {
                error: "apns 429 TooManyRequests".into(),
                after: Some(Duration::from_secs(7)),
            },
        ),
        (
            503,
            "ServiceUnavailable",
            None,
            Outcome::Retry {
                error: "apns 503 ServiceUnavailable".into(),
                after: None,
            },
        ),
        (
            400,
            "PayloadTooLarge",
            None,
            Outcome::Abandon("apns 400 PayloadTooLarge".into()),
        ),
    ] {
        mock.reply(status, json!({"reason": reason}), retry_after);
        assert_eq!(apns.send(APNS_TOKEN, &alert).await, expected);
    }
    mock.take();
    // An expired provider token is replaced before the retry.
    mock.reply(403, json!({"reason": "ExpiredProviderToken"}), None);
    assert!(matches!(
        apns.send(APNS_TOKEN, &alert).await,
        Outcome::Retry { .. }
    ));
    assert_eq!(apns.send(APNS_TOKEN, &alert).await, Outcome::Delivered);
    let requests = mock.take();
    assert_eq!(requests[0].headers["authorization"], bearer.as_str());
    assert_ne!(requests[1].headers["authorization"], bearer.as_str());

    // Nothing listening: a network failure is retried.
    let gone = apns::Apns::new(
        "http://127.0.0.1:9",
        "chat.caper.ios",
        "TEAM123456",
        "KEY1234567",
        &p8,
    )
    .unwrap();
    assert!(
        matches!(gone.send(APNS_TOKEN, &alert).await, Outcome::Retry { error, .. } if error == "apns network")
    );
}

#[tokio::test]
async fn fcm_sends_data_messages_with_a_cached_oauth_token_and_classifies_replies() {
    let (base, mock) = mock().await;
    let (rsa, public_key) = testing::rsa();
    let fcm = fcm::Fcm::new(&base, &service_account(&base, &rsa.replace('\n', "\\n"))).unwrap();
    let alert = channel_alert(&"x".repeat(300));
    assert_eq!(fcm.send("fcm:token", &alert).await, Outcome::Delivered);
    assert_eq!(fcm.send("fcm:token", &alert).await, Outcome::Delivered);
    let assertions = mock.assertions.lock().unwrap().clone();
    assert_eq!(assertions.len(), 1, "the access token is cached");
    let (header, claims) = testing::verify(&assertions[0], &public_key, false).unwrap();
    assert_eq!(
        header,
        json!({"alg": "RS256", "typ": "JWT", "kid": "key-1"})
    );
    assert_eq!(
        claims["iss"],
        "caper-push@caper-test.iam.gserviceaccount.com"
    );
    assert_eq!(
        claims["scope"],
        "https://www.googleapis.com/auth/firebase.messaging"
    );
    assert_eq!(claims["aud"], format!("{base}/token"));
    assert_eq!(
        claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
        3600
    );
    let requests = mock.take();
    assert_eq!(requests[0].path, "/v1/projects/caper-test/messages:send");
    assert_eq!(requests[0].headers["authorization"], "Bearer access-1");
    assert_eq!(
        requests[0].body,
        json!({"message": {
            "token": "fcm:token",
            "data": {
                "kind": "mention.user", "messageId": "message00000001",
                "title": "Alice · #general (Studio)", "body": format!("{}…", "x".repeat(179)),
                "sender": "Alice", "senderId": "alice0000001",
                "spaceId": "space0000001", "channelId": "chan00000001", "conversationTitle": "#general (Studio)",
            },
            "android": {"priority": "HIGH", "ttl": "86400s", "collapse_key": "chan00000001"},
        }})
    );
    let direct = Alert::new(
        "direct.message",
        "message00000002",
        Conversation::Direct {
            id: "dm0000000001".into(),
        },
        "Alice",
        "alice0000001",
        "hi",
    );
    fcm.send("fcm:token", &direct).await;
    let data = mock.take()[0].body["message"]["data"].clone();
    assert_eq!(data["conversationId"], "dm0000000001");
    assert!(data.get("conversationTitle").is_none() && data.get("spaceId").is_none());

    let error = |status: u16, code: &str, field: Option<&str>| {
        let mut details = vec![
            json!({"@type": "type.googleapis.com/google.firebase.fcm.v1.FcmError", "errorCode": code}),
        ];
        if let Some(field) = field {
            details.push(json!({"@type": "type.googleapis.com/google.rpc.BadRequest", "fieldViolations": [{"field": field}]}));
        }
        json!({"error": {"code": status, "status": code, "details": details}})
    };
    for (status, body, retry_after, expected) in [
        (
            404,
            error(404, "UNREGISTERED", None),
            None,
            Outcome::Revoke("fcm 404 UNREGISTERED".into()),
        ),
        (
            400,
            error(400, "INVALID_ARGUMENT", Some("message.token")),
            None,
            Outcome::Revoke("fcm 400 INVALID_ARGUMENT".into()),
        ),
        (
            400,
            error(400, "INVALID_ARGUMENT", Some("message.android.ttl")),
            None,
            Outcome::Abandon("fcm 400 INVALID_ARGUMENT".into()),
        ),
        (
            429,
            error(429, "QUOTA_EXCEEDED", None),
            Some(30),
            Outcome::Retry {
                error: "fcm 429 QUOTA_EXCEEDED".into(),
                after: Some(Duration::from_secs(30)),
            },
        ),
        (
            503,
            error(503, "UNAVAILABLE", None),
            None,
            Outcome::Retry {
                error: "fcm 503 UNAVAILABLE".into(),
                after: None,
            },
        ),
    ] {
        mock.reply(status, body, retry_after);
        assert_eq!(fcm.send("fcm:token", &alert).await, expected);
    }
    // A rejected access token is refreshed for the retry.
    mock.reply(401, json!({"error": {"status": "UNAUTHENTICATED"}}), None);
    assert!(matches!(
        fcm.send("fcm:token", &alert).await,
        Outcome::Retry { .. }
    ));
    assert_eq!(fcm.send("fcm:token", &alert).await, Outcome::Delivered);
    assert_eq!(mock.assertions.lock().unwrap().len(), 2);
    assert_eq!(
        mock.take().last().unwrap().headers["authorization"],
        "Bearer access-2"
    );
}

// ---- Database and broker integration ----------------------------------------

struct Person {
    id: i64,
    external_id: String,
    token: String,
    hash: Vec<u8>,
    chat: String,
}

struct Harness {
    app: Router,
    workers: Workers,
    presence: Presence,
    apns: Arc<Mock>,
    fcm: Arc<Mock>,
}

async fn harness(pool: &PgPool) -> Harness {
    let (apns_base, apns_mock) = mock().await;
    let (fcm_base, fcm_mock) = mock().await;
    let (p8, _) = testing::p8();
    let (rsa, _) = testing::rsa();
    let push = Push::with_providers(Providers {
        apns: Some(
            apns::Apns::new(
                &apns_base,
                "chat.caper.ios",
                "TEAM123456",
                "KEY1234567",
                &p8,
            )
            .unwrap(),
        ),
        apns_sandbox: None,
        fcm: Some(fcm::Fcm::new(&fcm_base, &service_account(&fcm_base, &rsa)).unwrap()),
    });
    let broker = redis::Client::open(std::env::var("CHAT_TEST_VALKEY_URL").unwrap()).unwrap();
    let mut config = Config::test(false);
    config.auth_fixture = false;
    let mut state =
        AppState::with_database(config, Arc::new(Cloudflare::new()), Some(pool.clone()));
    state.chat = Some(Chat::new(pool.clone(), broker.clone()));
    state.push = push.clone();
    Harness {
        app: crate::app(state),
        workers: Workers::new(
            pool.clone(),
            push,
            Some(broker.clone()),
            Duration::from_secs(600),
        ),
        presence: Presence::new(&broker, Duration::from_secs(600))
            .await
            .unwrap(),
        apns: apns_mock,
        fcm: fcm_mock,
    }
}

async fn person(pool: &PgPool, name: &str) -> Person {
    let external_id = random_id(12);
    let mut display = name.to_owned();
    display[..1].make_ascii_uppercase();
    let id: i64 = sqlx::query_scalar("INSERT INTO public.users (external_id,username,display_name) VALUES ($1,$2,$3) RETURNING id")
        .bind(&external_id).bind(name).bind(&display).fetch_one(pool).await.unwrap();
    let (token, hash) = session(pool, id).await;
    let chat = format!("chat-{name}-{}", Uuid::new_v4());
    sqlx::query("INSERT INTO public.chat_sessions (external_id,token_hash,user_id,account_session_hash,name) VALUES ($1,$2,$3,$4,$5)")
        .bind(random_id(12)).bind(Sha256::digest(chat.as_bytes()).to_vec()).bind(id).bind(&hash).bind(&display)
        .execute(pool).await.unwrap();
    Person {
        id,
        external_id,
        token,
        hash,
        chat,
    }
}

/// Another sign-in for the same account, as a second device would have.
async fn session(pool: &PgPool, user: i64) -> (String, Vec<u8>) {
    let token = format!("account-{}", Uuid::new_v4());
    let hash = Sha256::digest(token.as_bytes()).to_vec();
    sqlx::query("INSERT INTO public.account_sessions (token_hash,user_id,expires_at) VALUES ($1,$2,now()+interval '1 day')")
        .bind(&hash).bind(user).execute(pool).await.unwrap();
    (token, hash)
}

async fn space(pool: &PgPool, owner: &Person, members: &[&Person]) -> (i64, String) {
    let external = random_id(12);
    let id: i64 = sqlx::query_scalar("INSERT INTO public.spaces (external_id,name,owner_id) VALUES ($1,'Studio',$2) RETURNING id")
        .bind(&external).bind(owner.id).fetch_one(pool).await.unwrap();
    for member in std::iter::once(&owner).chain(members) {
        sqlx::query("INSERT INTO public.space_members (space_id,user_id) VALUES ($1,$2)")
            .bind(id)
            .bind(member.id)
            .execute(pool)
            .await
            .unwrap();
    }
    (id, external)
}

async fn channel(
    pool: &PgPool,
    space: i64,
    name: &str,
    private: bool,
    joined: &[&Person],
) -> String {
    let external = random_id(12);
    let id: i64 = sqlx::query_scalar("INSERT INTO public.channels (external_id,space_id,name,private) VALUES ($1,$2,$3,$4) RETURNING id")
        .bind(&external).bind(space).bind(name).bind(private).fetch_one(pool).await.unwrap();
    for person in joined {
        sqlx::query("INSERT INTO public.channel_joins (channel_id,user_id) VALUES ($1,$2)")
            .bind(id)
            .bind(person.id)
            .execute(pool)
            .await
            .unwrap();
    }
    external
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    account: Option<&str>,
    chat: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(account) = account {
        request = request.header("authorization", format!("Bearer {account}"));
    }
    if let Some(chat) = chat {
        request = request.header("x-caper-chat-token", chat);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 256 * 1024).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

async fn send(
    h: &Harness,
    author: &Person,
    channel: &str,
    text: &str,
    root: Option<&str>,
) -> String {
    let mut body = json!({"clientMessageId": Uuid::new_v4(), "text": text});
    if let Some(root) = root {
        body["threadRootId"] = json!(root);
    }
    let (status, message) = call(
        &h.app,
        "POST",
        &format!("/api/chat/channels/{channel}/messages"),
        None,
        Some(&author.chat),
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{message}");
    message["id"].as_str().unwrap().to_owned()
}

async fn direct(h: &Harness, from: &Person, username: &str) -> String {
    let (status, conversation) = call(
        &h.app,
        "POST",
        "/api/dms",
        Some(&from.token),
        None,
        json!({"username": username}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{conversation}");
    conversation["id"].as_str().unwrap().to_owned()
}

async fn expand(h: &Harness) {
    while expansion::expand_pending(&h.workers).await.unwrap() {}
}

async fn deliver(h: &Harness) {
    while delivery::deliver_pending(&h.workers).await.unwrap() {}
}

/// `(username, kind)` of everyone notified about a message.
async fn notified(pool: &PgPool, message: &str) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT u.username, n.kind FROM public.notifications n JOIN public.users u ON u.id=n.user_id
         JOIN public.messages m ON m.id=n.message_id WHERE m.external_id=$1 ORDER BY u.username",
    )
    .bind(message)
    .fetch_all(pool)
    .await
    .unwrap()
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

async fn register(h: &Harness, person: &Person, token: &str, platform: &str, device: &str) {
    let (status, body) = call(
        &h.app,
        "POST",
        "/api/push/devices",
        Some(token),
        None,
        json!({"platform": platform, "token": device}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "{} {body}",
        person.external_id
    );
}

/// Marks a gateway connection on the given session as in use right now.
async fn active(h: &Harness, person: &Person, session_hash: &[u8]) -> String {
    let connection = presence::connection_id(Some(session_hash));
    h.presence
        .renew(&person.external_id, &connection, 0)
        .await
        .unwrap();
    connection
}

async fn clear_presence(person: &Person) {
    let broker = redis::Client::open(std::env::var("CHAT_TEST_VALKEY_URL").unwrap()).unwrap();
    let mut connection = broker.get_multiplexed_async_connection().await.unwrap();
    redis::cmd("DEL")
        .arg(format!(
            "caper:presence:v1:{{{}}}:sessions",
            person.external_id
        ))
        .query_async::<()>(&mut connection)
        .await
        .unwrap();
}

async fn post_device(app: &Router, token: &str, body: Value) -> (StatusCode, Value) {
    call(app, "POST", "/api/push/devices", Some(token), None, body).await
}

async fn get_settings(app: &Router, token: &str) -> (StatusCode, Value) {
    call(
        app,
        "GET",
        "/api/notifications/settings",
        Some(token),
        None,
        Value::Null,
    )
    .await
}

async fn put_json(app: &Router, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
    call(app, "PUT", path, Some(token), None, body).await
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn device_settings_and_override_routes_validate_and_logout_revokes(pool: PgPool) {
    let h = harness(&pool).await;
    let alice = person(&pool, "alice").await;
    let bob = person(&pool, "bob").await;
    let carol = person(&pool, "carol").await;
    let (space_id, space_external) = space(&pool, &alice, &[&bob]).await;
    let general = channel(&pool, space_id, "general", false, &[&alice, &bob]).await;
    let secret = channel(&pool, space_id, "secret", true, &[&alice]).await;
    let (other_space, _) = space(&pool, &carol, &[&alice]).await;
    let elsewhere = channel(&pool, other_space, "elsewhere", false, &[&carol]).await;
    let app = &h.app;

    assert_eq!(
        call(app, "GET", "/api/push/config", None, None, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            app,
            "GET",
            "/api/push/config",
            Some(&alice.token),
            None,
            Value::Null
        )
        .await,
        (StatusCode::OK, json!({"platforms": ["apns", "fcm"]}))
    );
    let devices = |user: i64| {
        let pool = pool.clone();
        async move {
            sqlx::query_as::<_, (String, String, String, Option<String>)>(
                "SELECT transport,address,app_id,revoked_reason FROM public.notification_devices WHERE user_id=$1 ORDER BY id",
            )
            .bind(user)
            .fetch_all(&pool)
            .await
            .unwrap()
        }
    };
    assert_eq!(
        post_device(
            app,
            &alice.token,
            json!({"platform": "apnsSandbox", "token": APNS_TOKEN})
        )
        .await,
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "push platform unavailable"})
        )
    );
    assert_eq!(
        post_device(
            app,
            &alice.token,
            json!({"platform": "apns", "token": "abc"})
        )
        .await,
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "invalid push token"})
        )
    );
    assert_eq!(
        call(
            app,
            "POST",
            "/api/push/devices",
            None,
            None,
            json!({"platform": "fcm", "token": "t"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    for _ in 0..2 {
        assert_eq!(post_device(app, &alice.token, json!({"platform": "apns", "token": APNS_TOKEN.to_uppercase(), "appId": "chat.caper.ios"})).await.0, StatusCode::NO_CONTENT);
    }
    assert_eq!(
        devices(alice.id).await,
        vec![(
            "apns".into(),
            APNS_TOKEN.into(),
            "chat.caper.ios".into(),
            None
        )],
        "re-registering is idempotent"
    );
    // Bob registering the same address takes it over.
    assert_eq!(
        post_device(
            app,
            &bob.token,
            json!({"platform": "apns", "token": APNS_TOKEN})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(devices(alice.id).await[0].3.as_deref(), Some("replaced"));
    // A new token for the same session replaces the previous registration.
    post_device(
        app,
        &alice.token,
        json!({"platform": "fcm", "token": "fcm:alice-1"}),
    )
    .await;
    post_device(
        app,
        &alice.token,
        json!({"platform": "fcm", "token": "fcm:alice-2"}),
    )
    .await;
    let rows = devices(alice.id).await;
    assert_eq!(
        rows.iter()
            .map(|row| (row.1.as_str(), row.3.as_deref()))
            .collect::<Vec<_>>(),
        [
            (APNS_TOKEN, Some("replaced")),
            ("fcm:alice-1", Some("replaced")),
            ("fcm:alice-2", None)
        ]
    );
    // A second sign-in keeps its own registration.
    let (alice_phone, _) = session(&pool, alice.id).await;
    post_device(
        app,
        &alice_phone,
        json!({"platform": "fcm", "token": "fcm:alice-phone"}),
    )
    .await;
    // DELETE revokes only the caller's own device, idempotently, even for a platform no longer advertised.
    assert_eq!(
        call(
            app,
            "DELETE",
            "/api/push/devices",
            Some(&carol.token),
            None,
            json!({"platform": "apns", "token": APNS_TOKEN})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(devices(bob.id).await[0].3, None);
    for _ in 0..2 {
        assert_eq!(
            call(
                app,
                "DELETE",
                "/api/push/devices",
                Some(&alice.token),
                None,
                json!({"platform": "fcm", "token": "fcm:alice-2"})
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(devices(alice.id).await[2].3.as_deref(), Some("deleted"));
    assert_eq!(
        call(
            app,
            "DELETE",
            "/api/push/devices",
            Some(&alice.token),
            None,
            json!({"platform": "apnsSandbox", "token": APNS_TOKEN})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );

    // Account settings.
    assert_eq!(
        get_settings(app, &alice.token).await,
        (
            StatusCode::OK,
            json!({"level": "all", "mobile": "whenInactive", "overrides": []})
        )
    );
    for (body, error) in [
        (
            json!({"level": null}),
            "level must be all, mentions or nothing",
        ),
        (
            json!({"mobile": "never"}),
            "mobile must be always or whenInactive",
        ),
        (
            json!({"level": "all", "other": 1}),
            "invalid notification settings",
        ),
    ] {
        assert_eq!(
            put_json(app, "/api/notifications/settings", &alice.token, body).await,
            (StatusCode::BAD_REQUEST, json!({"error": error}))
        );
    }
    assert_eq!(
        put_json(
            app,
            "/api/notifications/settings",
            &alice.token,
            json!({"level": "mentions"})
        )
        .await
        .1,
        json!({"level": "mentions", "mobile": "whenInactive", "overrides": []})
    );
    assert_eq!(
        put_json(
            app,
            "/api/notifications/settings",
            &alice.token,
            json!({"mobile": "always"})
        )
        .await
        .1,
        json!({"level": "mentions", "mobile": "always", "overrides": []})
    );

    // Overrides.
    let space_path = format!("/api/spaces/{space_external}/notifications");
    let channel_path =
        |channel: &str| format!("/api/spaces/{space_external}/channels/{channel}/notifications");
    let soon = chrono::DateTime::from_timestamp(now() + 3600, 0).unwrap();
    let soon_text = soon.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    assert_eq!(
        put_json(app, &space_path, &alice.token, json!({"level": "mentions"})).await,
        (
            StatusCode::OK,
            json!({"spaceId": space_external, "level": "mentions", "mutedUntil": null})
        )
    );
    assert_eq!(
        put_json(
            app,
            &space_path,
            &alice.token,
            json!({"mutedUntil": soon.to_rfc3339()})
        )
        .await
        .1,
        json!({"spaceId": space_external, "level": "mentions", "mutedUntil": soon_text})
    );
    assert_eq!(
        put_json(app, &space_path, &alice.token, json!({"level": "loud"})).await,
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "level must be all, mentions, nothing or null"})
        )
    );
    assert_eq!(
        put_json(
            app,
            &space_path,
            &alice.token,
            json!({"mutedUntil": "2020-01-01T00:00:00Z"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        put_json(app, &space_path, &carol.token, json!({})).await,
        (StatusCode::NOT_FOUND, json!({"error": "space not found"}))
    );
    assert_eq!(
        put_json(
            app,
            &channel_path(&general),
            &bob.token,
            json!({"mutedUntil": "forever"})
        )
        .await
        .1,
        json!({"spaceId": space_external, "channelId": general, "level": null, "mutedUntil": "forever"})
    );
    assert_eq!(
        put_json(
            app,
            &channel_path(&secret),
            &bob.token,
            json!({"level": "all"})
        )
        .await,
        (StatusCode::NOT_FOUND, json!({"error": "channel not found"})),
        "no grant"
    );
    assert_eq!(
        put_json(
            app,
            &channel_path(&secret),
            &alice.token,
            json!({"level": "all"})
        )
        .await
        .0,
        StatusCode::OK,
        "the owner can read it"
    );
    assert_eq!(
        put_json(app, &channel_path(&elsewhere), &alice.token, json!({}))
            .await
            .0,
        StatusCode::NOT_FOUND,
        "a channel of another space"
    );
    let conversation = direct(&h, &alice, "bob").await;
    let dm_path = format!("/api/dms/{conversation}/notifications");
    assert_eq!(
        put_json(app, &dm_path, &alice.token, json!({"level": "mentions"})).await,
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "level must be nothing or null"})
        )
    );
    assert_eq!(
        put_json(app, &dm_path, &carol.token, json!({})).await,
        (
            StatusCode::NOT_FOUND,
            json!({"error": "conversation not found"})
        )
    );
    assert_eq!(
        put_json(
            app,
            &format!("/api/dms/{general}/notifications"),
            &alice.token,
            json!({})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        put_json(
            app,
            &dm_path,
            &alice.token,
            json!({"level": "nothing", "mutedUntil": "forever"})
        )
        .await
        .1,
        json!({"conversationId": conversation, "level": "nothing", "mutedUntil": "forever"})
    );
    assert_eq!(
        get_settings(app, &alice.token).await.1["overrides"],
        json!([
            {"spaceId": space_external, "level": "mentions", "mutedUntil": soon_text},
            {"spaceId": space_external, "channelId": secret, "level": "all", "mutedUntil": null},
            {"conversationId": conversation, "level": "nothing", "mutedUntil": "forever"},
        ])
    );
    // Null resets; an override with nothing left, or an expired mute, is omitted.
    assert_eq!(
        put_json(
            app,
            &dm_path,
            &alice.token,
            json!({"level": null, "mutedUntil": null})
        )
        .await
        .1,
        json!({"conversationId": conversation, "level": null, "mutedUntil": null})
    );
    sqlx::query("UPDATE public.notification_overrides SET muted_until=now()-interval '1 minute' WHERE user_id=$1 AND channel_id IS NOT NULL")
        .bind(bob.id).execute(&pool).await.unwrap();
    assert_eq!(
        get_settings(app, &bob.token).await.1["overrides"],
        json!([])
    );
    // Scopes the account can no longer read are omitted.
    sqlx::query(
        "UPDATE public.space_members SET deleted_at=now() WHERE user_id=$1 AND space_id=$2",
    )
    .bind(alice.id)
    .bind(space_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        get_settings(app, &alice.token).await.1["overrides"],
        json!([])
    );

    // Logout revokes the session's registrations, and only that session's.
    assert_eq!(
        call(
            app,
            "POST",
            "/api/auth/logout",
            Some(&alice_phone),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let rows = devices(alice.id).await;
    assert_eq!(rows.last().unwrap().3.as_deref(), Some("logout"));
    assert_eq!(
        call(
            app,
            "POST",
            "/api/auth/logout",
            Some(&bob.token),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(devices(bob.id).await[0].3.as_deref(), Some("logout"));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn sends_and_forwards_enqueue_in_their_transaction_and_edits_do_not(pool: PgPool) {
    let h = harness(&pool).await;
    let alice = person(&pool, "alice").await;
    let bob = person(&pool, "bob").await;
    let (space_id, _) = space(&pool, &alice, &[&bob]).await;
    let general = channel(&pool, space_id, "general", false, &[&alice, &bob]).await;
    let jobs = || async {
        sqlx::query_scalar::<_, String>(
            "SELECT m.external_id FROM public.notification_jobs j JOIN public.messages m ON m.id=j.message_id ORDER BY j.id",
        )
        .fetch_all(&pool)
        .await
        .unwrap()
    };
    let first = send(&h, &alice, &general, "hello", None).await;
    let reply = send(&h, &bob, &general, "in a thread", Some(&first)).await;
    assert_eq!(jobs().await, [first.clone(), reply.clone()]);
    let (status, _) = call(
        &h.app,
        "PUT",
        &format!("/api/chat/channels/{general}/messages/{first}"),
        None,
        Some(&alice.chat),
        json!({"text": "hello again", "expectedRevision": 1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, forward) = call(&h.app, "POST", &format!("/api/chat/channels/{general}/forwards"), None, Some(&bob.chat), json!({"sourceChannelId": general, "sourceMessageId": first, "clientMessageId": Uuid::new_v4()})).await;
    assert_eq!(status, StatusCode::OK, "{forward}");
    assert_eq!(
        jobs().await,
        [
            first.clone(),
            reply,
            forward["id"].as_str().unwrap().to_owned()
        ],
        "edits never enqueue"
    );
    // A send that fails after the message insert leaves neither message nor job.
    sqlx::query("ALTER TABLE public.channel_events ADD CONSTRAINT reject_push CHECK (seq < 5)")
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = call(
        &h.app,
        "POST",
        &format!("/api/chat/channels/{general}/messages"),
        None,
        Some(&alice.chat),
        json!({"clientMessageId": Uuid::new_v4(), "text": "rolled back"}),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(jobs().await.len(), 3);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM public.messages")
            .fetch_one(&pool)
            .await
            .unwrap(),
        3
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn forward_notes_show_mentions_without_notifying_them(pool: PgPool) {
    let h = harness(&pool).await;
    let alice = person(&pool, "alice").await;
    let bob = person(&pool, "bob").await;
    let carol = person(&pool, "carol").await;
    let (space_id, _) = space(&pool, &alice, &[&bob, &carol]).await;
    let general = channel(&pool, space_id, "general", false, &[&alice, &bob, &carol]).await;
    // Only a mention can reach carol.
    let (status, _) = call(
        &h.app,
        "PUT",
        "/api/notifications/settings",
        Some(&carol.token),
        None,
        json!({"level": "mentions"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first = send(&h, &alice, &general, "hello", None).await;
    let (status, forward) = call(
        &h.app,
        "POST",
        &format!("/api/chat/channels/{general}/forwards"),
        None,
        Some(&bob.chat),
        json!({"sourceChannelId": general, "sourceMessageId": first,
               "clientMessageId": Uuid::new_v4(), "text": "look @carol @everyone"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{forward}");
    // The note's mentions are resolved, so clients highlight them and open cards.
    assert_eq!(
        forward["content"]["mentions"],
        json!([
            {"type":"user","id":carol.external_id,"username":"carol"},
            {"type":"everyone"},
        ])
    );
    let message = send(&h, &bob, &general, "look @carol", None).await;
    expand(&h).await;
    let forwarded = notified(&pool, forward["id"].as_str().unwrap()).await;
    assert!(
        !forwarded.iter().any(|(user, _)| user == "carol"),
        "a forward's note never notifies its mentions: {forwarded:?}"
    );
    // The forward itself still notifies like a message, and the same text in
    // a message mentions her.
    assert!(forwarded.iter().any(|(user, _)| user == "alice"));
    assert_eq!(
        notified(&pool, &message).await,
        pairs(&[("alice", "channel.message"), ("carol", "mention.user")])
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn recipients_follow_requests_blocks_mutes_levels_mentions_and_threads(pool: PgPool) {
    let h = harness(&pool).await;
    let names = [
        "alice", "bob", "carol", "dave", "erin", "grace", "ivan", "frank", "judy", "kate",
    ];
    let mut people = Vec::new();
    for name in names {
        people.push(person(&pool, name).await);
    }
    let [
        alice,
        bob,
        carol,
        dave,
        erin,
        grace,
        ivan,
        _frank,
        judy,
        kate,
    ] = &people[..]
    else {
        unreachable!()
    };
    let (space_id, _) = space(
        &pool,
        alice,
        &[bob, carol, dave, erin, grace, ivan, judy, kate],
    )
    .await;
    // ivan can read #general but never joined it: only direct @mentions reach him.
    let general = channel(
        &pool,
        space_id,
        "general",
        false,
        &[alice, bob, carol, dave, erin, grace, judy, kate],
    )
    .await;
    let secret = channel(&pool, space_id, "secret", true, &[alice, bob]).await;
    sqlx::query("INSERT INTO public.channel_members (channel_id,user_id) SELECT id,$2 FROM public.channels WHERE external_id=$1")
        .bind(&secret).bind(bob.id).execute(&pool).await.unwrap();
    let settings = |token: &str, path: String, body: Value| {
        let app = h.app.clone();
        let token = token.to_owned();
        async move {
            let (status, value) = call(&app, "PUT", &path, Some(&token), None, body).await;
            assert_eq!(status, StatusCode::OK, "{value}");
        }
    };
    let space_external: String =
        sqlx::query_scalar("SELECT external_id FROM public.spaces WHERE id=$1")
            .bind(space_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    // carol: only @mentions. dave: space muted. erin: #general off. grace blocked
    // alice, and judy does too before the jobs expand. kate: notifications off.
    settings(
        &carol.token,
        "/api/notifications/settings".into(),
        json!({"level": "mentions"}),
    )
    .await;
    settings(
        &dave.token,
        format!("/api/spaces/{space_external}/notifications"),
        json!({"mutedUntil": "forever"}),
    )
    .await;
    settings(
        &erin.token,
        format!("/api/spaces/{space_external}/channels/{general}/notifications"),
        json!({"level": "nothing"}),
    )
    .await;
    settings(
        &kate.token,
        "/api/notifications/settings".into(),
        json!({"level": "nothing"}),
    )
    .await;
    call(
        &h.app,
        "PUT",
        &format!("/api/blocks/{}", alice.external_id),
        Some(&grace.token),
        None,
        Value::Null,
    )
    .await;

    let plain = send(&h, alice, &general, "hello", None).await;
    let named = send(
        &h,
        alice,
        &general,
        "@carol @dave @erin @kate @frank @ivan look",
        None,
    )
    .await;
    let everyone = send(&h, alice, &general, "@everyone standup", None).await;
    let thread = send(&h, alice, &general, "@bob thanks", Some(&plain)).await;
    let private = send(&h, alice, &secret, "just us", None).await;
    let rest = send(&h, alice, &general, "carol unmuted", None).await;
    let channel_message = |name: &'static str| (name, "channel.message");
    // A muted DM, a request, a later block, personal notes and an account that is off.
    let with_carol = direct(&h, alice, "carol").await;
    let with_ivan = direct(&h, alice, "ivan").await;
    settings(
        &ivan.token,
        format!("/api/dms/{with_ivan}/notifications"),
        json!({"mutedUntil": "forever"}),
    )
    .await;
    let with_frank = direct(&h, alice, "frank").await;
    let with_judy = direct(&h, alice, "judy").await;
    let notes = direct(&h, alice, "alice").await;
    let with_kate = direct(&h, alice, "kate").await;
    let to_carol = send(&h, alice, &with_carol, "hi carol", None).await;
    let to_ivan = send(&h, alice, &with_ivan, "hi ivan", None).await;
    let to_frank = send(&h, alice, &with_frank, "hi stranger", None).await;
    let to_judy = send(&h, alice, &with_judy, "hi judy", None).await;
    let note = send(&h, alice, &notes, "remember", None).await;
    let to_kate = send(&h, alice, &with_kate, "hi kate", None).await;
    call(
        &h.app,
        "PUT",
        &format!("/api/blocks/{}", alice.external_id),
        Some(&judy.token),
        None,
        Value::Null,
    )
    .await;
    expand(&h).await;

    assert_eq!(
        notified(&pool, &plain).await,
        pairs(&[channel_message("bob")]),
        "ivan hasn't joined #general"
    );
    assert_eq!(
        notified(&pool, &named).await,
        pairs(&[
            channel_message("bob"),
            ("carol", "mention.user"),
            ("dave", "mention.user"),
            ("ivan", "mention.user")
        ]),
        "a direct @mention ignores dave's mute and reaches ivan without a join; erin's #general is off; kate is off; frank is not a reader"
    );
    assert_eq!(
        notified(&pool, &everyone).await,
        pairs(&[("bob", "mention.everyone"), ("carol", "mention.everyone")])
    );
    assert_eq!(
        notified(&pool, &thread).await,
        pairs(&[("bob", "mention.user")]),
        "thread replies notify only the people they mention"
    );
    assert_eq!(
        notified(&pool, &private).await,
        pairs(&[channel_message("bob")])
    );
    assert_eq!(notified(&pool, &rest).await.len(), 1);
    assert_eq!(
        notified(&pool, &to_carol).await,
        pairs(&[("carol", "direct.message")]),
        "DMs notify under mentions"
    );
    for message in [&to_ivan, &to_frank, &to_judy, &note, &to_kate] {
        assert_eq!(notified(&pool, message).await, vec![], "{message}");
    }

    // @here reaches joined readers who are online; for others it is a channel message.
    active(&h, bob, &bob.hash).await;
    active(&h, carol, &carol.hash).await;
    let here = send(&h, alice, &general, "@here quick one", None).await;
    expand(&h).await;
    assert_eq!(
        notified(&pool, &here).await,
        pairs(&[("bob", "mention.everyone"), ("carol", "mention.everyone")]),
    );
    clear_presence(carol).await;
    let here_again = send(&h, alice, &general, "@here again", None).await;
    expand(&h).await;
    assert_eq!(
        notified(&pool, &here_again).await,
        pairs(&[("bob", "mention.everyone")]),
        "offline carol only wants mentions"
    );
    clear_presence(bob).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM public.notification_jobs WHERE expanded_at IS NULL"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn phone_push_waits_while_active_elsewhere_and_rechecks_at_send_time(pool: PgPool) {
    let h = harness(&pool).await;
    let alice = person(&pool, "alice").await;
    let bob = person(&pool, "bob").await;
    let (phone, phone_hash) = session(&pool, bob.id).await;
    register(&h, &bob, &phone, "apns", APNS_TOKEN).await;
    // Sharing a space makes the DM accepted rather than a message request.
    space(&pool, &alice, &[&bob]).await;
    let conversation = direct(&h, &alice, "bob").await;
    let deliveries = |message: String| {
        let pool = pool.clone();
        async move {
            sqlx::query_as::<_, (bool, bool, Option<String>, bool)>(
                "SELECT d.held, d.available_at > now() + interval '50 seconds', d.last_error, d.delivered_at IS NOT NULL
                 FROM public.notification_deliveries d JOIN public.notifications n ON n.id=d.notification_id
                 JOIN public.messages m ON m.id=n.message_id WHERE m.external_id=$1",
            )
            .bind(message)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let release = || async {
        sqlx::query("UPDATE public.notification_deliveries SET available_at=now() WHERE delivered_at IS NULL AND abandoned_at IS NULL")
            .execute(&pool).await.unwrap();
    };

    // Active on the desktop (another session): held for 60 s, then dropped
    // because bob is still active there.
    active(&h, &bob, &bob.hash).await;
    let first = send(&h, &alice, &conversation, "one", None).await;
    expand(&h).await;
    assert_eq!(deliveries(first.clone()).await, (true, true, None, false));
    deliver(&h).await;
    assert!(h.apns.take().is_empty(), "held deliveries wait");
    release().await;
    deliver(&h).await;
    assert_eq!(
        deliveries(first).await,
        (true, false, Some("active elsewhere".into()), false)
    );
    assert!(h.apns.take().is_empty());

    // Read on the desktop before the hold ends: dropped.
    let second = send(&h, &alice, &conversation, "two", None).await;
    expand(&h).await;
    clear_presence(&bob).await;
    let (status, _) = call(
        &h.app,
        "POST",
        &format!("/api/dms/{conversation}/read"),
        Some(&bob.token),
        None,
        json!({"seq": "2"}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    release().await;
    deliver(&h).await;
    assert_eq!(deliveries(second).await.2.as_deref(), Some("read"));

    // Went idle without reading: sent when the hold ends.
    active(&h, &bob, &bob.hash).await;
    let third = send(&h, &alice, &conversation, "three", None).await;
    expand(&h).await;
    clear_presence(&bob).await;
    release().await;
    deliver(&h).await;
    assert_eq!(deliveries(third).await, (true, false, None, true));
    assert_eq!(h.apns.take().len(), 1);

    // Active only in the phone app itself: not held.
    active(&h, &bob, &phone_hash).await;
    let fourth = send(&h, &alice, &conversation, "four", None).await;
    expand(&h).await;
    assert!(!deliveries(fourth.clone()).await.0);
    deliver(&h).await;
    assert!(deliveries(fourth).await.3);
    clear_presence(&bob).await;

    // An older gateway's connection ID (no session tag) counts as elsewhere.
    h.presence
        .renew(&bob.external_id, &Uuid::new_v4().to_string(), 0)
        .await
        .unwrap();
    let fifth = send(&h, &alice, &conversation, "five", None).await;
    expand(&h).await;
    assert!(deliveries(fifth).await.0);

    // `always` never holds.
    let (status, _) = call(
        &h.app,
        "PUT",
        "/api/notifications/settings",
        Some(&bob.token),
        None,
        json!({"mobile": "always"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let sixth = send(&h, &alice, &conversation, "six", None).await;
    expand(&h).await;
    assert!(!deliveries(sixth).await.0);
    clear_presence(&bob).await;
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn queued_push_rechecks_channel_access_and_consent(pool: PgPool) {
    let h = harness(&pool).await;
    let alice = person(&pool, "alice").await;
    let cases = [
        (
            "member",
            "UPDATE public.space_members SET deleted_at=now() WHERE user_id=$1",
        ),
        (
            "grant",
            "UPDATE public.channel_members SET deleted_at=now() WHERE user_id=$1",
        ),
        (
            "joined",
            "UPDATE public.channel_joins SET deleted_at=now() WHERE user_id=$1",
        ),
        (
            "blocked",
            "INSERT INTO public.user_blocks(blocker_id,blocked_id) SELECT $1,id FROM public.users WHERE username='alice'",
        ),
        (
            "deleted",
            "UPDATE public.users SET deleted_at=now() WHERE id=$1",
        ),
    ];
    for (name, revoke) in cases {
        let bob = person(&pool, name).await;
        let (space_id, _) = space(&pool, &alice, &[&bob]).await;
        let channel = channel(&pool, space_id, name, true, &[&alice, &bob]).await;
        sqlx::query("INSERT INTO public.channel_members(channel_id,user_id) SELECT id,$2 FROM public.channels WHERE external_id=$1")
            .bind(&channel).bind(bob.id).execute(&pool).await.unwrap();
        register(&h, &bob, &bob.token, "fcm", &format!("fcm:{name}")).await;
        let message = send(&h, &alice, &channel, "queued private content", None).await;
        expand(&h).await;
        assert_eq!(
            notified(&pool, &message).await,
            pairs(&[(name, "channel.message")])
        );
        sqlx::query(revoke)
            .bind(bob.id)
            .execute(&pool)
            .await
            .unwrap();
        deliver(&h).await;
        assert!(
            h.fcm.take().is_empty(),
            "{name}: revoked eligibility must prevent provider delivery"
        );
        let abandoned: bool = sqlx::query_scalar("SELECT abandoned_at IS NOT NULL FROM public.notification_deliveries d JOIN public.notifications n ON n.id=d.notification_id JOIN public.messages m ON m.id=n.message_id WHERE m.external_id=$1")
            .bind(&message).fetch_one(&pool).await.unwrap();
        assert!(
            abandoned,
            "{name}: a revoked delivery must not remain retryable"
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn queued_push_rechecks_preferences_without_silencing_direct_mentions(pool: PgPool) {
    let h = harness(&pool).await;
    let alice = person(&pool, "alice").await;
    for (name, mention, setting, allowed) in [
        ("off", true, "account-off", false),
        ("paused", true, "pause", false),
        ("channeloff", true, "channel-off", false),
        ("spacemute", false, "space-mute", false),
        ("channelmute", false, "channel-mute", false),
        ("mentions", false, "mentions", false),
        ("named", true, "space-mute", true),
        ("unjoined", true, "leave", true),
    ] {
        let bob = person(&pool, name).await;
        let (space_id, _) = space(&pool, &alice, &[&bob]).await;
        let channel = channel(&pool, space_id, name, false, &[&alice, &bob]).await;
        register(&h, &bob, &bob.token, "fcm", &format!("fcm:{name}")).await;
        let text = if mention {
            format!("@{name} queued mention")
        } else {
            "queued message".into()
        };
        let message = send(&h, &alice, &channel, &text, None).await;
        expand(&h).await;
        assert_eq!(
            notified(&pool, &message).await,
            pairs(&[(
                name,
                if mention {
                    "mention.user"
                } else {
                    "channel.message"
                }
            )])
        );
        match setting {
            "account-off" | "mentions" => {
                sqlx::query(
                    "INSERT INTO public.notification_settings(user_id,default_level) VALUES($1,$2)",
                )
                .bind(bob.id)
                .bind(if setting == "account-off" {
                    "nothing"
                } else {
                    "mentions"
                })
                .execute(&pool)
                .await
                .unwrap();
            }
            "pause" => {
                sqlx::query("INSERT INTO public.notification_settings(user_id,paused_until) VALUES($1,now()+interval '1 hour')")
                    .bind(bob.id).execute(&pool).await.unwrap();
            }
            "space-mute" => {
                sqlx::query("INSERT INTO public.notification_overrides(user_id,space_id,muted_until) VALUES($1,$2,now()+interval '1 hour')")
                    .bind(bob.id).bind(space_id).execute(&pool).await.unwrap();
            }
            "channel-off" | "channel-mute" => {
                sqlx::query("INSERT INTO public.notification_overrides(user_id,channel_id,level,muted_until) SELECT $1,id,$3,CASE WHEN $3::text IS NULL THEN now()+interval '1 hour' END FROM public.channels WHERE external_id=$2")
                    .bind(bob.id).bind(&channel).bind(if setting == "channel-off" { Some("nothing") } else { None }).execute(&pool).await.unwrap();
            }
            "leave" => {
                sqlx::query("UPDATE public.channel_joins SET deleted_at=now() WHERE user_id=$1")
                    .bind(bob.id)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        deliver(&h).await;
        let requests = h.fcm.take();
        assert_eq!(
            requests.len(),
            usize::from(allowed),
            "{name}: send-time notification policy"
        );
        if allowed {
            assert_eq!(requests[0].body["message"]["data"]["messageId"], message);
        }
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn queued_dm_push_rechecks_blocks_in_both_directions(pool: PgPool) {
    let h = harness(&pool).await;
    let alice = person(&pool, "alice").await;
    for (name, sender_blocks) in [("bob", false), ("carol", true)] {
        let recipient = person(&pool, name).await;
        space(&pool, &alice, &[&recipient]).await;
        let conversation = direct(&h, &alice, name).await;
        register(
            &h,
            &recipient,
            &recipient.token,
            "fcm",
            &format!("fcm:{name}"),
        )
        .await;
        let message = send(&h, &alice, &conversation, "queued DM", None).await;
        expand(&h).await;
        assert_eq!(
            notified(&pool, &message).await,
            pairs(&[(name, "direct.message")])
        );
        if sender_blocks {
            h.fcm
                .reply(503, json!({"error": {"status": "UNAVAILABLE"}}), None);
            deliver(&h).await;
            assert_eq!(
                h.fcm.take().len(),
                1,
                "the first authorized attempt reaches FCM"
            );
            sqlx::query("UPDATE public.notification_deliveries SET available_at=now() WHERE delivered_at IS NULL AND abandoned_at IS NULL")
                .execute(&pool).await.unwrap();
        }
        let (blocker, blocked) = if sender_blocks {
            (alice.id, recipient.id)
        } else {
            (recipient.id, alice.id)
        };
        sqlx::query("INSERT INTO public.user_blocks(blocker_id,blocked_id) VALUES($1,$2)")
            .bind(blocker)
            .bind(blocked)
            .execute(&pool)
            .await
            .unwrap();
        deliver(&h).await;
        assert!(
            h.fcm.take().is_empty(),
            "{name}: either direction of a DM block must suppress queued push"
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn fanout_preserves_per_device_holds_for_mixed_recipients(pool: PgPool) {
    let h = harness(&pool).await;
    let alice = person(&pool, "alice").await;
    let bob = person(&pool, "bob").await;
    let carol = person(&pool, "carol").await;
    let dave = person(&pool, "dave").await;
    let (space_id, _) = space(&pool, &alice, &[&bob, &carol, &dave]).await;
    let general = channel(
        &pool,
        space_id,
        "general",
        false,
        &[&alice, &bob, &carol, &dave],
    )
    .await;
    register(&h, &bob, &bob.token, "fcm", "bob-active").await;
    let (other_phone, _) = session(&pool, bob.id).await;
    register(&h, &bob, &other_phone, "fcm", "bob-held").await;
    register(&h, &carol, &carol.token, "fcm", "carol-always").await;
    sqlx::query("INSERT INTO public.notification_settings(user_id,mobile) VALUES($1,'always')")
        .bind(carol.id)
        .execute(&pool)
        .await
        .unwrap();
    active(&h, &bob, &bob.hash).await;
    let (_, carol_desktop) = session(&pool, carol.id).await;
    active(&h, &carol, &carol_desktop).await;
    let message = send(&h, &alice, &general, "mixed devices", None).await;
    expand(&h).await;
    assert_eq!(
        notified(&pool, &message).await.len(),
        3,
        "Dave is notified but has no phone"
    );
    let deliveries: Vec<(String, bool)> = sqlx::query_as("SELECT dev.address,d.held FROM public.notification_deliveries d JOIN public.notification_devices dev ON dev.id=d.device_id ORDER BY dev.address")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(
        deliveries,
        vec![
            ("bob-active".into(), false),
            ("bob-held".into(), true),
            ("carol-always".into(), false)
        ]
    );
    clear_presence(&bob).await;
    clear_presence(&carol).await;
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn deliveries_reach_providers_revoke_dead_tokens_and_retry_with_backoff(pool: PgPool) {
    let h = harness(&pool).await;
    let alice = person(&pool, "alice").await;
    let bob = person(&pool, "bob").await;
    let carol = person(&pool, "carol").await;
    let (space_id, space_external) = space(&pool, &alice, &[&bob, &carol]).await;
    let general = channel(&pool, space_id, "general", false, &[&alice, &bob, &carol]).await;
    register(&h, &bob, &bob.token, "apns", APNS_TOKEN).await;
    register(&h, &carol, &carol.token, "fcm", "fcm:carol").await;
    let state = |message: String, user: i64| {
        let pool = pool.clone();
        async move {
            sqlx::query_as::<_, (i16, Option<String>, bool, bool, i64)>(
                "SELECT d.attempts, d.last_error, d.delivered_at IS NOT NULL, d.abandoned_at IS NOT NULL,
                        GREATEST(0, EXTRACT(EPOCH FROM d.available_at - now()))::bigint
                 FROM public.notification_deliveries d JOIN public.notifications n ON n.id=d.notification_id
                 JOIN public.messages m ON m.id=n.message_id WHERE m.external_id=$1 AND n.user_id=$2",
            )
            .bind(message)
            .bind(user)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let release = || async {
        sqlx::query("UPDATE public.notification_deliveries SET available_at=now() WHERE delivered_at IS NULL AND abandoned_at IS NULL")
            .execute(&pool).await.unwrap();
    };

    let long = format!("  {}  ", "word ".repeat(60));
    let first = send(&h, &alice, &general, &long, None).await;
    expand(&h).await;
    deliver(&h).await;
    let apns_requests = h.apns.take();
    assert_eq!(apns_requests.len(), 1);
    assert_eq!(apns_requests[0].path, format!("/3/device/{APNS_TOKEN}"));
    let body = &apns_requests[0].body;
    assert_eq!(body["aps"]["alert"]["title"], "Alice · #general (Studio)");
    let preview = body["aps"]["alert"]["body"].as_str().unwrap();
    assert_eq!(preview.chars().count(), 180);
    assert!(preview.ends_with('…') && !preview.starts_with(' '));
    assert_eq!(
        (
            body["kind"].as_str(),
            body["messageId"].as_str(),
            body["spaceId"].as_str(),
            body["channelId"].as_str()
        ),
        (
            Some("channel.message"),
            Some(first.as_str()),
            Some(space_external.as_str()),
            Some(general.as_str())
        )
    );
    let fcm_requests = h.fcm.take();
    let data = &fcm_requests[0].body["message"]["data"];
    assert_eq!(fcm_requests[0].body["message"]["token"], "fcm:carol");
    assert_eq!(
        (
            data["sender"].as_str(),
            data["senderId"].as_str(),
            data["conversationTitle"].as_str()
        ),
        (
            Some("Alice"),
            Some(alice.external_id.as_str()),
            Some("#general (Studio)")
        )
    );
    assert!(state(first.clone(), bob.id).await.2);

    // A dead APNs token revokes bob's device; FCM throttling is retried after
    // Retry-After, then a 5xx doubles the backoff, then it goes through.
    h.apns.reply(410, json!({"reason": "Unregistered"}), None);
    h.fcm.reply(429, json!({"error": {"status": "RESOURCE_EXHAUSTED", "details": [{"errorCode": "QUOTA_EXCEEDED"}]}}), Some(120));
    let second = send(&h, &alice, &general, "second", None).await;
    expand(&h).await;
    deliver(&h).await;
    assert_eq!(
        state(second.clone(), bob.id).await,
        (1, Some("apns 410 Unregistered".into()), false, true, 0)
    );
    assert_eq!(
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT revoked_reason FROM public.notification_devices WHERE user_id=$1"
        )
        .bind(bob.id)
        .fetch_one(&pool)
        .await
        .unwrap()
        .as_deref(),
        Some("apns 410 Unregistered")
    );
    let retry = state(second.clone(), carol.id).await;
    assert_eq!(
        (retry.0, retry.1.as_deref(), retry.2, retry.3),
        (1, Some("fcm 429 QUOTA_EXCEEDED"), false, false)
    );
    assert!((115..=120).contains(&retry.4), "{retry:?}");
    h.fcm
        .reply(503, json!({"error": {"status": "UNAVAILABLE"}}), None);
    release().await;
    deliver(&h).await;
    let retry = state(second.clone(), carol.id).await;
    assert_eq!(
        (retry.0, retry.1.as_deref()),
        (2, Some("fcm 503 UNAVAILABLE"))
    );
    assert!(
        (25..=30).contains(&retry.4),
        "second attempt waits 30 s: {retry:?}"
    );
    release().await;
    deliver(&h).await;
    assert!(state(second, carol.id).await.2);
    h.fcm.take();
    h.apns.take();

    // Revoked devices get nothing new.
    let third = send(&h, &alice, &general, "third", None).await;
    expand(&h).await;
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM public.notification_deliveries d JOIN public.notifications n ON n.id=d.notification_id WHERE n.user_id=$1 AND n.message_id=(SELECT id FROM public.messages WHERE external_id=$2)").bind(bob.id).bind(&third).fetch_one(&pool).await.unwrap(), 0);
    deliver(&h).await;
    h.fcm.take();

    // Retries stop a day after the message.
    let fourth = send(&h, &alice, &general, "fourth", None).await;
    expand(&h).await;
    sqlx::query("UPDATE public.messages SET created_at=now()-interval '23 hours 59 minutes 50 seconds' WHERE external_id=$1").bind(&fourth).execute(&pool).await.unwrap();
    h.fcm
        .reply(500, json!({"error": {"status": "INTERNAL"}}), None);
    deliver(&h).await;
    assert_eq!(
        state(fourth, carol.id).await,
        (1, Some("fcm 500 INTERNAL".into()), false, true, 0)
    );
    let fifth = send(&h, &alice, &general, "fifth", None).await;
    expand(&h).await;
    sqlx::query(
        "UPDATE public.messages SET created_at=now()-interval '25 hours' WHERE external_id=$1",
    )
    .bind(&fifth)
    .execute(&pool)
    .await
    .unwrap();
    deliver(&h).await;
    assert_eq!(state(fifth, carol.id).await.1.as_deref(), Some("expired"));
    assert_eq!(h.fcm.take().len(), 1, "only the fourth message reached FCM");

    // A session that signs out before the send drops its pending deliveries.
    let sixth = send(&h, &alice, &general, "sixth", None).await;
    expand(&h).await;
    sqlx::query("UPDATE public.account_sessions SET revoked_at=now() WHERE token_hash=$1")
        .bind(&carol.hash)
        .execute(&pool)
        .await
        .unwrap();
    deliver(&h).await;
    assert_eq!(
        state(sixth, carol.id).await.1.as_deref(),
        Some("session ended")
    );
    assert!(h.fcm.take().is_empty());
}
