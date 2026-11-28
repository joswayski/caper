//! Unsent composer text kept on this device across restarts and updates, as
//! web keeps it in `localStorage`: one JSON file per account, written
//! whenever a draft changes and deleted at logout, so another account on the
//! same device never sees it. Text only; staged files, reply targets and the
//! thread's "also send to channel" choice stay in memory. Drafts are never
//! logged.
//!
//! The text also syncs between the account's devices (`docs/media.md`,
//! "Draft sync"): each local edit marks its draft dirty and saves it
//! (`PUT /api/drafts`) a second after typing stops, or at once when flushed;
//! a server draft (`GET /api/drafts`, `draft.updated`) applies unless the
//! draft is dirty or it is not newer than the last one applied; and after
//! `GET`, device drafts the server lacks are saved. Windows and Linux share
//! these rules; they only send the saves and show what changed.

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
    pub updated_at: String,
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
    /// Empty clears it.
    pub text: String,
}

impl DraftSave {
    pub fn body(&self) -> serde_json::Value {
        serde_json::json!({
            "channelId": self.channel_id,
            "threadRootId": self.thread_root_id,
            "text": self.text,
        })
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

/// Conversation drafts by channel or DM ID, and thread replies by
/// [`thread_key`]. Without a directory or account it keeps them in memory.
#[derive(Default)]
pub struct SavedDrafts {
    account: String,
    path: Option<PathBuf>,
    texts: BTreeMap<String, String>,
    sync: BTreeMap<String, Sync>,
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
    /// The text of the save in flight; one at a time per draft.
    #[serde(skip)]
    sending: Option<String>,
    /// A save failed (or the dirty draft is from before a restart): it
    /// waits for the next edit or reconnect.
    #[serde(skip)]
    waiting: bool,
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
            let state = self.sync.entry(key.to_owned()).or_default();
            state.dirty = true;
            state.edited = Some(Instant::now());
            state.waiting = false;
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
            state.sending = Some(text.clone());
            saves.push(DraftSave {
                key,
                channel_id,
                thread_root_id,
                text,
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
    /// `draft.updated` echo changes nothing; text typed since stays dirty.
    pub fn saved(&mut self, save: &DraftSave, result: Result<ServerDraft, SaveFailure>) {
        let unchanged = self.get(&save.key) == save.text.as_str();
        let Some(state) = self.sync.get_mut(&save.key) else {
            return;
        };
        if state.sending.as_deref() != Some(save.text.as_str()) {
            return;
        }
        state.sending = None;
        match result {
            Ok(draft) => {
                if draft.updated_at > state.applied {
                    state.applied = draft.updated_at;
                }
                state.dirty &= !unchanged;
            }
            // Text typed since then is the next edit, which saves as usual.
            Err(SaveFailure::Retry) => state.waiting = unchanged,
            Err(SaveFailure::Refused) => state.dirty &= !unchanged,
        }
        self.save();
    }

    /// A server draft (`draft.updated`, or one of `GET`'s): it applies
    /// unless the draft is dirty or it is not newer than the last applied.
    /// Returns its key and text when the device copy changed, for an
    /// on-screen composer to show (without a typing indicator).
    pub fn apply(&mut self, draft: ServerDraft) -> Option<(String, String)> {
        let key = draft.key();
        let state = self.sync.entry(key.clone()).or_default();
        if state.dirty || draft.updated_at <= state.applied {
            return None;
        }
        state.applied = draft.updated_at;
        if draft.thread_root_id.is_some() {
            state.channel = Some(draft.channel_id);
        }
        let changed = self.store(&key, &draft.text);
        self.save();
        changed.then_some((key, draft.text))
    }

    /// `GET /api/drafts`: each applies as [`apply`](Self::apply) does, then a
    /// non-empty device draft the server has no entry for saves now, so text
    /// typed before sync or offline reaches the other devices.
    pub fn loaded(&mut self, drafts: Vec<ServerDraft>) -> Vec<(String, String)> {
        let known = drafts.iter().map(ServerDraft::key).collect::<BTreeSet<_>>();
        let changed = drafts
            .into_iter()
            .filter_map(|draft| self.apply(draft))
            .collect();
        let device_only = self
            .texts
            .keys()
            .filter(|key| !known.contains(*key) && self.target(key).is_some())
            .cloned()
            .collect::<Vec<_>>();
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
            updated_at: format!("2026-11-27T18:04:{at}.000000Z"),
        }
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
            }]
        );
        assert_eq!(
            saves[0].body(),
            serde_json::json!({"channelId": "general", "threadRootId": null, "text": "hel"})
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
            drafts.apply(server("general", "newer", "08")),
            Some(("general".into(), "newer".into()))
        );
    }

    #[test]
    fn older_or_repeated_server_drafts_are_ignored() {
        let mut drafts = SavedDrafts::default();
        assert_eq!(
            drafts.apply(server("general", "second", "02")),
            Some(("general".into(), "second".into()))
        );
        assert_eq!(drafts.apply(server("general", "first", "01")), None);
        assert_eq!(drafts.apply(server("general", "second", "02")), None);
        assert_eq!(drafts.get("general"), "second");
        // A cleared draft clears the device copy; a thread reply finds its key.
        assert_eq!(
            drafts.apply(server("general", "", "03")),
            Some(("general".into(), String::new()))
        );
        assert_eq!(drafts.get("general"), "");
        assert_eq!(
            drafts.apply(server("general/root1", "reply", "04")),
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
        assert_eq!(changed, [("design".into(), "from the phone".into())]);
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
}
