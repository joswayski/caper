//! Link preview players (YouTube, Vimeo, Twitch), as egui plays them on
//! Wayland: Play starts the `caper-web-player` helper (WebKitGTK, with
//! Caper's player safety rules, `caper_web_player::engine`) showing only
//! Caper's `play` page in its own small window. One plays at a time; it
//! closes on another Play, a conversation change, or quit (the helper exits
//! when its stdin closes). Links followed out of the player open in the
//! system browser. Without the helper or WebKitGTK the card's link opens in
//! the browser instead.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};

use caper_web_player::protocol::{self, Event};

/// What the helper reported.
pub enum Notice {
    /// A link followed out of the player, for the system browser.
    Open(String),
    /// It closed its window, or exited after playing.
    Gone,
    /// It never got going (no WebKitGTK, or the page failed).
    Failed,
}

enum Line {
    Event(Event),
    End,
}

pub struct Player {
    /// The `play` URL it shows, so Play on the same card raises it.
    pub play: String,
    /// What the card opens when the player cannot.
    pub fallback: String,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    lines: Receiver<Line>,
    ready: bool,
}

/// `CAPER_WEB_PLAYER`, else beside the app, else the .deb's library folder
/// (as egui, the updater and FFmpeg).
fn helper() -> Option<PathBuf> {
    let name = "caper-web-player";
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.canonicalize().ok())
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)));
    std::env::var_os("CAPER_WEB_PLAYER")
        .map(PathBuf::from)
        .into_iter()
        .chain(beside)
        .chain([PathBuf::from("/usr/lib/caper-desktop").join(name)])
        .find(|path| path.is_file())
}

impl Player {
    /// Starts the helper for `play` (already checked by `play_page`) in a
    /// window of `size` logical pixels. `None` when there is no helper.
    pub fn start(
        play: &url::Url,
        provider: &str,
        size: (i32, i32),
        fallback: String,
        wake: impl Fn() + Send + 'static,
    ) -> Option<Self> {
        let mut child = Command::new(helper()?)
            .arg("--page")
            .arg(play.as_str())
            .arg("--window")
            .arg(format!("{},{}", size.0.max(1), size.1.max(1)))
            .arg("--title")
            .arg(format!(
                "Caper — {}",
                caper_web_player::provider_name(provider)
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let (sender, lines) = mpsc::channel();
        if let Some(stdout) = child.stdout.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if let Some(event) = Event::parse(&line) {
                        let _ = sender.send(Line::Event(event));
                        wake();
                    }
                }
                let _ = sender.send(Line::End);
                wake();
            });
        }
        Some(Self {
            play: play.to_string(),
            fallback,
            stdin: child.stdin.take(),
            child: Some(child),
            lines,
            ready: false,
        })
    }

    pub fn poll(&mut self) -> Vec<Notice> {
        let mut notices = Vec::new();
        while let Ok(line) = self.lines.try_recv() {
            match line {
                Line::Event(Event::Ready) => self.ready = true,
                Line::Event(Event::Open(url)) => notices.push(Notice::Open(url)),
                Line::Event(Event::Error(_)) if !self.ready => notices.push(Notice::Failed),
                Line::Event(Event::Error(_)) => {}
                Line::Event(Event::Closed) => notices.push(Notice::Gone),
                Line::End if self.ready => notices.push(Notice::Gone),
                Line::End => notices.push(Notice::Failed),
            }
        }
        notices
    }

    /// Brings its window forward (Play on the card that is playing).
    pub fn raise(&mut self) {
        if let Some(stdin) = &mut self.stdin
            && writeln!(stdin, "{}", protocol::Command::Raise)
                .and_then(|()| stdin.flush())
                .is_err()
        {
            self.stdin = None;
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = writeln!(stdin, "{}", protocol::Command::Close);
        }
        // Reap it without blocking the app; kill it if it lingers.
        let Some(mut child) = self.child.take() else {
            return;
        };
        std::thread::spawn(move || {
            for _ in 0..20 {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let _ = child.kill();
            let _ = child.wait();
        });
    }
}
