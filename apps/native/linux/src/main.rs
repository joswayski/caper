//! Caper's native Linux client, in GTK 4. `apps/native/desktop/build.sh`
//! ships it in the Linux release packages as `caper-desktop`.

mod app;
mod composing;
mod content;
mod file_drop;
mod fonts;
#[allow(dead_code)]
#[path = "../../../../shared/messages/formatting.rs"]
mod formatting;
mod host;
mod images;
#[allow(dead_code)]
#[path = "../../../../shared/messages/links.rs"]
mod links;
mod markup;
mod media;
mod people;
mod picker;
mod playback;
mod prefs;
mod rows;
mod suggest;
mod ui;
mod update_shine;
mod uploads;
mod video;
mod viewer;
mod web_player;
mod zoom;

use gtk::{gio, glib, prelude::*};

fn main() -> glib::ExitCode {
    let process_start = std::time::Instant::now();
    fonts::prepare();
    let args: Vec<String> = std::env::args().collect();
    let base = args
        .windows(2)
        .find(|pair| pair[0] == "--api-url")
        .map(|pair| pair[1].clone())
        .or_else(|| std::env::var("CAPER_API_URL").ok())
        .unwrap_or_else(|| "https://caper.chat".into());
    // caper:// invite links (`caper_client::app_link`), as on egui: a click
    // while Caper is open goes to that window instead of a second copy.
    let launch_invite = caper_client::app_link::launch_code(&args);
    if launch_invite
        .as_deref()
        .is_some_and(|code| caper_client::app_link::hand_off_at(&app::link_handoff(), code))
    {
        return glib::ExitCode::SUCCESS;
    }
    if !cfg!(debug_assertions)
        && let Err(error) = caper_client::app_link::register()
    {
        eprintln!("Caper could not register caper:// links: {error}");
    }
    let application = gtk::Application::builder()
        .application_id("chat.caper.linux")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application.connect_activate(move |application| {
        app::start(application, &base, process_start, launch_invite.clone())
    });
    // GTK must not read Caper's own arguments.
    application.run_with_args::<&str>(&[])
}
