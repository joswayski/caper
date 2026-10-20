//! The Linux helper itself (see `main.rs`).

use std::io::{Read, Write};
use std::process::ExitCode;
use std::rc::Rc;

use caper_web_player::engine::{self, wry};
use caper_web_player::protocol::{Bounds, Command, Event};
use gtk::prelude::*;
use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use wry::WebViewBuilderExtUnix;

enum Mode {
    /// Inside Caper's X11 window.
    Child { parent: u64, bounds: Bounds },
    /// Its own window, in GTK logical pixels.
    Window {
        width: i32,
        height: i32,
        title: String,
    },
}

struct Args {
    page: url::Url,
    mode: Mode,
}

impl Args {
    fn parse(mut args: impl Iterator<Item = String>) -> Option<Self> {
        let (mut page, mut parent, mut bounds, mut window, mut title) =
            (None, None, None, None, None);
        while let Some(flag) = args.next() {
            let value = args.next()?;
            match flag.as_str() {
                "--page" => page = caper_web_player::play_page(&value),
                "--parent" => parent = value.parse::<u64>().ok().filter(|xid| *xid != 0),
                "--bounds" => bounds = Bounds::parse(&value),
                "--window" => {
                    let (width, height) = value.split_once(',')?;
                    window = Some((width.parse::<i32>().ok()?, height.parse::<i32>().ok()?));
                }
                "--title" => title = Some(value),
                _ => return None,
            }
        }
        let mode = match (parent, bounds, window) {
            (Some(parent), Some(bounds), None) => Mode::Child { parent, bounds },
            (None, None, Some((width, height))) if width > 0 && height > 0 => Mode::Window {
                width,
                height,
                title: title.unwrap_or_else(|| "Caper".into()),
            },
            _ => return None,
        };
        Some(Self { page: page?, mode })
    }
}

/// Caper's X11 window, as wry's parent.
struct Parent(u64);

impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Xlib(raw_window_handle::XlibWindowHandle::new(self.0 as _));
        // SAFETY: the XID names Caper's top-level window. Caper closes this
        // program before that window goes away, and wry only uses the XID as
        // the parent of its own container window.
        #[allow(unsafe_code)]
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

/// One event line to Caper; quits when Caper is gone.
fn emit(event: &Event) {
    let mut out = std::io::stdout().lock();
    if writeln!(out, "{event}").and_then(|()| out.flush()).is_err() {
        gtk::main_quit();
    }
}

fn rect(bounds: Bounds) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::PhysicalPosition::new(bounds.x, bounds.y).into(),
        size: wry::dpi::PhysicalSize::new(bounds.width, bounds.height).into(),
    }
}

pub fn run() -> ExitCode {
    let Some(args) = Args::parse(std::env::args().skip(1)) else {
        eprintln!(
            "usage: caper-web-player --page URL (--parent XID --bounds X,Y,W,H | --window W,H [--title TITLE])"
        );
        return ExitCode::from(2);
    };
    // The window and taskbar entry belong to Caper (caper.desktop).
    gtk::glib::set_prgname(Some("caper"));
    if let Err(error) = gtk::init() {
        emit(&Event::Error(format!("GTK could not start: {error}")));
        return ExitCode::FAILURE;
    }
    let open: engine::Open = Rc::new(|url| emit(&Event::Open(url)));
    let builder = engine::builder(&args.page, open.clone());
    let mut window = None;
    let built = match &args.mode {
        Mode::Child { parent, bounds } => builder
            .with_bounds(rect(*bounds))
            .with_visible(false)
            .build_as_child(&Parent(*parent)),
        Mode::Window {
            width,
            height,
            title,
        } => {
            let top = gtk::Window::new(gtk::WindowType::Toplevel);
            top.set_title(title);
            top.set_icon_name(Some("caper"));
            top.set_default_size(*width, *height);
            top.connect_delete_event(|_, _| {
                emit(&Event::Closed);
                gtk::main_quit();
                gtk::glib::Propagation::Proceed
            });
            let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
            top.add(&container);
            let built = builder.build_gtk(&container);
            top.show_all();
            window = Some(top);
            built
        }
    };
    let view = match built {
        Ok(view) => view,
        Err(error) => {
            emit(&Event::Error(format!(
                "The web view could not start: {error}"
            )));
            return ExitCode::FAILURE;
        }
    };
    engine::guard_main_frame(&view, open);
    emit(&Event::Ready);

    let mut pending = Vec::new();
    let mut stdin = std::io::stdin();
    gtk::glib::unix_fd_add_local(
        0,
        gtk::glib::IOCondition::IN | gtk::glib::IOCondition::HUP | gtk::glib::IOCondition::ERR,
        move |_, _| {
            // Larger than stdin's own buffer, so each read is one read(2)
            // of what is waiting and never blocks.
            let mut buffer = [0; 16 * 1024];
            let read = match stdin.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    gtk::main_quit();
                    return gtk::glib::ControlFlow::Break;
                }
                Ok(read) => read,
            };
            pending.extend_from_slice(&buffer[..read]);
            if pending.len() > 64 * 1024 && !pending.contains(&b'\n') {
                // Not the protocol: no command is anywhere near this long.
                pending.clear();
            }
            while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = pending.drain(..=end).collect();
                let Some(command) = std::str::from_utf8(&line).ok().and_then(Command::parse) else {
                    continue;
                };
                let result = match command {
                    // Its own window is always shown, at the size the person chooses.
                    Command::Bounds(_) | Command::Show | Command::Hide if window.is_some() => {
                        Ok(())
                    }
                    Command::Bounds(bounds) => view.set_bounds(rect(bounds)),
                    Command::Show => view.set_visible(true),
                    Command::Hide => view.set_visible(false),
                    Command::Raise => {
                        if let Some(window) = &window {
                            window.present();
                        }
                        Ok(())
                    }
                    Command::Close => {
                        gtk::main_quit();
                        return gtk::glib::ControlFlow::Break;
                    }
                };
                if let Err(error) = result {
                    emit(&Event::Error(error.to_string()));
                }
            }
            gtk::glib::ControlFlow::Continue
        },
    );
    gtk::main();
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Option<Args> {
        Args::parse(list.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn arguments_need_a_caper_page_and_exactly_one_placement() {
        let page = "https://embed.caper.chat/play/youtube/dQw4w9WgXcQ";
        let child = args(&[
            "--page",
            page,
            "--parent",
            "4194311",
            "--bounds",
            "10,20,400,225",
        ]);
        assert!(matches!(
            child.map(|args| args.mode),
            Some(Mode::Child { parent: 4194311, bounds }) if bounds.width == 400
        ));
        let window = args(&[
            "--page",
            page,
            "--window",
            "400,225",
            "--title",
            "Caper — YouTube",
        ]);
        assert!(matches!(
            window.map(|args| args.mode),
            Some(Mode::Window { width: 400, height: 225, ref title }) if title == "Caper — YouTube"
        ));
        for bad in [
            &[
                "--page",
                "https://evil.example/play/youtube/a",
                "--window",
                "400,225",
            ][..],
            &["--window", "400,225"],
            &["--page", page],
            &["--page", page, "--parent", "0", "--bounds", "0,0,1,1"],
            &["--page", page, "--parent", "7"],
            &[
                "--page", page, "--parent", "7", "--bounds", "0,0,1,1", "--window", "1,1",
            ],
            &["--page", page, "--window", "0,225"],
            &["--page", page, "--window"],
            &["--page", page, "--window", "400,225", "--devtools", "1"],
        ] {
            assert!(args(bad).is_none(), "{bad:?}");
        }
    }
}
