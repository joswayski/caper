//! Link preview players (YouTube, Vimeo, Twitch) in the operating system's
//! own web engine, only for the player box and only after Play, as the
//! Android and Apple clients do (docs/media.md "Link embeds").
//!
//! Caper links this library for the play-page allowlist and the helper's
//! line protocol; it pulls in no web engine. The `engine` feature adds the
//! shared web view configuration ([`engine`]), used in-process on Windows
//! (WebView2) and by the `caper-web-player` helper on Linux (WebKitGTK), so
//! the main Linux binary never links GTK or WebKitGTK.

#[cfg(feature = "engine")]
pub mod engine;
pub mod protocol;

/// The only hosts whose `play` page loads.
pub const PLAY_HOSTS: [&str; 2] = ["embed.caper.chat", "embed.staging.caper.chat"];

/// A player block's `play` page, only when it is HTTPS on Caper's embed host
/// with no credentials or port, under `/play/`.
///
/// Debug builds read `CAPER_DEV_PLAY_ORIGIN` (`http://127.0.0.1:PORT` only):
/// such a page then loads from that loopback origin instead, so a stand-in
/// page can be served locally. Release builds never read it.
pub fn play_page(url: &str) -> Option<url::Url> {
    play_page_with(url, dev_origin().as_ref())
}

/// [`play_page`] with an explicit development origin.
pub fn play_page_with(url: &str, dev: Option<&url::Url>) -> Option<url::Url> {
    let parsed = url::Url::parse(url).ok()?;
    let plain = parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.path().starts_with("/play/");
    if !plain {
        return None;
    }
    if let Some(dev) = dev
        && parsed.origin() == dev.origin()
    {
        return Some(parsed);
    }
    let caper = parsed.scheme() == "https"
        && parsed.port().is_none()
        && parsed
            .host_str()
            .is_some_and(|host| PLAY_HOSTS.contains(&host));
    if !caper {
        return None;
    }
    match dev {
        Some(dev) => {
            let mut local = dev.clone();
            local.set_path(parsed.path());
            local.set_query(parsed.query());
            Some(local)
        }
        None => Some(parsed),
    }
}

/// A loopback-only development origin, from `CAPER_DEV_PLAY_ORIGIN` in debug
/// builds.
fn dev_origin() -> Option<url::Url> {
    #[cfg(debug_assertions)]
    {
        std::env::var("CAPER_DEV_PLAY_ORIGIN")
            .ok()
            .and_then(|value| loopback_origin(&value))
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// `http://127.0.0.1:PORT`, and nothing else.
pub fn loopback_origin(value: &str) -> Option<url::Url> {
    let url = url::Url::parse(value).ok()?;
    (url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.path(), "" | "/")
        && url.query().is_none())
    .then_some(url)
}

/// Whether the player's main frame may show `url`: only the play page.
/// The provider's player runs in the page's frames, which may go anywhere.
pub fn main_frame_allowed(url: &str) -> bool {
    play_page(url).is_some()
}

/// A link that may open in the system browser: http(s) only.
pub fn openable(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

/// The provider's name for window titles and labels.
pub fn provider_name(provider: &str) -> &str {
    match provider {
        "youtube" => "YouTube",
        "vimeo" => "Vimeo",
        "twitch" => "Twitch",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_caper_play_pages_load() {
        for good in [
            "https://embed.caper.chat/play/youtube/dQw4w9WgXcQ",
            "https://embed.staging.caper.chat/play/vimeo/76979871?start=12",
            "https://embed.caper.chat:443/play/twitch/clip%3Aabc",
        ] {
            assert!(play_page_with(good, None).is_some(), "{good}");
        }
        for bad in [
            "http://embed.caper.chat/play/youtube/a",
            "https://embed.caper.chat/m/fixture/poster",
            "https://embed.caper.chat/playx/youtube/a",
            "https://embed.caper.chat:8443/play/youtube/a",
            "https://user@embed.caper.chat/play/youtube/a",
            "https://user:pass@embed.caper.chat/play/youtube/a",
            "https://evil.embed.caper.chat/play/youtube/a",
            "https://embed.caper.chat.evil.example/play/youtube/a",
            "https://www.youtube-nocookie.com/embed/a",
            "javascript:alert(1)",
            "file:///play/youtube/a",
            "not a url",
        ] {
            assert!(play_page_with(bad, None).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_development_origin_is_loopback_only_and_replaces_the_host() {
        for bad in [
            "http://localhost:8791",
            "https://127.0.0.1:8791",
            "http://127.0.0.1",
            "http://10.0.0.1:8791",
            "http://127.0.0.1:8791/play/",
            "http://a@127.0.0.1:8791",
        ] {
            assert!(loopback_origin(bad).is_none(), "{bad}");
        }
        let dev = loopback_origin("http://127.0.0.1:8791").unwrap();
        assert_eq!(
            play_page_with(
                "https://embed.caper.chat/play/youtube/a?start=3",
                Some(&dev)
            )
            .unwrap()
            .as_str(),
            "http://127.0.0.1:8791/play/youtube/a?start=3"
        );
        assert!(play_page_with("http://127.0.0.1:8791/play/youtube/a", Some(&dev)).is_some());
        assert!(play_page_with("http://127.0.0.1:8792/play/youtube/a", Some(&dev)).is_none());
        assert!(play_page_with("http://127.0.0.1:8791/away", Some(&dev)).is_none());
        assert!(play_page_with("https://example.com/play/a", Some(&dev)).is_none());
    }

    #[test]
    fn only_web_links_open_outside() {
        assert!(openable("https://www.youtube.com/watch?v=a"));
        assert!(openable("http://example.com"));
        assert!(!openable("javascript:alert(1)"));
        assert!(!openable("file:///etc/passwd"));
        assert!(!openable("about:blank"));
    }
}
