use super::*;
use crate::{Cloudflare, Config, Participant, Registry, public_snapshot};
use std::sync::Arc;
use uuid::Uuid;

fn participant(user: Option<i64>, name: &str) -> Participant {
    serde_json::from_value(json!({
        "id": Uuid::new_v4(), "token": "token", "user_id": user, "avatar_id": 3,
        "name": name, "session": "session", "turn_usernames": [], "muted": false,
        "deafened": false, "lease": 0, "joined": 0, "tracks": {}, "subscriptions": {},
        "pending_offer": false, "operation": false, "operation_started": null,
        "operations": [], "monitor": null, "events": null,
    }))
    .unwrap()
}

#[tokio::test]
async fn calls_in_progress_take_a_changed_name_and_picture() {
    let state = AppState::new(Config::test(true), Arc::new(Cloudflare::new()));
    let mut room = Registry::default();
    let mine = participant(Some(7), "Old");
    let other = participant(Some(8), "Other");
    let (mine_id, other_id) = (mine.id, other.id);
    room.participants.insert(mine.id, mine);
    room.participants.insert(other.id, other);
    state
        .channel_registries
        .lock()
        .await
        .insert("voice".into(), room);

    refresh_calls(&state, 7, "New", 17, Some("picture")).await;
    let rooms = state.channel_registries.lock().await;
    let room = &rooms["voice"];
    assert_eq!(room.participants[&mine_id].name, "New");
    assert_eq!(
        room.participants[&mine_id].avatar_asset.as_deref(),
        Some("picture")
    );
    assert_eq!(room.participants[&other_id].name, "Other");
    assert!(room.participants[&other_id].avatar_asset.is_none());
    // Watchers see a new revision whose snapshot carries the picture.
    assert_eq!(room.revision, 1);
    let snapshot = public_snapshot(room);
    let me = snapshot["participants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == json!(mine_id))
        .unwrap();
    assert_eq!(me["avatarAsset"], "picture");
    assert_eq!(me["avatarId"], 17);
    assert_eq!(me["name"], "New");
    drop(rooms);

    // Removing the picture clears it; an unchanged call is not rewritten.
    refresh_calls(&state, 7, "New", 799, None).await;
    refresh_calls(&state, 9, "Nobody", 0, None).await;
    let rooms = state.channel_registries.lock().await;
    assert!(rooms["voice"].participants[&mine_id].avatar_asset.is_none());
    assert_eq!(rooms["voice"].participants[&mine_id].avatar_id, Some(799));
    assert_eq!(rooms["voice"].participants[&other_id].avatar_id, Some(3));
    assert_eq!(rooms["voice"].revision, 2);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL"]
async fn audiences_are_people_who_can_already_see_the_account(pool: PgPool) {
    let mut ids = Vec::new();
    for name in [
        "me",
        "member",
        "peer",
        "stranger",
        "former",
        "deleted_space",
    ] {
        let id: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES($1,$2,$2) RETURNING id")
            .bind(crate::auth::random_id(12)).bind(name).fetch_one(&pool).await.unwrap();
        ids.push(id);
    }
    let [me, member, peer, _stranger, former, deleted] = ids[..] else {
        unreachable!()
    };
    let space = |deleted: bool| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>("INSERT INTO public.spaces(external_id,name,owner_id,deleted_at) VALUES($1,'Space',$2,CASE WHEN $3 THEN now() END) RETURNING id")
                .bind(crate::auth::random_id(12)).bind(me).bind(deleted).fetch_one(&pool).await.unwrap()
        }
    };
    let (live, gone) = (space(false).await, space(true).await);
    for (space, user, left) in [
        (live, me, false),
        (live, member, false),
        (live, former, true),
        (gone, me, false),
        (gone, deleted, false),
    ] {
        sqlx::query("INSERT INTO public.space_members(space_id,user_id,deleted_at) VALUES($1,$2,CASE WHEN $3 THEN now() END)")
            .bind(space).bind(user).bind(left).execute(&pool).await.unwrap();
    }
    let channel: i64 = sqlx::query_scalar(
        "INSERT INTO public.channels(external_id,name,private) VALUES($1,'dm',true) RETURNING id",
    )
    .bind(crate::auth::random_id(12))
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO public.direct_conversations(channel_id,low_user_id,high_user_id,requested_by) VALUES($1,$2,$3,$2)")
        .bind(channel).bind(me.min(peer)).bind(me.max(peer)).execute(&pool).await.unwrap();

    let mut audience = account_audience(&pool, me).await.unwrap();
    audience.sort_unstable();
    let mut expected = vec![me, member, peer];
    expected.sort_unstable();
    assert_eq!(audience, expected);
}
