//! Device choices, carrying over egui's values until Win32 saves its own.
//! Fixture connections pass no directory and never read or write installed settings.

use std::collections::HashMap;
use std::path::PathBuf;

pub struct Preferences {
    path: Option<PathBuf>,
    values: HashMap<String, String>,
}

impl Preferences {
    pub fn load(appdata: Option<PathBuf>) -> Self {
        let path = appdata
            .as_ref()
            .map(|p| p.join("Caper/windows/preferences-v1.json"));
        let values = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .or_else(|| {
                let old = std::fs::read_to_string(appdata?.join("Caper/data/app.ron")).ok()?;
                let mut values: HashMap<String, String> = ron::from_str(&old).ok()?;
                values.retain(|key, _| {
                    matches!(key.as_str(), "sound-effects-v1" | "voice-send-on-stop-v1")
                        || key.starts_with("whats-new-v1:")
                });
                Some(values)
            })
            .unwrap_or_default();
        Self { path, values }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn sounds(&self) -> bool {
        self.get("sound-effects-v1")
            .and_then(|s| s.parse().ok())
            .unwrap_or(true)
    }

    pub fn voice_send_on_stop(&self) -> bool {
        self.get("voice-send-on-stop-v1")
            .and_then(|s| s.parse().ok())
            .unwrap_or(false)
    }

    pub fn theme_preference(&self) -> &str {
        self.get(caper_client::themes::THEME_PREF_KEY)
            .unwrap_or("obsidian")
    }

    pub fn set(&mut self, key: &str, value: String) {
        if self.get(key) == Some(&value) {
            return;
        }
        self.values.insert(key.into(), value);
        let Some(path) = &self.path else { return };
        let temporary = path.with_extension("json.tmp");
        let result = serde_json::to_vec(&self.values)
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                std::fs::create_dir_all(path.parent().unwrap())?;
                std::fs::write(&temporary, bytes)?;
                std::fs::rename(&temporary, path)
            });
        if let Err(error) = result {
            eprintln!("Could not save preferences: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_migrate_and_survive_reopening_without_changing_egui() {
        let root = std::env::temp_dir().join(format!("caper-prefs-{}", uuid::Uuid::new_v4()));
        let old = root.join("Caper/data/app.ron");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        let source = r#"{"sound-effects-v1":"false","voice-send-on-stop-v1":"true","whats-new-v1:https://caper.chat:windows":"7"}"#;
        std::fs::write(&old, source).unwrap();
        let mut prefs = Preferences::load(Some(root.clone()));
        assert!(!prefs.sounds());
        assert!(prefs.voice_send_on_stop());
        prefs.set("sound-effects-v1", "true".into());
        let restored = Preferences::load(Some(root.clone()));
        assert!(restored.sounds());
        assert!(restored.voice_send_on_stop());
        assert_eq!(
            restored.get("whats-new-v1:https://caper.chat:windows"),
            Some("7")
        );
        assert_eq!(std::fs::read_to_string(old).unwrap(), source);
        let isolated = Preferences::load(None);
        assert!(isolated.sounds());
        assert!(!isolated.voice_send_on_stop());
        std::fs::remove_dir_all(root).unwrap();
    }
}
