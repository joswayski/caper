//! Links to one message: `https://caper.chat/m/<channel>/<message>`, from a
//! message's Copy link, and `caper://m/<channel>/<message>` for the apps'
//! scheme. The channel is a space channel's or DM's 12-character ID and the
//! message its 15-character ID; neither names a space or thread, so a link
//! keeps working when the message's space or thread is unknown to the reader.
//!
//! Web's `messageLink` (`apps/web/src/chat/message-links.ts`) is the same rule;
//! every client runs `shared/messages/message-link-cases.json`. Only caper.chat
//! hosts count, so another site's `/m/...` path stays an ordinary link. The
//! API copy keeps message links out of link previews; clients draw them as
//! cards of the original instead, which the API shows only to readers who can
//! see it.

/// The web origin message links name.
pub const ORIGIN: &str = "https://caper.chat";

/// Hosts whose `/m/...` paths are message links.
const HOSTS: [&str; 3] = ["caper.chat", "www.caper.chat", "staging.caper.chat"];

/// A linked message's channel and message IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageLink<'a> {
    pub channel: &'a str,
    pub message: &'a str,
}

fn id(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// `<channel>/<message>`, then an optional `/`, query or fragment.
fn ids(path: &str) -> Option<MessageLink<'_>> {
    let end = path.find(['?', '#']).unwrap_or(path.len());
    let path = &path[..end];
    let path = path.strip_suffix('/').unwrap_or(path);
    let (channel, message) = path.split_once('/')?;
    (id(channel, 12) && id(message, 15)).then_some(MessageLink { channel, message })
}

fn strip_prefix_ignore_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|start| start.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}

/// The message a link opens, or `None` for anything else.
pub fn parse(href: &str) -> Option<MessageLink<'_>> {
    let value = href.trim();
    if let Some(rest) = strip_prefix_ignore_case(value, "caper://m/") {
        return ids(rest);
    }
    let rest = strip_prefix_ignore_case(value, "https://")
        .or_else(|| strip_prefix_ignore_case(value, "http://"))?;
    let slash = rest.find('/')?;
    let host = &rest[..slash];
    if !HOSTS.iter().any(|known| host.eq_ignore_ascii_case(known)) {
        return None;
    }
    ids(strip_prefix_ignore_case(&rest[slash..], "/m/")?)
}

/// The link Copy link puts on the clipboard.
pub fn url(channel: &str, message: &str) -> String {
    format!("{ORIGIN}/m/{channel}/{message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_shared_cases() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("message-link-cases.json"))
                .expect("valid shared message link cases");
        for case in cases["cases"].as_array().expect("cases") {
            let input = case["input"].as_str().expect("input");
            let expected = case["link"].as_object().map(|link| {
                (
                    link["channel"].as_str().unwrap(),
                    link["message"].as_str().unwrap(),
                )
            });
            assert_eq!(
                parse(input).map(|link| (link.channel, link.message)),
                expected,
                "{input:?}"
            );
        }
        assert_eq!(
            url("Chan12345678", "Msg123456789012"),
            "https://caper.chat/m/Chan12345678/Msg123456789012"
        );
    }
}
