//! Custom profile pictures and space icons. They upload through the assets
//! flow with a `purpose`, are verified like attachments plus their real pixel
//! size, and completing one applies it. Responses and socket frames carry only
//! the asset ID (`avatarAsset`, `iconAsset`); URLs are signed as they leave,
//! exactly like attachment URLs. Replaced and removed pictures keep their rows
//! and objects, outside the file storage quota.
use crate::{
    ApiError, AppState,
    assets::{CdnSigner, original_key},
    auth::Principal,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use serde_json::{Value, json};
use sqlx::PgPool;

/// Still pictures arrive cropped and resized to at most 512 px by the client.
pub(crate) const STILL_MAX_BYTES: i64 = 1024 * 1024;
/// GIFs upload unchanged so they keep their animation.
pub(crate) const GIF_MAX_BYTES: i64 = 5 * 1024 * 1024;
pub(crate) const MAX_DIMENSION: u32 = 1024;
/// Picture uploads (profile and space icons together) per person per day.
pub(crate) const PER_DAY: i64 = 50;
/// Enough of a file to reach a JPEG frame header behind EXIF and ICC segments.
pub(crate) const HEAD_BYTES: u64 = 64 * 1024;
/// Picture IDs are longer than attachment IDs (16), so a stray `avatarAsset`
/// in a payload can never be signed into access to someone's attachment.
pub(crate) const ID_LENGTH: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    Attachment,
    Avatar,
    SpaceIcon,
}

impl Purpose {
    pub(crate) fn parse(value: Option<&str>) -> Result<Self, ApiError> {
        match value {
            None | Some("attachment") => Ok(Self::Attachment),
            Some("avatar") => Ok(Self::Avatar),
            Some("spaceIcon") => Ok(Self::SpaceIcon),
            Some(_) => Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid purpose")),
        }
    }

    pub(crate) fn from_column(value: &str) -> Self {
        match value {
            "avatar" => Self::Avatar,
            "space_icon" => Self::SpaceIcon,
            _ => Self::Attachment,
        }
    }

    pub(crate) fn column(self) -> &'static str {
        match self {
            Self::Attachment => "attachment",
            Self::Avatar => "avatar",
            Self::SpaceIcon => "space_icon",
        }
    }

    pub(crate) fn is_picture(self) -> bool {
        self != Self::Attachment
    }
}

/// Formats every client can show, still or animated. AVIF and HEIC are left
/// out: clients re-encode stills to WebP/PNG/JPEG first.
pub(crate) fn allowed_type(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    )
}

pub(crate) fn max_bytes(content_type: &str) -> i64 {
    if content_type == "image/gif" {
        GIF_MAX_BYTES
    } else {
        STILL_MAX_BYTES
    }
}

/// Validates a picture reservation before anything is stored.
pub(crate) fn check_reservation(
    content_type: &str,
    byte_size: i64,
    width: Option<i32>,
    height: Option<i32>,
) -> Result<(), ApiError> {
    if !allowed_type(content_type) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "pictures must be PNG, JPEG, WebP or GIF",
        )
        .with_code("picture_type"));
    }
    let fits = |side: Option<i32>| {
        side.and_then(|side| u32::try_from(side).ok())
            .is_some_and(|side| (1..=MAX_DIMENSION).contains(&side))
    };
    if !fits(width) || !fits(height) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid media dimensions",
        ));
    }
    if byte_size > max_bytes(content_type) {
        return Err(
            ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "picture is too large")
                .with_code("picture_too_large"),
        );
    }
    Ok(())
}

pub(crate) fn rejected() -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "that picture could not be used",
    )
    .with_code("picture_invalid")
}

/// Pixel size read from a picture's leading bytes, never from client metadata.
pub(crate) fn dimensions(content_type: &str, bytes: &[u8]) -> Option<(u32, u32)> {
    let u16_le = |at: usize| {
        bytes
            .get(at..at + 2)
            .map(|b| u32::from(u16::from_le_bytes([b[0], b[1]])))
    };
    let u16_be = |at: usize| {
        bytes
            .get(at..at + 2)
            .map(|b| u32::from(u16::from_be_bytes([b[0], b[1]])))
    };
    let u24_le = |at: usize| {
        bytes
            .get(at..at + 3)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], 0]))
    };
    let at = |offset: usize, magic: &[u8]| bytes.get(offset..offset + magic.len()) == Some(magic);
    match content_type {
        // Logical screen descriptor; frames are drawn within it.
        "image/gif" if at(0, b"GIF87a") || at(0, b"GIF89a") => Some((u16_le(6)?, u16_le(8)?)),
        "image/png" if at(0, b"\x89PNG\r\n\x1a\n") && at(12, b"IHDR") => {
            let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
            let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
            Some((width, height))
        }
        "image/webp" if at(0, b"RIFF") && at(8, b"WEBP") => {
            if at(12, b"VP8X") {
                Some((u24_le(24)? + 1, u24_le(27)? + 1))
            } else if at(12, b"VP8L") && bytes.get(20) == Some(&0x2f) {
                let bits = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
                Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
            } else if at(12, b"VP8 ") && at(23, b"\x9d\x01\x2a") {
                Some((u16_le(26)? & 0x3fff, u16_le(28)? & 0x3fff))
            } else {
                None
            }
        }
        "image/jpeg" if at(0, b"\xff\xd8") => {
            let mut offset = 2;
            loop {
                // Markers may be preceded by any number of fill bytes.
                while bytes.get(offset) == Some(&0xff) && bytes.get(offset + 1) == Some(&0xff) {
                    offset += 1;
                }
                if *bytes.get(offset)? != 0xff {
                    return None;
                }
                let marker = *bytes.get(offset + 1)?;
                match marker {
                    // Start of frame, except DHT (C4), JPG (C8) and DAC (CC).
                    0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                        return Some((u16_be(offset + 7)?, u16_be(offset + 5)?));
                    }
                    // Start of scan or end of image before any frame header.
                    0xd9 | 0xda => return None,
                    0x01 | 0xd0..=0xd7 => offset += 2,
                    _ => offset += 2 + usize::try_from(u16_be(offset + 2)?).ok()?,
                }
            }
        }
        _ => None,
    }
}

/// Whether stored picture bytes have an acceptable real size.
pub(crate) fn fits(content_type: &str, head: &[u8]) -> bool {
    dimensions(content_type, head).is_some_and(|(width, height)| {
        (1..=MAX_DIMENSION).contains(&width) && (1..=MAX_DIMENSION).contains(&height)
    })
}

/// What completing a picture did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Applied,
    /// A picture reserved after this one already completed, so this one is
    /// kept like a replaced picture without undoing the newer choice.
    Superseded,
    /// The space is no longer the uploader's (or their account is gone), so
    /// the caller discards the upload.
    Gone,
}

/// Applies a just-completed picture unless a newer one already was.
pub(crate) async fn apply(
    tx: &mut sqlx::PgConnection,
    purpose: Purpose,
    asset: &str,
    owner: i64,
    space: Option<i64>,
) -> Result<Outcome, sqlx::Error> {
    // Lock the account or space first: concurrent completions then check for a
    // newer picture one after another, each seeing the other's committed row.
    let locked = match (purpose, space) {
        (Purpose::Avatar, _) => {
            sqlx::query("SELECT 1 FROM public.users WHERE id=$1 AND deleted_at IS NULL FOR UPDATE")
                .bind(owner)
                .fetch_optional(&mut *tx)
                .await?
        }
        (Purpose::SpaceIcon, Some(space)) => sqlx::query(
            "SELECT 1 FROM public.spaces WHERE id=$1 AND owner_id=$2 AND deleted_at IS NULL AND NOT demo FOR UPDATE",
        )
        .bind(space)
        .bind(owner)
        .fetch_optional(&mut *tx)
        .await?,
        _ => None,
    };
    if locked.is_none() {
        return Ok(Outcome::Gone);
    }
    if purpose == Purpose::Avatar {
        let before_selection: bool = sqlx::query_scalar(
            "SELECT a.created_at <= u.avatar_selected_at FROM public.assets a
             JOIN public.users u ON u.id=$2 WHERE a.external_id=$1",
        )
        .bind(asset)
        .bind(owner)
        .fetch_one(&mut *tx)
        .await
        .map(|value: Option<bool>| value.unwrap_or(false))?;
        if before_selection {
            return Ok(Outcome::Superseded);
        }
    }
    // Reservation order is the order the person chose pictures in. A picture
    // that completed was applied (or discarded and deleted), and a removal
    // after it must not be undone either.
    let newer: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM public.assets n JOIN public.assets a ON a.external_id=$1
         WHERE n.owner_id=$2 AND n.purpose=a.purpose AND n.space_id IS NOT DISTINCT FROM a.space_id
           AND n.uploaded_at IS NOT NULL AND n.deleted_at IS NULL
           AND (n.created_at,n.id) > (a.created_at,a.id))",
    )
    .bind(asset)
    .bind(owner)
    .fetch_one(&mut *tx)
    .await?;
    if newer {
        return Ok(Outcome::Superseded);
    }
    let applied = match (purpose, space) {
        (Purpose::Avatar, _) => sqlx::query(
            "UPDATE public.users SET avatar_asset=$1, updated_at=now()
             WHERE id=$2 AND deleted_at IS NULL",
        )
        .bind(asset)
        .bind(owner)
        .execute(&mut *tx)
        .await?,
        (Purpose::SpaceIcon, Some(space)) => sqlx::query(
            "UPDATE public.spaces SET icon_asset=$1 WHERE id=$2 AND owner_id=$3 AND deleted_at IS NULL AND NOT demo",
        )
        .bind(asset)
        .bind(space)
        .bind(owner)
        .execute(&mut *tx)
        .await?,
        _ => return Ok(Outcome::Gone),
    };
    Ok(if applied.rows_affected() == 1 {
        Outcome::Applied
    } else {
        Outcome::Gone
    })
}

/// What a completed picture upload returns besides its description: the
/// caller's account, or the space, as they are now.
pub(crate) async fn applied_body(
    state: &AppState,
    pool: &PgPool,
    purpose: Purpose,
    owner: i64,
    space: Option<i64>,
) -> Result<Option<(&'static str, Value)>, ApiError> {
    match (purpose, space) {
        (Purpose::Avatar, _) => {
            let user = crate::accounts::load(pool, owner)
                .await
                .map_err(|_| crate::assets::unavailable())?
                .ok_or_else(crate::assets::not_found)?;
            Ok(Some(("account", crate::own_account(state, &user).await)))
        }
        (Purpose::SpaceIcon, Some(space)) => {
            Ok(Some(("space", crate::spaces::summary(pool, space).await?)))
        }
        _ => Ok(None),
    }
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/account/avatar", delete(remove_avatar))
        .route("/api/account/avatar/history", get(avatar_history))
        .route("/api/account/avatar/select", post(select_avatar))
        .route("/api/account/avatar/regenerate", post(regenerate_avatar))
        .route("/api/spaces/{space}/icon", delete(remove_icon))
}

fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.database.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "account service unavailable",
        )
    })
}

/// Pick one of the other 799 mascots. The guard and choice use the current row,
/// so a concurrent photo upload cannot be replaced by a stale regeneration.
async fn regenerate_avatar(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Value>, ApiError> {
    let user: crate::accounts::User = sqlx::query_as(
        "UPDATE public.users SET avatar_id=(avatar_id + 1 + floor(random()*799)::int)%800,
         updated_at=now() WHERE id=$1 AND deleted_at IS NULL AND avatar_asset IS NULL
         RETURNING id, external_id, avatar_id, avatar_asset, email, username, display_name",
    )
    .bind(principal.user.id)
    .fetch_optional(pool(&state)?)
    .await
    .map_err(|_| crate::assets::unavailable())?
    .ok_or_else(|| {
        ApiError::new(
            StatusCode::CONFLICT,
            "custom picture present or account unavailable",
        )
    })?;
    crate::profiles::account_changed(&state, user.id);
    Ok(Json(crate::own_account(&state, &user).await))
}

/// Back to the same saved mascot: only the custom reference is cleared.
pub(crate) async fn remove_avatar(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Value>, ApiError> {
    let pool = pool(&state)?;
    sqlx::query(
        "UPDATE public.users SET avatar_selected_at=clock_timestamp(),
         avatar_asset=NULL, updated_at=now()
         WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(principal.user.id)
    .execute(pool)
    .await
    .map_err(|_| crate::assets::unavailable())?;
    crate::profiles::account_changed(&state, principal.user.id);
    let user = crate::accounts::load(pool, principal.user.id)
        .await
        .map_err(|_| crate::assets::unavailable())?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized"))?;
    Ok(Json(crate::own_account(&state, &user).await))
}

#[derive(Default, serde::Deserialize)]
pub(crate) struct AvatarHistoryQuery {
    pub cursor: Option<String>,
}

pub(crate) async fn avatar_history(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<AvatarHistoryQuery>,
) -> Result<Json<Value>, ApiError> {
    let mut pictures: Vec<String> = sqlx::query_scalar(
        "SELECT a.external_id FROM public.assets a
         WHERE a.owner_id=$1 AND a.purpose='avatar'
           AND a.uploaded_at IS NOT NULL AND a.deleted_at IS NULL AND a.purged_at IS NULL
           AND ($2::text IS NULL OR (a.created_at,a.id) <
             (SELECT c.created_at,c.id FROM public.assets c
              WHERE c.external_id=$2 AND c.owner_id=$1 AND c.purpose='avatar'))
         ORDER BY a.created_at DESC,a.id DESC LIMIT 41",
    )
    .bind(principal.user.id)
    .bind(query.cursor)
    .fetch_all(pool(&state)?)
    .await
    .map_err(|_| crate::assets::unavailable())?;
    let next = (pictures.len() > 40).then(|| pictures[39].clone());
    pictures.truncate(40);
    Ok(Json(
        json!({"pictures": pictures.into_iter().map(|id| json!({"avatarAsset": id})).collect::<Vec<_>>(), "nextCursor": next}),
    ))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SelectAvatar {
    pub avatar_asset: String,
    // Value keeps the field required while allowing an explicit JSON null.
    pub expected_avatar_asset: Value,
}

pub(crate) async fn select_avatar(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<SelectAvatar>,
) -> Result<Json<Value>, ApiError> {
    let expected = match &request.expected_avatar_asset {
        Value::Null => None,
        Value::String(asset) => Some(asset.as_str()),
        _ => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid expectedAvatarAsset",
            ));
        }
    };
    let pool = pool(&state)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|_| crate::assets::unavailable())?;
    let current: Option<String> = sqlx::query_scalar(
        "SELECT avatar_asset FROM public.users WHERE id=$1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(principal.user.id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| crate::assets::unavailable())?
    .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized"))?;
    if current.as_deref() != expected {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Your profile picture changed. Refresh and try again.",
        ));
    }
    // Do not lock assets: completion locks its asset before locking the user.
    let target: Option<String> = sqlx::query_scalar(
        "SELECT external_id FROM public.assets WHERE external_id=$1 AND owner_id=$2
         AND purpose='avatar' AND uploaded_at IS NOT NULL AND deleted_at IS NULL AND purged_at IS NULL",
    )
        .bind(&request.avatar_asset)
        .bind(principal.user.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| crate::assets::unavailable())?;
    let target = target
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Profile picture is unavailable."))?;
    sqlx::query(
        "UPDATE public.users SET avatar_asset=$2,
         avatar_selected_at=clock_timestamp(), updated_at=now() WHERE id=$1",
    )
    .bind(principal.user.id)
    .bind(target)
    .execute(&mut *tx)
    .await
    .map_err(|_| crate::assets::unavailable())?;
    tx.commit()
        .await
        .map_err(|_| crate::assets::unavailable())?;
    crate::profiles::account_changed(&state, principal.user.id);
    let user = crate::accounts::load(pool, principal.user.id)
        .await
        .map_err(|_| crate::assets::unavailable())?
        .ok_or_else(crate::assets::not_found)?;
    Ok(Json(crate::own_account(&state, &user).await))
}

pub(crate) async fn remove_icon(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(space): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let pool = pool(&state)?;
    let id: i64 = sqlx::query_scalar(
        "UPDATE public.spaces SET icon_asset=NULL WHERE external_id=$1 AND owner_id=$2 AND deleted_at IS NULL AND NOT demo RETURNING id",
    )
    .bind(&space)
    .bind(principal.user.id)
    .fetch_optional(pool)
    .await
    .map_err(|_| crate::assets::unavailable())?
    .ok_or_else(crate::spaces::not_found)?;
    crate::profiles::space_changed(&state, id);
    Ok(Json(crate::spaces::summary(pool, id).await?))
}

/// Sets every person's current custom picture (`avatarAsset`) in messages
/// and events by account ID, including nested people: thread participants,
/// reply and forward authors. Stored payloads never keep a picture, so a
/// changed or removed one is never shown from an old snapshot. One query.
pub(crate) async fn enrich_people<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    values: &mut [Value],
) -> Result<(), sqlx::Error> {
    let mut ids = std::collections::HashSet::new();
    for value in values.iter() {
        people(value, &mut |id| {
            ids.insert(id.to_owned());
        });
    }
    if ids.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = ids.into_iter().collect();
    let pictures: std::collections::HashMap<String, String> = sqlx::query_as(
        "SELECT external_id, avatar_asset FROM public.users
         WHERE external_id = ANY($1) AND avatar_asset IS NOT NULL AND deleted_at IS NULL",
    )
    .bind(&ids)
    .fetch_all(executor)
    .await?
    .into_iter()
    .collect();
    for value in values.iter_mut() {
        set_pictures(value, &pictures);
    }
    Ok(())
}

/// A person in a message: an object with an account ID and a saved avatar.
fn person(object: &serde_json::Map<String, Value>) -> Option<&str> {
    object
        .contains_key("avatarId")
        .then(|| object.get("id").and_then(Value::as_str))
        .flatten()
}

fn people(value: &Value, found: &mut impl FnMut(&str)) {
    match value {
        Value::Object(object) => {
            if let Some(id) = person(object) {
                found(id);
            }
            for child in object.values() {
                people(child, found);
            }
        }
        Value::Array(items) => {
            for item in items {
                people(item, found);
            }
        }
        _ => {}
    }
}

fn set_pictures(value: &mut Value, pictures: &std::collections::HashMap<String, String>) {
    match value {
        Value::Object(object) => {
            if let Some(id) = person(object) {
                match pictures.get(id) {
                    Some(asset) => {
                        let asset = json!(asset);
                        object.insert("avatarAsset".into(), asset);
                    }
                    None => {
                        object.remove("avatarAsset");
                    }
                }
            }
            for child in object.values_mut() {
                set_pictures(child, pictures);
            }
        }
        Value::Array(items) => {
            for item in items {
                set_pictures(item, pictures);
            }
        }
        _ => {}
    }
}

/// Cheap check before parsing a response: does it mention a picture at all?
pub(crate) fn mentioned(bytes: &[u8]) -> bool {
    [b"\"avatarAsset\"".as_slice(), b"\"iconAsset\"".as_slice()]
        .iter()
        .any(|needle| bytes.windows(needle.len()).any(|window| window == *needle))
}

/// Adds a signed `avatarUrl`/`iconUrl` beside every `avatarAsset`/`iconAsset`.
pub(crate) fn sign(value: &mut Value, signer: &CdnSigner, now: i64) {
    match value {
        Value::Object(object) => {
            for (asset, url) in [("avatarAsset", "avatarUrl"), ("iconAsset", "iconUrl")] {
                match object.get(asset).and_then(Value::as_str) {
                    Some(id) if valid_id(id) => {
                        let signed = signer.url(&original_key(id), now);
                        object.insert(url.into(), json!(signed));
                    }
                    _ => {
                        // SQL builds `null` for people without a picture.
                        if object.get(asset).is_some_and(Value::is_null) {
                            object.remove(asset);
                        }
                        object.remove(url);
                    }
                }
            }
            for child in object.values_mut() {
                sign(child, signer, now);
            }
        }
        Value::Array(items) => {
            for item in items {
                sign(item, signer, now);
            }
        }
        _ => {}
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == ID_LENGTH && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Signs a socket frame on its way out; untouched when it has no pictures.
pub(crate) fn sign_frame(mut frame: Value, signer: Option<&CdnSigner>) -> Value {
    if let Some(signer) = signer {
        sign(&mut frame, signer, chrono::Utc::now().timestamp());
    }
    frame
}

#[cfg(test)]
mod tests;
