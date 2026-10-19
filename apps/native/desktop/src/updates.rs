//! Self-updates through `caper-updater` (apps/native/updater), which ships
//! beside the app in release packages.
//!
//! A background thread asks the updater shortly after launch and every minute
//! whether a newer signed release exists. When the user accepts, the app
//! starts `caper-updater apply` and quits; the updater swaps the new version in
//! and reopens Caper. Development builds carry no build number and never check.

use eframe::egui;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

/// The installer for a copy that cannot update in place, e.g. the Linux .deb.
pub const DOWNLOAD_URL: &str = if cfg!(windows) {
    "https://github.com/joswayski/caper/releases/download/native-latest/Caper-Windows-x64-Setup.exe"
} else {
    "https://github.com/joswayski/caper/releases/download/native-latest/Caper-Linux-x64.deb"
};
const FIRST_CHECK: Duration = Duration::from_secs(20);
const CHECK_EVERY: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Available {
    pub version: String,
    pub notes: String,
    pub changelog: Vec<ChangelogEntry>,
    pub history_complete: bool,
    /// Every release being installed, stacked newest first. `None` when the
    /// highlights could not be read, so the plain `notes` explain why.
    pub highlights: Option<Vec<Highlight>>,
    /// The newest highlighted release's date (YYYY-MM-DD).
    pub released: Option<String>,
    /// The update's source commit, linked from its version.
    pub revision: Option<String>,
    /// The update's highlights sequence, marked read when it is installed.
    pub sequence: Option<u64>,
    pub last_error: Option<String>,
    /// False when the updater cannot replace this install, e.g. the .deb.
    pub can_apply: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Highlight {
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ChangelogEntry {
    pub version: String,
    pub notes: String,
}

#[derive(Deserialize)]
struct CheckOutput {
    update: bool,
    #[serde(default)]
    deferred: bool,
    #[serde(default)]
    version: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    changelog: Vec<ChangelogEntry>,
    #[serde(default)]
    history_complete: bool,
    #[serde(default)]
    highlights: Option<Vec<Highlight>>,
    #[serde(default)]
    released: Option<String>,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    sequence: Option<u64>,
    #[serde(default)]
    last_error: Option<String>,
    #[serde(default)]
    can_apply: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available,
    Deferred,
    Failed(String),
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Checked(Option<Available>),
    Deferred,
}

#[derive(Default)]
struct State {
    available: Option<Available>,
    status: Status,
    reviewing: bool,
    show_after_check: bool,
}

impl State {
    fn finish(&mut self, result: Result<Option<Available>, String>) {
        match result {
            Ok(found) => {
                self.status = if found.is_some() {
                    Status::Available
                } else {
                    Status::UpToDate
                };
                self.reviewing = found.is_some() && (self.reviewing || self.show_after_check);
                self.available = found;
            }
            Err(error) => self.status = Status::Failed(error),
        }
        self.show_after_check = false;
    }
}

pub struct Updates {
    install: Option<Install>,
    state: Arc<Mutex<State>>,
    requests: Option<mpsc::Sender<()>>,
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
        Self::with_install(context, enabled.then(locate).flatten())
    }

    fn with_install(context: &egui::Context, install: Option<Install>) -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let mut requests = None;
        if let Some(install) = install.clone() {
            let state = Arc::clone(&state);
            let context = context.clone();
            let (sender, receiver) = mpsc::channel();
            if std::thread::Builder::new()
                .name("caper-update-check".into())
                .spawn(move || {
                    let mut delay = FIRST_CHECK;
                    while let Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) =
                        receiver.recv_timeout(delay)
                    {
                        state.lock().unwrap().status = Status::Checking;
                        context.request_repaint();
                        let result = check(&install);
                        let mut state = state.lock().unwrap();
                        match result {
                            Ok(Outcome::Deferred) => {
                                state.status = Status::Deferred;
                                state.show_after_check = false;
                            }
                            Ok(Outcome::Checked(found)) => state.finish(Ok(found)),
                            Err(error) => state.finish(Err(error)),
                        }
                        context.request_repaint();
                        delay = CHECK_EVERY;
                    }
                })
                .is_ok()
            {
                requests = Some(sender);
            }
        }
        Self {
            install,
            state,
            requests,
            error: None,
        }
    }

    /// The notice as a release would show it, for the `parity-update` fixture.
    pub fn preview(available: Available) -> Self {
        Self {
            install: None,
            state: Arc::new(Mutex::new(State {
                available: Some(available),
                status: Status::Available,
                reviewing: true,
                show_after_check: false,
            })),
            requests: None,
            error: None,
        }
    }

    /// Static check feedback for Settings fixtures; never runs the updater.
    pub fn preview_status(status: Status) -> Self {
        let updates = Self::with_install(&egui::Context::default(), None);
        updates.state.lock().unwrap().status = status;
        updates
    }

    pub fn available(&self) -> Option<Available> {
        let state = self.state.lock().unwrap();
        state.available.clone().filter(|_| state.reviewing)
    }

    /// Later hides the notice, not the account bar's update action.
    pub fn known_available(&self) -> Option<Available> {
        self.state.lock().unwrap().available.clone()
    }

    pub fn show_available(&mut self) {
        self.state.lock().unwrap().reviewing = true;
    }

    pub fn dismiss(&mut self) {
        self.state.lock().unwrap().reviewing = false;
    }

    pub fn can_check(&self) -> bool {
        self.requests.is_some()
    }

    pub fn status(&self) -> Status {
        self.state.lock().unwrap().status.clone()
    }

    /// Wakes the same worker used by automatic checks, without overlapping them.
    pub fn check_now(&mut self) {
        let Some(requests) = &self.requests else {
            return;
        };
        self.error = None;
        let mut state = self.state.lock().unwrap();
        state.show_after_check = true;
        if state.status != Status::Checking {
            state.status = match requests.send(()) {
                Ok(()) => Status::Checking,
                Err(_) => Status::Failed(
                    "Could not start the update check. Restart Caper to retry.".into(),
                ),
            };
        }
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

/// A build's source on GitHub, or `None` for anything but a full commit SHA.
pub fn commit_url(revision: &str) -> Option<String> {
    (revision.len() == 40
        && revision
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')))
    .then(|| format!("https://github.com/joswayski/caper/commit/{revision}"))
}

/// The installed app's version in What's new, distinct from an available update.
pub fn version_label() -> String {
    build_number().map_or_else(
        || "Development build".into(),
        |build| format!("Installed version 0.1.{build}"),
    )
}

/// The installed app, not the latest release returned by the update server.
pub fn installed_version() -> String {
    let version = match build_number() {
        Some(build) => format!("Caper 0.1.{build} (build {build})"),
        None => "Development build".into(),
    };
    format!("Installed: {version} ·")
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

fn check(install: &Install) -> Result<Outcome, String> {
    let mut command = Command::new(&install.updater);
    command
        .arg("check")
        .arg("--current-build")
        .arg(install.build.to_string())
        .arg("--install")
        .arg(&install.folder)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    if !crate::whats_new::REVISION.is_empty() {
        command
            .arg("--current-revision")
            .arg(crate::whats_new::REVISION);
    }
    hide_console(&mut command);
    let output = command.output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("update check failed with {}", output.status));
    }
    parse(&output.stdout)
}

fn parse(stdout: &[u8]) -> Result<Outcome, String> {
    let line = stdout
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let output: CheckOutput = serde_json::from_slice(line).map_err(|error| error.to_string())?;
    if output.deferred {
        return Ok(Outcome::Deferred);
    }
    Ok(Outcome::Checked(output.update.then_some(Available {
        version: output.version,
        notes: output.notes,
        changelog: output.changelog,
        history_complete: output.history_complete,
        highlights: output.highlights,
        released: output.released,
        revision: output.revision,
        sequence: output.sequence,
        last_error: output.last_error,
        can_apply: output.can_apply,
    })))
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

    fn release(version: &str) -> Available {
        Available {
            version: version.into(),
            notes: "Release notes".into(),
            changelog: Vec::new(),
            history_complete: false,
            highlights: None,
            released: None,
            revision: None,
            sequence: None,
            last_error: None,
            can_apply: true,
        }
    }

    #[test]
    fn checks_every_minute_with_the_existing_startup_delay() {
        assert_eq!(FIRST_CHECK.as_secs(), 20);
        assert_eq!(CHECK_EVERY.as_secs(), 60);
    }

    #[test]
    fn manual_checks_coalesce_and_restore_a_dismissed_update() {
        let mut updates = Updates::preview(release("0.1.18"));
        let (sender, receiver) = mpsc::channel();
        updates.requests = Some(sender);
        updates.error = Some("Previous install failed".into());
        updates.dismiss();
        assert!(updates.available().is_none());

        updates.check_now();
        assert!(
            updates.available().is_none(),
            "manual review waits for completion"
        );
        assert_eq!(updates.status(), Status::Checking);
        assert!(updates.error.is_none());
        receiver.try_recv().unwrap();
        updates.check_now();
        assert_eq!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty));

        updates
            .state
            .lock()
            .unwrap()
            .finish(Ok(Some(release("0.1.19"))));
        assert_eq!(updates.available(), Some(release("0.1.19")));
        updates.state.lock().unwrap().finish(Ok(None));
        assert_eq!(updates.status(), Status::UpToDate);
        assert!(updates.available().is_none());
        updates.check_now();
        receiver.try_recv().unwrap();
        updates.state.lock().unwrap().finish(Err("Offline".into()));
        assert_eq!(updates.status(), Status::Failed("Offline".into()));
        updates.check_now();
        receiver.try_recv().unwrap();
        assert_eq!(updates.status(), Status::Checking);
    }

    #[test]
    fn automatic_checks_advertise_quietly_and_failed_checks_keep_known_updates() {
        let mut updates = Updates::preview(release("0.1.18"));
        updates.dismiss();
        assert_eq!(updates.known_available(), Some(release("0.1.18")));
        updates.show_available();
        assert_eq!(updates.available(), Some(release("0.1.18")));
        updates.dismiss();
        updates
            .state
            .lock()
            .unwrap()
            .finish(Ok(Some(release("0.1.18"))));
        assert!(updates.available().is_none());
        updates
            .state
            .lock()
            .unwrap()
            .finish(Ok(Some(release("0.1.19"))));
        assert!(updates.available().is_none());
        assert_eq!(updates.known_available(), Some(release("0.1.19")));
        updates.state.lock().unwrap().finish(Err("Offline".into()));
        assert!(updates.available().is_none());
        assert_eq!(updates.known_available(), Some(release("0.1.19")));
        assert_eq!(updates.status(), Status::Failed("Offline".into()));
    }

    #[test]
    fn a_manual_check_wakes_the_worker_without_waiting_twenty_seconds() {
        let context = egui::Context::default();
        let mut updates = Updates::with_install(
            &context,
            Some(Install {
                build: 18,
                updater: std::env::temp_dir().join(uuid::Uuid::new_v4().to_string()),
                folder: std::env::temp_dir(),
            }),
        );
        assert!(updates.can_check());
        assert_eq!(updates.status(), Status::Idle);
        updates.check_now();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while updates.status() == Status::Checking {
            assert!(
                std::time::Instant::now() < deadline,
                "manual check stayed asleep"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(matches!(updates.status(), Status::Failed(_)));
    }

    #[test]
    #[cfg(unix)]
    fn deferred_checks_keep_known_updates() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::Instant;
        let folder = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&folder).unwrap();
        let updater = folder.join("updater");
        std::fs::write(
            &updater,
            "#!/bin/sh\nprintf '%s\\n' '{\"update\":false,\"deferred\":true}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&updater, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut updates = Updates::with_install(
            &egui::Context::default(),
            Some(Install {
                build: 18,
                updater,
                folder: folder.clone(),
            }),
        );
        {
            let mut state = updates.state.lock().unwrap();
            state.finish(Ok(Some(release("0.1.19"))));
        }
        updates.check_now();
        let deadline = Instant::now() + Duration::from_secs(3);
        while updates.status() == Status::Checking {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(updates.status(), Status::Deferred);
        assert_eq!(updates.known_available(), Some(release("0.1.19")));
        assert!(
            updates.available().is_none(),
            "throttling must not open a review"
        );
        updates.state.lock().unwrap().finish(Err("Offline".into()));
        assert_eq!(updates.known_available(), Some(release("0.1.19")));
        updates.state.lock().unwrap().finish(Ok(None));
        assert!(updates.known_available().is_none());
        drop(updates);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn development_builds_and_fixtures_never_start_manual_checks() {
        let mut updates = Updates::start(&egui::Context::default(), false);
        assert!(!updates.can_check());
        updates.check_now();
        assert_eq!(updates.status(), Status::Idle);
        let mut preview = Updates::preview(release("0.1.18"));
        assert!(!preview.can_check());
        preview.check_now();
        assert_eq!(preview.status(), Status::Available);
    }

    #[test]
    fn account_menu_check_reports_progress_success_and_retryable_failure_in_place() {
        let context = egui::Context::default();
        context.enable_accesskit();
        let mut app = crate::CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.updates = Updates::preview(release("0.1.98"));
        app.updates.dismiss();
        let (sender, receiver) = mpsc::channel();
        app.updates.requests = Some(sender);
        let frame = |app: &mut crate::CaperApp, events| {
            context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |context| app.page(context),
            )
        };
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        let bounds = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("User Settings"))
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let settings = egui::pos2(
            ((bounds.x0 + bounds.x1) / 2.0) as f32,
            ((bounds.y0 + bounds.y1) / 2.0) as f32,
        );
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(settings),
                    egui::Event::PointerButton {
                        pos: settings,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.job.text == "Check for updates"
                {
                    Some(text.pos + text.galley.size() * 0.5)
                } else {
                    None
                }
            })
            .expect("manual check is directly in the account menu");
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        receiver
            .try_recv()
            .expect("the button wakes the update worker");
        assert!(app.dialog.is_none(), "checking must not open Settings");
        let checking = frame(&mut app, vec![]);
        assert!(
            egui::Popup::is_any_open(&context),
            "keep the account menu open"
        );
        let button = checking
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Check for updates"))
            .unwrap();
        assert!(button.1.is_disabled(), "a second check must not overlap");
        let checking_bounds = button.1.bounds().unwrap();
        for (result, expected) in [
            (Ok(None), "No newer published release is available."),
            (
                Err("Offline".into()),
                "Could not check for updates: Offline",
            ),
        ] {
            app.updates.state.lock().unwrap().finish(result);
            frame(&mut app, vec![]);
            let output = frame(&mut app, vec![]);
            assert!(
                egui::Popup::is_any_open(&context),
                "removing an outdated update notice must not close the menu"
            );
            assert!(
                output.shapes.iter().any(|shape| matches!(
                    &shape.shape, egui::Shape::Text(text) if text.galley.job.text == expected
                )),
                "missing check feedback: {expected}"
            );
            let button = output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some("Check for updates"))
                .unwrap();
            assert!(
                !button.1.is_disabled(),
                "success and failure allow another check"
            );
            let bounds = button.1.bounds().unwrap();
            assert_eq!(
                bounds, checking_bounds,
                "a fast check must not move the action"
            );
            let pos = egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.0) as f32,
                ((bounds.y0 + bounds.y1) / 2.0) as f32,
            );
            for pressed in [true, false] {
                frame(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            receiver.try_recv().expect("retry wakes the update worker");
            assert!(app.dialog.is_none());
            assert!(egui::Popup::is_any_open(&context));
        }
        app.updates
            .state
            .lock()
            .unwrap()
            .finish(Ok(Some(release("0.1.99"))));
        frame(&mut app, vec![]);
        assert_eq!(app.updates.available().unwrap().version, "0.1.99");
        assert!(
            !egui::Popup::is_any_open(&context),
            "only a found update leaves the menu for review"
        );
        assert!(app.dialog.is_none());
    }

    #[test]
    fn parses_the_updater_check_line() {
        assert_eq!(
            parse(b"{\"update\":false}\n").unwrap(),
            Outcome::Checked(None)
        );
        assert_eq!(
            parse(br#"{"update":false,"deferred":true}"#).unwrap(),
            Outcome::Deferred
        );
        assert_eq!(
            parse(
                b"{\"update\":true,\"build\":7,\"version\":\"0.1.7\",\"notes\":\"Screen sharing\",\"can_apply\":true}\n"
            )
            .unwrap(),
            Outcome::Checked(Some(Available {
                version: "0.1.7".into(),
                notes: "Screen sharing".into(),
                changelog: Vec::new(),
                history_complete: false,
                highlights: None,
                released: None,
                revision: None,
                sequence: None,
                last_error: None,
                can_apply: true,
            }))
        );
        // Without --install permission details the app offers a direct download.
        assert!(
            !parsed_update(b"{\"update\":true,\"version\":\"0.1.7\"}")
                .unwrap()
                .can_apply
        );
        assert!(parse(b"not json").is_err());
        let failed =
            parsed_update(br#"{"update":true,"version":"0.1.7","last_error":"Access denied"}"#)
                .unwrap();
        assert_eq!(failed.last_error.as_deref(), Some("Access denied"));
    }

    fn parsed_update(bytes: &[u8]) -> Option<Available> {
        let Outcome::Checked(found) = parse(bytes).unwrap() else {
            panic!("expected a completed check")
        };
        found
    }

    #[test]
    fn parses_stacked_notes_and_the_explicit_history_gap() {
        let output = br#"{"update":true,"version":"0.1.42","notes":"Latest only","changelog":[{"build":42,"version":"0.1.42","notes":"Newest"},{"build":39,"version":"0.1.39","notes":"Skipped"}],"history_complete":true,"can_apply":true}"#;
        let available = parsed_update(output).unwrap();
        assert!(available.history_complete);
        assert_eq!(available.changelog.len(), 2);
        assert_eq!(available.changelog[1].notes, "Skipped");
        assert_eq!(available.changelog[1].version, "0.1.39");
        assert_eq!(available.highlights, None);
    }

    #[test]
    fn parses_structured_highlights_and_their_date() {
        let output = br#"{"update":true,"version":"0.1.82","notes":"plain","history_complete":true,"highlights":[{"title":"Voice in the channel list","body":"Hop in with one click."}],"released":"2026-10-18","revision":"377205cd7c68e109b0d77f31a97c2cb3b2d40c39","sequence":4,"can_apply":true}"#;
        let available = parsed_update(output).unwrap();
        assert_eq!(
            available.highlights,
            Some(vec![Highlight {
                title: "Voice in the channel list".into(),
                body: "Hop in with one click.".into(),
            }])
        );
        assert_eq!(available.released.as_deref(), Some("2026-10-18"));
        assert_eq!(
            available.revision.as_deref(),
            Some("377205cd7c68e109b0d77f31a97c2cb3b2d40c39")
        );
        assert_eq!(available.sequence, Some(4));
    }
}
