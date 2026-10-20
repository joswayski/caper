//! `caper-web-player`: Caper Desktop's link preview player on Linux, a small
//! separate program so the app itself never links GTK or WebKitGTK and still
//! starts where they are missing. Caper starts it on Play and talks to it
//! over stdin/stdout ([`caper_web_player::protocol`]); it exits on `close`
//! or when Caper goes away (stdin closes).
//!
//! ```text
//! caper-web-player --page URL --parent XID --bounds X,Y,W,H   # X11: inside Caper's window
//! caper-web-player --page URL --window W,H --title TITLE      # Wayland: its own window
//! ```
//!
//! On X11 the web view is a child of Caper's window (wry creates a container
//! window under the XID), so it moves with the window and takes its own
//! clicks. Wayland has no cross-process child surfaces, so there it opens a
//! small window of its own.

#[cfg(target_os = "linux")]
mod helper;

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    helper::run()
}

/// Windows plays in Caper's own process; there is nothing to run here.
#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("caper-web-player is only used on Linux");
    std::process::ExitCode::FAILURE
}
