use crate::api::Api;
use crate::gateway::{self, GatewayEvent};
use crate::invites::{InviteLink, InvitePreview, JoinedInvite};
use crate::model::{
    Account, BlockedAccount, Channel, ChatSession, DirectConversation, History, Member, Message,
    NotificationSettings, Person, Space, SpaceDetail, Spaces,
};
use crate::notifications::{Change, Saved, Scope};
use eframe::egui;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, Sender},
};
use std::thread;
use std::time::{Duration, Instant};

pub enum Command {
    AvatarHistory {
        epoch: u64,
        request: Instant,
        token: String,
        cursor: Option<String>,
    },
    SelectAvatar {
        epoch: u64,
        token: String,
        asset: String,
        expected: Option<String>,
    },
    /// Upload (`path`) or remove (`None`) a profile picture or space icon.
    /// `animate` decodes an uploaded GIF's frames for immediate display.
    Picture {
        epoch: u64,
        token: String,
        target: crate::pictures::Target,
        path: Option<PathBuf>,
        animate: bool,
        edit: Option<crate::attachment_edit::Edit>,
        scratch: Option<Arc<crate::uploads::Scratch>>,
    },
    SendProfile {
        epoch: u64,
        person: String,
        token: String,
        send: Box<crate::profile_dm::Send>,
    },
    WhatsNew,
    Forward {
        generation: u64,
        request: u64,
        operation: crate::forwarding::Operation,
    },
    LoadDirects {
        generation: u64,
        token: String,
    },
    OpenInvitation {
        epoch: u64,
        navigation: u64,
        token: String,
        space: String,
        channel: Option<String>,
    },
    /// Web's background membership reconcile: `GET /api/spaces`, then the
    /// open space's detail while it is still listed. `revision` and
    /// `navigation` come back unchanged so the app can discard a snapshot
    /// that a change made meanwhile has overtaken.
    RefreshMembership {
        epoch: u64,
        request: u64,
        revision: u64,
        navigation: u64,
        token: String,
        space: Option<String>,
    },
    /// `GET /api/feedback`: the Caper team's inbox (`before` pages older).
    LoadFeedback {
        epoch: u64,
        request: u64,
        token: String,
        before: Option<String>,
        /// The archived view instead of the inbox.
        archived: bool,
    },
    /// `PUT /api/feedback/archive` for one reported thread.
    ArchiveFeedback {
        epoch: u64,
        token: String,
        channel: String,
        thread_root: String,
        archived: bool,
    },
    /// `GET /api/people`, for `@` suggestions in DMs and the space invite and
    /// new DM username fields.
    LoadPeople {
        generation: u64,
        token: String,
    },
    LookupDirect {
        generation: u64,
        navigation: u64,
        request: u64,
        token: String,
        username: String,
    },
    /// Message requests, blocks, DM privacy and notification settings, fenced
    /// by the account epoch rather than the conversation generation.
    Account {
        epoch: u64,
        token: String,
        operation: AccountOperation,
    },
    CreateDirect {
        generation: u64,
        navigation: u64,
        token: String,
        username: String,
    },
    ReadDirect {
        token: String,
        id: String,
        seq: String,
    },
    LoadUnread {
        epoch: u64,
        revision: u64,
        visit: Option<u64>,
        token: String,
    },
    LoadVoiceActivity {
        epoch: u64,
        token: String,
    },
    ThreadUnread {
        scope: (u64, u64, String),
        revision: u64,
        token: String,
        read: Option<(String, String)>,
    },
    ReadChannel {
        epoch: u64,
        token: String,
        space: String,
        channel: String,
        seq: String,
    },
    Restore {
        generation: u64,
    },
    RequestCode {
        generation: u64,
        email: String,
    },
    VerifyCode {
        generation: u64,
        challenge: String,
        code: String,
    },
    Profile {
        generation: u64,
        token: String,
        username: String,
        display_name: String,
    },
    PrepareNavigation {
        generation: u64,
        navigation: u64,
        token: Option<String>,
        space: Option<String>,
        channel: Option<String>,
        name: String,
        cached: Option<History>,
        /// Open `channel` by id even when it is a space channel outside the
        /// lists: the Caper team visiting a feedback report.
        visit: bool,
    },
    PrefetchNavigation {
        generation: u64,
        request: u64,
        space: Option<String>,
        channel: Option<String>,
        token: Option<String>,
    },
    LoadChannel {
        generation: u64,
        token: Option<String>,
        channel: String,
        name: String,
        general: bool,
    },
    CreateSession {
        generation: u64,
        token: Option<String>,
        name: String,
    },
    LoadOlder {
        generation: u64,
        request: u64,
        token: Option<String>,
        channel: String,
        before: String,
    },
    LoadContext {
        generation: u64,
        request: u64,
        token: Option<String>,
        channel: String,
        root: Option<String>,
        anchor: String,
        newer: bool,
    },
    LoadThread {
        generation: u64,
        request: u64,
        token: Option<String>,
        channel: String,
        root: String,
        before: Option<String>,
    },
    MediaStatus {
        generation: u64,
        root: String,
        token: Option<String>,
        channel: Option<String>,
    },
    /// Web's `media.prepare`: best effort, no event.
    PrepareVoice {
        token: String,
        channel: String,
    },
    CheckVoice {
        request: u64,
        voice_generation: u64,
        token: String,
        space: String,
        channel: String,
    },
    Connect {
        generation: u64,
        token: Option<String>,
        channel: String,
        cursor: String,
        presence: Option<(String, Vec<String>)>,
        media: gateway::MediaWatch,
    },
    Send {
        generation: u64,
        token: Option<String>,
        chat_token: String,
        channel: String,
        client_id: String,
        text: String,
        thread_root_id: Option<String>,
        broadcast: bool,
        reply_to_id: Option<String>,
        /// Resent unchanged on retry: ids join the idempotency hash.
        attachment_ids: Vec<String>,
        spoiler_attachment_ids: Vec<String>,
    },
    /// Whether uploads are configured, and the compression settings.
    AssetUsage {
        token: String,
    },
    RefreshAttachmentUrls {
        token: String,
        ids: Vec<String>,
    },
    CancelAssets {
        token: String,
        ids: Vec<String>,
    },
    /// Prepare (compress) and upload one file off the UI thread, reporting
    /// progress.
    Upload {
        key: u64,
        token: String,
        channel: String,
        path: PathBuf,
        /// The original's name, which an edited upload keeps with the output's extension.
        name: String,
        /// Crop, turns and marks applied to the original before upload.
        edit: Option<crate::attachment_edit::Edit>,
        source_scratch: Option<Arc<crate::uploads::Scratch>>,
        prepared: Option<Arc<crate::uploads::Prepared>>,
        settings: crate::compress::Compression,
        cancel: Arc<AtomicBool>,
    },
    React {
        generation: u64,
        token: Option<String>,
        chat_token: String,
        channel: String,
        message: String,
        emoji: String,
        active: bool,
    },
    Pin {
        generation: u64,
        token: Option<String>,
        chat_token: String,
        channel: String,
        message: String,
        active: bool,
    },
    EditMessage {
        generation: u64,
        request: u64,
        token: Option<String>,
        chat_token: String,
        original: Box<Message>,
        text: String,
    },
    DeleteMessage {
        generation: u64,
        request: u64,
        token: Option<String>,
        chat_token: String,
        channel: String,
        message: String,
    },
    ReloadMessage {
        generation: u64,
        request: u64,
        token: Option<String>,
        channel: String,
        message: String,
    },
    MessageVersions {
        generation: u64,
        request: u64,
        token: Option<String>,
        channel: String,
        message: String,
        before: Option<u32>,
    },
    Typing {
        token: Option<String>,
        chat_token: String,
        channel: String,
        typing: bool,
    },
    /// Who reacted to one message, read like history. `revision` is the
    /// snapshot's `reactionSeq` the result is cached under.
    LoadReactors {
        generation: u64,
        token: Option<String>,
        channel: String,
        message: String,
        revision: String,
    },
    Admin {
        generation: u64,
        token: String,
        operation: AdminOperation,
    },
    Logout {
        generation: u64,
        token: String,
    },
    PersistCredential {
        generation: u64,
        token: String,
    },
    ClearCredential {
        generation: u64,
    },
    Activity,
    StopGateway,
    StartNotifications {
        epoch: u64,
        token: String,
    },
    StopNotifications,
}

#[derive(Clone, Debug)]
pub enum AccountOperation {
    LoadBlocks,
    /// `PUT` or `DELETE /api/blocks/{id}`; `request` is a DM request to drop.
    SetBlock {
        account: BlockedAccount,
        blocked: bool,
        request: Option<String>,
    },
    InviteToSpace {
        space: Space,
        username: String,
        user: String,
        request: u64,
    },
    Accept(String),
    Decline(String),
    LoadPrivacy,
    SavePrivacy {
        value: String,
        previous: Option<String>,
    },
    /// `revision` comes back so an answer older than a change is dropped.
    LoadNotifications {
        revision: u64,
    },
    SaveNotifications {
        scope: Scope,
        change: Change,
    },
    /// An owner's live invite links for one space.
    LoadInviteLinks {
        space: String,
        request: u64,
    },
    EnsureDefaultInviteLink {
        space: String,
        channel: Option<String>,
        request: u64,
    },
    RegenerateInviteLink {
        space: String,
        code: String,
        request: u64,
    },
    CreateInviteLink {
        space: String,
        request: u64,
        channel: Option<String>,
        max_age: Option<u32>,
        max_uses: Option<u32>,
    },
    RevokeInviteLink {
        space: String,
        request: u64,
        code: String,
    },
    /// The "Join with invite link" preview and join; `request` drops answers
    /// for an input the person has since changed or a dialog they closed.
    PreviewInvite {
        request: u64,
        code: String,
    },
    JoinInvite {
        request: u64,
        code: String,
    },
}

#[derive(Debug)]
pub enum AccountResult {
    Blocks(Result<Vec<BlockedAccount>, String>),
    Block {
        account: BlockedAccount,
        blocked: bool,
        request: Option<String>,
        result: Result<(), String>,
    },
    Invitation {
        space: Space,
        user: String,
        request: u64,
        result: Result<Member, String>,
    },
    Accepted {
        id: String,
        result: Result<DirectConversation, String>,
    },
    Declined {
        id: String,
        result: Result<(), String>,
    },
    Privacy(Result<String, String>),
    PrivacySaved {
        previous: Option<String>,
        result: Result<String, String>,
    },
    Notifications {
        revision: u64,
        result: Result<NotificationSettings, String>,
    },
    NotificationsSaved {
        scope: Scope,
        result: Result<Saved, String>,
    },
    InviteLinks {
        space: String,
        request: u64,
        result: Result<Vec<InviteLink>, String>,
    },
    InviteLinkCreated {
        space: String,
        request: u64,
        result: Result<InviteLink, String>,
    },
    DefaultInviteLink {
        space: String,
        request: u64,
        result: Result<InviteLink, String>,
    },
    InviteLinkRevoked {
        space: String,
        request: u64,
        code: String,
        result: Result<(), String>,
    },
    InvitePreview {
        request: u64,
        result: Result<InvitePreview, String>,
    },
    InviteJoined {
        request: u64,
        result: Result<JoinedInvite, String>,
    },
}

#[derive(Clone)]
pub enum AdminOperation {
    CreateSpace {
        name: String,
    },
    UpdateSpace {
        space: String,
        name: String,
    },
    DeleteSpace {
        space: String,
    },
    LeaveSpace {
        space: String,
        member: String,
    },
    CreateChannel {
        space: String,
        name: String,
        private: bool,
    },
    UpdateChannel {
        space: String,
        channel: String,
        name: String,
        private: bool,
    },
    DeleteChannel {
        space: String,
        channel: String,
    },
    LoadMembers {
        space: String,
        channel: Option<String>,
    },
    LoadInvitations {
        space: String,
    },
    CancelInvitation {
        space: String,
        user: String,
    },
    AcceptInvitation {
        space: String,
    },
    DeclineInvitation {
        space: String,
    },
    AddMember {
        space: String,
        channel: Option<String>,
        username: String,
    },
    RemoveMember {
        space: String,
        channel: Option<String>,
        member: String,
    },
    JoinChannel {
        space: String,
        channel: String,
    },
    LeaveChannel {
        space: String,
        channel: String,
    },
    AcceptChannelInvitation {
        space: String,
        channel: String,
    },
    DeclineChannelInvitation {
        space: String,
        channel: String,
    },
}

pub enum AdminResult {
    SpaceCreated(Space),
    SpaceUpdated(Space),
    SpaceDeleted(String),
    SpaceLeft(String),
    ChannelCreated(Channel),
    ChannelUpdated(Channel),
    ChannelDeleted(String),
    Members {
        channel: Option<String>,
        members: Vec<Member>,
        invitations: Vec<Member>,
    },
    MemberAdded {
        channel: Option<String>,
        member: Member,
    },
    InvitationCreated(Member),
    MemberRemoved {
        channel: Option<String>,
        member: String,
    },
    Invitations(Vec<Member>),
    InvitationCancelled(String),
    InvitationAccepted(Space),
    InvitationDeclined(String),
    ChannelMembership {
        detail: SpaceDetail,
        channel: String,
        joined: bool,
    },
    ChannelLeft(String),
    ChannelInvitationDeclined(String),
}

pub enum Event {
    AvatarHistory {
        epoch: u64,
        request: Instant,
        result: Result<crate::pictures::History, String>,
    },
    AvatarSelected {
        epoch: u64,
        result: Result<crate::pictures::Changed, crate::pictures::Failure>,
        refreshed: Option<Account>,
    },
    /// A live `profiles` change from the account feed.
    Profiles {
        epoch: u64,
        event: gateway::ProfileEvent,
    },
    Picture {
        epoch: u64,
        target: crate::pictures::Target,
        result: Result<crate::pictures::Changed, crate::pictures::Failure>,
    },
    ProfileStatus {
        epoch: u64,
        person: String,
        status: String,
    },
    ProfileSent {
        epoch: u64,
        person: String,
        send: Box<crate::profile_dm::Send>,
        result: Result<DirectConversation, crate::profile_dm::Failure>,
    },
    WhatsNew(Result<crate::whats_new::Snapshot, String>),
    OpenNotification {
        epoch: u64,
        alert: crate::os_notifications::Alert,
    },
    /// A live notification arrived, shown or not: unread state changed.
    NotificationActivity {
        epoch: u64,
    },
    NotificationError {
        epoch: u64,
        error: String,
    },
    InvitationLoaded {
        epoch: u64,
        navigation: u64,
        space: String,
        channel: Option<String>,
        result: Result<(Spaces, Option<SpaceDetail>), String>,
    },
    Forward {
        generation: u64,
        request: u64,
        result: Result<crate::forwarding::Response, SendFailure>,
    },
    DirectsLoaded {
        generation: u64,
        result: Result<Vec<DirectConversation>, String>,
    },
    UnreadLoaded {
        epoch: u64,
        revision: u64,
        visit: Option<u64>,
        result: Result<crate::model::UnreadSpaces, String>,
    },
    VoiceActivityLoaded {
        epoch: u64,
        result: Result<Vec<String>, String>,
    },
    ThreadUnread {
        scope: (u64, u64, String),
        revision: u64,
        read: Option<(String, String)>,
        result: Result<std::collections::BTreeMap<String, String>, String>,
    },
    ChannelRead {
        epoch: u64,
        space: String,
        channel: String,
        seq: String,
        result: Result<(), String>,
    },
    MembershipRefreshed {
        epoch: u64,
        request: u64,
        revision: u64,
        navigation: u64,
        /// The open space's detail; `None` when not requested, no longer
        /// listed, or unreadable (the list still applies).
        result: Result<(Spaces, Option<SpaceDetail>), String>,
    },
    FeedbackLoaded {
        epoch: u64,
        request: u64,
        result: Result<crate::feedback::Page, String>,
    },
    FeedbackArchived {
        epoch: u64,
        result: Result<(), String>,
    },
    PeopleLoaded {
        generation: u64,
        result: Result<Vec<Person>, String>,
    },
    DirectLookedUp {
        generation: u64,
        navigation: u64,
        request: u64,
        username: String,
        result: Result<Person, String>,
    },
    Account {
        epoch: u64,
        result: AccountResult,
    },
    DirectCreated {
        generation: u64,
        navigation: u64,
        result: Result<DirectConversation, String>,
    },
    Restored {
        generation: u64,
        result: Result<Option<(String, Account, Spaces)>, String>,
    },
    CodeRequested {
        generation: u64,
        result: Result<String, String>,
    },
    Verified {
        generation: u64,
        result: Result<(String, Account, Spaces), String>,
        attempts_remaining: Option<u64>,
    },
    Profiled {
        generation: u64,
        result: Result<(Account, Spaces), String>,
    },
    NavigationPrepared {
        generation: u64,
        navigation: u64,
        result: Result<PreparedNavigation, LoadError>,
    },
    NavigationPrefetched {
        generation: u64,
        request: u64,
        space: Option<String>,
        channel: Option<String>,
        result: Result<crate::navigation::Read, LoadError>,
    },
    ChannelLoaded {
        generation: u64,
        channel: String,
        general: bool,
        result: Result<(History, SessionResult), LoadError>,
    },
    /// Web's Retry session.
    SessionCreated {
        generation: u64,
        result: SessionResult,
    },
    OlderLoaded {
        generation: u64,
        request: u64,
        channel: String,
        result: Result<History, LoadError>,
    },
    ContextLoaded {
        generation: u64,
        request: u64,
        channel: String,
        root: Option<String>,
        anchor: String,
        newer: bool,
        result: Result<crate::model::MessageContext, LoadError>,
    },
    ThreadLoaded {
        generation: u64,
        request: u64,
        channel: String,
        root: String,
        result: Result<crate::model::ThreadHistory, LoadError>,
    },
    MediaStatus {
        generation: u64,
        root: String,
        enabled: bool,
    },
    VoiceChecked {
        request: u64,
        voice_generation: u64,
        space: String,
        channel: String,
        result: Result<(), LoadError>,
    },
    Sent {
        generation: u64,
        channel: String,
        client_id: String,
        result: Result<Message, SendFailure>,
    },
    Reacted {
        generation: u64,
        channel: String,
        message: String,
        emoji: String,
        active: bool,
        result: Result<crate::model::ReactionUpdate, SendFailure>,
    },
    /// Fenced by account token: channel loads advance the generation.
    AssetUsage {
        token: String,
        settings: Option<crate::compress::Compression>,
    },
    AttachmentUrls {
        token: String,
        ids: Vec<String>,
        result: Result<BTreeMap<String, crate::attachments::FreshUrl>, String>,
    },
    /// Keep the upload's account for cleanup even after sign-out/navigation.
    Upload {
        key: u64,
        token: String,
        update: UploadUpdate,
    },
    Reactors {
        generation: u64,
        channel: String,
        message: String,
        revision: String,
        result: Result<crate::model::Reactors, String>,
    },
    Pinned {
        generation: u64,
        channel: String,
        message: String,
        active: bool,
        result: Result<crate::model::PinUpdate, SendFailure>,
    },
    EditSnapshot {
        generation: u64,
        request: u64,
        channel: String,
        message: String,
        reloaded: bool,
        result: Result<Box<Message>, String>,
    },
    MessageVersions {
        generation: u64,
        request: u64,
        channel: String,
        message: String,
        result: Result<crate::model::MessageVersions, String>,
    },
    /// The tombstone from a successful delete, or why it failed.
    Deleted {
        generation: u64,
        request: u64,
        channel: String,
        message: String,
        result: Result<Box<Message>, String>,
    },
    Credential {
        generation: u64,
        result: Result<(), String>,
    },
    LoggedOut {
        generation: u64,
        result: Result<(), String>,
    },
    Admin {
        generation: u64,
        result: Result<AdminResult, String>,
    },
    Gateway(GatewayEvent),
}

pub enum UploadUpdate {
    Retained(Arc<crate::uploads::Prepared>),
    Prepared {
        name: String,
        kind: crate::model::AttachmentKind,
        source_size: u64,
        stored_size: u64,
    },
    /// The sender's own decoded copy, when the format decodes here.
    Thumbnail(egui::ColorImage),
    /// Video compression before upload, 0–1.
    Compressing(f32),
    Progress(f32),
    Finished(Result<crate::model::Attachment, crate::uploads::UploadError>),
}

pub struct SendFailure {
    pub status: Option<u16>,
    pub message: String,
    /// The server's `code`, such as `dm_blocked`.
    pub code: Option<String>,
}

#[derive(Debug)]
pub struct LoadError {
    pub message: String,
    pub access_denied: bool,
    pub space_access_denied: bool,
}

impl From<crate::api::ApiError> for LoadError {
    fn from(error: crate::api::ApiError) -> Self {
        // A jump to a message or thread that is not in this conversation
        // (`around`/thread anchors) is a 404 too, but the conversation itself
        // is still readable: show the error rather than closing it.
        let missing_message = error.status.is_some_and(|status| status.as_u16() == 404)
            && matches!(
                error.message.as_str(),
                "Message not found." | "Thread not found."
            );
        Self {
            access_denied: !missing_message
                && error
                    .status
                    .is_some_and(|status| matches!(status.as_u16(), 401 | 403 | 404)),
            space_access_denied: false,
            message: error.to_string(),
        }
    }
}

pub struct PreparedNavigation {
    pub detail: Option<SpaceDetail>,
    pub conversation: Option<(History, SessionResult)>,
}

/// Web keeps a loaded conversation when only its chat session fails, with Retry session.
pub type SessionResult = Result<ChatSession, String>;

fn session_result(
    result: Result<ChatSession, crate::api::ApiError>,
) -> Result<SessionResult, crate::api::ApiError> {
    match result {
        Ok(session) => Ok(Ok(session)),
        Err(error)
            if error
                .status
                .is_some_and(|status| matches!(status.as_u16(), 401 | 403 | 404)) =>
        {
            Err(error)
        }
        Err(error) => Ok(Err(error.to_string())),
    }
}

fn prepare_navigation(
    api: &Api,
    token: Option<&str>,
    space: Option<&str>,
    channel: Option<&str>,
    name: &str,
    cached: Option<History>,
    visit: bool,
) -> Result<PreparedNavigation, LoadError> {
    let detail = space
        .map(|space| api.space(token.unwrap_or_default(), space))
        .transpose()
        .map_err(|error| {
            let missing = error.status == Some(reqwest::StatusCode::NOT_FOUND);
            let mut error = LoadError::from(error);
            error.space_access_denied = error.access_denied;
            if missing {
                error.message = "This space is no longer available.".into();
            }
            error
        })?;
    if let Some(detail) = &detail
        && Some(detail.space.id.as_str()) != space
    {
        return Err(crate::api::ApiError {
            status: None,
            message: "Caper returned another space.".into(),
            attempts_remaining: None,
            code: None,
        }
        .into());
    }
    let selected = if let Some(detail) = &detail {
        if let Some(id) = channel {
            Some(
                detail
                    .channels
                    .iter()
                    .find(|entry| entry.id == id)
                    .ok_or_else(|| crate::api::ApiError {
                        status: Some(reqwest::StatusCode::NOT_FOUND),
                        message: "This channel is no longer accessible.".into(),
                        attempts_remaining: None,
                        code: None,
                    })?,
            )
        } else {
            detail.channels.iter().find(|entry| entry.joined)
        }
    } else {
        None
    };
    let history = if let Some(selected) = selected {
        match cached.filter(|history| {
            history.space.id == detail.as_ref().unwrap().space.id
                && history.channel.id == selected.id
        }) {
            Some(history) => Some(history),
            None => Some(api.history(token, &selected.id, None)?),
        }
    } else if let Some(id) = channel.filter(|_| space.is_none()) {
        let history = match cached
            .filter(|history| !visit && history.space.id.is_empty() && history.channel.id == id)
        {
            Some(history) => history,
            None => api.history(token, id, None)?,
        };
        // A visit may land in a space channel; the app decides how to show it.
        if (!visit && !history.space.id.is_empty()) || history.channel.id != id {
            return Err(LoadError {
                message: "Caper returned another conversation.".into(),
                access_denied: false,
                space_access_denied: false,
            });
        }
        Some(history)
    } else {
        None
    };
    if let (Some(detail), Some(history)) = (&detail, &history)
        && (detail.space.id != history.space.id
            || selected.is_none_or(|entry| entry.id != history.channel.id))
    {
        return Err(crate::api::ApiError {
            status: None,
            message: "Caper returned another conversation.".into(),
            attempts_remaining: None,
            code: None,
        }
        .into());
    }
    if let Some(history) = &history {
        if history
            .messages
            .iter()
            .any(|message| message.channel_id != history.channel.id)
        {
            return Err(crate::api::ApiError {
                status: None,
                message: "Caper returned messages from another channel.".into(),
                attempts_remaining: None,
                code: None,
            }
            .into());
        }
        crate::model::Timeline::default()
            .reset(history.messages.clone(), &history.cursor)
            .map_err(|message| crate::api::ApiError {
                status: None,
                message,
                attempts_remaining: None,
                code: None,
            })?;
    }
    let joined =
        selected.is_some_and(|channel| channel.joined) || space.is_none() && channel.is_some();
    let conversation = history
        .map(|history| {
            if joined {
                session_result(api.chat_session(token, name)).map(|session| (history, session))
            } else {
                Ok((history, Err("Join this channel to chat.".into())))
            }
        })
        .transpose()?;
    Ok(PreparedNavigation {
        detail,
        conversation,
    })
}

fn prefetch_navigation(
    api: &Api,
    token: Option<&str>,
    space: Option<&str>,
    channel: Option<&str>,
) -> Result<crate::navigation::Read, LoadError> {
    let prepared = prepare_navigation_read(api, token, space, channel)?;
    Ok(crate::navigation::Read {
        detail: prepared.0,
        history: prepared.1,
    })
}

fn prepare_navigation_read(
    api: &Api,
    token: Option<&str>,
    space: Option<&str>,
    channel: Option<&str>,
) -> Result<(Option<SpaceDetail>, Option<History>), LoadError> {
    let detail = space
        .map(|space| api.space(token.unwrap_or_default(), space))
        .transpose()
        .map_err(|error| {
            let missing = error.status == Some(reqwest::StatusCode::NOT_FOUND);
            let mut error = LoadError::from(error);
            error.space_access_denied = error.access_denied;
            if missing {
                error.message = "This space is no longer available.".into();
            }
            error
        })?;
    let selected = detail.as_ref().and_then(|detail| match channel {
        Some(id) => detail.channels.iter().find(|entry| entry.id == id),
        None => detail.channels.iter().find(|entry| entry.joined),
    });
    if channel.is_some() && detail.is_some() && selected.is_none() {
        return Err(LoadError {
            message: "This channel is no longer accessible.".into(),
            access_denied: true,
            space_access_denied: false,
        });
    }
    let history = if let Some(selected) = selected {
        Some(api.history(token, &selected.id, None)?)
    } else if let Some(id) = channel.filter(|_| space.is_none()) {
        let history = api.history(token, id, None)?;
        if !history.space.id.is_empty() || history.channel.id != id {
            return Err(LoadError {
                message: "Caper returned another conversation.".into(),
                access_denied: false,
                space_access_denied: false,
            });
        }
        Some(history)
    } else {
        None
    };
    Ok((detail, history))
}

pub struct Worker {
    commands: Sender<Command>,
    pub events: Receiver<Event>,
    context: egui::Context,
    visible_conversation: Arc<Mutex<Option<(String, Instant)>>>,
}

impl Worker {
    pub fn new(api: Api, context: egui::Context) -> Self {
        let (commands, incoming) = mpsc::channel();
        let (events, outgoing) = mpsc::channel();
        let repaint = context.clone();
        let visible_conversation = Arc::new(Mutex::new(None));
        let visible = visible_conversation.clone();
        thread::spawn(move || manage(api, repaint, incoming, events, visible));
        Self {
            commands,
            events: outgoing,
            context,
            visible_conversation,
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn focus(&self) {
        self.context
            .send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        self.context.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    pub fn showing(&self, conversation: Option<&str>) {
        *self.visible_conversation.lock().unwrap() =
            conversation.map(|id| (id.to_owned(), Instant::now()));
        if conversation.is_some() {
            self.context.request_repaint_after(Duration::from_secs(1));
        }
    }
}

fn manage(
    api: Api,
    context: egui::Context,
    incoming: Receiver<Command>,
    events: Sender<Event>,
    visible: Arc<Mutex<Option<(String, Instant)>>>,
) {
    let mut gateway: Option<gateway::GatewayControl> = None;
    let mut notifications: Option<(u64, crate::os_notifications::Control)> = None;
    let badge = Arc::new(crate::app_badge::Badge::new(api.clone(), context.clone()));
    let mut credential_generation = 0;
    while let Ok(command) = incoming.recv() {
        match command {
            Command::StartNotifications { epoch, token } => {
                badge.account(Some(token.clone()));
                notifications.take();
                let events = events.clone();
                let context = context.clone();
                let visible = visible.clone();
                notifications = Some((
                    epoch,
                    crate::os_notifications::watch(
                        api.base(),
                        token,
                        {
                            let events = events.clone();
                            let context = context.clone();
                            move |alert, lifetime| {
                                send(&events, &context, Event::NotificationActivity { epoch });
                                // A focused UI publishes a short lease. If minimized
                                // windows stop repainting, stale focus cannot suppress alerts.
                                if !alert.fresh()
                                    || visible.lock().unwrap().as_ref().is_some_and(|(id, at)| {
                                        Some(id.as_str()) == alert.conversation()
                                            && at.elapsed() < Duration::from_secs(2)
                                    })
                                {
                                    return;
                                }
                                let events = events.clone();
                                let context = context.clone();
                                let visible = visible.clone();
                                thread::spawn(move || {
                                    let target = alert.clone();
                                    let conversation = alert.conversation().map(str::to_owned);
                                    let clicks = events.clone();
                                    let repaint = context.clone();
                                    if let Err(error) = crate::os_notifications::show(
                                        alert,
                                        lifetime,
                                        || {
                                            visible.lock().unwrap().as_ref().is_some_and(
                                                |(id, at)| {
                                                    Some(id.as_str()) == conversation.as_deref()
                                                        && at.elapsed() < Duration::from_secs(2)
                                                },
                                            )
                                        },
                                        move || {
                                            repaint.send_viewport_cmd(
                                                egui::ViewportCommand::Minimized(false),
                                            );
                                            repaint.send_viewport_cmd(egui::ViewportCommand::Focus);
                                            send(
                                                &clicks,
                                                &repaint,
                                                Event::OpenNotification {
                                                    epoch,
                                                    alert: target.clone(),
                                                },
                                            );
                                        },
                                    ) {
                                        send(
                                            &events,
                                            &context,
                                            Event::NotificationError { epoch, error },
                                        );
                                    }
                                });
                            }
                        },
                        {
                            let events = events.clone();
                            let context = context.clone();
                            move |event| send(&events, &context, Event::Profiles { epoch, event })
                        },
                    ),
                ));
            }
            Command::StopNotifications => {
                badge.account(None);
                notifications.take();
            }
            Command::Connect {
                generation,
                token,
                channel,
                cursor,
                presence,
                media,
            } => {
                if let Some(control) = gateway.take() {
                    control.stop();
                }
                let (gateway_tx, gateway_rx) = mpsc::channel();
                gateway = Some(gateway::spawn(
                    api.base(),
                    token,
                    generation,
                    channel,
                    cursor,
                    presence,
                    media,
                    gateway_tx,
                ));
                let events = events.clone();
                let context = context.clone();
                thread::spawn(move || {
                    while let Ok(event) = gateway_rx.recv() {
                        send(&events, &context, Event::Gateway(event));
                    }
                });
            }
            Command::StopGateway => {
                if let Some(control) = gateway.take() {
                    control.stop();
                }
            }
            Command::Activity => {
                if let Some(control) = &gateway {
                    control.activity();
                }
                if let Some((_, control)) = &notifications {
                    control.activity();
                }
            }
            Command::PersistCredential { generation, token } => {
                if advance_generation(&mut credential_generation, generation) {
                    let result = crate::credentials::save(api.base(), &token);
                    send(&events, &context, Event::Credential { generation, result });
                }
            }
            Command::ClearCredential { generation } => {
                if advance_generation(&mut credential_generation, generation) {
                    let result = crate::credentials::delete(api.base());
                    send(&events, &context, Event::Credential { generation, result });
                }
            }
            command => {
                let api = api.clone();
                let events = events.clone();
                let context = context.clone();
                let badge = badge.clone();
                let refresh_badge = matches!(
                    command,
                    Command::ReadDirect { .. }
                        | Command::ReadChannel { .. }
                        | Command::ThreadUnread { read: Some(_), .. }
                        | Command::Account { .. }
                        | Command::Admin { .. }
                );
                thread::spawn(move || {
                    execute(&api, command, &events, &context);
                    if refresh_badge {
                        badge.refresh();
                    }
                });
            }
        }
    }
    badge.account(None);
    if let Some(control) = gateway {
        control.stop();
    }
}

fn advance_generation(current: &mut u64, candidate: u64) -> bool {
    if candidate < *current {
        return false;
    }
    *current = candidate;
    true
}

/// One upload's original: its path, name and any edit to apply first.
struct Source<'a> {
    path: &'a std::path::Path,
    name: &'a str,
    edit: Option<&'a crate::attachment_edit::Edit>,
}

#[allow(clippy::too_many_arguments)]
fn upload_file(
    api: &Api,
    key: u64,
    token: &str,
    channel: &str,
    source: Source,
    retained: Option<Arc<crate::uploads::Prepared>>,
    settings: &crate::compress::Compression,
    cancel: Arc<AtomicBool>,
    events: &Sender<Event>,
    context: &egui::Context,
) {
    let update = |update| {
        send(
            events,
            context,
            Event::Upload {
                key,
                token: token.to_owned(),
                update,
            },
        )
    };
    // Whole-percent steps, like upload progress below.
    let compressed = std::sync::Mutex::new(-1_i32);
    let compressing = |fraction: f32| {
        let percent = (fraction.clamp(0.0, 1.0) * 100.0) as i32;
        let mut last = compressed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if percent > *last {
            *last = percent;
            update(UploadUpdate::Compressing(fraction));
        }
    };
    let prepared = if let Some(prepared) = retained {
        prepared
    } else {
        let prepared = match source.edit {
            Some(edit) => crate::uploads::prepare_edited_path(
                source.path,
                source.name,
                edit,
                settings,
                &compressing,
                &cancel,
            ),
            None => crate::uploads::prepare_path_with(source.path, settings, &compressing, &cancel),
        };
        let mut prepared = match prepared.and_then(|mut prepared| {
            prepared.retain_file()?;
            Ok(prepared)
        }) {
            Ok(prepared) => prepared,
            Err(message) => {
                update(UploadUpdate::Finished(Err(
                    crate::uploads::UploadError::new(message),
                )));
                return;
            }
        };
        if let Some(image) = prepared.thumbnail.take() {
            update(UploadUpdate::Thumbnail(
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                ),
            ));
        }
        let prepared = Arc::new(prepared);
        update(UploadUpdate::Retained(prepared.clone()));
        prepared
    };
    update(UploadUpdate::Prepared {
        name: prepared.name.clone(),
        kind: prepared.kind,
        source_size: prepared.source_size,
        stored_size: prepared.size,
    });
    // Whole-percent steps keep the event queue small for large files.
    let progress: Arc<dyn Fn(f32) + Send + Sync> = {
        let events = events.clone();
        let context = context.clone();
        let token = token.to_owned();
        let last = std::sync::Mutex::new(-1_i32);
        Arc::new(move |fraction: f32| {
            let percent = (fraction.clamp(0.0, 1.0) * 100.0) as i32;
            let mut last = last
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if percent > *last {
                *last = percent;
                send(
                    &events,
                    &context,
                    Event::Upload {
                        key,
                        token: token.clone(),
                        update: UploadUpdate::Progress(fraction),
                    },
                );
            }
        })
    };
    let result = crate::uploads::upload(
        api,
        token,
        channel,
        prepared.as_ref().clone(),
        progress,
        cancel,
    );
    update(UploadUpdate::Finished(result));
}

fn execute(api: &Api, command: Command, events: &Sender<Event>, context: &egui::Context) {
    let event = match command {
        Command::CancelAssets { token, ids } => {
            for id in ids {
                api.cancel_asset(&token, &id);
            }
            return;
        }
        Command::SendProfile {
            epoch,
            person,
            token,
            send: mut operation,
        } => {
            let result = operation.run(api, &token, &|status| {
                send(
                    events,
                    context,
                    Event::ProfileStatus {
                        epoch,
                        person: person.clone(),
                        status,
                    },
                )
            });
            Event::ProfileSent {
                epoch,
                person,
                send: operation,
                result,
            }
        }
        Command::AvatarHistory {
            epoch,
            request,
            token,
            cursor,
        } => Event::AvatarHistory {
            epoch,
            request,
            result: api
                .avatar_history(&token, cursor.as_deref())
                .map_err(|error| error.message),
        },
        Command::SelectAvatar {
            epoch,
            token,
            asset,
            expected,
        } => {
            let result = crate::pictures::select(api, &token, &asset, expected.as_deref());
            // Refresh after a failed selection so reopening the chooser can recover
            // from another device's change even if its live event was missed.
            let refreshed = result.is_err().then(|| api.me(&token).ok()).flatten();
            Event::AvatarSelected {
                epoch,
                result,
                refreshed,
            }
        }
        Command::Picture {
            epoch,
            token,
            target,
            path,
            animate,
            edit,
            scratch,
        } => Event::Picture {
            result: match path {
                Some(path) => {
                    let _keep = scratch;
                    crate::pictures::upload(api, &token, &target, &path, animate, edit.as_ref())
                }
                None => crate::pictures::remove(api, &token, &target),
            },
            epoch,
            target,
        },
        Command::WhatsNew => Event::WhatsNew(api.whats_new()),
        Command::Upload {
            key,
            token,
            channel,
            path,
            name,
            edit,
            source_scratch,
            prepared,
            settings,
            cancel,
        } => {
            let source = Source {
                path: &path,
                name: &name,
                edit: edit.as_ref(),
            };
            upload_file(
                api, key, &token, &channel, source, prepared, &settings, cancel, events, context,
            );
            // Removal can drop the draft while this worker is still reading its source.
            drop(source_scratch);
            return;
        }
        Command::AssetUsage { token } => Event::AssetUsage {
            settings: api.asset_usage(&token).ok(),
            token,
        },
        Command::RefreshAttachmentUrls { token, ids } => Event::AttachmentUrls {
            result: api
                .attachment_urls(&token, &ids)
                .map_err(|error| error.to_string()),
            token,
            ids,
        },
        Command::OpenInvitation {
            epoch,
            navigation,
            token,
            space,
            channel,
        } => {
            let result = api
                .spaces(&token)
                .and_then(|spaces| {
                    let detail =
                        if channel.is_some() && spaces.spaces.iter().any(|item| item.id == space) {
                            Some(api.space(&token, &space)?)
                        } else {
                            None
                        };
                    Ok((spaces, detail))
                })
                .map_err(|error| error.to_string());
            Event::InvitationLoaded {
                epoch,
                navigation,
                space,
                channel,
                result,
            }
        }
        Command::RefreshMembership {
            epoch,
            request,
            revision,
            navigation,
            token,
            space,
        } => Event::MembershipRefreshed {
            epoch,
            request,
            revision,
            navigation,
            result: api
                .spaces(&token)
                .map(|spaces| {
                    let detail = space
                        .filter(|id| spaces.spaces.iter().any(|item| item.id == *id))
                        .and_then(|id| api.space(&token, &id).ok());
                    (spaces, detail)
                })
                .map_err(|error| error.to_string()),
        },
        Command::LoadDirects { generation, token } => Event::DirectsLoaded {
            generation,
            result: api
                .direct_conversations(&token)
                .map(|value| value.conversations)
                .map_err(|error| error.to_string()),
        },
        Command::LoadFeedback {
            epoch,
            request,
            token,
            before,
            archived,
        } => Event::FeedbackLoaded {
            epoch,
            request,
            result: api
                .feedback(&token, before.as_deref(), archived)
                .map_err(|error| error.to_string()),
        },
        Command::ArchiveFeedback {
            epoch,
            token,
            channel,
            thread_root,
            archived,
        } => Event::FeedbackArchived {
            epoch,
            result: api
                .archive_feedback(&token, &channel, &thread_root, archived)
                .map_err(|error| error.to_string()),
        },
        Command::LoadPeople { generation, token } => Event::PeopleLoaded {
            generation,
            result: api
                .people(&token)
                .map(|value| value.people)
                .map_err(|error| error.to_string()),
        },
        Command::LookupDirect {
            generation,
            navigation,
            request,
            token,
            username,
        } => Event::DirectLookedUp {
            generation,
            navigation,
            request,
            result: api
                .lookup_direct(&token, &username)
                .map_err(|error| error.to_string()),
            username,
        },
        Command::Account {
            epoch,
            token,
            operation,
        } => Event::Account {
            epoch,
            result: execute_account(api, &token, operation),
        },
        Command::CreateDirect {
            generation,
            navigation,
            token,
            username,
        } => Event::DirectCreated {
            generation,
            navigation,
            result: api
                .create_direct(&token, &username)
                .map_err(|error| error.to_string()),
        },
        Command::ReadDirect { token, id, seq } => {
            let _ = api.read_direct(&token, &id, &seq);
            return;
        }
        Command::LoadUnread {
            epoch,
            revision,
            visit,
            token,
        } => Event::UnreadLoaded {
            epoch,
            revision,
            visit,
            result: api.unread_spaces(&token).map_err(|error| error.to_string()),
        },
        Command::LoadVoiceActivity { epoch, token } => Event::VoiceActivityLoaded {
            epoch,
            result: api
                .voice_activity(&token)
                .map_err(|error| error.to_string()),
        },
        Command::ThreadUnread {
            scope,
            revision,
            token,
            read,
        } => {
            let result = if let Some((root, seq)) = &read {
                api.read_thread(&token, &scope.2, root, seq)
                    .map(|()| std::collections::BTreeMap::new())
            } else {
                api.unread_threads(&token, &scope.2)
            };
            Event::ThreadUnread {
                scope,
                revision,
                read,
                result: result.map_err(|error| error.to_string()),
            }
        }
        Command::ReadChannel {
            epoch,
            token,
            space,
            channel,
            seq,
        } => Event::ChannelRead {
            epoch,
            result: api
                .read_channel(&token, &space, &channel, &seq)
                .map_err(|error| error.to_string()),
            space,
            channel,
            seq,
        },
        Command::PrepareVoice { token, channel } => {
            api.prepare_voice(&token, &channel);
            return;
        }
        Command::MediaStatus {
            generation,
            root,
            token,
            channel,
        } => Event::MediaStatus {
            generation,
            root,
            enabled: api.media_status(token.as_deref(), channel.as_deref()),
        },
        Command::CheckVoice {
            request,
            voice_generation,
            token,
            space,
            channel,
        } => {
            let result = prepare_navigation_read(api, Some(&token), Some(&space), Some(&channel))
                .and_then(|(detail, history)| {
                    if detail.as_ref().is_some_and(|detail| {
                        detail.space.id == space
                            && detail
                                .channels
                                .iter()
                                .any(|item| item.id == channel && item.joined)
                    }) && history.as_ref().is_some_and(|history| {
                        history.space.id == space && history.channel.id == channel
                    }) {
                        Ok(())
                    } else {
                        Err(LoadError {
                            message: "This channel is no longer accessible.".into(),
                            access_denied: true,
                            space_access_denied: false,
                        })
                    }
                });
            Event::VoiceChecked {
                request,
                voice_generation,
                space,
                channel,
                result,
            }
        }
        Command::Restore { generation } => {
            let result = crate::credentials::load(api.base()).and_then(|token| {
                token.map_or(Ok(None), |token| {
                    let account = api.me(&token).map_err(|error| error.to_string())?;
                    let spaces = api.spaces(&token).map_err(|error| error.to_string())?;
                    Ok(Some((token, account, spaces)))
                })
            });
            Event::Restored { generation, result }
        }
        Command::RequestCode { generation, email } => Event::CodeRequested {
            generation,
            result: api
                .request_code(&email)
                .map_err(|error| login_error(&error)),
        },
        Command::VerifyCode {
            generation,
            challenge,
            code,
        } => {
            let mut attempts_remaining = None;
            let result = match api.verify_code(&challenge, &code) {
                Ok((account, token)) => api
                    .spaces(&token)
                    .map(|spaces| (token, account, spaces))
                    .map_err(|error| error.to_string()),
                Err(error) => {
                    if error.status == Some(reqwest::StatusCode::UNAUTHORIZED) {
                        attempts_remaining = error.attempts_remaining;
                    }
                    Err(login_error(&error))
                }
            };
            Event::Verified {
                generation,
                result,
                attempts_remaining,
            }
        }
        Command::Profile {
            generation,
            token,
            username,
            display_name,
        } => Event::Profiled {
            generation,
            result: api
                .update_profile(&token, &username, &display_name)
                .map_err(|error| profile_error(&error))
                .and_then(|account| {
                    api.spaces(&token)
                        .map(|spaces| (account, spaces))
                        .map_err(|error| error.to_string())
                }),
        },
        Command::PrepareNavigation {
            generation,
            navigation,
            token,
            space,
            channel,
            name,
            cached,
            visit,
        } => Event::NavigationPrepared {
            generation,
            navigation,
            result: prepare_navigation(
                api,
                token.as_deref(),
                space.as_deref(),
                channel.as_deref(),
                &name,
                cached,
                visit,
            ),
        },
        Command::PrefetchNavigation {
            generation,
            request,
            token,
            space,
            channel,
        } => Event::NavigationPrefetched {
            generation,
            request,
            result: prefetch_navigation(
                api,
                token.as_deref(),
                space.as_deref(),
                channel.as_deref(),
            ),
            space,
            channel,
        },
        Command::CreateSession {
            generation,
            token,
            name,
        } => Event::SessionCreated {
            generation,
            result: api
                .chat_session(token.as_deref(), &name)
                .map_err(|error| error.to_string()),
        },
        Command::LoadChannel {
            generation,
            token,
            channel,
            name,
            general,
        } => {
            let result = (if general {
                api.general_history(token.as_deref())
            } else {
                api.history(token.as_deref(), &channel, None)
            })
            .and_then(|history| {
                session_result(api.chat_session(token.as_deref(), &name))
                    .map(|session| (history, session))
            })
            .map_err(LoadError::from);
            Event::ChannelLoaded {
                generation,
                channel,
                general,
                result,
            }
        }
        Command::LoadOlder {
            generation,
            request,
            token,
            channel,
            before,
        } => Event::OlderLoaded {
            generation,
            request,
            channel: channel.clone(),
            result: api
                .history(token.as_deref(), &channel, Some(&before))
                .map_err(LoadError::from),
        },
        Command::LoadContext {
            generation,
            request,
            token,
            channel,
            root,
            anchor,
            newer,
        } => Event::ContextLoaded {
            generation,
            request,
            channel: channel.clone(),
            root: root.clone(),
            anchor: anchor.clone(),
            newer,
            result: api
                .message_context(token.as_deref(), &channel, root.as_deref(), &anchor, newer)
                .map_err(LoadError::from),
        },
        Command::LoadThread {
            generation,
            request,
            token,
            channel,
            root,
            before,
        } => Event::ThreadLoaded {
            generation,
            request,
            channel: channel.clone(),
            root: root.clone(),
            result: api
                .thread(token.as_deref(), &channel, &root, before.as_deref())
                .map_err(LoadError::from),
        },
        Command::Forward {
            generation,
            request,
            operation,
        } => Event::Forward {
            generation,
            request,
            result: operation.run(api),
        },
        Command::Send {
            generation,
            token,
            chat_token,
            channel,
            client_id,
            text,
            attachment_ids,
            spoiler_attachment_ids,
            thread_root_id,
            broadcast,
            reply_to_id,
        } => Event::Sent {
            generation,
            channel: channel.clone(),
            client_id: client_id.clone(),
            result: api
                .send(
                    token.as_deref(),
                    &chat_token,
                    &channel,
                    &client_id,
                    &text,
                    &attachment_ids,
                    &spoiler_attachment_ids,
                    (thread_root_id.as_deref(), broadcast),
                    reply_to_id.as_deref(),
                )
                .map_err(|error| SendFailure {
                    status: error.status.map(|status| status.as_u16()),
                    message: error.to_string(),
                    code: error.code,
                }),
        },
        Command::Typing {
            token,
            chat_token,
            channel,
            typing,
        } => {
            let _ = api.typing(token.as_deref(), &chat_token, &channel, typing);
            return;
        }
        Command::React {
            generation,
            token,
            chat_token,
            channel,
            message,
            emoji,
            active,
        } => Event::Reacted {
            generation,
            channel: channel.clone(),
            message: message.clone(),
            emoji: emoji.clone(),
            active,
            result: api
                .react(
                    token.as_deref(),
                    &chat_token,
                    &channel,
                    &message,
                    &emoji,
                    active,
                )
                .map_err(|error| SendFailure {
                    status: error.status.map(|status| status.as_u16()),
                    message: error.to_string(),
                    code: error.code,
                }),
        },
        Command::LoadReactors {
            generation,
            token,
            channel,
            message,
            revision,
        } => Event::Reactors {
            generation,
            result: api
                .reactors(token.as_deref(), &channel, &message)
                .map_err(|error| error.to_string()),
            channel,
            message,
            revision,
        },
        Command::Pin {
            generation,
            token,
            chat_token,
            channel,
            message,
            active,
        } => Event::Pinned {
            generation,
            channel: channel.clone(),
            message: message.clone(),
            active,
            result: api
                .pin(token.as_deref(), &chat_token, &channel, &message, active)
                .map_err(|error| SendFailure {
                    status: error.status.map(|status| status.as_u16()),
                    message: error.to_string(),
                    code: error.code,
                }),
        },
        Command::EditMessage {
            generation,
            request,
            token,
            chat_token,
            original,
            text,
        } => Event::EditSnapshot {
            generation,
            request,
            channel: original.channel_id.clone(),
            message: original.id.clone(),
            reloaded: false,
            result: api
                .edit_message(token.as_deref(), &chat_token, &original, &text)
                .map(Box::new)
                .map_err(|error| error.to_string()),
        },
        Command::DeleteMessage {
            generation,
            request,
            token,
            chat_token,
            channel,
            message,
        } => Event::Deleted {
            generation,
            request,
            channel: channel.clone(),
            message: message.clone(),
            result: api
                .delete_message(token.as_deref(), &chat_token, &channel, &message)
                .map(Box::new)
                .map_err(|error| error.to_string()),
        },
        Command::ReloadMessage {
            generation,
            request,
            token,
            channel,
            message,
        } => Event::EditSnapshot {
            generation,
            request,
            channel: channel.clone(),
            message: message.clone(),
            reloaded: true,
            result: api
                .load_message(token.as_deref(), &channel, &message)
                .map(Box::new)
                .map_err(|error| error.to_string()),
        },
        Command::MessageVersions {
            generation,
            request,
            token,
            channel,
            message,
            before,
        } => Event::MessageVersions {
            generation,
            request,
            channel: channel.clone(),
            message: message.clone(),
            result: api
                .message_versions(token.as_deref(), &channel, &message, before)
                .map_err(|error| error.to_string()),
        },
        Command::Admin {
            generation,
            token,
            operation,
        } => Event::Admin {
            generation,
            result: execute_admin(api, &token, operation).map_err(|error| error.to_string()),
        },
        Command::Logout { generation, token } => Event::LoggedOut {
            generation,
            result: api.logout(&token).map_err(|error| error.to_string()),
        },
        Command::Connect { .. }
        | Command::StopGateway
        | Command::StartNotifications { .. }
        | Command::StopNotifications
        | Command::Activity
        | Command::PersistCredential { .. }
        | Command::ClearCredential { .. } => return,
    };
    send(events, context, event);
}

fn execute_account(api: &Api, token: &str, operation: AccountOperation) -> AccountResult {
    let text = |error: crate::api::ApiError| error.to_string();
    match operation {
        AccountOperation::LoadBlocks => {
            AccountResult::Blocks(api.blocks(token).map(|value| value.blocks).map_err(text))
        }
        AccountOperation::SetBlock {
            account,
            blocked,
            request,
        } => AccountResult::Block {
            result: api.set_block(token, &account.id, blocked).map_err(text),
            account,
            blocked,
            request,
        },
        AccountOperation::InviteToSpace {
            space,
            username,
            user,
            request,
        } => AccountResult::Invitation {
            result: api
                .add_member(token, &space.id, None, &username)
                .map_err(text),
            space,
            user,
            request,
        },
        AccountOperation::Accept(id) => AccountResult::Accepted {
            result: api.accept_direct(token, &id).map_err(text),
            id,
        },
        AccountOperation::Decline(id) => AccountResult::Declined {
            result: api.decline_direct(token, &id).map_err(text),
            id,
        },
        AccountOperation::LoadPrivacy => AccountResult::Privacy(
            api.privacy(token)
                .map(|value| value.direct_messages)
                .map_err(text),
        ),
        AccountOperation::SavePrivacy { value, previous } => AccountResult::PrivacySaved {
            previous,
            result: api
                .save_privacy(token, &value)
                .map(|value| value.direct_messages)
                .map_err(text),
        },
        AccountOperation::LoadNotifications { revision } => AccountResult::Notifications {
            revision,
            result: api.notification_settings(token).map_err(text),
        },
        AccountOperation::SaveNotifications { scope, change } => {
            let result = match (&scope, &change) {
                (Scope::Account, Change::VoiceStarted(Some(enabled))) => {
                    api.save_voice_started(token, *enabled).map(Saved::Settings)
                }
                (Scope::Account, Change::Level(Some(level))) => api
                    .save_notification_level(token, *level)
                    .map(Saved::Settings),
                _ => api
                    .save_notification_override(token, &scope, &change)
                    .map(Saved::Override),
            };
            AccountResult::NotificationsSaved {
                scope,
                result: result.map_err(text),
            }
        }
        AccountOperation::LoadInviteLinks { space, request } => AccountResult::InviteLinks {
            result: api.invite_links(token, &space).map_err(text),
            space,
            request,
        },
        AccountOperation::EnsureDefaultInviteLink {
            space,
            channel,
            request,
        } => AccountResult::DefaultInviteLink {
            result: api
                .default_invite_link(token, &space, channel.as_deref())
                .map_err(text),
            space,
            request,
        },
        AccountOperation::RegenerateInviteLink {
            space,
            code,
            request,
        } => AccountResult::DefaultInviteLink {
            result: api
                .regenerate_invite_link(token, &space, &code)
                .map_err(text),
            space,
            request,
        },
        AccountOperation::CreateInviteLink {
            space,
            request,
            channel,
            max_age,
            max_uses,
        } => AccountResult::InviteLinkCreated {
            result: api
                .create_invite_link(token, &space, channel.as_deref(), max_age, max_uses)
                .map_err(text),
            space,
            request,
        },
        AccountOperation::RevokeInviteLink {
            space,
            code,
            request,
        } => AccountResult::InviteLinkRevoked {
            result: api.revoke_invite_link(token, &space, &code).map_err(text),
            space,
            code,
            request,
        },
        AccountOperation::PreviewInvite { request, code } => AccountResult::InvitePreview {
            request,
            result: api.invite_preview(Some(token), &code).map_err(text),
        },
        AccountOperation::JoinInvite { request, code } => AccountResult::InviteJoined {
            request,
            result: api.join_invite(token, &code).map_err(text),
        },
    }
}

fn execute_admin(
    api: &Api,
    token: &str,
    operation: AdminOperation,
) -> Result<AdminResult, crate::api::ApiError> {
    Ok(match operation {
        AdminOperation::CreateSpace { name } => {
            AdminResult::SpaceCreated(api.create_space(token, &name)?)
        }
        AdminOperation::UpdateSpace { space, name } => {
            AdminResult::SpaceUpdated(api.update_space(token, &space, &name)?)
        }
        AdminOperation::DeleteSpace { space } => {
            api.delete_space(token, &space)?;
            AdminResult::SpaceDeleted(space)
        }
        AdminOperation::LeaveSpace { space, member } => {
            api.remove_member(token, &space, None, &member)?;
            AdminResult::SpaceLeft(space)
        }
        AdminOperation::CreateChannel {
            space,
            name,
            private,
        } => AdminResult::ChannelCreated(api.create_channel(token, &space, &name, private)?),
        AdminOperation::UpdateChannel {
            space,
            channel,
            name,
            private,
        } => AdminResult::ChannelUpdated(
            api.update_channel(token, &space, &channel, &name, private)?,
        ),
        AdminOperation::DeleteChannel { space, channel } => {
            api.delete_channel(token, &space, &channel)?;
            AdminResult::ChannelDeleted(channel)
        }
        AdminOperation::LoadMembers { space, channel } => {
            let response = api.members(token, &space, channel.as_deref())?;
            AdminResult::Members {
                channel,
                members: response.members,
                invitations: response.invitations,
            }
        }
        AdminOperation::LoadInvitations { space } => {
            AdminResult::Invitations(api.invitations(token, &space)?.members)
        }
        AdminOperation::CancelInvitation { space, user } => {
            api.cancel_invitation(token, &space, &user)?;
            AdminResult::InvitationCancelled(user)
        }
        AdminOperation::AcceptInvitation { space } => {
            AdminResult::InvitationAccepted(api.accept_invitation(token, &space)?)
        }
        AdminOperation::DeclineInvitation { space } => {
            api.decline_invitation(token, &space)?;
            AdminResult::InvitationDeclined(space)
        }
        AdminOperation::AddMember {
            space,
            channel,
            username,
        } => {
            let member = api.add_member(token, &space, channel.as_deref(), &username)?;
            match channel {
                Some(channel) => AdminResult::MemberAdded {
                    channel: Some(channel),
                    member,
                },
                None => AdminResult::InvitationCreated(member),
            }
        }
        AdminOperation::RemoveMember {
            space,
            channel,
            member,
        } => {
            api.remove_member(token, &space, channel.as_deref(), &member)?;
            AdminResult::MemberRemoved { channel, member }
        }
        AdminOperation::JoinChannel { space, channel } => AdminResult::ChannelMembership {
            detail: {
                api.join_channel(token, &space, &channel)?;
                api.space(token, &space)?
            },
            channel,
            joined: true,
        },
        AdminOperation::LeaveChannel { space, channel } => {
            api.leave_channel(token, &space, &channel)?;
            AdminResult::ChannelLeft(channel)
        }
        AdminOperation::AcceptChannelInvitation { space, channel } => {
            AdminResult::ChannelMembership {
                detail: {
                    api.accept_channel_invitation(token, &space, &channel)?;
                    api.space(token, &space)?
                },
                channel,
                joined: true,
            }
        }
        AdminOperation::DeclineChannelInvitation { space, channel } => {
            api.decline_channel_invitation(token, &space, &channel)?;
            AdminResult::ChannelInvitationDeclined(channel)
        }
    })
}

fn send(events: &Sender<Event>, context: &egui::Context, event: Event) {
    let _ = events.send(event);
    context.request_repaint();
}

pub fn current(
    generation: u64,
    expected: u64,
    event_channel: Option<&str>,
    selected: Option<&str>,
) -> bool {
    generation == expected && event_channel.is_none_or(|channel| Some(channel) == selected)
}

#[cfg(test)]
mod tests {
    use super::{advance_generation, current};

    #[test]
    fn people_load_with_the_account_bearer() {
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api =
            crate::api::Api::new(&format!("http://{}", server.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            let (stream, _) = server.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            assert_eq!(request, "GET /api/people HTTP/1.1\r\n");
            let mut authorized = false;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                authorized |= line.eq_ignore_ascii_case("authorization: Bearer account\r\n");
                if line == "\r\n" {
                    break;
                }
            }
            assert!(authorized);
            let body = r#"{"people":[{"id":"user00000001","username":"alex","displayName":"Alex","avatarId":null}]}"#;
            write!(reader.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let (events, receiver) = std::sync::mpsc::channel();
        super::execute(
            &api,
            super::Command::LoadPeople {
                generation: 7,
                token: "account".into(),
            },
            &events,
            &eframe::egui::Context::default(),
        );
        let super::Event::PeopleLoaded { generation, result } = receiver.recv().unwrap() else {
            panic!("expected people");
        };
        assert_eq!(generation, 7);
        assert_eq!(result.unwrap()[0].username, "alex");
        worker.join().unwrap();
    }

    #[test]
    fn navigation_rechecks_access_and_prepares_requested_channel_not_first() {
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api =
            crate::api::Api::new(&format!("http://{}", server.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            let detail = r#"{"space":{"id":"s","name":"Space","ownerId":"owner"},"channels":[{"id":"first","spaceId":"s","name":"first","private":false},{"id":"second","spaceId":"s","name":"second","private":true}],"members":[]}"#;
            for (path, body) in [
                ("GET /api/spaces/s", detail),
                (
                    "GET /api/chat/channels/second/messages",
                    r#"{"space":{"id":"s","name":"Space"},"channel":{"id":"second","name":"second"},"messages":[],"cursor":"0","hasMore":false}"#,
                ),
                (
                    "POST /api/chat/session",
                    r#"{"token":"chat","author":{"id":"u","name":"User","isGuest":false}}"#,
                ),
                ("GET /api/spaces/s", detail),
                ("GET /api/spaces/s", detail),
            ] {
                let (stream, _) = server.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(request, format!("{path} HTTP/1.1\r\n"));
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                }
                write!(reader.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let prepared = super::prepare_navigation(
            &api,
            Some("account"),
            Some("s"),
            Some("second"),
            "User",
            None,
            false,
        )
        .unwrap();
        assert_eq!(prepared.detail.unwrap().space.id, "s");
        let mut warm = prepared.conversation.unwrap().0;
        assert_eq!(warm.channel.id, "second");
        warm.channel.id = "revoked".into();
        let failure = super::prepare_navigation(
            &api,
            Some("account"),
            Some("s"),
            Some("revoked"),
            "User",
            Some(warm),
            false,
        )
        .err()
        .unwrap();
        assert!(failure.access_denied);
        assert!(!failure.space_access_denied);
        let speculative =
            super::prefetch_navigation(&api, Some("account"), Some("s"), Some("revoked"));
        assert!(
            speculative.err().unwrap().access_denied,
            "an explicit revoked target must not fall back to the first channel"
        );
        worker.join().unwrap();
    }

    /// A tiny HTTP server answering each expected request line in order.
    type Bodies = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

    /// Also collects request bodies.
    fn serve(
        exchanges: Vec<(&'static str, &'static str, &'static str)>,
    ) -> (crate::api::Api, std::thread::JoinHandle<()>, Bodies) {
        let bodies = Bodies::default();
        let received = bodies.clone();
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api =
            crate::api::Api::new(&format!("http://{}", server.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            for (path, status, body) in exchanges {
                let (stream, _) = server.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(request, format!("{path} HTTP/1.1\r\n"));
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut sent = vec![0; length];
                std::io::Read::read_exact(&mut reader, &mut sent).unwrap();
                if !sent.is_empty() {
                    received
                        .lock()
                        .unwrap()
                        .push(String::from_utf8(sent).unwrap());
                }
                write!(reader.get_mut(), "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        (api, worker, bodies)
    }

    #[test]
    fn visits_open_a_space_channel_by_id_and_other_navigation_still_refuses_it() {
        let channel = r#"{"space":{"id":"space0000001","name":"Studio"},"channel":{"id":"chan00000001","name":"plans"},"messages":[],"pinnedMessages":[],"cursor":"4","hasMore":false,"visitor":true}"#;
        let session =
            r#"{"token":"chat","author":{"id":"u","name":"Jose","isGuest":false,"admin":true}}"#;
        let (api, worker, _bodies) = serve(vec![
            (
                "GET /api/chat/channels/chan00000001/messages",
                "200 OK",
                channel,
            ),
            ("POST /api/chat/session", "200 OK", session),
            (
                "GET /api/chat/channels/chan00000001/messages",
                "200 OK",
                channel,
            ),
        ]);
        let prepared = super::prepare_navigation(
            &api,
            Some("account"),
            None,
            Some("chan00000001"),
            "Jose",
            None,
            true,
        )
        .unwrap();
        assert!(prepared.detail.is_none());
        let (history, session) = prepared.conversation.unwrap();
        assert!(history.visitor);
        assert_eq!(history.space.name, "Studio");
        assert!(
            session.unwrap().author.admin,
            "visitors still get a chat session"
        );
        let refused = super::prepare_navigation(
            &api,
            Some("account"),
            None,
            Some("chan00000001"),
            "Jose",
            None,
            false,
        );
        assert_eq!(
            refused.err().unwrap().message,
            "Caper returned another conversation."
        );
        worker.join().unwrap();
    }

    #[test]
    fn feedback_waiting_count_survives_unread_poll_and_defaults_on_older_servers() {
        let (api, worker, _) = serve(vec![
            (
                "GET /api/spaces/unread",
                "200 OK",
                r#"{"spaces":[],"feedbackWaiting":12}"#,
            ),
            ("GET /api/spaces/unread", "200 OK", r#"{"spaces":[]}"#),
        ]);
        let (sender, receiver) = std::sync::mpsc::channel();
        for expected in [12, 0] {
            super::execute(
                &api,
                super::Command::LoadUnread {
                    epoch: 7,
                    revision: 3,
                    visit: None,
                    token: "account".into(),
                },
                &sender,
                &eframe::egui::Context::default(),
            );
            match receiver.recv().unwrap() {
                super::Event::UnreadLoaded {
                    epoch: 7,
                    revision: 3,
                    visit: None,
                    result: Ok(unread),
                } => {
                    assert_eq!(unread.feedback_waiting, expected);
                    assert!(unread.spaces.is_empty());
                }
                _ => panic!("expected unread result"),
            }
        }
        worker.join().unwrap();
    }

    #[test]
    fn voice_activity_uses_the_member_endpoint_and_preserves_account_epoch() {
        let (api, worker, _) = serve(vec![(
            "GET /api/spaces/voice-activity",
            "200 OK",
            r#"{"spaceIds":["space-one","space-two"]}"#,
        )]);
        let (sender, receiver) = std::sync::mpsc::channel();
        super::execute(
            &api,
            super::Command::LoadVoiceActivity {
                epoch: 7,
                token: "account".into(),
            },
            &sender,
            &eframe::egui::Context::default(),
        );
        match receiver.recv().unwrap() {
            super::Event::VoiceActivityLoaded {
                epoch: 7,
                result: Ok(ids),
            } => {
                assert_eq!(ids, ["space-one", "space-two"]);
            }
            _ => panic!("expected voice activity result"),
        }
        worker.join().unwrap();
    }

    #[test]
    fn feedback_pages_load_with_the_account_bearer_and_their_cursor() {
        let page = r#"{"reports":[{"id":"m2","messageId":"m2","threadRootId":"m1","createdAt":"2026-10-11T19:00:00Z","text":"@feedback hi","attachments":0,"reporter":{"id":"u1","username":"alice","displayName":"Alice","avatarId":3},"channel":{"id":"c1","name":"general","direct":false},"space":{"id":"s1","name":"Studio"},"replyCount":1,"answered":false}],"cursor":"41"}"#;
        let (api, worker, bodies) = serve(vec![
            ("GET /api/feedback", "200 OK", page),
            (
                "GET /api/feedback?before=41",
                "200 OK",
                r#"{"reports":[],"cursor":null}"#,
            ),
            (
                "GET /api/feedback",
                "404 Not Found",
                r#"{"error":"not found"}"#,
            ),
            (
                "GET /api/feedback?archived=true&before=7",
                "200 OK",
                r#"{"reports":[],"cursor":null}"#,
            ),
        ]);
        let (events, receiver) = std::sync::mpsc::channel();
        for (before, archived) in [
            (None, false),
            (Some("41".to_owned()), false),
            (None, false),
            (Some("7".to_owned()), true),
        ] {
            super::execute(
                &api,
                super::Command::LoadFeedback {
                    epoch: 2,
                    request: 5,
                    token: "account".into(),
                    before,
                    archived,
                },
                &events,
                &eframe::egui::Context::default(),
            );
        }
        let mut results = receiver.try_iter().map(|event| match event {
            super::Event::FeedbackLoaded {
                epoch: 2,
                request: 5,
                result,
            } => result,
            _ => panic!("expected feedback"),
        });
        let first = results.next().unwrap().unwrap();
        assert_eq!(first.reports[0].place(), "#general · Studio");
        assert_eq!(first.cursor.as_deref(), Some("41"));
        let last = results.next().unwrap().unwrap();
        assert!(last.reports.is_empty() && last.cursor.is_none());
        assert!(results.next().unwrap().is_err(), "not on the team");
        assert!(results.next().unwrap().unwrap().reports.is_empty());
        worker.join().unwrap();
        assert!(bodies.lock().unwrap().is_empty());
    }

    #[test]
    fn archiving_puts_the_thread_and_reports_failures() {
        let (api, worker, bodies) = serve(vec![
            ("PUT /api/feedback/archive", "204 No Content", ""),
            (
                "PUT /api/feedback/archive",
                "404 Not Found",
                r#"{"error":"not found"}"#,
            ),
        ]);
        let (events, receiver) = std::sync::mpsc::channel();
        for archived in [true, false] {
            super::execute(
                &api,
                super::Command::ArchiveFeedback {
                    epoch: 3,
                    token: "account".into(),
                    channel: "chan00000001".into(),
                    thread_root: "msg000000001".into(),
                    archived,
                },
                &events,
                &eframe::egui::Context::default(),
            );
        }
        let results: Vec<_> = receiver
            .try_iter()
            .map(|event| match event {
                super::Event::FeedbackArchived { epoch: 3, result } => result.is_ok(),
                _ => panic!("expected an archive result"),
            })
            .collect();
        assert_eq!(results, [true, false]);
        worker.join().unwrap();
        let bodies: Vec<serde_json::Value> = bodies
            .lock()
            .unwrap()
            .iter()
            .map(|body| serde_json::from_str(body).unwrap())
            .collect();
        assert_eq!(
            bodies,
            [
                serde_json::json!({"channelId": "chan00000001", "threadRootId": "msg000000001", "archived": true}),
                serde_json::json!({"channelId": "chan00000001", "threadRootId": "msg000000001", "archived": false}),
            ]
        );
    }

    #[test]
    fn speculative_navigation_only_performs_reads_and_creates_no_session() {
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api =
            crate::api::Api::new(&format!("http://{}", server.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            for (path, body) in [
                (
                    "GET /api/spaces/s",
                    r#"{"space":{"id":"s","name":"Space","ownerId":"owner"},"channels":[{"id":"c","spaceId":"s","name":"channel","private":true}],"members":[]}"#,
                ),
                (
                    "GET /api/chat/channels/c/messages",
                    r#"{"space":{"id":"s","name":"Space"},"channel":{"id":"c","name":"channel"},"messages":[],"cursor":"7","hasMore":false}"#,
                ),
            ] {
                let (stream, _) = server.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(request, format!("{path} HTTP/1.1\r\n"));
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                }
                write!(reader.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let read = super::prefetch_navigation(&api, Some("account"), Some("s"), Some("c")).unwrap();
        assert_eq!(read.history.unwrap().cursor, "7");
        worker.join().unwrap();
    }

    #[test]
    fn direct_prefetch_is_read_only_and_cached_navigation_does_not_read_history_again() {
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api =
            crate::api::Api::new(&format!("http://{}", server.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            for (path, body) in [
                (
                    "GET /api/chat/channels/dm0000000001/messages",
                    r#"{"space":{"id":"","name":"Direct messages"},"channel":{"id":"dm0000000001","name":"Alpha","direct":true},"messages":[],"cursor":"9007199254740993","hasMore":true}"#,
                ),
                (
                    "POST /api/chat/session",
                    r#"{"token":"chat","author":{"id":"u","name":"User","isGuest":false}}"#,
                ),
            ] {
                let (stream, _) = server.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(request, format!("{path} HTTP/1.1\r\n"));
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                }
                write!(reader.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let read =
            super::prefetch_navigation(&api, Some("account"), None, Some("dm0000000001")).unwrap();
        assert!(read.detail.is_none());
        let first = super::prepare_navigation(
            &api,
            Some("account"),
            None,
            Some("dm0000000001"),
            "User",
            read.history,
            false,
        )
        .unwrap();
        let mut retained = first.conversation.unwrap().0;
        retained.cursor = "9007199254740994".into();
        let revisited = super::prepare_navigation(
            &api,
            Some("account"),
            None,
            Some("dm0000000001"),
            "User",
            Some(retained),
            false,
        )
        .unwrap();
        assert!(revisited.detail.is_none());
        let history = revisited.conversation.unwrap().0;
        assert_eq!(history.channel.id, "dm0000000001");
        assert_eq!(history.cursor, "9007199254740994");
        assert!(history.has_more);
        worker.join().unwrap();
    }

    #[test]
    fn voice_permission_checks_exact_history_target_before_replacement() {
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api =
            crate::api::Api::new(&format!("http://{}", server.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            for (joined, status, history) in [
                (
                    true,
                    200,
                    r#"{"space":{"id":"s","name":"Space"},"channel":{"id":"c","name":"channel"},"messages":[],"cursor":"7","hasMore":false}"#,
                ),
                (
                    true,
                    200,
                    r#"{"space":{"id":"s","name":"Space"},"channel":{"id":"other","name":"channel"},"messages":[],"cursor":"7","hasMore":false}"#,
                ),
                (true, 403, r#"{"error":"Grant revoked"}"#),
                (
                    false,
                    200,
                    r#"{"space":{"id":"s","name":"Space"},"channel":{"id":"c","name":"channel"},"messages":[],"cursor":"7","hasMore":false}"#,
                ),
            ] {
                let detail = format!(
                    r#"{{"space":{{"id":"s","name":"Space","ownerId":"owner"}},"channels":[{{"id":"c","spaceId":"s","name":"channel","private":false,"joined":{joined}}}],"members":[]}}"#
                );
                for (path, status, body) in [
                    ("/api/spaces/s", 200, detail.as_str()),
                    ("/api/chat/channels/c/messages", status, history),
                ] {
                    let (stream, _) = server.accept().unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut request = String::new();
                    reader.read_line(&mut request).unwrap();
                    assert_eq!(request, format!("GET {path} HTTP/1.1\r\n"));
                    let mut headers = String::new();
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" {
                            break;
                        }
                        headers.push_str(&line);
                    }
                    assert!(
                        headers
                            .to_lowercase()
                            .contains("authorization: bearer permission-only\r\n")
                    );
                    write!(reader.get_mut(), "HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
            }
        });
        let (sender, receiver) = std::sync::mpsc::channel();
        for allowed in [true, false, false, false] {
            super::execute(
                &api,
                super::Command::CheckVoice {
                    request: 13,
                    voice_generation: 27,
                    token: "permission-only".into(),
                    space: "s".into(),
                    channel: "c".into(),
                },
                &sender,
                &eframe::egui::Context::default(),
            );
            let super::Event::VoiceChecked {
                request,
                voice_generation,
                space,
                channel,
                result,
            } = receiver.recv().unwrap()
            else {
                panic!("missing permission result")
            };
            assert_eq!(
                (request, voice_generation, space.as_str(), channel.as_str()),
                (13, 27, "s", "c")
            );
            assert_eq!(result.is_ok(), allowed);
            if !allowed {
                assert!(result.unwrap_err().access_denied);
            }
        }
        worker.join().unwrap();
    }

    #[test]
    fn leave_space_removes_only_self_membership_and_propagates_denial() {
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api =
            crate::api::Api::new(&format!("http://{}/", server.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            for denied in [false, true] {
                let (stream, _) = server.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(
                    request,
                    "DELETE /api/spaces/space-a/members/self-b HTTP/1.1\r\n"
                );
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line);
                }
                assert!(
                    headers
                        .to_ascii_lowercase()
                        .contains("authorization: bearer fixture-token\r\n")
                );
                let response = if denied {
                    "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n"
                };
                reader.get_mut().write_all(response.as_bytes()).unwrap();
            }
        });
        let leave = || {
            super::execute_admin(
                &api,
                "fixture-token",
                super::AdminOperation::LeaveSpace {
                    space: "space-a".into(),
                    member: "self-b".into(),
                },
            )
        };
        assert!(matches!(leave(), Ok(super::AdminResult::SpaceLeft(id)) if id == "space-a"));
        assert!(leave().is_err());
        worker.join().unwrap();
    }

    #[test]
    fn channel_join_routes_to_membership_then_refreshes_detail() {
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api =
            crate::api::Api::new(&format!("http://{}", server.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            for (request_line, status, body) in [
                (
                    "POST /api/spaces/s/channels/c/membership",
                    200,
                    r#"{"id":"c","spaceId":"s","name":"lobby","private":false,"joined":true}"#,
                ),
                (
                    "GET /api/spaces/s",
                    200,
                    r#"{"space":{"id":"s","name":"Studio","ownerId":"owner"},"channels":[{"id":"c","spaceId":"s","name":"lobby","private":false,"joined":true}],"members":[],"channelInvitations":[]}"#,
                ),
                ("DELETE /api/spaces/s/channels/c/membership", 204, ""),
            ] {
                let (stream, _) = server.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert_eq!(line, format!("{request_line} HTTP/1.1\r\n"));
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                }
                write!(reader.get_mut(), "HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let result = super::execute_admin(
            &api,
            "token",
            super::AdminOperation::JoinChannel {
                space: "s".into(),
                channel: "c".into(),
            },
        )
        .unwrap();
        assert!(
            matches!(result, super::AdminResult::ChannelMembership { detail, channel, joined: true } if channel == "c" && detail.channels[0].joined)
        );
        let result = super::execute_admin(
            &api,
            "token",
            super::AdminOperation::LeaveChannel {
                space: "s".into(),
                channel: "c".into(),
            },
        )
        .unwrap();
        assert!(
            matches!(result, super::AdminResult::ChannelLeft(channel) if channel == "c"),
            "Acknowledged leave must not depend on a second HTTP request"
        );
        worker.join().unwrap();
    }

    #[test]
    fn logout_and_channel_switch_isolate_late_results() {
        assert!(
            !current(4, 5, Some("old"), Some("old")),
            "pre-logout event must be discarded"
        );
        assert!(
            !current(5, 5, Some("old"), Some("new")),
            "old-channel event must be discarded"
        );
        assert!(current(5, 5, Some("new"), Some("new")));
    }

    #[test]
    fn stale_auth_cannot_overwrite_or_delete_newer_vault_generation() {
        let mut generation = 8;
        assert!(!advance_generation(&mut generation, 7));
        assert_eq!(generation, 8);
        assert!(advance_generation(&mut generation, 9));
        assert_eq!(generation, 9);
        assert!(!advance_generation(&mut generation, 8));
    }
}

/// Web's sign-in copy (`routes/login.tsx`); raw server messages are not shown.
pub fn login_error(error: &crate::api::ApiError) -> String {
    use reqwest::StatusCode;
    match error.status {
        Some(StatusCode::UNAUTHORIZED) if error.attempts_remaining == Some(0) => {
            "That code can no longer be used. Request a new one."
        }
        Some(StatusCode::UNAUTHORIZED) => {
            "That code is incorrect or expired. Request a new one if needed."
        }
        Some(StatusCode::BAD_REQUEST) => "Enter a valid email address.",
        Some(StatusCode::SERVICE_UNAVAILABLE) => {
            "Sign-in is temporarily unavailable. Please try again later."
        }
        _ => "Something went wrong. Please try again.",
    }
    .into()
}

/// Web's profile save copy (`account/ProfileForm.tsx`).
pub fn profile_error(error: &crate::api::ApiError) -> String {
    use reqwest::StatusCode;
    match error.status {
        Some(StatusCode::CONFLICT) => "That username is already taken.",
        Some(StatusCode::BAD_REQUEST) => "Check the username and display name requirements.",
        _ => "Your profile could not be saved. Please try again.",
    }
    .into()
}

#[cfg(test)]
mod login_copy_tests {
    use super::{login_error, profile_error};
    use crate::api::ApiError;
    use reqwest::StatusCode;

    fn error(status: StatusCode, attempts_remaining: Option<u64>) -> ApiError {
        ApiError {
            status: Some(status),
            message: "raw server text".into(),
            attempts_remaining,
            code: None,
        }
    }

    #[test]
    fn sign_in_and_profile_errors_use_web_copy() {
        assert_eq!(
            login_error(&error(StatusCode::UNAUTHORIZED, Some(2))),
            "That code is incorrect or expired. Request a new one if needed."
        );
        assert_eq!(
            login_error(&error(StatusCode::UNAUTHORIZED, Some(0))),
            "That code can no longer be used. Request a new one."
        );
        assert_eq!(
            login_error(&error(StatusCode::SERVICE_UNAVAILABLE, None)),
            "Sign-in is temporarily unavailable. Please try again later."
        );
        assert_eq!(
            profile_error(&error(StatusCode::CONFLICT, None)),
            "That username is already taken."
        );
        assert_eq!(
            profile_error(&error(StatusCode::INTERNAL_SERVER_ERROR, None)),
            "Your profile could not be saved. Please try again."
        );
    }

    #[test]
    fn a_missing_jump_target_is_an_error_not_lost_access() {
        let not_found = |message: &str| {
            super::LoadError::from(ApiError {
                message: message.into(),
                ..error(StatusCode::NOT_FOUND, None)
            })
        };
        for missing in ["Message not found.", "Thread not found."] {
            let error = not_found(missing);
            assert!(!error.access_denied, "{missing}");
            assert_eq!(error.message, missing);
        }
        assert!(not_found("This channel is no longer available.").access_denied);
        assert!(super::LoadError::from(error(StatusCode::FORBIDDEN, None)).access_denied);
    }
}
