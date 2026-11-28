//! Unsent composer text kept on this device across restarts and updates, as
//! web keeps it in `localStorage`: one JSON file per account, written
//! whenever a draft changes and deleted at logout, so another account on the
//! same device never sees it. Text only; staged files, reply targets and the
//! thread's "also send to channel" choice stay in memory. Drafts are never
//! logged.
//!
//! The text and the staged files that finished uploading also sync between
//! the account's devices (`docs/media.md`, "Draft sync"): each local edit
//! (typing, or a file finishing, removed or re-marked) marks its draft dirty
//! and saves it (`PUT /api/drafts`) a second after it stops, or at once when
//! flushed; a server draft (`GET /api/drafts`, `draft.updated`) applies
//! unless the draft is dirty or it is not newer than the last one applied;
//! and after `GET`, device drafts the server lacks are saved. Windows and
//! Linux share these rules (and `drafts::adopt` for the tiles); they only
//! send the saves and show what changed.

use crate::drafts::Staged;
use crate::model::Attachment;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long after the last edit a dirty draft is saved to the server.
pub const SAVE_DELAY: Duration = Duration::from_secs(1);

/// A draft as the server keeps it: `GET /api/drafts`' entries, the saved
/// draft `PUT` answers, and `draft.updated`'s. `updated_at` is fixed width,
/// so two compare as strings.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerDraft {
    pub channel_id: String,
    #[serde(default)]
    pub thread_root_id: Option<String>,
    #[serde(default)]
    pub text: String,
    /// Its staged files, in order, with signed URLs and spoiler marks.
    #[serde(default, deserialize_with = "attachments")]
    pub attachments: Vec<Attachment>,
    pub updated_at: String,
}

/// Files shaped differently by a newer server are skipped, not fatal.
fn attachments<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Attachment>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_array()
        .map(|items| items.iter().filter_map(Attachment::parse).collect())
        .unwrap_or_default())
}

impl ServerDraft {
    /// Its key on this device: the conversation, or [`thread_key`].
    pub fn key(&self) -> String {
        match self
            .thread_root_id
            .as_deref()
            .filter(|root| !root.is_empty())
        {
            Some(root) => thread_key(root),
            None => self.channel_id.clone(),
        }
    }

    fn files(&self) -> Vec<File> {
        self.attachments
            .iter()
            .map(|attachment| File {
                id: attachment.id.clone(),
                spoiler: attachment.spoiler,
            })
            .collect()
    }
}

/// `GET /api/drafts`.
#[derive(Debug, Deserialize)]
pub struct DraftList {
    pub drafts: Vec<ServerDraft>,
}

/// One draft's save (`PUT /api/drafts`), as [`SavedDrafts::due`] hands it
/// out; its answer goes back to [`SavedDrafts::saved`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftSave {
    pub key: String,
    pub channel_id: String,
    pub thread_root_id: Option<String>,
    /// Empty (with no files) clears it.
    pub text: String,
    /// The staged files that finished uploading, in tile order. None leaves
    /// the server's files as they are: this device has not seen them yet.
    pub attachment_ids: Option<Vec<String>>,
    pub spoiler_attachment_ids: Vec<String>,
}

impl DraftSave {
    pub fn body(&self) -> serde_json::Value {
        let mut body = serde_json::json!({
            "channelId": self.channel_id,
            "threadRootId": self.thread_root_id,
            "text": self.text,
        });
        if let Some(ids) = &self.attachment_ids {
            body["attachmentIds"] = serde_json::json!(ids);
            body["spoilerAttachmentIds"] = serde_json::json!(self.spoiler_attachment_ids);
        }
        body
    }
}

/// Why a save failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveFailure {
    /// Offline, or the server failed: the draft stays dirty and saves again
    /// with its next edit or the next reconnect.
    Retry,
    /// Refused (too long, or the conversation can no longer be read): saving
    /// it again would be refused too, so it stays on this device only.
    Refused,
}

impl SaveFailure {
    pub fn from_status(status: Option<u16>) -> Self {
        match status {
            Some(400 | 403 | 404 | 413 | 422) => Self::Refused,
            _ => Self::Retry,
        }
    }
}

/// A server draft that changed this device's copy: show it where its
/// composer is on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub key: String,
    pub channel_id: String,
    pub thread_root_id: Option<String>,
    /// The new text, when it changed: replaces the composer's (cursor at the
    /// end, no typing indicator).
    pub text: Option<String>,
    /// The draft's files, when they changed: `drafts::adopt` them into the
    /// composer's tiles.
    pub files: Option<Vec<Attachment>>,
}

/// Conversation drafts by channel or DM ID, and thread replies by
/// [`thread_key`]. Without a directory or account it keeps them in memory.
#[derive(Default)]
pub struct SavedDrafts {
    account: String,
    path: Option<PathBuf>,
    texts: BTreeMap<String, String>,
    sync: BTreeMap<String, Sync>,
}

/// A synced draft's file: its ID and the composer's spoiler mark.
#[derive(Clone, Debug, PartialEq, Eq)]
struct File {
    id: String,
    spoiler: bool,
}

/// A draft's sync state. The first three fields are kept in the file.
#[derive(Clone, Default, Deserialize, Serialize)]
struct Sync {
    /// A thread reply's conversation; a conversation draft's is its key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    channel: Option<String>,
    /// The newest server `updatedAt` applied, or answered to a save.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    applied: String,
    /// Edited here since the server last had it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    dirty: bool,
    /// The last edit, which the save waits `SAVE_DELAY` after.
    #[serde(skip)]
    edited: Option<Instant>,
    /// Flushed: saves without waiting.
    #[serde(skip)]
    now: bool,
    /// The text and files of the save in flight; one at a time per draft.
    #[serde(skip)]
    sending: Option<(String, Option<Vec<File>>)>,
    /// A save failed (or the dirty draft is from before a restart): it
    /// waits for the next edit or reconnect.
    #[serde(skip)]
    waiting: bool,
    /// Its finished files as this device's composer holds them. None until
    /// this run has seen them (tiles are not kept across restarts), so a
    /// save then leaves the server's files alone.
    #[serde(skip)]
    files: Option<Vec<File>>,
}

impl Sync {
    fn edit(&mut self) {
        self.dirty = true;
        self.edited = Some(Instant::now());
        self.waiting = false;
    }
}

/// The file: the drafts' text and their sync state. Files from before sync
/// hold only the text map.
#[derive(Deserialize, Serialize)]
struct Stored {
    texts: BTreeMap<String, String>,
    #[serde(default)]
    sync: BTreeMap<String, Sync>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OnDisk {
    Stored(Stored),
    Texts(BTreeMap<String, String>),
}

/// A thread reply's key: root message IDs never collide with channel IDs.
pub fn thread_key(root: &str) -> String {
    format!("thread:{root}")
}

fn draft_key(channel: &str, root: Option<&str>) -> String {
    root.map_or_else(|| channel.to_owned(), thread_key)
}

impl SavedDrafts {
    /// `account`'s saved drafts in `dir`. A missing or unreadable file means none.
    pub fn open(dir: Option<&Path>, account: &str) -> Self {
        let file_safe = !account.is_empty()
            && account
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        let path = dir
            .filter(|_| file_safe)
            .map(|dir| dir.join(format!("{account}.json")));
        let (texts, mut sync) = match path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            Some(OnDisk::Stored(stored)) => (stored.texts, stored.sync),
            Some(OnDisk::Texts(texts)) => (texts, BTreeMap::new()),
            None => Default::default(),
        };
        // Edits left unsaved by the last run go up once sync is ready.
        for state in sync.values_mut() {
            state.waiting = state.dirty;
        }
        Self {
            account: account.to_owned(),
            path,
            texts,
            sync,
        }
    }

    /// Opens `account`'s drafts unless they are the ones open.
    pub fn open_for(&mut self, dir: Option<&Path>, account: &str) {
        if self.account != account {
            *self = Self::open(dir, account);
        }
    }

    pub fn get(&self, key: &str) -> &str {
        self.texts.get(key).map_or("", String::as_str)
    }

    /// Keeps `text` as `key`'s draft; blank text (a send, a clear) removes it.
    /// A change is an edit: the draft is dirty and saves after `SAVE_DELAY`.
    pub fn set(&mut self, key: &str, text: &str) {
        if self.store(key, text) {
            self.sync.entry(key.to_owned()).or_default().edit();
            self.save();
        }
    }

    /// [`set`](Self::set) for a thread reply, which syncs with its conversation.
    pub fn set_thread(&mut self, channel: &str, root: &str, text: &str) {
        let key = thread_key(root);
        let state = self.sync.entry(key.clone()).or_default();
        if state.channel.as_deref() != Some(channel) {
            state.channel = Some(channel.to_owned());
            if !self.get(&key).is_empty() {
                self.save();
            }
        }
        self.set(&key, text);
    }

    /// Notes every composer's finished files from `tiles` (all of them, in
    /// tile order, across conversations and threads): a file that finished
    /// uploading, was removed (or sent) or re-marked is an edit like typing.
    pub fn note_files<T: Staged>(&mut self, tiles: &[T]) {
        let mut composers: BTreeMap<(String, Option<String>), Vec<File>> = BTreeMap::new();
        for tile in tiles.iter().filter(|tile| tile.in_composer()) {
            let upload = tile.upload();
            let files = composers
                .entry((upload.channel.clone(), upload.thread_root_id.clone()))
                .or_default();
            if let Some(attachment) = &upload.attachment {
                files.push(File {
                    id: attachment.id.clone(),
                    spoiler: upload.spoiler,
                });
            }
        }
        // Drafts that had files and have no tile left were emptied.
        let emptied = self
            .sync
            .iter()
            .filter(|(_, state)| state.files.as_ref().is_some_and(|files| !files.is_empty()))
            .filter_map(|(key, _)| self.target(key))
            .filter(|target| !composers.contains_key(target))
            .collect::<Vec<_>>();
        for target in emptied {
            composers.insert(target, Vec::new());
        }
        for ((channel, root), files) in composers {
            self.set_files(&channel, root.as_deref(), files);
        }
    }

    fn set_files(&mut self, channel: &str, root: Option<&str>, files: Vec<File>) {
        let state = self.sync.entry(draft_key(channel, root)).or_default();
        if root.is_some() && state.channel.as_deref() != Some(channel) {
            state.channel = Some(channel.to_owned());
        }
        if state.files.as_ref() == Some(&files) {
            return;
        }
        // No finished file yet, and the server's unseen: nothing to say.
        if state.files.is_none() && files.is_empty() {
            return;
        }
        state.files = Some(files);
        state.edit();
        self.save();
    }

    /// Writes the device copy; returns whether it changed.
    fn store(&mut self, key: &str, text: &str) -> bool {
        if text.trim().is_empty() {
            self.texts.remove(key).is_some()
        } else if self.get(key) == text {
            false
        } else {
            self.texts.insert(key.to_owned(), text.to_owned());
            true
        }
    }

    /// The conversation `key` saves to, and its thread root.
    fn target(&self, key: &str) -> Option<(String, Option<String>)> {
        match key.strip_prefix("thread:") {
            Some(root) => {
                let channel = self.sync.get(key)?.channel.clone()?;
                Some((channel, Some(root.to_owned())))
            }
            None => Some((key.to_owned(), None)),
        }
    }

    /// Leaving a conversation or thread, closing or quitting, and sending:
    /// every dirty draft saves now rather than after the pause.
    pub fn flush(&mut self) {
        for state in self.sync.values_mut().filter(|state| state.dirty) {
            state.now = true;
        }
    }

    /// The profiles subscription is (again) ready: failed saves, and edits
    /// from before a restart, save now. The client reloads `GET` beside it.
    pub fn reconnected(&mut self) {
        for state in self.sync.values_mut().filter(|state| state.dirty) {
            state.waiting = false;
            state.now = true;
        }
    }

    /// The saves to send at `now`: dirty drafts whose edits paused for
    /// `SAVE_DELAY` (or flushed), each with nothing of its own in flight.
    pub fn due(&mut self, now: Instant) -> Vec<DraftSave> {
        let ready = self
            .sync
            .iter()
            .filter(|(_, state)| {
                state.dirty
                    && state.sending.is_none()
                    && !state.waiting
                    && (state.now
                        || state
                            .edited
                            .is_none_or(|at| now.saturating_duration_since(at) >= SAVE_DELAY))
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let mut saves = Vec::new();
        for key in ready {
            let target = self.target(&key);
            let text = self.get(&key).to_owned();
            let state = self.sync.get_mut(&key).expect("collected above");
            state.now = false;
            // A thread reply from before sync knows no conversation: it
            // stays on this device.
            let Some((channel_id, thread_root_id)) = target else {
                state.dirty = false;
                continue;
            };
            let files = state.files.clone();
            state.sending = Some((text.clone(), files.clone()));
            saves.push(DraftSave {
                key,
                channel_id,
                thread_root_id,
                text,
                spoiler_attachment_ids: files
                    .iter()
                    .flatten()
                    .filter(|file| file.spoiler)
                    .map(|file| file.id.clone())
                    .collect(),
                attachment_ids: files.map(|files| files.into_iter().map(|file| file.id).collect()),
            });
        }
        saves
    }

    /// How long until the next save is due, for the client's timer; `None`
    /// when nothing waits on time.
    pub fn next_save(&self, now: Instant) -> Option<Duration> {
        self.sync
            .values()
            .filter(|state| state.dirty && state.sending.is_none() && !state.waiting)
            .map(|state| match state.edited {
                Some(at) if !state.now => {
                    SAVE_DELAY.saturating_sub(now.saturating_duration_since(at))
                }
                _ => Duration::ZERO,
            })
            .min()
    }

    /// A save's answer. Its `updatedAt` counts as applied, so the save's own
    /// `draft.updated` echo changes nothing; edits since stay dirty. A save
    /// that left the server's files alone (this run had not seen them)
    /// returns them, to show.
    pub fn saved(
        &mut self,
        save: &DraftSave,
        result: Result<ServerDraft, SaveFailure>,
    ) -> Option<Applied> {
        let text = self.get(&save.key).to_owned();
        let state = self.sync.get_mut(&save.key)?;
        let (sent_text, sent_files) = state.sending.clone()?;
        let ids = sent_files
            .as_ref()
            .map(|files| files.iter().map(|file| file.id.clone()).collect::<Vec<_>>());
        if sent_text != save.text || ids != save.attachment_ids {
            return None;
        }
        state.sending = None;
        let unchanged = text == sent_text && state.files == sent_files;
        let mut applied = None;
        match result {
            Ok(draft) => {
                if draft.updated_at > state.applied {
                    state.applied.clone_from(&draft.updated_at);
                }
                state.dirty &= !unchanged;
                if state.files.is_none() {
                    state.files = Some(draft.files());
                    applied = (!draft.attachments.is_empty()).then(|| Applied {
                        key: save.key.clone(),
                        channel_id: draft.channel_id,
                        thread_root_id: draft.thread_root_id,
                        text: None,
                        files: Some(draft.attachments),
                    });
                }
            }
            // Edits since then are the next edit, which saves as usual.
            Err(SaveFailure::Retry) => state.waiting = unchanged,
            Err(SaveFailure::Refused) => state.dirty &= !unchanged,
        }
        self.save();
        applied
    }

    /// A server draft (`draft.updated`, or one of `GET`'s): it applies
    /// unless the draft is dirty or it is not newer than the last applied.
    /// Returns what changed on this device, for an on-screen composer to
    /// show (without a typing indicator).
    pub fn apply(&mut self, draft: ServerDraft) -> Option<Applied> {
        let key = draft.key();
        let state = self.sync.entry(key.clone()).or_default();
        if state.dirty || draft.updated_at <= state.applied {
            return None;
        }
        state.applied.clone_from(&draft.updated_at);
        if draft.thread_root_id.is_some() {
            state.channel = Some(draft.channel_id.clone());
        }
        let files = draft.files();
        let files_changed = match &state.files {
            Some(known) => *known != files,
            None => !files.is_empty(),
        };
        state.files = Some(files);
        let text_changed = self.store(&key, &draft.text);
        self.save();
        (text_changed || files_changed).then(|| Applied {
            key,
            text: text_changed.then_some(draft.text),
            files: files_changed.then_some(draft.attachments),
            channel_id: draft.channel_id,
            thread_root_id: draft.thread_root_id,
        })
    }

    /// `GET /api/drafts`: each applies as [`apply`](Self::apply) does, then a
    /// device draft with text or finished files that the server has no entry
    /// for saves now, so what was written before sync or offline reaches the
    /// other devices.
    pub fn loaded(&mut self, drafts: Vec<ServerDraft>) -> Vec<Applied> {
        let known = drafts.iter().map(ServerDraft::key).collect::<BTreeSet<_>>();
        let changed = drafts
            .into_iter()
            .filter_map(|draft| self.apply(draft))
            .collect();
        let with_files = self
            .sync
            .iter()
            .filter(|(_, state)| state.files.as_ref().is_some_and(|files| !files.is_empty()))
            .map(|(key, _)| key);
        let device_only = self
            .texts
            .keys()
            .chain(with_files)
            .filter(|key| !known.contains(*key) && self.target(key).is_some())
            .cloned()
            .collect::<BTreeSet<_>>();
        for key in device_only {
            let state = self.sync.entry(key).or_default();
            state.dirty = true;
            state.now = true;
            state.waiting = false;
        }
        changed
    }

    /// Logout: forgets every draft, deletes the account's file and keeps
    /// nothing more until the next account opens its own. The server keeps
    /// them for the next sign-in.
    pub fn erase(&mut self) {
        self.account.clear();
        if let Some(path) = self.path.take()
            && let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("Could not remove saved drafts: {}", error.kind());
        }
        self.texts.clear();
        self.sync.clear();
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let result = if self.texts.is_empty() && self.sync.is_empty() {
            std::fs::remove_file(path).or_else(|error| match error.kind() {
                std::io::ErrorKind::NotFound => Ok(()),
                _ => Err(error),
            })
        } else {
            // Write beside it, then rename: a crash never leaves half a file.
            let temporary = path.with_extension("json.tmp");
            serde_json::to_vec(&serde_json::json!({
                "texts": self.texts,
                "sync": self.sync,
            }))
            .map_err(std::io::Error::other)
            .and_then(|json| {
                path.parent().map_or(Ok(()), std::fs::create_dir_all)?;
                std::fs::write(&temporary, json)?;
                std::fs::rename(&temporary, path)
            })
        };
        // The kind only: the path names the account and the file holds drafts.
        if let Err(error) = result {
            eprintln!("Could not save drafts: {}", error.kind());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drafts::{Upload, adopt};
    use crate::model::{AttachmentKind, AttachmentStatus};
    use crate::worker::UploadUpdate;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("caper-drafts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn drafts_survive_a_restart_per_account_and_end_at_send_and_logout() {
        let dir = dir("restart");
        let mut first = SavedDrafts::open(Some(&dir), "alice123");
        first.set("general", "half a thought");
        first.set_thread("general", "root1", "a reply");
        first.set("design", "sent soon");
        first.set("design", "");

        let mut reopened = SavedDrafts::open(Some(&dir), "alice123");
        assert_eq!(reopened.get("general"), "half a thought");
        assert_eq!(reopened.get(&thread_key("root1")), "a reply");
        assert_eq!(reopened.get("design"), "");
        assert_eq!(reopened.get("root1"), "");
        // Another account on the device starts with none.
        assert_eq!(SavedDrafts::open(Some(&dir), "bob4567").get("general"), "");

        reopened.erase();
        reopened.set("general", "after logout");
        assert_eq!(SavedDrafts::open(Some(&dir), "alice123").get("general"), "");
        // Signing back in opens the (now empty) file again.
        reopened.open_for(Some(&dir), "alice123");
        reopened.set("general", "signed in again");
        assert_eq!(
            SavedDrafts::open(Some(&dir), "alice123").get("general"),
            "signed in again"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsafe_account_ids_and_corrupt_files_keep_drafts_in_memory_only() {
        let dir = dir("unsafe");
        let mut escaping = SavedDrafts::open(Some(&dir), "../outside");
        escaping.set("general", "kept in memory");
        assert_eq!(escaping.get("general"), "kept in memory");
        assert!(!dir.exists());

        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("alice123.json"), "not json").unwrap();
        assert_eq!(SavedDrafts::open(Some(&dir), "alice123").get("general"), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn server(key: &str, text: &str, at: &str) -> ServerDraft {
        let (channel_id, thread_root_id) = match key.split_once('/') {
            Some((channel, root)) => (channel.to_owned(), Some(root.to_owned())),
            None => (key.to_owned(), None),
        };
        ServerDraft {
            channel_id,
            thread_root_id,
            text: text.into(),
            attachments: Vec::new(),
            updated_at: format!("2026-11-27T18:04:{at}.000000Z"),
        }
    }

    /// Only what text an applied draft changed, by key.
    fn text(applied: Option<Applied>) -> Option<(String, String)> {
        applied.and_then(|applied| Some((applied.key, applied.text?)))
    }

    fn later() -> Instant {
        Instant::now() + SAVE_DELAY
    }

    /// Sends what is due and answers each as the server would at `at`.
    fn sync(drafts: &mut SavedDrafts, at: &str) -> Vec<DraftSave> {
        let saves = drafts.due(later());
        for save in &saves {
            let mut answer = server(&save.channel_id, &save.text, at);
            answer.thread_root_id = save.thread_root_id.clone();
            drafts.saved(save, Ok(answer));
        }
        saves
    }

    #[test]
    fn edits_save_after_a_pause_one_at_a_time_and_flush_at_once() {
        let mut drafts = SavedDrafts::default();
        drafts.set("general", "hel");
        assert!(drafts.due(Instant::now()).is_empty(), "typing continues");
        assert!(drafts.next_save(Instant::now()).unwrap() <= SAVE_DELAY);
        let saves = drafts.due(later());
        assert_eq!(
            saves,
            [DraftSave {
                key: "general".into(),
                channel_id: "general".into(),
                thread_root_id: None,
                text: "hel".into(),
                attachment_ids: None,
                spoiler_attachment_ids: Vec::new(),
            }]
        );
        assert_eq!(
            saves[0].body(),
            serde_json::json!({"channelId": "general", "threadRootId": null, "text": "hel"}),
            "files this run has not seen are left alone"
        );
        // Typed while it saves: the next save waits for this one.
        drafts.set("general", "hello");
        drafts.flush();
        assert!(drafts.due(later()).is_empty());
        assert_eq!(drafts.next_save(Instant::now()), None);
        drafts.saved(&saves[0], Ok(server("general", "hel", "01")));
        assert_eq!(drafts.next_save(Instant::now()), Some(Duration::ZERO));
        assert_eq!(sync(&mut drafts, "02")[0].text, "hello");
        assert!(drafts.due(later()).is_empty(), "nothing is dirty");

        // A thread reply saves to its conversation; a send saves empty text.
        drafts.set_thread("general", "root1", "a reply");
        drafts.set_thread("general", "root1", "");
        drafts.flush();
        let saves = drafts.due(Instant::now());
        assert_eq!(saves[0].channel_id, "general");
        assert_eq!(saves[0].thread_root_id.as_deref(), Some("root1"));
        assert_eq!(saves[0].text, "");
    }

    #[test]
    fn a_dirty_draft_is_not_overwritten() {
        let mut drafts = SavedDrafts::default();
        drafts.set("general", "mine");
        assert_eq!(drafts.apply(server("general", "theirs", "05")), None);
        assert_eq!(drafts.get("general"), "mine");
        // Still dirty while its save is in flight.
        let saves = drafts.due(later());
        assert_eq!(drafts.apply(server("general", "theirs", "06")), None);
        assert_eq!(drafts.get("general"), "mine");
        drafts.saved(&saves[0], Ok(server("general", "mine", "07")));
        assert_eq!(
            text(drafts.apply(server("general", "newer", "08"))),
            Some(("general".into(), "newer".into()))
        );
    }

    #[test]
    fn older_or_repeated_server_drafts_are_ignored() {
        let mut drafts = SavedDrafts::default();
        assert_eq!(
            text(drafts.apply(server("general", "second", "02"))),
            Some(("general".into(), "second".into()))
        );
        assert_eq!(drafts.apply(server("general", "first", "01")), None);
        assert_eq!(drafts.apply(server("general", "second", "02")), None);
        assert_eq!(drafts.get("general"), "second");
        // A cleared draft clears the device copy; a thread reply finds its key.
        assert_eq!(
            text(drafts.apply(server("general", "", "03"))),
            Some(("general".into(), String::new()))
        );
        assert_eq!(drafts.get("general"), "");
        assert_eq!(
            text(drafts.apply(server("general/root1", "reply", "04"))),
            Some((thread_key("root1"), "reply".into()))
        );
        assert!(drafts.due(later()).is_empty(), "applying is not an edit");
    }

    #[test]
    fn a_saves_own_echo_changes_nothing() {
        let mut drafts = SavedDrafts::default();
        drafts.set("general", "hi");
        let saves = drafts.due(later());
        // The echo can beat the answer: the draft is still dirty then.
        let echo = server("general", "hi", "09");
        assert_eq!(drafts.apply(echo.clone()), None);
        drafts.saved(&saves[0], Ok(echo.clone()));
        assert_eq!(drafts.apply(echo), None);
        assert_eq!(drafts.get("general"), "hi");
        assert!(drafts.due(later()).is_empty());
    }

    #[test]
    fn device_only_drafts_are_uploaded_after_get() {
        let mut drafts = SavedDrafts::default();
        drafts.set("general", "offline words");
        drafts.set_thread("design", "root1", "offline reply");
        drafts.set("design", "both have it");
        sync(&mut drafts, "01");
        // Gone from the server (another device's view of it is older).
        let changed = drafts.loaded(vec![server("design", "from the phone", "02")]);
        assert_eq!(
            changed
                .into_iter()
                .map(Some)
                .filter_map(text)
                .collect::<Vec<_>>(),
            [("design".into(), "from the phone".into())]
        );
        let mut saves = drafts.due(Instant::now());
        saves.sort_by(|a, b| a.key.cmp(&b.key));
        assert_eq!(
            saves
                .iter()
                .map(|save| (save.key.as_str(), save.text.as_str()))
                .collect::<Vec<_>>(),
            [
                ("general", "offline words"),
                ("thread:root1", "offline reply")
            ]
        );
        assert_eq!(saves[1].channel_id, "design");
    }

    #[test]
    fn failed_saves_wait_for_an_edit_or_reconnect_and_survive_a_restart() {
        let dir = dir("sync");
        let mut drafts = SavedDrafts::open(Some(&dir), "alice123");
        drafts.set("general", "unsent");
        let saves = drafts.due(later());
        drafts.saved(&saves[0], Err(SaveFailure::Retry));
        assert!(drafts.due(later()).is_empty(), "no retry loop");
        assert_eq!(drafts.apply(server("general", "theirs", "01")), None);

        // Restarted: still dirty, saved once sync is ready.
        let mut reopened = SavedDrafts::open(Some(&dir), "alice123");
        assert!(reopened.due(later()).is_empty());
        assert_eq!(reopened.apply(server("general", "theirs", "02")), None);
        reopened.reconnected();
        let saves = reopened.due(Instant::now());
        assert_eq!(saves[0].text, "unsent");
        // A refused save stops trying.
        reopened.saved(&saves[0], Err(SaveFailure::Refused));
        reopened.reconnected();
        assert!(reopened.due(later()).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_from_before_sync_still_open() {
        let dir = dir("legacy");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("alice123.json"),
            r#"{"general":"kept","thread:root1":"old reply"}"#,
        )
        .unwrap();
        let mut drafts = SavedDrafts::open(Some(&dir), "alice123");
        assert_eq!(drafts.get("general"), "kept");
        assert_eq!(drafts.get(&thread_key("root1")), "old reply");
        // Without its conversation, the old reply stays on this device.
        drafts.loaded(Vec::new());
        let saves = drafts.due(Instant::now());
        assert_eq!(
            saves.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(),
            ["general"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Files (rule 4) ----

    fn file(id: &str) -> Attachment {
        Attachment {
            spoiler: false,
            id: id.into(),
            kind: AttachmentKind::Image,
            content_type: "image/png".into(),
            name: format!("{id}.png"),
            size: 2048,
            width: None,
            height: None,
            duration_ms: None,
            preview: true,
            url: Some(format!("https://cdn.example/{id}")),
            preview_url: Some(format!("https://cdn.example/{id}/preview")),
            unavailable: false,
            status: AttachmentStatus::Ready,
            animated: false,
            voice: false,
            waveform: None,
            transcript: None,
        }
    }

    fn with_files(mut draft: ServerDraft, files: &[Attachment]) -> ServerDraft {
        draft.attachments = files.to_vec();
        draft
    }

    /// A file picked on this device for `general`, still uploading.
    fn picked(key: u64) -> Upload {
        Upload::picked(
            key,
            "general".into(),
            None,
            "photo.png".into(),
            Some("photo.png".into()),
        )
    }

    fn ids(tiles: &[Upload]) -> Vec<Option<&str>> {
        tiles
            .iter()
            .map(|tile| tile.attachment.as_ref().map(|file| file.id.as_str()))
            .collect()
    }

    #[test]
    fn server_files_parse_with_their_spoiler_marks() {
        let draft: ServerDraft = serde_json::from_value(serde_json::json!({
            "channelId": "chan00000001", "threadRootId": null, "text": "",
            "attachments": [
                {"id": "a1", "kind": "image", "contentType": "image/png", "name": "a.png",
                 "size": 5, "url": "https://cdn.example/a1", "spoiler": true},
                {"unexpected": "shape"}
            ],
            "updatedAt": "2026-11-27T18:04:05.123456Z"
        }))
        .unwrap();
        assert_eq!(draft.attachments.len(), 1, "a malformed file is skipped");
        assert!(draft.attachments[0].spoiler);
    }

    #[test]
    fn a_finished_upload_saves_its_id_and_an_uploading_one_waits() {
        let mut drafts = SavedDrafts::default();
        let mut tiles = vec![picked(1), picked(2)];
        drafts.note_files(&tiles);
        assert!(drafts.due(later()).is_empty(), "nothing finished yet");

        tiles[1].apply(UploadUpdate::Finished(Ok(file("a2"))));
        tiles[1].spoiler = true;
        drafts.note_files(&tiles);
        let saves = drafts.due(later());
        assert_eq!(saves[0].attachment_ids, Some(vec!["a2".to_owned()]));
        assert_eq!(saves[0].spoiler_attachment_ids, ["a2"]);
        assert_eq!(
            saves[0].body()["attachmentIds"],
            serde_json::json!(["a2"]),
            "the uploading file is not sent"
        );
        let mut answer = with_files(server("general", "", "01"), &[file("a2")]);
        answer.attachments[0].spoiler = true;
        drafts.saved(&saves[0], Ok(answer));

        // Noting the same tiles again is no edit; finishing the other is.
        drafts.note_files(&tiles);
        assert!(drafts.due(later()).is_empty());
        tiles[0].apply(UploadUpdate::Finished(Ok(file("a1"))));
        drafts.note_files(&tiles);
        assert_eq!(
            drafts.due(later())[0].attachment_ids,
            Some(vec!["a1".to_owned(), "a2".to_owned()]),
            "in tile order"
        );
    }

    #[test]
    fn server_files_become_ready_tiles_beside_uploads_and_dropped_ones_go() {
        let mut drafts = SavedDrafts::default();
        // This device: one file sent up already (a1), one still uploading.
        let mut done = picked(1);
        done.apply(UploadUpdate::Finished(Ok(file("a1"))));
        let mut tiles = vec![done, picked(2)];
        let mut other = Upload::synced(9, "design".into(), None, file("d1"));
        other.spoiler = true;
        tiles.push(other);
        drafts.note_files(&tiles);
        sync(&mut drafts, "01");

        // Another device removed a1 and added b1 and b2 (b2 marked).
        let mut b2 = file("b2");
        b2.spoiler = true;
        let applied = drafts
            .apply(with_files(
                server("general", "", "02"),
                &[file("b1"), b2.clone()],
            ))
            .unwrap();
        assert_eq!(applied.text, None, "the text did not change");
        let server_files = applied.files.unwrap();
        let mut key = 10;
        let dropped = adopt(&mut tiles, "general", None, &server_files, |attachment| {
            key += 1;
            Upload::synced(key, "general".into(), None, attachment.clone())
        });
        assert_eq!(ids(&dropped), [Some("a1")]);
        let general = tiles
            .iter()
            .filter(|tile| tile.channel == "general")
            .map(|tile| tile.attachment.as_ref().map(|file| file.id.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(general, [Some("b1"), Some("b2"), None]);
        let b2_tile = tiles.iter().find(|tile| tile.key == 12).unwrap();
        assert!(b2_tile.synced && b2_tile.spoiler && !b2_tile.uploading);
        assert!(!b2_tile.needs_upload(), "no upload, no local original");
        assert_eq!(
            b2_tile.thumbnail_url(),
            Some("https://cdn.example/b2/preview")
        );
        assert!(
            tiles.iter().any(|tile| tile.channel == "design"),
            "another composer's tiles stay"
        );
        // Showing them is no edit; the uploading file joins once it finishes.
        drafts.note_files(&tiles);
        assert!(drafts.due(later()).is_empty());
        let uploading = tiles.iter_mut().find(|tile| tile.key == 2).unwrap();
        uploading.apply(UploadUpdate::Finished(Ok(file("a2"))));
        drafts.note_files(&tiles);
        let saves = drafts.due(later());
        assert_eq!(
            saves[0].attachment_ids,
            Some(vec!["b1".into(), "b2".into(), "a2".into()])
        );
        assert_eq!(saves[0].spoiler_attachment_ids, ["b2"]);
    }

    #[test]
    fn removing_every_file_saves_none_and_own_file_echo_changes_nothing() {
        let mut drafts = SavedDrafts::default();
        let mut done = picked(1);
        done.apply(UploadUpdate::Finished(Ok(file("a1"))));
        let mut tiles = vec![done];
        drafts.note_files(&tiles);
        let saves = drafts.due(later());
        let echo = with_files(server("general", "", "01"), &[file("a1")]);
        assert_eq!(drafts.apply(echo.clone()), None, "dirty while saving");
        drafts.saved(&saves[0], Ok(echo.clone()));
        assert_eq!(drafts.apply(echo), None, "its own echo");

        tiles.clear();
        drafts.note_files(&tiles);
        let saves = drafts.due(later());
        assert_eq!(saves[0].attachment_ids, Some(Vec::new()));
        assert_eq!(saves[0].body()["attachmentIds"], serde_json::json!([]));
    }

    #[test]
    fn a_save_that_left_unseen_files_alone_shows_them() {
        let mut drafts = SavedDrafts::default();
        drafts.set("general", "typed before the server's files arrived");
        let saves = drafts.due(later());
        assert_eq!(saves[0].attachment_ids, None);
        let answer = with_files(server("general", &saves[0].text, "01"), &[file("x1")]);
        let applied = drafts.saved(&saves[0], Ok(answer)).unwrap();
        assert_eq!(applied.text, None);
        assert_eq!(applied.files.unwrap()[0].id, "x1");
    }

    #[test]
    fn a_synced_tile_never_uploads_again_and_retry_clears_its_failure() {
        let mut tile = Upload::synced(1, "general".into(), None, file("s1"));
        tile.error = Some("Failed to download.".into());
        tile.upload_expired = true;
        assert!(!tile.needs_upload());
        assert_eq!(tile.restart(), None);
        assert_eq!(tile.error, None);
        assert_eq!(tile.attachment.as_ref().unwrap().id, "s1");
    }
}
