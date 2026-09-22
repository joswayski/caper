//! Account channels use the existing media state machine with an isolated room.
//! A media capability never grants space membership or survives account logout.
use super::*;
use axum::{
    extract::{Path, Request},
    routing::any,
};
use tower::ServiceExt;

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/api/channels/{channel}/media/{*operation}", any(dispatch))
}

async fn dispatch(
    State(mut state): State<AppState>,
    Path((channel, operation)): Path<(String, String)>,
    mut request: Request,
) -> Result<Response, ApiError> {
    if channel.len() != 12 || !channel.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "channel not found"));
    }
    let token = account_token(request.headers())
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "sign in required"))?;
    state.media_channel = Some(channel);
    state.media_session = Some(Sha256::digest(token.as_bytes()).to_vec());
    state.check_media_access().await?;
    if let Some(token) = request
        .headers()
        .get("x-caper-media-token")
        .and_then(|v| v.to_str().ok())
    {
        state
            .read(|r| {
                let id = authenticate(r, token)?;
                if r.participants[&id].account_session != state.media_session {
                    return Err(ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized"));
                }
                Ok(())
            })
            .await?;
    }
    // Reuse every signaling handler, including monitor and TURN lifecycles. The
    // channel is server-selected state, never a client-supplied provider session.
    let query = request
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    *request.uri_mut() = format!("/api/media/{operation}{query}")
        .parse()
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid media operation"))?;
    let response = media_routes().with_state(state).oneshot(request).await;
    match response {
        Ok(response) => Ok(response),
        Err(never) => match never {},
    }
}

impl AppState {
    pub(super) async fn check_media_access(&self) -> Result<(), ApiError> {
        let Some(channel) = &self.media_channel else {
            return Ok(());
        };
        let pool = self.database.as_ref().ok_or_else(chat::unavailable)?;
        let session = self
            .media_session
            .as_deref()
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "sign in required"))?;
        let user = crate::spaces::session_user(pool, session).await?;
        let access = crate::spaces::channel_access(pool, channel, Some(user)).await?;
        // The guest demo has exactly one media room, on its original endpoints.
        if access.demo {
            return Err(ApiError::new(StatusCode::NOT_FOUND, "channel not found"));
        }
        Ok(())
    }

    pub(super) async fn media_rooms(&self) -> Result<Vec<Self>, ApiError> {
        let channels = if let Some(store) = &self.store {
            store.active_channels().await?
        } else {
            self.channel_registries
                .lock()
                .await
                .iter()
                .filter(|(_, r)| {
                    !r.participants.is_empty()
                        || !r.reservations.is_empty()
                        || !r.cleanup.is_empty()
                })
                .map(|(channel, _)| channel.clone())
                .collect()
        };
        let mut rooms = Vec::with_capacity(channels.len() + 1);
        rooms.push(self.clone());
        for channel in channels {
            let mut room = self.clone();
            room.media_channel = Some(channel);
            room.media_session = None;
            rooms.push(room);
        }
        Ok(rooms)
    }

    /// Removal, privacy changes, deletion and expired/revoked account sessions
    /// end existing calls, not just future joins. Provider cleanup is queued.
    pub(super) async fn revoke_media_access(&self) -> Result<(), ApiError> {
        if self.media_channel.is_none() {
            return Ok(());
        }
        let participants = self
            .read(|r| {
                Ok(r.participants
                    .values()
                    .map(|p| (p.id, p.account_session.clone()))
                    .collect::<Vec<_>>())
            })
            .await?;
        let mut revoked = Vec::new();
        for (id, session) in participants {
            let mut account = self.clone();
            account.media_session = session;
            if let Err(error) = account.check_media_access().await {
                // On database outages stop disclosing/control immediately, but
                // don't mass-delete healthy SFU sessions for a transient error.
                if error.status.is_server_error() {
                    return Err(error);
                }
                revoked.push(id);
            }
        }
        if !revoked.is_empty() {
            self.update(|r| {
                for id in &revoked {
                    remove_participant_locked(r, *id);
                }
                Ok(())
            })
            .await?;
        }
        Ok(())
    }
}
