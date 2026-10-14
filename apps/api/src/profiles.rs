//! Live profile and space changes. A changed picture or name reaches everyone
//! who can see that person (sharing a space or a DM, and their own other
//! sessions) and calls already in progress; a space's icon or name reaches its
//! members. Gateways deliver them to `profiles` subscriptions and sign picture
//! URLs on the way out. Best effort, with no replay: the next load repairs
//! anything a disconnected client missed.
use crate::{AppState, chat::PROFILE_TOPIC};
use serde_json::{Value, json};
use sqlx::PgPool;

/// After a committed picture, name or username change. Returns at once.
pub(crate) fn account_changed(state: &AppState, user: i64) {
    let state = state.clone();
    tokio::spawn(async move {
        if publish_account(&state, user).await.is_err() {
            tracing::warn!(
                event_name = "profile_update_skipped",
                "profile update was not published"
            );
        }
    });
}

/// After a committed space icon or name change. Returns at once.
pub(crate) fn space_changed(state: &AppState, space: i64) {
    let state = state.clone();
    tokio::spawn(async move {
        if publish_space(&state, space).await.is_err() {
            tracing::warn!(
                event_name = "space_update_skipped",
                "space update was not published"
            );
        }
    });
}

type Person = (String, String, String, i16, Option<String>);

pub(crate) async fn publish_account(state: &AppState, user: i64) -> Result<(), ()> {
    let pool = state.database.as_ref().ok_or(())?;
    let Some((id, username, display_name, avatar_id, avatar_asset)) =
        sqlx::query_as::<_, Person>(
            "SELECT external_id,username,display_name,avatar_id,avatar_asset FROM public.users
             WHERE id=$1 AND deleted_at IS NULL AND username IS NOT NULL AND display_name IS NOT NULL",
        )
        .bind(user)
        .fetch_optional(pool)
        .await
        .map_err(|_| ())?
    else {
        return Ok(());
    };
    refresh_calls(state, user, &display_name, avatar_asset.as_deref()).await;
    let mut person =
        json!({"id":id,"username":username,"displayName":display_name,"avatarId":avatar_id});
    if let Some(asset) = avatar_asset {
        person["avatarAsset"] = json!(asset);
    }
    let audience = account_audience(pool, user).await.map_err(|_| ())?;
    publish(
        state,
        audience,
        json!({"type":"profile.updated","person":person}),
    )
    .await
}

pub(crate) async fn publish_space(state: &AppState, space: i64) -> Result<(), ()> {
    let pool = state.database.as_ref().ok_or(())?;
    let summary = crate::spaces::summary(pool, space).await.map_err(|_| ())?;
    let audience: Vec<i64> = sqlx::query_scalar(
        "SELECT user_id FROM public.space_members WHERE space_id=$1 AND deleted_at IS NULL",
    )
    .bind(space)
    .fetch_all(pool)
    .await
    .map_err(|_| ())?;
    publish(
        state,
        audience,
        json!({"type":"space.updated","space":summary}),
    )
    .await
}

/// Everyone who can already see this person: themselves, anyone sharing an
/// active space, and DM peers.
pub(crate) async fn account_audience(pool: &PgPool, user: i64) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT $1::bigint
         UNION SELECT theirs.user_id FROM public.space_members mine
           JOIN public.space_members theirs ON theirs.space_id=mine.space_id AND theirs.deleted_at IS NULL
           JOIN public.spaces s ON s.id=mine.space_id AND s.deleted_at IS NULL AND NOT s.demo
           WHERE mine.user_id=$1 AND mine.deleted_at IS NULL
         UNION SELECT CASE WHEN d.low_user_id=$1 THEN d.high_user_id ELSE d.low_user_id END
           FROM public.direct_conversations d JOIN public.channels c ON c.id=d.channel_id AND c.deleted_at IS NULL
           WHERE $1 IN (d.low_user_id,d.high_user_id)",
    )
    .bind(user)
    .fetch_all(pool)
    .await
}

async fn publish(state: &AppState, audience: Vec<i64>, event: Value) -> Result<(), ()> {
    let Some(chat) = &state.chat else {
        return Ok(());
    };
    if chat
        .publish(PROFILE_TOPIC, &json!({"audience":audience,"event":event}))
        .await
    {
        Ok(())
    } else {
        Err(())
    }
}

/// Participants carry the name and picture they joined with; update them in
/// every active call this account is in, so the room snapshot carries the
/// change to everyone watching it.
async fn refresh_calls(state: &AppState, user: i64, name: &str, asset: Option<&str>) {
    let Ok(rooms) = state.media_rooms().await else {
        return;
    };
    for room in rooms {
        let present = room
            .read(|r| Ok(r.participants.values().any(|p| p.user_id == Some(user))))
            .await
            .unwrap_or(false);
        if !present {
            continue;
        }
        let _ = room
            .update(|r| {
                for participant in r
                    .participants
                    .values_mut()
                    .filter(|p| p.user_id == Some(user))
                {
                    name.clone_into(&mut participant.name);
                    participant.avatar_asset = asset.map(str::to_owned);
                }
                Ok(())
            })
            .await;
    }
}

#[cfg(test)]
mod tests;
