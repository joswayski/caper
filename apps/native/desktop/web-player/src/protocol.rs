//! The line protocol between Caper and the `caper-web-player` helper: one
//! command or event per line on the helper's stdin and stdout. Lines that do
//! not parse are ignored on both sides, so stray library output on stdout
//! can never be mistaken for an event.

use std::fmt;

/// Where the player sits in Caper's window, in physical pixels from the
/// top-left of its client area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Bounds {
    /// `x,y,width,height`, as on the helper's command line.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split(',');
        let bounds = Self {
            x: parts.next()?.parse().ok()?,
            y: parts.next()?.parse().ok()?,
            width: parts.next()?.parse().ok()?,
            height: parts.next()?.parse().ok()?,
        };
        (parts.next().is_none() && bounds.width > 0 && bounds.height > 0).then_some(bounds)
    }
}

impl fmt::Display for Bounds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{},{},{}", self.x, self.y, self.width, self.height)
    }
}

/// Caper → helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Move and resize the embedded player.
    Bounds(Bounds),
    Show,
    /// Hide it, still playing (a menu or dialog is over it, or it scrolled
    /// partly out of view).
    Hide,
    /// Bring the separate player window forward (Wayland).
    Raise,
    /// Stop playing and exit.
    Close,
}

impl Command {
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim_end_matches(['\r', '\n']);
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        Some(match (verb, rest) {
            ("bounds", bounds) => Self::Bounds(Bounds::parse(bounds)?),
            ("show", "") => Self::Show,
            ("hide", "") => Self::Hide,
            ("raise", "") => Self::Raise,
            ("close", "") => Self::Close,
            _ => return None,
        })
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bounds(bounds) => write!(f, "bounds {bounds}"),
            Self::Show => f.write_str("show"),
            Self::Hide => f.write_str("hide"),
            Self::Raise => f.write_str("raise"),
            Self::Close => f.write_str("close"),
        }
    }
}

/// Helper → Caper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The web view exists and is loading the play page.
    Ready,
    /// The person followed a link out of the player: Caper opens it in the
    /// system browser (http(s) only, checked again on receipt).
    Open(String),
    /// The web view could not be created or the page failed.
    Error(String),
    /// The person closed the separate player window.
    Closed,
}

impl Event {
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim_end_matches(['\r', '\n']);
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        Some(match (verb, rest) {
            ("ready", "") => Self::Ready,
            ("closed", "") => Self::Closed,
            ("open", url) if crate::openable(url) => Self::Open(url.to_owned()),
            ("error", text) if !text.is_empty() => Self::Error(text.to_owned()),
            _ => return None,
        })
    }
}

/// One line, whatever the text holds.
fn single_line(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready => f.write_str("ready"),
            Self::Closed => f.write_str("closed"),
            Self::Open(url) => write!(f, "open {}", single_line(url)),
            Self::Error(text) => write!(f, "error {}", single_line(text)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip_and_reject_anything_else() {
        let bounds = Bounds {
            x: -4,
            y: 120,
            width: 400,
            height: 225,
        };
        for command in [
            Command::Bounds(bounds),
            Command::Show,
            Command::Hide,
            Command::Raise,
            Command::Close,
        ] {
            let line = format!("{command}\n");
            assert_eq!(Command::parse(&line), Some(command), "{line:?}");
        }
        assert_eq!(
            Command::parse("bounds 10,20,30,40\r\n"),
            Some(Command::Bounds(Bounds {
                x: 10,
                y: 20,
                width: 30,
                height: 40
            }))
        );
        for bad in [
            "",
            "show now",
            "SHOW",
            "bounds",
            "bounds 1,2,3",
            "bounds 1,2,3,4,5",
            "bounds 1,2,0,4",
            "bounds 1,2,-3,4",
            "bounds a,b,c,d",
            "open https://example.com",
        ] {
            assert_eq!(Command::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn events_round_trip_stay_on_one_line_and_open_only_web_links() {
        for event in [
            Event::Ready,
            Event::Closed,
            Event::Open("https://www.youtube.com/watch?v=a".into()),
            Event::Error("WebKitGTK failed".into()),
        ] {
            let line = format!("{event}\n");
            assert_eq!(Event::parse(&line), Some(event), "{line:?}");
        }
        let split = Event::Error("first\nopen https://evil.example".into()).to_string();
        assert!(!split.contains('\n'));
        assert_eq!(
            Event::parse(&split),
            Some(Event::Error("first open https://evil.example".into()))
        );
        for bad in [
            "",
            "ready now",
            "open javascript:alert(1)",
            "open file:///etc/passwd",
            "open ",
            "error",
            "error ",
            "(WebKitWebProcess:12): GLib-WARNING **: something",
            "CONSOLE LOG ready",
        ] {
            assert_eq!(Event::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn bounds_parse_from_the_command_line() {
        assert_eq!(
            Bounds::parse("0,0,640,360"),
            Some(Bounds {
                x: 0,
                y: 0,
                width: 640,
                height: 360
            })
        );
        assert_eq!(Bounds::parse("0,0,640"), None);
        assert_eq!(Bounds::parse(" 0,0,640,360"), None);
    }
}
