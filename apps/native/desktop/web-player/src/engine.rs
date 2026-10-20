//! The one web view configuration every engine gets, matching the Android
//! and Apple players: ephemeral storage, no bridge to app code (no IPC or
//! custom protocols), no developer tools, downloads or clipboard access,
//! autoplay after Play, and only the play page in the main frame. Every
//! other main-frame navigation and every new window is refused and handed to
//! `open` (http(s) only), which opens it in the system browser. The
//! provider's player runs in the page's frames and may navigate freely.
//!
//! Where the main-frame rule is enforced differs by engine, because wry's
//! navigation handler only sees a URL:
//!
//! - WebView2 reports main-frame navigations only (`NavigationStarting`;
//!   frames have their own event), so the handler decides.
//! - WebKitGTK asks the handler about frame navigations too, with no way to
//!   tell them apart, so [`guard_main_frame`] stops any main-frame load after
//!   the play page instead, before it commits.

use std::rc::Rc;

pub use wry;

/// Receives links the person followed out of the player.
pub type Open = Rc<dyn Fn(String)>;

/// A web view showing `page` (already checked with [`crate::play_page`]).
pub fn builder<'a>(page: &url::Url, open: Open) -> wry::WebViewBuilder<'a> {
    let window_open = open.clone();
    let builder = wry::WebViewBuilder::new()
        .with_url(page.as_str())
        .with_incognito(true)
        .with_devtools(false)
        .with_autoplay(true)
        .with_clipboard(false)
        .with_hotkeys_zoom(false)
        .with_back_forward_navigation_gestures(false)
        .with_background_color((0, 0, 0, 255))
        .with_focused(false)
        .with_download_started_handler(|_, _| false)
        .with_new_window_req_handler(move |url, _| {
            if crate::openable(&url) {
                window_open(url);
            }
            wry::NewWindowResponse::Deny
        });
    #[cfg(not(target_os = "linux"))]
    let builder = builder.with_navigation_handler(move |url| {
        let allowed = crate::main_frame_allowed(&url);
        if !allowed && crate::openable(&url) {
            open(url);
        }
        allowed
    });
    #[cfg(target_os = "linux")]
    drop(open);
    builder
}

/// WebKitGTK: once the play page has committed, any new main-frame load is
/// stopped before it commits, and its URL goes to `open`. Load events are
/// main-frame only, so the player's frames are unaffected. Also turns off
/// WebKit's own context menu and console output on stdout (the helper's
/// protocol channel).
#[cfg(target_os = "linux")]
pub fn guard_main_frame(view: &wry::WebView, open: Open) {
    use std::cell::Cell;

    use webkit2gtk::{LoadEvent, SettingsExt, WebViewExt};
    use wry::WebViewExtUnix;

    let web = view.webview();
    if let Some(settings) = WebViewExt::settings(&web) {
        settings.set_enable_write_console_messages_to_stdout(false);
        settings.set_enable_developer_extras(false);
        settings.set_javascript_can_open_windows_automatically(false);
        settings.set_javascript_can_access_clipboard(false);
    }
    web.connect_context_menu(|_, _, _, _| true);
    let shown = Rc::new(Cell::new(false));
    let leaving = Rc::new(Cell::new(false));
    {
        let (shown, leaving) = (shown.clone(), leaving.clone());
        web.connect_load_changed(move |web, event| match event {
            LoadEvent::Started if shown.get() => {
                leaving.set(true);
                web.stop_loading();
            }
            LoadEvent::Redirected | LoadEvent::Committed => {
                let uri = web.uri();
                let uri = uri.as_deref();
                if uri.is_some_and(crate::main_frame_allowed) {
                    shown.set(shown.get() || event == LoadEvent::Committed);
                } else if uri != Some("about:blank") {
                    // The first load went somewhere else: show nothing.
                    web.stop_loading();
                    if event == LoadEvent::Committed {
                        web.load_uri("about:blank");
                    }
                }
            }
            _ => {}
        });
    }
    web.connect_load_failed(move |_, _, uri, _| {
        if !leaving.replace(false) {
            return false;
        }
        if !crate::main_frame_allowed(uri) && crate::openable(uri) {
            open(uri.to_owned());
        }
        // No error page: the player stays as it was.
        true
    });
}
