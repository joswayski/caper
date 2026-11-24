//! Unsent composer text kept on this device across restarts and updates, as
//! web keeps it in `localStorage`: one JSON file per account, written
//! whenever a draft changes and deleted at logout, so another account on the
//! same device never sees it. Text only; staged files and reply targets stay
//! in memory. Drafts are never logged or sent to the server.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Conversation drafts by channel or DM ID, and thread replies by
/// [`thread_key`]. Without a directory or account it keeps them in memory.
#[derive(Default)]
pub struct SavedDrafts {
    account: String,
    path: Option<PathBuf>,
    texts: BTreeMap<String, String>,
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
        let texts = path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            account: account.to_owned(),
            path,
            texts,
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
    pub fn set(&mut self, key: &str, text: &str) {
        let changed = if text.trim().is_empty() {
            self.texts.remove(key).is_some()
        } else if self.get(key) == text {
            false
        } else {
            self.texts.insert(key.to_owned(), text.to_owned());
            true
        };
        if changed {
            self.save();
        }
    }

    /// Logout: forgets every draft, deletes the account's file and keeps
    /// nothing more until the next account opens its own.
    pub fn erase(&mut self) {
        self.account.clear();
        if let Some(path) = self.path.take()
            && let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("Could not remove saved drafts: {}", error.kind());
        }
        self.texts.clear();
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let result = if self.texts.is_empty() {
            std::fs::remove_file(path).or_else(|error| match error.kind() {
                std::io::ErrorKind::NotFound => Ok(()),
                _ => Err(error),
            })
        } else {
            // Write beside it, then rename: a crash never leaves half a file.
            let temporary = path.with_extension("json.tmp");
            serde_json::to_vec(&self.texts)
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
        first.set(&thread_key("root1"), "a reply");
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
}
