//! Self-updates through `caper-updater` (apps/native/updater), which ships
//! beside the app in release packages.
//!
//! A background thread asks the updater shortly after launch and every hour
//! whether a newer signed release exists. When the user accepts, the app
//! starts `caper-updater apply` and quits; the updater swaps the new version in
//! and reopens Caper. Development builds carry no build number and never check.

use eframe::egui;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The installer for a copy that cannot update in place, e.g. the Linux .deb.
pub const DOWNLOAD_URL: &str = if cfg!(windows) {
    "https://github.com/joswayski/caper/releases/download/native-latest/Caper-Windows-x64-Setup.exe"
} else {
    "https://github.com/joswayski/caper/releases/download/native-latest/Caper-Linux-x64.deb"
};
const FIRST_CHECK: Duration = Duration::from_secs(20);
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Available {
    pub version: String,
    pub notes: String,
    /// False when the updater cannot replace this install, e.g. the .deb.
    pub can_apply: bool,
}

#[derive(Deserialize)]
struct CheckOutput {
    update: bool,
    #[serde(default)]
    version: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    can_apply: bool,
}

pub struct Updates {
    install: Option<Install>,
    available: Arc<Mutex<Option<Available>>>,
    pub dismissed: bool,
    pub error: Option<String>,
}

#[derive(Clone)]
struct Install {
    build: u64,
    updater: PathBuf,
    folder: PathBuf,
}

impl Updates {
    /// Starts background checks when this is a release build with an updater.
    pub fn start(context: &egui::Context, enabled: bool) -> Self {
        let install = enabled.then(locate).flatten();
        let available = Arc::new(Mutex::new(None));
        if let Some(install) = install.clone() {
            let available = Arc::clone(&available);
            let context = context.clone();
            let _ = std::thread::Builder::new()
                .name("caper-update-check".into())
                .spawn(move || {
                    std::thread::sleep(FIRST_CHECK);
                    loop {
                        if let Ok(found) = check(&install) {
                            let changed = {
                                let mut current = available.lock().unwrap();
                                let changed = *current != found;
                                *current = found;
                                changed
                            };
                            if changed {
                                context.request_repaint();
                            }
                        }
                        std::thread::sleep(CHECK_EVERY);
                    }
                });
        }
        Self {
            install,
            available,
            dismissed: false,
            error: None,
        }
    }

    /// The banner as a release would show it, for the `parity-update` fixture.
    pub fn preview(available: Available) -> Self {
        Self {
            install: None,
            available: Arc::new(Mutex::new(Some(available))),
            dismissed: false,
            error: None,
        }
    }

    pub fn available(&self) -> Option<Available> {
        if self.dismissed {
            return None;
        }
        self.available.lock().unwrap().clone()
    }

    /// Starts the updater, which waits for this process to exit. The caller
    /// closes the window once this succeeds.
    pub fn apply(&mut self) -> bool {
        let Some(install) = &self.install else {
            return false;
        };
        let mut command = Command::new(&install.updater);
        command
            .arg("apply")
            .arg("--current-build")
            .arg(install.build.to_string())
            .arg("--install")
            .arg(&install.folder)
            .arg("--wait-pid")
            .arg(std::process::id().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console(&mut command);
        match command.spawn() {
            Ok(_) => true,
            Err(error) => {
                self.error = Some(format!("Could not start the update: {error}"));
                false
            }
        }
    }
}

fn build_number() -> Option<u64> {
    option_env!("CAPER_BUILD_NUMBER")
        .and_then(|build| build.parse().ok())
        .filter(|build| *build > 0)
}

fn locate() -> Option<Install> {
    let build = build_number()?;
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let folder = exe.parent()?.to_path_buf();
    let name = if cfg!(windows) {
        "caper-updater.exe"
    } else {
        "caper-updater"
    };
    let updater = [
        folder.join(name),
        // The .deb keeps it beside its libraries; it can only check there.
        Path::new("/usr/lib/caper-desktop").join(name),
    ]
    .into_iter()
    .find(|path| path.is_file())?;
    Some(Install {
        build,
        updater,
        folder,
    })
}

fn check(install: &Install) -> Result<Option<Available>, String> {
    let mut command = Command::new(&install.updater);
    command
        .arg("check")
        .arg("--current-build")
        .arg(install.build.to_string())
        .arg("--install")
        .arg(&install.folder)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut command);
    let output = command.output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("update check failed with {}", output.status));
    }
    parse(&output.stdout)
}

fn parse(stdout: &[u8]) -> Result<Option<Available>, String> {
    let line = stdout
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let output: CheckOutput = serde_json::from_slice(line).map_err(|error| error.to_string())?;
    Ok(output.update.then_some(Available {
        version: output.version,
        notes: output.notes,
        can_apply: output.can_apply,
    }))
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_updater_check_line() {
        assert_eq!(parse(b"{\"update\":false}\n").unwrap(), None);
        assert_eq!(
            parse(
                b"{\"update\":true,\"build\":7,\"version\":\"0.1.7\",\"notes\":\"Screen sharing\",\"can_apply\":true}\n"
            )
            .unwrap(),
            Some(Available {
                version: "0.1.7".into(),
                notes: "Screen sharing".into(),
                can_apply: true,
            })
        );
        // Without --install permission details the app offers a direct download.
        assert!(
            !parse(b"{\"update\":true,\"version\":\"0.1.7\"}")
                .unwrap()
                .unwrap()
                .can_apply
        );
        assert!(parse(b"not json").is_err());
    }
}
