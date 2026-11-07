//! Message text as Pango markup: the shared bold/italic grammar
//! (`shared/messages/formatting.rs`), mentions the server resolved, then
//! links found between them (`shared/messages/links.rs`), as the desktop
//! client draws them.

use caper_client::model::Mention;
use gtk::glib;
use std::ops::Range;

/// Mention pills, as desktop: one weight bolder, primary text on a 24%
/// terracotta wash.
const MENTION: &str = "background=\"#b64d32\" bgalpha=\"24%\" foreground=\"#f3f4f5\" \
     weight=\"500\" underline=\"none\"";

/// The link scheme of a person's mention pill: `caper-person:<account id>`.
pub const PERSON: &str = "caper-person:";

pub fn message(text: &str, mentions: &[Mention]) -> String {
    let highlighted = caper_client::mentions::highlights(text, mentions);
    let pills: Vec<Range<usize>> = highlighted.iter().map(|(range, _)| range.clone()).collect();
    // A person's pill links to their profile card.
    let people: Vec<Option<&str>> = highlighted
        .iter()
        .map(|(_, mention)| {
            (mention.kind == "user")
                .then_some(mention.id.as_deref())
                .flatten()
        })
        .collect();
    let mut links: Vec<(Range<usize>, String)> = Vec::new();
    // Links only in the plain text between mentions.
    let mut gap_start = 0;
    for gap_end in pills
        .iter()
        .map(|pill| (pill.start, pill.end))
        .chain([(text.len(), text.len())])
    {
        let mut offset = gap_start;
        for segment in crate::links::segments(&text[gap_start..gap_end.0]) {
            let next = offset + segment.text.len();
            if let Some(href) = segment.href {
                links.push((offset..next, href));
            }
            offset = next;
        }
        gap_start = gap_end.1;
    }
    let mut out = String::new();
    crate::formatting::visit_runs(text, |range, bold, italic| {
        let mut cuts = vec![range.start, range.end];
        for edge in pills
            .iter()
            .chain(links.iter().map(|(link, _)| link))
            .flat_map(|span| [span.start, span.end])
        {
            if range.start < edge && edge < range.end {
                cuts.push(edge);
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for piece in cuts.windows(2) {
            let (start, end) = (piece[0], piece[1]);
            let pill = pills.iter().position(|pill| pill.contains(&start));
            let link = links
                .iter()
                .find(|(link, _)| link.contains(&start))
                .map(|(_, href)| href);
            let mut close = Vec::new();
            if let Some(href) = link {
                out.push_str(&format!("<a href=\"{}\">", glib::markup_escape_text(href)));
                close.push("</a>");
            }
            if let Some(index) = pill {
                if let Some(id) = people[index] {
                    out.push_str(&format!(
                        "<a href=\"{PERSON}{}\">",
                        glib::markup_escape_text(id)
                    ));
                    close.push("</a>");
                }
                out.push_str(&format!("<span {MENTION}>"));
                close.push("</span>");
            }
            if bold {
                out.push_str("<b>");
                close.push("</b>");
            }
            if italic {
                out.push_str("<i>");
                close.push("</i>");
            }
            out.push_str(&glib::markup_escape_text(&text[start..end]));
            for tag in close.into_iter().rev() {
                out.push_str(tag);
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(name: &str) -> Mention {
        Mention {
            kind: "user".into(),
            id: Some("id".into()),
            username: Some(name.into()),
        }
    }

    #[test]
    fn formatting_hides_delimiters_and_escapes_text() {
        assert_eq!(
            message("**bold** and _it_ <x> & y", &[]),
            "<b>bold</b> and <i>it</i> &lt;x&gt; &amp; y"
        );
    }

    #[test]
    fn links_keep_their_underscores_and_escape_hrefs() {
        assert_eq!(
            message("see https://caper.chat/a_b_c?x=1&y=2.", &[]),
            "see <a href=\"https://caper.chat/a_b_c?x=1&amp;y=2\">https://caper.chat/a_b_c?x=1&amp;y=2</a>."
        );
    }

    #[test]
    fn only_resolved_mentions_become_pills() {
        // A token starts after whitespace or a bracket, so `**@maya**` is not one.
        let text = "hi @Maya and @nobody, **see @maya** **@maya**";
        assert_eq!(
            message(text, &[user("maya")]),
            format!(
                "hi <a href=\"{PERSON}id\"><span {MENTION}>@Maya</span></a> and @nobody, <b>see </b><a href=\"{PERSON}id\"><span {MENTION}><b>@maya</b></span></a> <b>@maya</b>"
            )
        );
        assert_eq!(
            message(text, &[]),
            "hi @Maya and @nobody, <b>see @maya</b> <b>@maya</b>"
        );
    }

    #[test]
    fn links_are_found_between_mentions() {
        let markup = message("@maya www.caper.chat", &[user("maya")]);
        assert!(markup.starts_with(&format!(
            "<a href=\"{PERSON}id\"><span {MENTION}>@maya</span></a> <a href=\""
        )));
        assert!(markup.ends_with(">www.caper.chat</a>"));
    }
}
