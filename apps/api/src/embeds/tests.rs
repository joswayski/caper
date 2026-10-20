use super::*;
use axum::{http::HeaderMap, routing::post as route_post};
use std::future::IntoFuture;

fn card(url: &str) -> Value {
    json!({
        "version": 1,
        "url": url,
        "site": {"name": "X", "domain": "x.com"},
        "blocks": [
            {"type": "author", "name": "jack", "handle": "@jack", "icon": "https://embed.caper.chat/m/sig/i?u=x"},
            {"type": "text", "text": "just setting up my twttr"},
            {"type": "images", "items": [{"url": "https://embed.caper.chat/m/sig/i?u=y", "width": 1200}]},
            {"type": "footer", "text": "309K likes", "timestamp": "2006-03-21T20:50:14.000Z"}
        ]
    })
}

#[test]
fn previews_follow_client_link_detection_without_repeats_or_bracketed_links() {
    assert_eq!(
        links("look https://x.com/a/status/1, www.example.com and https://x.com/a/status/1 again"),
        ["https://x.com/a/status/1", "https://www.example.com"]
    );
    // Written `<like this>`, a link stays clickable but gets no card.
    assert_eq!(
        links("<https://quiet.example/a> but https://loud.example/b"),
        ["https://loud.example/b"]
    );
    assert!(links("mailto:a@b.com javascript:alert(1) example.com").is_empty());
    let many: String = (0..8)
        .map(|i| format!("https://site{i}.example/ "))
        .collect();
    assert_eq!(links(&many).len(), MAX_PER_MESSAGE);
}

#[test]
fn only_well_formed_bounded_cards_for_the_requested_link_are_kept() {
    let url = "https://x.com/jack/status/20";
    assert!(valid(&card(url), url));
    assert!(!valid(&card(url), "https://x.com/jack/status/21"));
    let mut script = card(url);
    script["blocks"][0]["icon"] = json!("javascript:alert(1)");
    assert!(!valid(&script, url));
    let mut huge = card(url);
    huge["blocks"][1]["text"] = json!("a".repeat(MAX_STRING + 1));
    assert!(!valid(&huge, url));
    let mut crowded = card(url);
    crowded["blocks"] = json!(vec![json!({"type": "text", "text": "a"}); MAX_BLOCKS + 1]);
    assert!(!valid(&crowded, url));
    let mut nameless = card(url);
    nameless["site"] = json!({"domain": "x.com"});
    assert!(!valid(&nameless, url));
    assert!(!valid(&json!(null), url));
}

#[test]
fn edits_keep_cards_only_for_links_still_in_the_text() {
    let previous = json!({"text": "a https://a.example/ b https://b.example/", "embeds": [card("https://a.example/"), card("https://b.example/")]});
    let mut content = json!({"version": 1, "type": "text", "text": "now just https://b.example/"});
    carry_over(&previous, &mut content);
    assert_eq!(content["embeds"], json!([card("https://b.example/")]));
    let mut bare = json!({"version": 1, "type": "text", "text": "no links"});
    carry_over(&previous, &mut bare);
    assert!(bare.get("embeds").is_none());
}

#[test]
fn jobs_come_from_live_messages_with_links_or_edits_that_dropped_them() {
    let event = |kind: &str, message: Value| json!({"type": kind, "channelId": "c1", "seq": "7", "message": message});
    let job = Job::from_event(&event(
        "message.created",
        json!({"id": "m1", "content": {"text": "see https://x.com/a/status/1"}}),
    ))
    .unwrap();
    assert_eq!(
        (
            job.channel.as_str(),
            job.message.as_str(),
            job.revision,
            job.seq
        ),
        ("c1", "m1", 1, 7)
    );
    assert_eq!(job.links, ["https://x.com/a/status/1"]);
    assert!(
        Job::from_event(&event(
            "message.created",
            json!({"id": "m1", "content": {"text": "hi"}})
        ))
        .is_none()
    );
    // An edit that removed every link still runs, to clear the old cards.
    let cleared = Job::from_event(&event(
        "message.edited",
        json!({"id": "m1", "revision": 2, "content": {"text": "hi", "embeds": [card("https://a.example/")]}}),
    ))
    .unwrap();
    assert!(cleared.links.is_empty());
    assert!(Job::from_event(&event("message.edited", json!({"id": "m1", "revision": 3, "deletedAt": "now", "content": {"text": "https://a.example/"}}))).is_none());
    assert!(
        Job::from_event(&json!({"type": "message.reactions", "channelId": "c1", "seq": "8"}))
            .is_none()
    );
}

#[test]
fn configuration_requires_an_https_origin_and_a_long_secret_together() {
    let secret = "s".repeat(32);
    assert!(
        Embeds::from_env(&RuntimeEnvironment::from_values_for_test([]))
            .unwrap()
            .is_none()
    );
    assert!(
        Embeds::from_env(&RuntimeEnvironment::from_values_for_test([(
            "EMBED_ORIGIN",
            "https://embed.caper.chat"
        )]))
        .is_err()
    );
    assert!(
        Embeds::from_env(&RuntimeEnvironment::from_values_for_test([
            ("EMBED_ORIGIN", "http://embed.caper.chat"),
            ("EMBED_API_SECRET", &secret)
        ]))
        .is_err()
    );
    assert!(
        Embeds::from_env(&RuntimeEnvironment::from_values_for_test([
            ("EMBED_ORIGIN", "https://embed.caper.chat"),
            ("EMBED_API_SECRET", "short")
        ]))
        .is_err()
    );
    let embeds = Embeds::from_env(&RuntimeEnvironment::from_values_for_test([
        ("EMBED_ORIGIN", "https://embed.caper.chat/"),
        ("EMBED_API_SECRET", &secret),
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(embeds.endpoint, "https://embed.caper.chat/v1/unfurl");
}

#[tokio::test]
async fn unfurl_sends_the_secret_and_drops_invalid_answers() {
    async fn worker(headers: HeaderMap, Json(input): Json<Value>) -> Json<Value> {
        assert_eq!(
            headers["authorization"],
            format!("Bearer {}", "k".repeat(32))
        );
        let url = input["url"].as_str().unwrap();
        Json(match url {
            "https://good.example/" => json!({"embed": card(url)}),
            "https://other.example/" => json!({"embed": card("https://somewhere-else.example/")}),
            _ => json!({"embed": null}),
        })
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(
        axum::serve(
            listener,
            Router::new().route("/v1/unfurl", route_post(worker)),
        )
        .into_future(),
    );
    let embeds = Embeds::new(format!("http://{address}/v1/unfurl"), "k".repeat(32));
    assert_eq!(
        embeds.unfurl("https://good.example/").await,
        Some(card("https://good.example/"))
    );
    assert_eq!(embeds.unfurl("https://other.example/").await, None);
    assert_eq!(embeds.unfurl("https://none.example/").await, None);
    let unreachable = Embeds::new("http://127.0.0.1:9/v1/unfurl".into(), "k".repeat(32));
    assert_eq!(unreachable.unfurl("https://good.example/").await, None);
}

#[test]
fn composer_previews_are_rate_limited_per_account() {
    let embeds = Embeds::new("http://127.0.0.1:9/v1/unfurl".into(), "k".repeat(32));
    for _ in 0..PREVIEWS_PER_MINUTE {
        assert!(embeds.allow_preview(1));
    }
    assert!(!embeds.allow_preview(1));
    assert!(embeds.allow_preview(2));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL"]
async fn cards_are_stored_on_the_current_revision_and_its_replay_event(pool: sqlx::PgPool) {
    let channel: i64 = sqlx::query_scalar("INSERT INTO public.channels(external_id,name,private) VALUES('embed-channel','embeds',true) RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap();
    let session: i64 = sqlx::query_scalar("INSERT INTO public.chat_sessions(external_id,token_hash,name) VALUES('guest','\\x00','Guest') RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap();
    let url = "https://x.com/jack/status/20";
    let message = json!({"id":"m1","channelId":"embed-channel","seq":"1","content":{"version":1,"type":"text","text":format!("look {url}")}});
    sqlx::query("INSERT INTO public.messages(external_id,channel_id,session_id,client_message_id,request_hash,channel_seq,payload) VALUES('m1',$1,$2,$3,'\\x00',1,$4)")
        .bind(channel)
        .bind(session)
        .bind(uuid::Uuid::new_v4())
        .bind(&message)
        .execute(&pool)
        .await
        .unwrap();
    let event = json!({"type":"message.created","schemaVersion":1,"channelId":"embed-channel","seq":"1","message":message});
    sqlx::query("INSERT INTO public.channel_events(channel_id,seq,payload) VALUES($1,1,$2)")
        .bind(channel)
        .bind(&event)
        .execute(&pool)
        .await
        .unwrap();
    let chat = Chat::new(
        pool.clone(),
        redis::Client::open("redis://127.0.0.1:9").unwrap(),
    );
    let job = Job::from_event(&event).unwrap();

    assert!(apply(&chat, &job, vec![card(url)]).await.unwrap());
    let stored: Value =
        sqlx::query_scalar("SELECT payload FROM public.messages WHERE external_id='m1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored["content"]["embeds"], json!([card(url)]));
    assert_eq!(stored["content"]["text"], format!("look {url}"));
    let replayed: Value = sqlx::query_scalar(
        "SELECT payload FROM public.channel_events WHERE channel_id=$1 AND seq=1",
    )
    .bind(channel)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(replayed["message"]["content"]["embeds"], json!([card(url)]));
    // The same cards again change nothing.
    assert!(!apply(&chat, &job, vec![card(url)]).await.unwrap());

    // Cards made for an older revision never land on a newer edit.
    sqlx::query("UPDATE public.messages SET payload=jsonb_set(payload,'{revision}','2') WHERE external_id='m1'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!apply(&chat, &job, vec![]).await.unwrap());

    // Nor on a deleted message.
    sqlx::query("UPDATE public.messages SET payload=jsonb_set(payload,'{revision}','1'), deleted_at=now() WHERE external_id='m1'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!apply(&chat, &job, vec![]).await.unwrap());
}
