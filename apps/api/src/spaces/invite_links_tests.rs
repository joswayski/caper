use super::invite_links::{
    PrepareInput, accept, create_link, default_link, list_links, preview, regenerate_link,
    revoke_link,
};
use super::*;
use crate::{Cloudflare, Config, accounts::User};
use axum::{extract::Query, http::HeaderMap};
use sha2::{Digest, Sha256};
use sqlx::{
    Connection, Executor,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{str::FromStr, sync::Arc};
use uuid::Uuid;

fn principal(id: i64, external_id: &str, username: &str) -> Principal {
    Principal {
        user: User {
            id,
            external_id: external_id.to_owned(),
            avatar_id: 7,
            avatar_asset: None,
            email: None,
            username: Some(username.to_owned()),
            display_name: Some(username.to_owned()),
        },
        token_hash: Vec::new(),
    }
}

#[test]
fn codes_are_fixed_length_alphanumeric() {
    assert!(super::invite_links::valid_code("aB3dE6gH9k"));
    for code in ["", "short", "aB3dE6gH9k1", "aB3dE6gH9-", "../../etc/"] {
        assert!(!super::invite_links::valid_code(code), "{code}");
    }
}

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL"]
async fn invite_links_join_preview_limits_and_exclusions() {
    let url = std::env::var("CHAT_TEST_DATABASE_URL")
        .expect("CHAT_TEST_DATABASE_URL must point at disposable Postgres");
    let options = PgConnectOptions::from_str(&url).unwrap();
    assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
    let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
    let database = format!("invite_links_test_{}", Uuid::new_v4().simple());
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
    crate::chat::seed(&pool).await.unwrap();

    let mut people = Vec::new();
    for username in [
        "owner", "member", "newcomer", "late", "leaver", "blocked", "invited",
    ] {
        let external = random_id(12);
        let id: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES($1,$2,$2) RETURNING id")
            .bind(&external).bind(username).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 hour')")
            .bind(Sha256::digest(username.as_bytes()).to_vec()).bind(id).execute(&pool).await.unwrap();
        people.push(principal(id, &external, username));
    }
    let [owner, member, newcomer, late, leaver, blocked, invited] = people.as_slice() else {
        unreachable!()
    };
    let space = random_id(12);
    let space_id: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES($1,'Captures',$2) RETURNING id")
        .bind(&space).bind(owner.user.id).fetch_one(&pool).await.unwrap();
    for person in [owner, member] {
        sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
            .bind(space_id)
            .bind(person.user.id)
            .execute(&pool)
            .await
            .unwrap();
    }
    let mut channels = Vec::new();
    for (name, private) in [("general", false), ("feedback", false), ("staff", true)] {
        let external = random_id(12);
        sqlx::query(
            "INSERT INTO public.channels(external_id,space_id,name,private) VALUES($1,$2,$3,$4)",
        )
        .bind(&external)
        .bind(space_id)
        .bind(name)
        .bind(private)
        .execute(&pool)
        .await
        .unwrap();
        channels.push(external);
    }
    let [general, feedback, staff] = channels.as_slice() else {
        unreachable!()
    };
    let mut state = AppState::with_database(
        Config::test(false),
        Arc::new(Cloudflare::new()),
        Some(pool.clone()),
    );
    state.auth = crate::auth::AuthVerifier::new();
    let create = |who: &Principal, body: Value| {
        let state = state.clone();
        let who = who.clone();
        let space = space.clone();
        async move {
            create_link(
                State(state),
                Extension(who),
                Path(space),
                Json(serde_json::from_value(body).unwrap()),
            )
            .await
            .map(|(status, Json(link))| (status, serde_json::to_value(link).unwrap()))
        }
    };
    let join = |who: &Principal, code: &str| {
        let state = state.clone();
        let who = who.clone();
        let code = code.to_owned();
        async move {
            accept(State(state), Extension(who), Path(code))
                .await
                .map(|json| json.0)
        }
    };
    let view = |code: &str| {
        let state = state.clone();
        let code = code.to_owned();
        async move {
            preview(State(state), Path(code), HeaderMap::new())
                .await
                .map(|json| json.0)
        }
    };
    let view_as = |code: &str, token: &str, cookie: bool| {
        let state = state.clone();
        let code = code.to_owned();
        let mut headers = HeaderMap::new();
        if cookie {
            headers.insert("cookie", format!("caper_session={token}").parse().unwrap());
        } else {
            headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        }
        async move {
            preview(State(state), Path(code), headers)
                .await
                .map(|json| json.0)
        }
    };

    // Only owners create links, expiring within a minute to a year and capped
    // at 10,000 uses, and never into private channels.
    assert_eq!(
        create(member, json!({})).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    for body in [
        json!({"maxAge":59}),
        json!({"maxAge":31_536_001}),
        json!({"maxUses":0}),
        json!({"maxUses":10_001}),
    ] {
        assert_eq!(
            create(owner, body).await.unwrap_err().status,
            StatusCode::BAD_REQUEST
        );
    }
    // Any whole value inside those bounds is accepted.
    let (status, custom) = create(owner, json!({"maxAge":31_536_000,"maxUses":7}))
        .await
        .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(custom["maxUses"], 7);
    revoke_link(
        State(state.clone()),
        Extension(owner.clone()),
        Path((space.clone(), custom["code"].as_str().unwrap().to_owned())),
    )
    .await
    .unwrap();
    assert_eq!(
        create(owner, json!({"channelId":staff}))
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        create(owner, json!({"channelId":"missing12345"}))
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );

    let (status, single) = create(
        owner,
        json!({"channelId":feedback,"maxUses":1,"maxAge":604800}),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    let code = single["code"].as_str().unwrap().to_owned();
    assert!(super::invite_links::valid_code(&code));
    assert_eq!(single["channel"]["name"], "feedback");
    assert_eq!(single["uses"], 0);
    assert_eq!(single["maxUses"], 1);
    assert!(single["expiresAt"].is_string());
    assert_eq!(single["createdBy"]["username"], "owner");

    // The public preview shares only the space, landing channel and inviter.
    let shown = view(&code).await.unwrap();
    assert_eq!(shown["space"], json!({"name":"Captures"}));
    assert_eq!(shown["channel"], json!({"name":"feedback"}));
    assert_eq!(shown["inviter"]["username"], "owner");
    assert_eq!(shown["inviter"]["admin"], false);
    sqlx::query("INSERT INTO public.feedback_team(user_id) VALUES($1)")
        .bind(owner.user.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(view(&code).await.unwrap()["inviter"]["admin"], true);
    sqlx::query("UPDATE public.feedback_team SET deleted_at=now() WHERE user_id=$1")
        .bind(owner.user.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(view(&code).await.unwrap()["inviter"]["admin"], false);
    assert_eq!(shown["memberCount"], 2);
    assert_eq!(shown["alreadyMember"], false);
    for cookie in [false, true] {
        assert_eq!(
            view_as(&code, "member", cookie).await.unwrap()["alreadyMember"],
            true
        );
        assert_eq!(
            view_as(&code, "newcomer", cookie).await.unwrap()["alreadyMember"],
            false
        );
        assert_eq!(
            view_as(&code, "invalid-session", cookie).await.unwrap()["alreadyMember"],
            false
        );
    }
    // A preview never joins a space or consumes the invite's single use.
    assert_eq!(view(&code).await.unwrap()["memberCount"], 2);
    assert_eq!(
        view("aaaaaaaaaa").await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        view("../x").await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );

    // Joining lands in the link's channel and consumes a use.
    let joined = join(newcomer, &code).await.unwrap();
    assert_eq!(joined["space"]["id"], space.as_str());
    assert_eq!(joined["channelId"], feedback.as_str());
    assert_eq!(joined["joined"], true);
    assert!(
        channel_participation(&pool, feedback, Some(newcomer.user.id))
            .await
            .is_ok()
    );
    assert!(
        channel_access(&pool, staff, Some(newcomer.user.id))
            .await
            .is_err()
    );
    // Existing members reuse an exhausted link as a shortcut without a use.
    let again = join(member, &code).await.unwrap();
    assert_eq!(again["joined"], false);
    assert_eq!(again["channelId"], feedback.as_str());
    let uses: i32 = sqlx::query_scalar("SELECT uses FROM public.space_invite_links WHERE code=$1")
        .bind(&code)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(uses, 1);
    // Exhausted: outsiders can neither preview nor join.
    assert_eq!(view(&code).await.unwrap_err().status, StatusCode::NOT_FOUND);
    assert_eq!(
        view_as(&code, "member", false).await.unwrap()["alreadyMember"],
        true
    );
    assert_eq!(
        view_as(&code, "late", true).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        join(late, &code).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );

    // Revoked and expired links stop working and leave the owner's list.
    let (_, open) = create(owner, json!({})).await.unwrap();
    let open = open["code"].as_str().unwrap().to_owned();
    let (_, revoked) = create(owner, json!({})).await.unwrap();
    let revoked = revoked["code"].as_str().unwrap().to_owned();
    assert_eq!(
        revoke_link(
            State(state.clone()),
            Extension(member.clone()),
            Path((space.clone(), revoked.clone()))
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        revoke_link(
            State(state.clone()),
            Extension(owner.clone()),
            Path((space.clone(), revoked.clone()))
        )
        .await
        .unwrap(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        join(late, &revoked).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    let (_, expired) = create(owner, json!({"maxAge":90})).await.unwrap();
    let expired = expired["code"].as_str().unwrap().to_owned();
    sqlx::query(
        "UPDATE public.space_invite_links SET expires_at=now()-interval '1 second' WHERE code=$1",
    )
    .bind(&expired)
    .execute(&pool)
    .await
    .unwrap();
    for dead in [&expired, &revoked] {
        assert_eq!(
            view_as(dead, "member", true).await.unwrap()["alreadyMember"],
            true
        );
        assert_eq!(
            view_as(dead, "late", false).await.unwrap_err().status,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        join(late, &expired).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    let listed = list_links(
        State(state.clone()),
        Extension(owner.clone()),
        Path(space.clone()),
    )
    .await
    .unwrap()
    .0;
    let codes: Vec<&str> = listed["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|link| link["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, [open.as_str()]);
    assert_eq!(
        list_links(
            State(state.clone()),
            Extension(member.clone()),
            Path(space.clone())
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::NOT_FOUND
    );

    // Without a landing channel, joins use the starter channel.
    let joined = join(late, &open).await.unwrap();
    assert_eq!(joined["channelId"], general.as_str());

    // Leaving keeps link access; an owner removal or block does not.
    for who in [leaver, blocked] {
        join(who, &open).await.unwrap();
    }
    remove_space_member(
        State(state.clone()),
        Extension(leaver.clone()),
        Path((space.clone(), leaver.user.external_id.clone())),
    )
    .await
    .unwrap();
    assert_eq!(join(leaver, &open).await.unwrap()["joined"], true);
    remove_space_member(
        State(state.clone()),
        Extension(owner.clone()),
        Path((space.clone(), late.user.external_id.clone())),
    )
    .await
    .unwrap();
    assert_eq!(
        view_as(&open, "late", false).await.unwrap()["alreadyMember"],
        false
    );
    assert_eq!(
        view_as(&code, "late", true).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        join(late, &open).await.unwrap_err().status,
        StatusCode::FORBIDDEN
    );
    // A re-invitation that is cancelled or declined leaves the removal in
    // force; only a live or accepted invitation lifts it.
    let reinvite = || {
        let state = state.clone();
        let owner = owner.clone();
        let space = space.clone();
        let pool = pool.clone();
        async move {
            // Past the invitation cooldown and any earlier invitation's expiry.
            sqlx::query("UPDATE public.space_invitations SET updated_at=now()-interval '8 days' WHERE space_id=$1 AND user_id=$2")
                .bind(space_id).bind(late.user.id).execute(&pool).await.unwrap();
            let (status, _) = add_space_member(
                State(state),
                Extension(owner),
                Path(space),
                Json(MemberInput {
                    username: "late".into(),
                }),
            )
            .await
            .unwrap();
            assert_eq!(status, StatusCode::CREATED);
        }
    };
    reinvite().await;
    cancel_invitation(
        State(state.clone()),
        Extension(owner.clone()),
        Path((space.clone(), late.user.external_id.clone())),
    )
    .await
    .unwrap();
    assert_eq!(
        join(late, &open).await.unwrap_err().status,
        StatusCode::FORBIDDEN
    );
    reinvite().await;
    decline_invitation(
        State(state.clone()),
        Extension(late.clone()),
        Path(space.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        join(late, &open).await.unwrap_err().status,
        StatusCode::FORBIDDEN
    );
    reinvite().await;
    sqlx::query("UPDATE public.space_invitations SET updated_at=now()-interval '8 days' WHERE space_id=$1 AND user_id=$2")
        .bind(space_id).bind(late.user.id).execute(&pool).await.unwrap();
    assert_eq!(
        join(late, &open).await.unwrap_err().status,
        StatusCode::FORBIDDEN
    );
    // A live re-invitation is fulfilled by the link, and once accepted a
    // later departure keeps link access like any other leaver.
    reinvite().await;
    assert_eq!(join(late, &open).await.unwrap()["joined"], true);
    remove_space_member(
        State(state.clone()),
        Extension(late.clone()),
        Path((space.clone(), late.user.external_id.clone())),
    )
    .await
    .unwrap();
    assert_eq!(join(late, &open).await.unwrap()["joined"], true);
    remove_space_member(
        State(state.clone()),
        Extension(owner.clone()),
        Path((space.clone(), late.user.external_id.clone())),
    )
    .await
    .unwrap();
    // Rows written before `removed_at` existed still bar the link.
    sqlx::query(
        "UPDATE public.space_invitations SET removed_at=NULL WHERE space_id=$1 AND user_id=$2",
    )
    .bind(space_id)
    .bind(late.user.id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        join(late, &open).await.unwrap_err().status,
        StatusCode::FORBIDDEN
    );
    remove_space_member(
        State(state.clone()),
        Extension(blocked.clone()),
        Path((space.clone(), blocked.user.external_id.clone())),
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO public.user_blocks(blocker_id,blocked_id) VALUES($1,$2)")
        .bind(owner.user.id)
        .bind(blocked.user.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        join(blocked, &open).await.unwrap_err().status,
        StatusCode::FORBIDDEN
    );

    // A pending username invitation is fulfilled by joining through a link.
    sqlx::query(
        "INSERT INTO public.space_invitations(space_id,user_id,status) VALUES($1,$2,'pending')",
    )
    .bind(space_id)
    .bind(invited.user.id)
    .execute(&pool)
    .await
    .unwrap();
    // A landing channel made private later falls back and stays hidden.
    let (_, landing) = create(owner, json!({"channelId":feedback})).await.unwrap();
    let landing = landing["code"].as_str().unwrap().to_owned();
    sqlx::query("UPDATE public.channels SET private=true WHERE external_id=$1")
        .bind(feedback)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(view(&landing).await.unwrap()["channel"], Value::Null);
    assert_eq!(
        join(invited, &landing).await.unwrap()["channelId"],
        general.as_str()
    );
    let status: String = sqlx::query_scalar(
        "SELECT status FROM public.space_invitations WHERE space_id=$1 AND user_id=$2",
    )
    .bind(space_id)
    .bind(invited.user.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "accepted");
    let recorded: i64 = sqlx::query_scalar("SELECT count(*) FROM public.space_invite_link_uses")
        .fetch_one(&pool)
        .await
        .unwrap();
    // newcomer, late (three times), leaver (twice), blocked, invited.
    assert_eq!(recorded, 8);

    let prepare = |who: &Principal| {
        let state = state.clone();
        let who = who.clone();
        let space = space.clone();
        async move {
            default_link(
                State(state),
                Extension(who),
                Path(space),
                Query(PrepareInput::default()),
            )
            .await
            .map(|Json(link)| serde_json::to_value(link).unwrap())
        }
    };
    assert_eq!(
        prepare(member).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    // More simultaneous opens than pool connections must still create one link.
    let mut opens = tokio::task::JoinSet::new();
    for _ in 0..12 {
        opens.spawn(prepare(owner));
    }
    let prepared = tokio::time::timeout(std::time::Duration::from_secs(10), opens.join_all())
        .await
        .unwrap();
    let first = prepared[0].as_ref().unwrap();
    for result in &prepared {
        assert_eq!(result.as_ref().unwrap(), first);
    }
    assert_eq!(first["maxUses"], 100);
    assert_eq!(first["channel"]["id"], general.as_str());
    let created =
        chrono::DateTime::parse_from_rfc3339(first["createdAt"].as_str().unwrap()).unwrap();
    let expires =
        chrono::DateTime::parse_from_rfc3339(first["expiresAt"].as_str().unwrap()).unwrap();
    assert_eq!(expires - created, chrono::Duration::days(30));
    let code = first["code"].as_str().unwrap();
    // Separate links with custom limits leave the default unchanged.
    let (_, separate) = create(owner, json!({"maxAge":86_400,"maxUses":100}))
        .await
        .unwrap();
    assert_eq!(prepare(owner).await.unwrap()["code"], code);
    revoke_link(
        State(state.clone()),
        Extension(owner.clone()),
        Path((space.clone(), separate["code"].as_str().unwrap().to_owned())),
    )
    .await
    .unwrap();
    // Reopening doesn't reset a partially used link or its expiry.
    sqlx::query("UPDATE public.space_invite_links SET uses=99 WHERE code=$1")
        .bind(code)
        .execute(&pool)
        .await
        .unwrap();
    let reused = prepare(owner).await.unwrap();
    assert_eq!(reused["code"], code);
    assert_eq!(reused["uses"], 99);
    assert_eq!(reused["expiresAt"], first["expiresAt"]);
    sqlx::query("UPDATE public.space_invite_links SET uses=100 WHERE code=$1")
        .bind(code)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(view(code).await.unwrap_err().status, StatusCode::NOT_FOUND);
    let next = prepare(owner).await.unwrap();
    assert_ne!(next["code"], code);
    // Expiration also replaces instead of extending the old record.
    sqlx::query("UPDATE public.space_invite_links SET created_at=created_at-interval '31 days', expires_at=expires_at-interval '31 days' WHERE code=$1")
        .bind(next["code"].as_str().unwrap()).execute(&pool).await.unwrap();
    assert_eq!(
        view(next["code"].as_str().unwrap())
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    let next = prepare(owner).await.unwrap();
    let code = next["code"].as_str().unwrap();
    revoke_link(
        State(state.clone()),
        Extension(owner.clone()),
        Path((space.clone(), code.to_owned())),
    )
    .await
    .unwrap();
    assert_eq!(view(code).await.unwrap_err().status, StatusCode::NOT_FOUND);
    let older = prepare(owner).await.unwrap();
    assert_ne!(older["code"], code);

    let regenerate = |who: &Principal, code: &str| {
        let state = state.clone();
        let who = who.clone();
        let space = space.clone();
        let code = code.to_owned();
        async move {
            regenerate_link(State(state), Extension(who), Path((space, code)))
                .await
                .map(|(status, Json(link))| (status, serde_json::to_value(link).unwrap()))
        }
    };
    // Even if another matching link exists, regeneration must mint a new code
    // and the next open must select it, not the older compatible link.
    let (_, displayed) = create(
        owner,
        json!({"channelId":general,"maxAge":2_592_000,"maxUses":100}),
    )
    .await
    .unwrap();
    let code = displayed["code"].as_str().unwrap();
    assert_eq!(prepare(owner).await.unwrap()["code"], code);
    sqlx::query("UPDATE public.space_invite_links SET uses=18, created_at=created_at-interval '28 days', expires_at=expires_at-interval '28 days' WHERE code=$1")
        .bind(code).execute(&pool).await.unwrap();
    assert_eq!(
        regenerate(member, code).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    let (left, right) = tokio::join!(regenerate(owner, code), regenerate(owner, code));
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    let (success, failure) = if left.is_ok() {
        (left, right)
    } else {
        (right, left)
    };
    assert_eq!(failure.unwrap_err().status, StatusCode::NOT_FOUND);
    let (status, fresh) = success.unwrap();
    assert_eq!(status, StatusCode::CREATED);
    assert_ne!(fresh["code"], code);
    assert_eq!(fresh["uses"], 0);
    assert_eq!(fresh["maxUses"], 100);
    assert_eq!(fresh["channel"]["id"], general.as_str());
    let created =
        chrono::DateTime::parse_from_rfc3339(fresh["createdAt"].as_str().unwrap()).unwrap();
    let expires =
        chrono::DateTime::parse_from_rfc3339(fresh["expiresAt"].as_str().unwrap()).unwrap();
    assert_eq!(expires - created, chrono::Duration::days(30));
    assert!(created.timestamp() >= chrono::Utc::now().timestamp() - 10);
    assert_eq!(prepare(owner).await.unwrap(), fresh);
    assert_eq!(view(code).await.unwrap_err().status, StatusCode::NOT_FOUND);
    assert_eq!(
        view_as(code, "member", false).await.unwrap()["alreadyMember"],
        true
    );
    assert!(view(&open).await.is_ok(), "Other links remain usable");

    // Exhausting the latest link creates a fresh default, never resurfaces an
    // older matching link that is still live. Its own code remains untouched.
    sqlx::query("UPDATE public.space_invite_links SET uses=100 WHERE code=$1")
        .bind(fresh["code"].as_str().unwrap())
        .execute(&pool)
        .await
        .unwrap();
    let fresh = prepare(owner).await.unwrap();
    assert_ne!(fresh["code"], older["code"]);
    assert!(view(older["code"].as_str().unwrap()).await.is_ok());
    assert_eq!(fresh["uses"], 0);
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(fresh["expiresAt"].as_str().unwrap()).unwrap()
            - chrono::DateTime::parse_from_rfc3339(fresh["createdAt"].as_str().unwrap()).unwrap(),
        chrono::Duration::days(30)
    );

    // Reusing is allowed even when the owner's new-invitation budget is spent.
    sqlx::query("UPDATE public.space_invite_limits SET attempts=20 WHERE user_id=$1")
        .bind(owner.user.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(prepare(owner).await.is_ok());
    // A failure after soft-revocation rolls back the whole replacement.
    let code = fresh["code"].as_str().unwrap();
    assert_eq!(
        regenerate(owner, code).await.unwrap_err().status,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert!(view(code).await.is_ok());
    assert_eq!(prepare(owner).await.unwrap(), fresh);

    // Isolate channel-selection cases from the creation budget tested above.
    sqlx::query("UPDATE public.space_invite_limits SET attempts=0 WHERE user_id=$1")
        .bind(owner.user.id)
        .execute(&pool)
        .await
        .unwrap();
    let prepare_for = |who: &Principal, channel: &str| {
        let state = state.clone();
        let who = who.clone();
        let space = space.clone();
        let channel_id = Some(channel.to_owned());
        async move {
            default_link(
                State(state),
                Extension(who),
                Path(space),
                Query(PrepareInput { channel_id }),
            )
            .await
            .map(|Json(link)| serde_json::to_value(link).unwrap())
        }
    };
    sqlx::query("UPDATE public.channels SET name='welcome' WHERE external_id=$1")
        .bind(general)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE public.channels SET name='general',private=false WHERE external_id=$1")
        .bind(feedback)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        prepare(owner).await.unwrap()["channel"]["id"],
        general.as_str(),
        "Oldest wins even when a newer channel is named general"
    );
    assert_eq!(
        prepare_for(member, feedback).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        prepare_for(owner, staff).await.unwrap_err().status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        prepare_for(owner, "missing00001").await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    let selected = prepare_for(owner, feedback).await.unwrap();
    assert_eq!(selected["channel"]["id"], feedback.as_str());
    assert_eq!(prepare_for(owner, feedback).await.unwrap(), selected);
    assert_eq!(prepare(owner).await.unwrap()["code"], fresh["code"]);
    assert!(view(fresh["code"].as_str().unwrap()).await.is_ok());
    let (_, replaced) = regenerate(owner, selected["code"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(replaced["channel"]["id"], feedback.as_str());
    assert_eq!(
        join(member, replaced["code"].as_str().unwrap())
            .await
            .unwrap()["channelId"],
        feedback.as_str()
    );
    // Deleted destinations fall back to the oldest public channel on regeneration.
    sqlx::query("UPDATE public.channels SET deleted_at=now() WHERE external_id=$1")
        .bind(feedback)
        .execute(&pool)
        .await
        .unwrap();
    let (_, fallback) = regenerate(owner, replaced["code"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(fallback["channel"]["id"], general.as_str());
    // Space-only invitations remain usable when no public channels exist.
    sqlx::query("UPDATE public.channels SET private=true WHERE external_id=$1")
        .bind(general)
        .execute(&pool)
        .await
        .unwrap();
    let space_only = prepare(owner).await.unwrap();
    assert_eq!(space_only["channel"], Value::Null);
    assert_eq!(prepare(owner).await.unwrap(), space_only);
    assert_eq!(
        join(member, space_only["code"].as_str().unwrap())
            .await
            .unwrap()["channelId"],
        Value::Null
    );

    pool.close().await;
    admin
        .execute(format!("DROP DATABASE {database} WITH (FORCE)").as_str())
        .await
        .unwrap();
}
