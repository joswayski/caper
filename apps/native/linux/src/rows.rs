//! The message list's rows: what each shows, how a new list differs from the
//! shown one, and the recycled row widgets of the `GtkListView`.

use caper_client::grouping;
use caper_client::model::{Author, Mention, Message};
use chrono::{DateTime, Local};
use gtk::{glib, pango, prelude::*};
use std::collections::HashMap;
use std::ops::Range;

/// One message row, compared whole: an unchanged row keeps its item and
/// widget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The message id, or the client id of your message while it sends.
    pub id: String,
    /// The local day, on the first row of each day.
    pub day: Option<String>,
    /// Drawn under the previous row by the same author (no avatar or name).
    pub grouped: bool,
    pub author: String,
    pub initials: String,
    pub tint: u8,
    pub time: String,
    /// Pango markup for the text.
    pub markup: String,
    pub pending: bool,
}

/// One change to the shown list, applied in order. Positions are already
/// those of the new list.
#[derive(Debug, PartialEq, Eq)]
pub enum Op {
    /// The row at this index changed in place.
    Replace(usize),
    /// Remove `remove` items at `position`, then insert these new rows there.
    Splice {
        position: usize,
        remove: usize,
        insert: Range<usize>,
    },
}

/// The changes from `old` to `new`: rows are matched by id at both ends
/// (history arrives at the top, live messages at the bottom), the middle is
/// replaced, and matched rows whose content changed are replaced alone.
pub fn diff(old: &[Row], new: &[Row]) -> Vec<Op> {
    let prefix = old
        .iter()
        .zip(new)
        .take_while(|(old, new)| old.id == new.id)
        .count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(old, new)| old.id == new.id)
        .count();
    let mut ops: Vec<Op> = (0..prefix)
        .filter(|&index| old[index] != new[index])
        .map(Op::Replace)
        .collect();
    let (removed, inserted) = (old.len() - prefix - suffix, new.len() - prefix - suffix);
    if removed > 0 || inserted > 0 {
        ops.push(Op::Splice {
            position: prefix,
            remove: removed,
            insert: prefix..prefix + inserted,
        });
    }
    ops.extend(
        (0..suffix)
            .map(|offset| (old.len() - suffix + offset, new.len() - suffix + offset))
            .filter(|&(before, after)| old[before] != new[after])
            .map(|(_, after)| Op::Replace(after)),
    );
    ops
}

/// Message markup by id, rebuilt only when the text or its mentions change.
#[derive(Default)]
pub struct Markup(HashMap<String, (String, Vec<Mention>, String)>);

impl Markup {
    fn text(&mut self, message: &Message) -> String {
        let content = &message.content;
        if let Some((text, mentions, markup)) = self.0.get(&message.id)
            && *text == content.text
            && *mentions == content.mentions
        {
            return markup.clone();
        }
        let markup = crate::markup::message(&content.text, &content.mentions);
        self.0.insert(
            message.id.clone(),
            (
                content.text.clone(),
                content.mentions.clone(),
                markup.clone(),
            ),
        );
        markup
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

fn body(message: &Message, markup: &mut Markup) -> String {
    const MUTED: &str = "<span foreground=\"#b9bcbe\"><i>";
    if message.is_deleted() {
        return format!("{MUTED}This message was deleted.</i></span>");
    }
    let mut text = markup.text(message);
    let content = &message.content;
    let extra = if !content.attachments.is_empty() {
        Some(caper_client::model::attachments_label(&content.attachments))
    } else {
        content.gif.as_ref().map(|_| "GIF")
    };
    if let Some(extra) = extra {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!(
            "{MUTED}{}</i></span>",
            glib::markup_escape_text(extra)
        ));
    }
    text
}

/// Your message while it sends: its client id, author, time and text.
pub type Draft<'a> = (&'a str, &'a Author, &'a str, &'a str);

/// The channel's rows, oldest first: channel messages, then `draft`.
pub fn build<'a>(
    messages: impl Iterator<Item = &'a Message>,
    draft: Option<Draft<'a>>,
    markup: &mut Markup,
) -> Vec<Row> {
    let items = messages
        .filter(|message| message.is_channel_message())
        .map(|message| {
            (
                message.id.as_str(),
                &message.author,
                message.created_at.as_str(),
                body(message, markup),
                false,
            )
        })
        .chain(draft.map(|(id, author, created_at, text)| {
            (
                id,
                author,
                created_at,
                crate::markup::message(text, &[]),
                true,
            )
        }));
    let mut rows = Vec::new();
    let mut previous: Option<grouping::Row> = None;
    let mut day: Option<String> = None;
    for (id, author, created_at, markup, pending) in items {
        let local = DateTime::parse_from_rfc3339(created_at)
            .ok()
            .map(|date| date.with_timezone(&Local));
        let this_day = local.map(|date| date.format("%A, %B %-d, %Y").to_string());
        let first_of_day = this_day.is_some() && this_day != day;
        if first_of_day {
            previous = None;
            day.clone_from(&this_day);
        }
        let current = grouping::Row::new(&author.id, created_at, false);
        let grouped = grouping::grouped(previous.as_ref(), &current);
        previous = Some(current);
        rows.push(Row {
            id: id.to_owned(),
            day: this_day.filter(|_| first_of_day),
            grouped,
            author: author.name.clone(),
            initials: initials(&author.name),
            tint: tint(&author.id),
            time: local
                .map(|date| date.format("%-I:%M %p").to_string())
                .unwrap_or_default(),
            markup,
            pending,
        });
    }
    rows
}

/// Up to two letters: the first of the first two words.
pub fn initials(name: &str) -> String {
    let letters: String = name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    if letters.is_empty() {
        "?".into()
    } else {
        letters
    }
}

/// A stable avatar colour (`tint-0`…`tint-5` in the stylesheet).
pub fn tint(id: &str) -> u8 {
    (id.bytes().fold(0u32, |sum, byte| {
        sum.wrapping_mul(31).wrapping_add(byte.into())
    }) % 6) as u8
}

/// Recycled message row widgets: setup builds one, bind fills it from its
/// item.
pub fn factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        item.set_activatable(false);
        item.set_selectable(false);
        item.set_child(Some(&widget()));
    });
    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        if let (Some(object), Some(root)) = (
            item.item().and_downcast::<glib::BoxedAnyObject>(),
            item.child().and_downcast::<gtk::Box>(),
        ) {
            fill(&root, &object.borrow::<Row>());
        }
    });
    factory
}

fn widget() -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let day = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(["day"])
        .build();
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let avatar = gtk::Label::builder()
        .valign(gtk::Align::Start)
        .halign(gtk::Align::Start)
        .css_classes(["avatar"])
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.set_hexpand(true);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let author = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(["author"])
        .build();
    let time = gtk::Label::builder()
        .xalign(0.0)
        .valign(gtk::Align::Baseline)
        .css_classes(["time"])
        .build();
    let body = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .selectable(true)
        .css_classes(["body"])
        .build();
    header.append(&author);
    header.append(&time);
    content.append(&header);
    content.append(&body);
    line.append(&avatar);
    line.append(&content);
    root.append(&day);
    root.append(&line);
    root
}

fn fill(root: &gtk::Box, row: &Row) -> Option<()> {
    let day = root.first_child()?.downcast::<gtk::Label>().ok()?;
    let line = day.next_sibling()?;
    let avatar = line.first_child()?.downcast::<gtk::Label>().ok()?;
    let header = avatar.next_sibling()?.first_child()?;
    let author = header.first_child()?.downcast::<gtk::Label>().ok()?;
    let time = author.next_sibling()?.downcast::<gtk::Label>().ok()?;
    let body = header.next_sibling()?.downcast::<gtk::Label>().ok()?;
    let mut classes = vec!["message"];
    if row.grouped {
        classes.push("grouped");
    }
    if row.pending {
        classes.push("pending");
    }
    root.set_css_classes(&classes);
    day.set_visible(row.day.is_some());
    day.set_label(row.day.as_deref().unwrap_or_default());
    header.set_visible(!row.grouped);
    if row.grouped {
        avatar.set_label("");
        avatar.set_css_classes(&["avatar", "gap"]);
    } else {
        avatar.set_label(&row.initials);
        avatar.set_css_classes(&["avatar", &format!("tint-{}", row.tint)]);
    }
    author.set_label(&row.author);
    time.set_label(&row.time);
    body.set_markup(&row.markup);
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, text: &str) -> Row {
        Row {
            id: id.into(),
            day: None,
            grouped: false,
            author: "Maya".into(),
            initials: "M".into(),
            tint: 0,
            time: String::new(),
            markup: text.into(),
            pending: false,
        }
    }

    fn rows(ids: &[&str]) -> Vec<Row> {
        ids.iter().map(|id| row(id, "")).collect()
    }

    #[test]
    fn unchanged_rows_produce_no_changes() {
        assert!(diff(&rows(&["a", "b"]), &rows(&["a", "b"])).is_empty());
    }

    #[test]
    fn history_inserts_at_the_top_and_live_messages_at_the_bottom() {
        assert_eq!(
            diff(&rows(&["c", "d"]), &rows(&["a", "b", "c", "d"])),
            [Op::Splice {
                position: 0,
                remove: 0,
                insert: 0..2
            }]
        );
        assert_eq!(
            diff(&rows(&["a", "b"]), &rows(&["a", "b", "c"])),
            [Op::Splice {
                position: 2,
                remove: 0,
                insert: 2..3
            }]
        );
    }

    #[test]
    fn a_confirmed_send_replaces_only_its_draft() {
        let mut new = rows(&["a", "b", "sent"]);
        new[1].grouped = true;
        assert_eq!(
            diff(&rows(&["a", "b", "draft"]), &new),
            [
                Op::Replace(1),
                Op::Splice {
                    position: 2,
                    remove: 1,
                    insert: 2..3
                }
            ]
        );
    }

    #[test]
    fn edits_replace_their_row_in_place_at_either_end() {
        let old = rows(&["a", "b", "c", "d"]);
        let mut new = old.clone();
        new.insert(2, row("x", ""));
        new[0].markup = "edited".into();
        new[4].markup = "edited".into();
        assert_eq!(
            diff(&old, &new),
            [
                Op::Replace(0),
                Op::Splice {
                    position: 2,
                    remove: 0,
                    insert: 2..3
                },
                Op::Replace(4)
            ]
        );
    }

    #[test]
    fn a_new_channel_replaces_everything() {
        assert_eq!(
            diff(&rows(&["a", "b"]), &rows(&["x"])),
            [Op::Splice {
                position: 0,
                remove: 2,
                insert: 0..1
            }]
        );
    }

    #[test]
    fn initials_take_two_words() {
        assert_eq!(initials("Mateo Silva Ruiz"), "MS");
        assert_eq!(initials("maya"), "M");
        assert_eq!(initials("  "), "?");
    }
}
