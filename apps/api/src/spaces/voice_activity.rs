//! Coarse polling projection: never expose rooms or participant identities.
use super::*;
use std::collections::{BTreeSet, HashMap};

pub(super) async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Value>, ApiError> {
    let pool = pool(&state)?;
    // Explicit membership also excludes feedback-team access and DMs from the
    // rail. Channel participation below adds visibility and persistent join.
    let spaces: HashMap<i64, String> = sqlx::query_as(
        "SELECT s.id,s.external_id FROM public.spaces s
         JOIN public.space_members sm ON sm.space_id=s.id
         WHERE sm.user_id=$1 AND sm.deleted_at IS NULL
           AND s.deleted_at IS NULL AND NOT s.demo",
    )
    .bind(principal.user.id)
    .fetch_all(pool)
    .await
    .map_err(database_error)?
    .into_iter()
    .collect();
    let mut active = BTreeSet::new();
    if !spaces.is_empty() {
        for room in state.media_rooms().await? {
            let Some(channel) = &room.media_channel else {
                continue; // The retired public/demo room is not a member space.
            };
            let access = match channel_participation(pool, channel, Some(principal.user.id)).await {
                Ok(access) => access,
                Err(error) if error.status == StatusCode::NOT_FOUND => continue,
                Err(error) => return Err(error),
            };
            let Some(space) = access.space_id.and_then(|id| spaces.get(&id)) else {
                continue;
            };
            if active.contains(space) {
                continue;
            }
            let occupied = room
                .read(|registry| {
                    Ok(crate::presence_snapshot(registry)["participants"]
                        .as_array()
                        .is_some_and(|participants| !participants.is_empty()))
                })
                .await?;
            if occupied {
                active.insert(space.clone());
            }
        }
    }
    Ok(Json(json!({"spaceIds":active})))
}
