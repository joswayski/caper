//! Preferences kept between runs, as the egui desktop keeps them in its
//! storage: one JSON object in `$XDG_CONFIG_HOME/caper/linux.json`, written
//! whenever a value changes. A missing or unreadable file means defaults.

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::path::PathBuf;

pub struct Prefs {
    path: Option<PathBuf>,
    values: RefCell<Map<String, Value>>,
}

impl Prefs {
    pub fn load() -> Self {
        let path = Some(
            gtk::glib::user_config_dir()
                .join("caper")
                .join("linux.json"),
        );
        let values = path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            values: RefCell::new(values),
        }
    }

    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.values
            .borrow()
            .get(key)
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }

    pub fn set(&self, key: &str, value: impl Serialize) {
        let Ok(value) = serde_json::to_value(value) else {
            return;
        };
        if self.values.borrow().get(key) == Some(&value) {
            return;
        }
        self.values.borrow_mut().insert(key.to_owned(), value);
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let Ok(json) = serde_json::to_vec_pretty(&*self.values.borrow()) else {
            return;
        };
        // Write beside it, then rename: a crash never leaves half a file.
        let temporary = path.with_extension("json.tmp");
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&temporary, json))
            .and_then(|()| std::fs::rename(&temporary, path));
        if let Err(error) = written {
            eprintln!("Could not save preferences: {error}");
        }
    }
}

/// The window's restore size and mode, as the egui desktop keeps them. GTK
/// on Wayland can't place windows, so the position stays the system's.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Window {
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
    pub fullscreen: bool,
}

impl Default for Window {
    /// The desktop's first run: maximized over a 1440×900 restore size.
    fn default() -> Self {
        Self {
            width: 1440,
            height: 900,
            maximized: true,
            fullscreen: false,
        }
    }
}

const WINDOW: &str = "window-v1";

impl Prefs {
    /// Restores the saved size and mode, then keeps them as they change.
    pub fn follow_window(self: &std::rc::Rc<Self>, window: &gtk::ApplicationWindow) {
        use gtk::prelude::*;
        let saved = self
            .get::<Window>(WINDOW)
            .filter(|saved| {
                (360..=16384).contains(&saved.width) && (360..=16384).contains(&saved.height)
            })
            .unwrap_or_default();
        window.set_default_size(saved.width, saved.height);
        if saved.fullscreen {
            window.fullscreen();
        } else if saved.maximized {
            window.maximize();
        }
        let save = {
            let prefs = self.clone();
            move |window: &gtk::ApplicationWindow| {
                let mut next = prefs.get::<Window>(WINDOW).unwrap_or(saved);
                next.maximized = window.is_maximized();
                next.fullscreen = window.is_fullscreen();
                // Only the normal size is a restore size.
                if !next.maximized && !next.fullscreen {
                    let (width, height) = window.default_size();
                    if width > 0 && height > 0 {
                        next.width = width;
                        next.height = height;
                    }
                }
                prefs.set(WINDOW, next);
            }
        };
        for property in ["maximized", "fullscreened"] {
            let save = save.clone();
            window.connect_notify_local(Some(property), move |window, _| save(window));
        }
        window.connect_close_request(move |window| {
            save(window);
            gtk::glib::Propagation::Proceed
        });
    }
}
