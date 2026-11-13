//! Message text for GTK labels: the shared formatting grammar
//! (`shared/messages/formatting.rs`: bold, italic, underline,
//! strikethrough, spoilers and quotes), mentions the server resolved, then
//! links found between them (`shared/messages/links.rs`), as web draws them.
//! Each quote block is its own label with a bar on its left (`View`), and a
//! spoiler hides its text until it is clicked.

use crate::formatting::{BOLD, ITALIC, QUOTE, SPOILER, STRIKETHROUGH, UNDERLINE};
use caper_client::model::Mention;
use gtk::{glib, pango, prelude::*};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ops::Range;
use std::rc::Rc;

/// Mention pills, as desktop: one weight bolder, primary text on a 24%
/// terracotta wash.
const MENTION: &str = "background=\"#b64d32\" bgalpha=\"24%\" foreground=\"#f3f4f5\" \
     weight=\"500\" underline=\"none\"";

/// The link scheme of a person's mention pill: `caper-person:<account id>`.
pub const PERSON: &str = "caper-person:";

/// The link scheme of a hidden spoiler: `caper-spoiler:<index>`.
const SPOILER_LINK: &str = "caper-spoiler:";

/// A hidden spoiler, as web's: its text drawn invisibly on the border
/// colour (#34383B).
const HIDDEN: &str =
    "background=\"#34383b\" foreground=\"#34383b\" fgalpha=\"1\" underline=\"none\"";

/// A revealed spoiler, as web's: a rgb(255 255 255 / 6%) wash.
const REVEALED: &str = "background=\"#ffffff\" bgalpha=\"6%\"";

/// What a hidden spoiler is called, as web's button.
pub const SPOILER_NAME: &str = "Spoiler, reveal";

/// One stretch of a message's text with a single look.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Piece {
    text: String,
    /// The shared style bits.
    style: i32,
    link: Option<String>,
    /// A mention pill, with the account id its link opens for a person.
    pill: Option<Option<String>>,
    /// The contiguous spoiler it belongs to, which reveals as one.
    spoiler: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Block {
    /// Fixed Pango markup, such as a deleted message's notice.
    Notice(String),
    Text {
        quote: bool,
        pieces: Vec<Piece>,
    },
}

/// A message's text, block by block: consecutive quoted lines form one
/// block, and the text around them others.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Body(Vec<Block>);

impl Body {
    /// Fixed Pango `markup`, shown as is.
    pub fn notice(markup: &str) -> Self {
        Self(vec![Block::Notice(markup.into())])
    }

    pub fn message(text: &str, mentions: &[Mention]) -> Self {
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
        let mut pieces: Vec<Piece> = Vec::new();
        crate::formatting::visit_runs(text, |range, style| {
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
                pieces.push(Piece {
                    text: text[start..end].into(),
                    style,
                    link: links
                        .iter()
                        .find(|(link, _)| link.contains(&start))
                        .map(|(_, href)| href.clone()),
                    pill: pills
                        .iter()
                        .position(|pill| pill.contains(&start))
                        .map(|index| people[index].map(str::to_owned)),
                    spoiler: None,
                });
            }
        });
        // Neighbouring spoiler pieces, such as one with a bold word, reveal
        // together.
        let mut spoilers = 0;
        let mut previous = false;
        for piece in &mut pieces {
            let spoiler = piece.style & SPOILER != 0;
            if spoiler && !previous {
                spoilers += 1;
            }
            previous = spoiler;
            piece.spoiler = spoiler.then(|| spoilers - 1);
        }
        let mut blocks: Vec<(bool, Vec<Piece>)> = Vec::new();
        for piece in pieces {
            let quote = piece.style & QUOTE != 0;
            match blocks.last_mut() {
                Some((last, block)) if *last == quote => block.push(piece),
                _ => blocks.push((quote, vec![piece])),
            }
        }
        // A quote block takes the line break on each side of it.
        for index in 0..blocks.len() {
            if blocks[index].0 {
                continue;
            }
            let after_quote = index > 0;
            let before_quote = index + 1 < blocks.len();
            let pieces = &mut blocks[index].1;
            if after_quote
                && let Some(first) = pieces.first_mut()
                && first.text.starts_with('\n')
            {
                first.text.remove(0);
            }
            if before_quote
                && let Some(last) = pieces.last_mut()
                && last.text.ends_with('\n')
            {
                last.text.pop();
            }
            pieces.retain(|piece| !piece.text.is_empty());
        }
        Self(
            blocks
                .into_iter()
                .map(|(quote, pieces)| Block::Text { quote, pieces })
                .collect(),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Each block: whether it is a quote, and its Pango markup with the
    /// spoilers in `revealed` shown.
    pub fn blocks(&self, revealed: &BTreeSet<usize>) -> Vec<(bool, String)> {
        self.0
            .iter()
            .map(|block| match block {
                Block::Notice(markup) => (false, markup.clone()),
                Block::Text { quote, pieces } => (*quote, markup(pieces, revealed)),
            })
            .collect()
    }

    /// The text a screen reader reads, each hidden spoiler named as its
    /// button.
    pub fn plain(&self) -> String {
        let mut lines = Vec::new();
        for block in &self.0 {
            match block {
                Block::Notice(markup) => {
                    // Tags out first: links (`<a>`) are GtkLabel markup, which
                    // Pango's parser rejects. Text is escaped, so every `<`
                    // opens a tag.
                    let mut plain = String::with_capacity(markup.len());
                    let mut tag = false;
                    for c in markup.chars() {
                        match c {
                            '<' => tag = true,
                            '>' if tag => tag = false,
                            c if !tag => plain.push(c),
                            _ => {}
                        }
                    }
                    lines.push(
                        pango::parse_markup(&plain, '\0')
                            .map(|(_, text, _)| text.to_string())
                            .unwrap_or_default(),
                    );
                }
                Block::Text { pieces, .. } => {
                    let mut line = String::new();
                    let mut spoiler = None;
                    for piece in pieces {
                        match piece.spoiler {
                            Some(index) if spoiler != Some(index) => line.push_str(SPOILER_NAME),
                            Some(_) => {}
                            None => line.push_str(&piece.text),
                        }
                        spoiler = piece.spoiler;
                    }
                    lines.push(line);
                }
            }
        }
        lines.join("\n")
    }
}

/// `pieces` as GtkLabel markup. A hidden spoiler is one link over its
/// text, without the links and pills inside it.
fn markup(pieces: &[Piece], revealed: &BTreeSet<usize>) -> String {
    let mut out = String::new();
    // The hidden spoiler whose link is open.
    let mut open = None;
    for piece in pieces {
        let hidden = piece.spoiler.filter(|index| !revealed.contains(index));
        if open.is_some() && open != hidden {
            out.push_str("</span></a>");
            open = None;
        }
        if let Some(index) = hidden
            && open.is_none()
        {
            out.push_str(&format!(
                "<a href=\"{SPOILER_LINK}{index}\" title=\"{SPOILER_NAME}\" class=\"spoiler\"><span {HIDDEN}>"
            ));
            open = Some(index);
        }
        let mut close = Vec::new();
        if hidden.is_none() {
            if piece.spoiler.is_some() {
                out.push_str(&format!("<span {REVEALED}>"));
                close.push("</span>");
            }
            if let Some(href) = &piece.link {
                out.push_str(&format!("<a href=\"{}\">", glib::markup_escape_text(href)));
                close.push("</a>");
            }
            if let Some(person) = &piece.pill {
                if let Some(id) = person {
                    out.push_str(&format!(
                        "<a href=\"{PERSON}{}\">",
                        glib::markup_escape_text(id)
                    ));
                    close.push("</a>");
                }
                out.push_str(&format!("<span {MENTION}>"));
                close.push("</span>");
            }
        }
        for (bit, open, end) in [
            (BOLD, "<b>", "</b>"),
            (ITALIC, "<i>", "</i>"),
            (UNDERLINE, "<u>", "</u>"),
            (STRIKETHROUGH, "<s>", "</s>"),
        ] {
            if piece.style & bit != 0 {
                out.push_str(open);
                close.push(end);
            }
        }
        out.push_str(&glib::markup_escape_text(&piece.text));
        for tag in close.into_iter().rev() {
            out.push_str(tag);
        }
    }
    if open.is_some() {
        out.push_str("</span></a>");
    }
    out
}

type LinkHandler = Rc<dyn Fn(&gtk::Label, &str) -> glib::Propagation>;

#[derive(Default)]
struct Shown {
    body: RefCell<Body>,
    revealed: RefCell<BTreeSet<usize>>,
    labels: RefCell<Vec<gtk::Label>>,
    link: RefCell<Option<LinkHandler>>,
}

impl Shown {
    fn render(&self) {
        let blocks = self.body.borrow().blocks(&self.revealed.borrow());
        for (label, (_, markup)) in self.labels.borrow().iter().zip(blocks) {
            label.set_markup(&markup);
        }
    }
}

/// A message body's labels: one per block, quote blocks with a bar. A hidden
/// spoiler is a link that reveals it; revealed spoilers stay revealed while
/// the view shows the same body.
pub struct View {
    pub root: gtk::Box,
    shown: Rc<Shown>,
}

impl Default for View {
    fn default() -> Self {
        Self {
            root: gtk::Box::new(gtk::Orientation::Vertical, 0),
            shown: Rc::default(),
        }
    }
}

impl View {
    /// `link` opens any link but a spoiler's, from the label it is in.
    pub fn connect_link(&self, link: impl Fn(&gtk::Label, &str) -> glib::Propagation + 'static) {
        self.shown.link.replace(Some(Rc::new(link)));
    }

    pub fn set(&self, body: &Body) {
        if *self.shown.body.borrow() == *body && !self.shown.labels.borrow().is_empty() {
            return;
        }
        self.shown.body.replace(body.clone());
        self.shown.revealed.borrow_mut().clear();
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }
        let mut labels = Vec::new();
        for (quote, _) in body.blocks(&BTreeSet::new()) {
            let label = gtk::Label::builder()
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(pango::WrapMode::WordChar)
                .selectable(true)
                .css_classes(["body"])
                .build();
            if quote {
                label.add_css_class("message-quote");
            }
            let shown = Rc::downgrade(&self.shown);
            label.connect_activate_link(move |label, uri| {
                let Some(shown) = shown.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                if let Some(index) = uri.strip_prefix(SPOILER_LINK) {
                    if let Ok(index) = index.parse() {
                        shown.revealed.borrow_mut().insert(index);
                        // Not inside the label's own link handling.
                        glib::idle_add_local_once(move || shown.render());
                    }
                    return glib::Propagation::Stop;
                }
                let link = shown.link.borrow().clone();
                link.map_or(glib::Propagation::Proceed, |link| link(label, uri))
            });
            self.root.append(&label);
            labels.push(label);
        }
        self.shown.labels.replace(labels);
        self.shown.render();
    }

    /// The selected text, in whichever label has a selection.
    pub fn selected(&self) -> Option<String> {
        self.shown.labels.borrow().iter().find_map(|label| {
            let (start, end) = label.selection_bounds()?;
            let (start, end) = (start.min(end), start.max(end));
            Some(
                label
                    .text()
                    .chars()
                    .skip(start as usize)
                    .take((end - start) as usize)
                    .collect::<String>(),
            )
        })
    }
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

    /// The markup of a message with a single block, all spoilers hidden.
    fn message(text: &str, mentions: &[Mention]) -> String {
        let blocks = Body::message(text, mentions).blocks(&BTreeSet::new());
        assert_eq!(blocks.len(), 1, "{text}");
        blocks[0].1.clone()
    }

    #[test]
    fn formatting_hides_delimiters_and_escapes_text() {
        assert_eq!(
            message("**bold** and _it_ <x> & y", &[]),
            "<b>bold</b> and <i>it</i> &lt;x&gt; &amp; y"
        );
        assert_eq!(
            message("__under__ ~~struck~~ **__both__**", &[]),
            "<u>under</u> <s>struck</s> <b><u>both</u></b>"
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

    /// A contiguous spoiler is one link, with no links or pills inside it
    /// until it is revealed.
    #[test]
    fn spoilers_hide_until_revealed_together() {
        let text = "a ||b **c** www.caper.chat @maya|| d ||e||";
        let body = Body::message(text, &[user("maya")]);
        let hidden = |index: usize| {
            format!(
                "<a href=\"{SPOILER_LINK}{index}\" title=\"{SPOILER_NAME}\" class=\"spoiler\"><span {HIDDEN}>"
            )
        };
        assert_eq!(
            body.blocks(&BTreeSet::new())[0].1,
            format!(
                "a {}b <b>c</b> www.caper.chat @maya</span></a> d {}e</span></a>",
                hidden(0),
                hidden(1)
            )
        );
        let revealed = &body.blocks(&BTreeSet::from([0]))[0].1;
        assert!(revealed.starts_with(&format!(
            "a <span {REVEALED}>b </span><span {REVEALED}><b>c</b></span>"
        )));
        assert!(revealed.contains(&format!(
            "<span {REVEALED}><a href=\"{PERSON}id\"><span {MENTION}>@maya</span></a></span>"
        )));
        assert!(revealed.contains(">www.caper.chat</a></span>"));
        assert!(revealed.ends_with(&format!(" d {}e</span></a>", hidden(1))));
        assert_eq!(body.plain(), format!("a {SPOILER_NAME} d {SPOILER_NAME}"));
    }

    /// Consecutive quoted lines are one block, which takes the line break on
    /// each side of it.
    #[test]
    fn quoted_lines_form_blocks() {
        let body = Body::message("before\n> one\n> **two**\nafter\n\n> again", &[]);
        assert_eq!(
            body.blocks(&BTreeSet::new()),
            [
                (false, "before".to_owned()),
                (true, "one\n<b>two</b>".to_owned()),
                (false, "after\n".to_owned()),
                (true, "again".to_owned()),
            ]
        );
        assert_eq!(body.plain(), "before\none\ntwo\nafter\n\nagain");
        // `>` without text after it is not a quote.
        assert_eq!(message("> \n>", &[]), "&gt; \n&gt;");
    }
}
