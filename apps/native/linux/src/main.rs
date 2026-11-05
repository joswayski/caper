//! Caper's native Linux client, in GTK 4. In development: it is not part of
//! any release or package yet.

mod app;
mod content;
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
mod rows;
mod ui;
mod viewer;

use gtk::{gio, glib, prelude::*};

fn main() -> glib::ExitCode {
    fonts::prepare();
    let args: Vec<String> = std::env::args().collect();
    let base = args
        .windows(2)
        .find(|pair| pair[0] == "--api-url")
        .map(|pair| pair[1].clone())
        .or_else(|| std::env::var("CAPER_API_URL").ok())
        .unwrap_or_else(|| "https://caper.chat".into());
    let application = gtk::Application::builder()
        .application_id("chat.caper.linux")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application.connect_activate(move |application| app::start(application, &base));
    // GTK must not read Caper's own arguments.
    application.run_with_args::<&str>(&[])
}
