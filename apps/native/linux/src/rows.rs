//! The message lists' rows: what each shows, how a new list differs from the
//! shown one, and the recycled row widgets of a `GtkListView`.

use crate::images::{AvatarView, Icon, Images};
use caper_client::grouping;
use caper_client::model::{Author, Mention, Message};
use chrono::{DateTime, Local};
use gtk::{glib, pango, prelude::*};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

/// The quick reactions on each row's toolbar, as desktop shows them.
pub const QUICK_REACTIONS: [&str; 3] = ["👍", "❤️", "😂"];

/// One reaction chip: the emoji, how many reacted, and whether you did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub emoji: String,
    pub count: usize,
    pub mine: bool,
    /// Who reacted, as every client words it: names once loaded, else a
    /// count. The chip's hover card.
    pub summary: String,
}

/// What a row offers from its toolbar and menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Offers {
    /// React, reply and pin: a sent message in a joined conversation.
    pub act: bool,
    /// Edit and delete: your own message.
    pub mine: bool,
    pub pinned: bool,
    /// Open its thread (main-list messages only).
    pub thread: bool,
}

/// A reply's quote card, as desktop: the quoted author (with their
/// avatar) and up to three lines of their text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quote {
    /// "Blocked message" in place of a blocked author.
    pub author: String,
    /// None for a blocked author.
    pub avatar: Option<crate::images::Avatar>,
    /// The whole text, an attachments label, or "Message"; None once the
    /// original is deleted, or for a blocked author.
    pub text: Option<String>,
    pub deleted: bool,
}

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
    /// Who wrote it, for their profile card.
    pub person: Author,
    pub avatar: crate::images::Avatar,
    /// The uploaded picture loaded, so a row redraws once it arrives.
    pub picture_ready: bool,
    pub time: String,
    /// Pango markup for the text.
    pub markup: String,
    pub pending: bool,
    pub edited: bool,
    /// "Pinned by …".
    pub pinned_by: Option<String>,
    /// The quoted message, drawn as desktop's card.
    pub reply: Option<Quote>,
    /// The quoted message and its thread, which a click jumps to.
    pub reply_target: Option<(String, Option<String>)>,
    /// The quote shows in full rather than three lines.
    pub reply_expanded: bool,
    /// A line under the message: an empty thread's invitation to reply.
    pub footer: Option<String>,
    pub reactions: Vec<Chip>,
    /// Your reaction failed: shown under the row with Retry and Dismiss.
    pub reaction_error: Option<String>,
    /// "3 replies ● Unread View thread" (Pango markup), on a thread root in
    /// the main list.
    pub replies: Option<String>,
    /// A thread reply also sent to the channel, in the main list: "Replied
    /// to a thread · View thread" above it.
    pub broadcast: bool,
    /// It mentions you: a terracotta-tinted row, as desktop.
    pub mentioned: bool,
    /// Someone replied to you: the same row, tinted a little deeper.
    pub replied: bool,
    /// The message opened from elsewhere (a forward's original): highlighted.
    pub focused: bool,
    /// The first message that was unread when you opened the conversation:
    /// a "New messages" divider above it.
    pub unread: bool,
    /// The oldest message there is: "Beginning of conversation" above it.
    pub beginning: bool,
    /// It asked the Caper team, so it carries their notice.
    pub team_notice: bool,
    pub offers: Offers,
    /// Files, link previews and a GIF below the text.
    pub content: crate::content::Content,
    /// A run of blocked authors' messages: its key, "N blocked messages"
    /// and whether it is shown. Such a row draws only that notice.
    pub blocked_run: Option<(String, String, bool)>,
    /// Written by someone you blocked (inside a run you chose to show).
    pub blocked: bool,
    /// A forward: its card, with the original when it is still available.
    pub forward: Option<Option<Original>>,
}

/// A forwarded original, as its card shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Original {
    pub author: String,
    pub person: Author,
    pub avatar: crate::images::Avatar,
    pub markup: String,
    pub edited: bool,
    /// "👍 2  🎉 1".
    pub reactions: String,
    pub content: crate::content::Content,
    /// "2 replies · View conversation", on cards in the list.
    pub view: Option<String>,
}

impl Original {
    pub fn new(
        message: &Message,
        images: &Images,
        revealed: &crate::media::Revealed,
        scope: String,
        view: bool,
    ) -> Self {
        let deleted = message.is_deleted();
        Self {
            author: message.author.name.clone(),
            person: message.author.clone(),
            avatar: author_avatar(&message.author),
            markup: if deleted {
                "<span foreground=\"#b9bcbe\"><i>This message was deleted.</i></span>".into()
            } else if message.content.text.is_empty()
                && message.content.attachments.iter().any(|file| file.voice)
            {
                "<span foreground=\"#b9bcbe\">Voice message</span>".into()
            } else {
                crate::markup::message(&message.content.text, &message.content.mentions)
            },
            edited: message.edited_at.is_some() && !deleted,
            reactions: if deleted {
                String::new()
            } else {
                message
                    .reactions
                    .iter()
                    .map(|reaction| format!("{} {}", reaction.emoji, reaction.author_ids.len()))
                    .collect::<Vec<_>>()
                    .join("  ")
            },
            content: if deleted {
                crate::content::Content::default()
            } else {
                crate::content::Content {
                    scope: Some(scope.clone()),
                    attachments: message
                        .content
                        .attachments
                        .iter()
                        .map(|attachment| {
                            (
                                images.media.resolve(attachment),
                                attachment.spoiler
                                    && !revealed
                                        .get(&scope)
                                        .is_some_and(|ids| ids.contains(&attachment.id)),
                            )
                        })
                        .collect(),
                    embeds: Vec::new(),
                    gif: message.content.gif.clone(),
                }
            },
            view: view.then(|| {
                let count = message
                    .thread
                    .as_ref()
                    .map_or(0, |summary| summary.reply_count);
                match count {
                    0 => "View conversation".to_owned(),
                    1 => "1 reply · View conversation".to_owned(),
                    count => format!("{count} replies · View conversation"),
                }
            }),
        }
    }

    /// The original's author line, text, files and reactions.
    pub fn view(&self, images: &Rc<Images>, handler: &Handler, open: Option<&str>) -> gtk::Box {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.add_css_class("original");
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let avatar = AvatarView::new(24, false);
        avatar.set(images, &self.avatar);
        line.append(&avatar.root);
        let name = gtk::Label::builder()
            .label(&self.author)
            .xalign(0.0)
            .css_classes(["author"])
            .build();
        line.append(&name);
        if self.edited {
            line.append(
                &gtk::Label::builder()
                    .label("(edited)")
                    .css_classes(["time"])
                    .build(),
            );
        }
        root.append(&line);
        if !self.markup.is_empty() {
            let body = gtk::Label::builder()
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(pango::WrapMode::WordChar)
                .selectable(true)
                .css_classes(["body"])
                .build();
            body.set_markup(&self.markup);
            let (handler, anchor) = (handler.clone(), body.clone());
            body.connect_activate_link(move |_, uri| {
                if let Some(id) = uri.strip_prefix(crate::markup::PERSON) {
                    handler(Action::Profile {
                        id: id.to_owned(),
                        author: None,
                        anchor: anchor.clone().upcast(),
                    });
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            root.append(&body);
        }
        if let Some(view) = crate::content::view(images, &self.content, handler) {
            root.append(&view);
        }
        if !self.reactions.is_empty() {
            root.append(
                &gtk::Label::builder()
                    .label(&self.reactions)
                    .xalign(0.0)
                    .css_classes(["muted", "small"])
                    .build(),
            );
        }
        if let (Some(label), Some(id)) = (&self.view, open) {
            let button = gtk::Button::with_label(label);
            button.add_css_class("flat");
            button.add_css_class("inline");
            button.set_halign(gtk::Align::Start);
            let (handler, id) = (handler.clone(), id.to_owned());
            button.connect_clicked(move |_| handler(Action::ViewForward(id.clone())));
            root.append(&button);
        }
        root
    }
}

impl Row {
    /// What a screen reader announces for the row (GTK 4.12's list item
    /// label): "Maya, 2:15 AM, see you at noon", then its files, edits and
    /// sending state; a blocked run reads its notice.
    pub fn accessible_label(&self) -> String {
        if let Some((_, notice, _)) = &self.blocked_run {
            return notice.clone();
        }
        // Tags out first: links (`<a>`) are GtkLabel markup, which Pango's
        // parser rejects. Text is escaped, so every `<` opens a tag.
        let mut plain = String::with_capacity(self.markup.len());
        let mut tag = false;
        for c in self.markup.chars() {
            match c {
                '<' => tag = true,
                '>' if tag => tag = false,
                c if !tag => plain.push(c),
                _ => {}
            }
        }
        let text = pango::parse_markup(&plain, '\0')
            .map(|(_, text, _)| text.to_string())
            .unwrap_or_default();
        let files = self.content.attachments.len() + usize::from(self.content.gif.is_some());
        let mut parts = vec![self.author.clone(), self.time.clone()];
        parts.extend((!text.trim().is_empty()).then(|| text.trim().to_owned()));
        parts.extend(match files {
            0 => None,
            1 => Some("1 attachment".to_owned()),
            n => Some(format!("{n} attachments")),
        });
        parts.extend(self.edited.then(|| "edited".to_owned()));
        parts.extend(self.pending.then(|| "sending".to_owned()));
        parts.retain(|part| !part.is_empty());
        parts.join(", ")
    }

    /// "⊘ N blocked messages — Show" for a run by blocked accounts.
    fn notice(key: String, label: String, shown: bool, author: &Author) -> Self {
        Self {
            id: format!("blocked:{key}"),
            day: None,
            grouped: false,
            author: String::new(),
            person: author.clone(),
            avatar: author_avatar(author),
            picture_ready: false,
            time: String::new(),
            markup: String::new(),
            pending: false,
            edited: false,
            pinned_by: None,
            reply: None,
            reply_target: None,
            reply_expanded: false,
            footer: None,
            reactions: Vec::new(),
            reaction_error: None,
            replies: None,
            broadcast: false,
            mentioned: false,
            replied: false,
            focused: false,
            unread: false,
            beginning: false,
            team_notice: false,
            offers: Offers::default(),
            content: crate::content::Content::default(),
            blocked_run: Some((key, label, shown)),
            blocked: false,
            forward: None,
        }
    }
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

/// Applies `new` to `store`, which shows `old`: only new or changed rows get
/// new items, so only they are bound again. Returns whether anything changed.
pub fn apply(store: &gtk::gio::ListStore, old: &[Row], new: &[Row]) -> bool {
    let item = |index: usize| glib::BoxedAnyObject::new(new[index].clone());
    let ops = diff(old, new);
    for op in &ops {
        let (position, remove, items): (usize, usize, Vec<_>) = match op {
            Op::Replace(index) => (*index, 1, vec![item(*index)]),
            Op::Splice {
                position,
                remove,
                insert,
            } => (*position, *remove, insert.clone().map(item).collect()),
        };
        store.splice(position as u32, remove as u32, &items);
    }
    !ops.is_empty()
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
    markup.text(message)
}

/// Your message while it sends: its client id, author, time and text.
pub struct Draft<'a> {
    pub id: &'a str,
    pub author: &'a Author,
    pub created_at: &'a str,
    pub text: &'a str,
    /// The files it sends, shown from your local copies.
    pub content: crate::content::Content,
}

/// Who is reading, for reactions and the actions each row offers.
pub struct Viewer<'a> {
    /// Your account id, as reactions list it.
    pub me: Option<&'a str>,
    /// Your chat author id, which your messages carry.
    pub author: Option<&'a str>,
    /// You can write here (a joined channel or a DM, with a chat session).
    pub joined: bool,
    /// Rows in the thread pane: no thread action or reply counts.
    pub in_thread: bool,
    pub images: &'a Images,
    /// Your reactions still on their way, drawn over the server's counts.
    pub reactions: &'a caper_client::reactions::Reactions,
    /// Spoiler attachments you revealed.
    pub revealed: &'a crate::media::Revealed,
    /// Independent presentation of a message (channel, thread or pins).
    pub surface: &'static str,
    /// People you blocked, and the blocked runs you chose to show.
    pub blocked: &'a std::collections::BTreeSet<String>,
    pub shown_blocked: &'a std::collections::HashSet<String>,
    /// Thread roots with replies you haven't read.
    pub thread_unread: &'a std::collections::BTreeMap<String, String>,
    /// Who reacted, by message, as far as loaded.
    pub reactors: &'a std::collections::BTreeMap<String, caper_client::reactions::ReactorCache>,
    /// Messages whose reply quote shows in full.
    pub expanded_quotes: &'a std::collections::HashSet<String>,
}

impl Viewer<'_> {
    /// The id reactions list you by: your chat author, else your account.
    pub fn reactor(&self) -> Option<&str> {
        self.author.or(self.me)
    }
}

fn author_avatar(author: &Author) -> crate::images::Avatar {
    crate::images::Avatar::new(
        &author.name,
        author.avatar_id,
        &author.avatar_asset,
        &author.avatar_url,
    )
}

/// The rows for `messages`, oldest first, then `draft`.
pub fn build<'a>(
    messages: impl Iterator<Item = &'a Message>,
    draft: Option<Draft<'a>>,
    markup: &mut Markup,
    viewer: &Viewer<'_>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut previous: Option<grouping::Row> = None;
    let mut day: Option<String> = None;
    let mut push = |rows: &mut Vec<Row>,
                    id: &str,
                    author: &Author,
                    created_at: &str,
                    row: Row,
                    separate: bool| {
        if separate {
            previous = None;
        }
        let local = DateTime::parse_from_rfc3339(created_at)
            .ok()
            .map(|date| date.with_timezone(&Local));
        let this_day = local.map(|date| date.format("%A, %B %-d, %Y").to_string());
        let first_of_day = this_day.is_some() && this_day != day;
        if first_of_day {
            previous = None;
            day.clone_from(&this_day);
        }
        // A reply quote, pin or thread link keeps its header, like desktop.
        let header = row.reply.is_some() || row.pinned_by.is_some() || row.broadcast;
        let current = grouping::Row::new(&author.id, created_at, header);
        let grouped = grouping::grouped(previous.as_ref(), &current);
        previous = Some(current);
        rows.push(Row {
            id: id.to_owned(),
            day: this_day.filter(|_| first_of_day),
            grouped,
            time: local
                .map(|date| date.format("%-I:%M %p").to_string())
                .unwrap_or_default(),
            ..row
        });
    };
    let messages: Vec<&Message> = messages.collect();
    // Each message, whether a shown blocked run holds it, and whether a
    // notice comes right before it (so it starts a new group).
    enum Step {
        Notice(Box<Row>),
        Message(usize, bool),
    }
    let mut plan = Vec::new();
    for item in caper_client::blocking::rows(&messages, viewer.blocked, viewer.me) {
        match item {
            caper_client::blocking::Row::Message(index) => plan.push(Step::Message(index, false)),
            caper_client::blocking::Row::Blocked { range, key } => {
                let shown = viewer.shown_blocked.contains(&key);
                plan.push(Step::Notice(Box::new(Row::notice(
                    key,
                    caper_client::blocking::label(range.len()),
                    shown,
                    &messages[range.start].author,
                ))));
                if shown {
                    plan.extend(range.map(|index| Step::Message(index, true)));
                }
            }
        }
    }
    let mut separate_next = false;
    for step in plan {
        let (index, blocked) = match step {
            Step::Notice(row) => {
                rows.push(*row);
                separate_next = true;
                continue;
            }
            Step::Message(index, blocked) => (index, blocked),
        };
        let message = messages[index];
        let separate = std::mem::take(&mut separate_next);
        let avatar = author_avatar(&message.author);
        let deleted = message.is_deleted();
        let act = viewer.joined && !deleted;
        let replies_to_me = viewer.me.is_some_and(|me| {
            message.author.id != me
                && message
                    .reply_to
                    .as_ref()
                    .is_some_and(|reply| reply.author.id == me)
        });
        let row = Row {
            id: String::new(),
            day: None,
            grouped: false,
            author: message.author.name.clone(),
            person: message.author.clone(),
            picture_ready: viewer.images.picture_ready(&avatar),
            avatar,
            time: String::new(),
            markup: body(message, markup),
            pending: false,
            edited: message.edited_at.is_some() && !deleted,
            pinned_by: message
                .pin
                .as_ref()
                .map(|pin| format!("Pinned by {}", pin.author.name)),
            reply_target: message
                .reply_to
                .as_ref()
                .filter(|reply| !viewer.blocked.contains(&reply.author.id))
                .map(|reply| (reply.id.clone(), reply.thread_root_id.clone())),
            reply_expanded: viewer.expanded_quotes.contains(&message.id),
            footer: None,
            reply: message.reply_to.as_ref().map(|reply| {
                if viewer.blocked.contains(&reply.author.id) {
                    return Quote {
                        author: "Blocked message".to_owned(),
                        avatar: None,
                        text: None,
                        deleted: reply.deleted,
                    };
                }
                let text = reply.full_text.as_deref().unwrap_or(&reply.text);
                let text = if !text.trim().is_empty() {
                    text.to_owned()
                } else if reply.has_attachments {
                    caper_client::model::attachments_label(&reply.attachments).to_owned()
                } else {
                    "Message".to_owned()
                };
                Quote {
                    author: reply.author.name.clone(),
                    avatar: Some(author_avatar(&reply.author)),
                    text: (!reply.deleted).then_some(text),
                    deleted: reply.deleted,
                }
            }),
            reactions: if deleted {
                Vec::new()
            } else {
                viewer
                    .reactions
                    .projected(&message.reactions, &message.id, viewer.reactor())
                    .into_iter()
                    .filter(|reaction| !reaction.author_ids.is_empty())
                    .map(|reaction| Chip {
                        mine: viewer
                            .reactor()
                            .is_some_and(|me| reaction.author_ids.iter().any(|id| id == me)),
                        count: reaction.author_ids.len(),
                        summary: caper_client::reactions::reactor_summary(
                            viewer.reactors.get(&message.id),
                            &reaction,
                            viewer.reactor(),
                        ),
                        emoji: reaction.emoji,
                    })
                    .collect()
            },
            reaction_error: viewer.reactions.error(&message.id).map(str::to_owned),
            replies: message
                .thread
                .as_ref()
                // A broadcast reply's channel copy carries its root's summary.
                .filter(|thread| {
                    !viewer.in_thread && message.thread_root_id.is_none() && thread.reply_count > 0
                })
                .map(|thread| {
                    let count = match thread.reply_count {
                        1 => "1 reply".to_owned(),
                        count => format!("{count} replies"),
                    };
                    // As desktop: "3 replies ● Unread View thread".
                    let unread = if viewer.thread_unread.contains_key(&message.id) {
                        "  <span foreground=\"#e4c76a\">● Unread</span>"
                    } else {
                        ""
                    };
                    format!(
                        "<b>{count}</b>{unread}  <span foreground=\"#b9bcbe\">View thread</span>"
                    )
                }),
            broadcast: !viewer.in_thread && message.broadcast && message.thread_root_id.is_some(),
            // As desktop: a pinned row keeps its pin wash instead.
            focused: false,
            unread: false,
            beginning: false,
            mentioned: !deleted
                && message.pin.is_none()
                && !replies_to_me
                && caper_client::mentions::mentions_me(
                    &message.content.mentions,
                    &message.author.id,
                    viewer.me,
                ),
            replied: !deleted && message.pin.is_none() && replies_to_me,
            team_notice: !deleted
                && caper_client::mentions::notifies_team(&message.content.mentions),
            content: if deleted {
                crate::content::Content::default()
            } else {
                crate::content::Content {
                    scope: Some(format!("{}:{}", viewer.surface, message.id)),
                    attachments: message
                        .content
                        .attachments
                        .iter()
                        .map(|attachment| {
                            (
                                viewer.images.media.resolve(attachment),
                                attachment.spoiler
                                    && !viewer
                                        .revealed
                                        .get(&format!("{}:{}", viewer.surface, message.id))
                                        .is_some_and(|ids| ids.contains(&attachment.id)),
                            )
                        })
                        .collect(),
                    embeds: if message.content.suppress_embeds {
                        Vec::new()
                    } else {
                        message.content.embeds.clone()
                    },
                    gif: message.content.gif.clone(),
                }
            },
            offers: Offers {
                act,
                mine: act && viewer.author == Some(message.author.id.as_str()),
                pinned: message.pin.is_some(),
                thread: act && !viewer.in_thread && message.thread_root_id.is_none(),
            },
            blocked_run: None,
            blocked,
            forward: message
                .forward
                .as_ref()
                .filter(|_| !deleted)
                .map(|forward| {
                    forward.message.as_ref().map(|original| {
                        Original::new(
                            original,
                            viewer.images,
                            viewer.revealed,
                            format!("{}:forward:{}", viewer.surface, message.id),
                            true,
                        )
                    })
                }),
        };
        push(
            &mut rows,
            &message.id,
            &message.author,
            &message.created_at,
            row,
            separate,
        );
    }
    if let Some(draft) = draft {
        let avatar = author_avatar(draft.author);
        let row = Row {
            id: String::new(),
            day: None,
            grouped: false,
            author: draft.author.name.clone(),
            person: draft.author.clone(),
            picture_ready: viewer.images.picture_ready(&avatar),
            avatar,
            time: String::new(),
            markup: crate::markup::message(draft.text, &[]),
            pending: true,
            edited: false,
            pinned_by: None,
            reply: None,
            reply_target: None,
            reply_expanded: false,
            footer: None,
            reactions: Vec::new(),
            reaction_error: None,
            replies: None,
            broadcast: false,
            mentioned: false,
            replied: false,
            focused: false,
            unread: false,
            beginning: false,
            team_notice: false,
            offers: Offers::default(),
            content: draft.content,
            blocked_run: None,
            blocked: false,
            forward: None,
        };
        let separate = std::mem::take(&mut separate_next);
        push(
            &mut rows,
            draft.id,
            draft.author,
            draft.created_at,
            row,
            separate,
        );
    }
    rows
}

/// What a row's controls ask for.
pub enum Action {
    /// Add (`active`) or remove your reaction.
    React {
        id: String,
        emoji: String,
        active: bool,
    },
    /// The full reaction picker, anchored to `anchor`.
    Pick {
        id: String,
        anchor: gtk::Widget,
    },
    /// Send your failed reaction again.
    RetryReaction(String),
    /// Drop your failed reaction and its error.
    DismissReaction(String),
    /// The reply quote: jump to the quoted message (in its thread).
    Quote {
        id: String,
        root: Option<String>,
    },
    /// Show more or less of a reply quote.
    ExpandQuote(String),
    /// "(edited)": the message's retained versions.
    History(String),
    /// A chip is hovered: load who reacted to this message.
    LoadReactors(String),
    Reply(String),
    Thread(String),
    /// Edit, delete and pin, anchored to `anchor`.
    Menu {
        id: String,
        anchor: gtk::Widget,
    },
    /// Show or re-hide this attachment only at its originating message/presentation.
    Reveal {
        scope: String,
        id: String,
        revealed: bool,
    },
    /// Show these images in the viewer, starting at the index.
    View(Vec<caper_client::model::Attachment>, usize, Option<String>),
    /// Open the full file in the browser or the system's player.
    Open(caper_client::model::Attachment),
    /// An image's own menu (Copy image, Copy image link, Open image in
    /// browser), anchored to its tile.
    ImageMenu {
        attachment: caper_client::model::Attachment,
        anchor: gtk::Widget,
    },
    /// Open an http(s) link.
    OpenUrl(String),
    /// A link preview's player: Caper's `play` page in the web player, or
    /// `fallback` in the browser when it cannot play here.
    PlayEmbed {
        play: String,
        provider: String,
        size: (i32, i32),
        fallback: String,
    },
    /// Show or hide a run of blocked messages.
    ToggleBlocked(String),
    /// Open the forwarded conversation behind this forward.
    ViewForward(String),
    /// A person's profile card, from an author or a mention pill.
    Profile {
        id: String,
        /// The message's author, when the click was on it.
        author: Option<Author>,
        anchor: gtk::Widget,
    },
}

pub type Handler = Rc<dyn Fn(Action)>;

/// One recycled row's widgets and the row it shows.
struct Widgets {
    root: gtk::Box,
    day: gtk::Label,
    /// The day's centered label between two rules.
    day_row: gtk::Box,
    unread: gtk::Box,
    beginning: gtk::Label,
    /// The quote card: its avatar, author and "View original" heading, its
    /// text, and Show more/less.
    reply: gtk::Box,
    /// `Row::footer`.
    footer: gtk::Label,
    quote_heading: gtk::Box,
    quote_avatar: AvatarView,
    quote_author: gtk::Label,
    quote_view: gtk::Label,
    quote_text: gtk::Label,
    quote_more: gtk::Button,
    avatar: AvatarView,
    header: gtk::Box,
    author: gtk::Label,
    time: gtk::Label,
    /// "(edited)" in the header, and a grouped row's inline marker: both
    /// open the message's history.
    edited: gtk::Button,
    edited_inline: gtk::Button,
    pinned: gtk::Label,
    body: gtk::Label,
    content: gtk::Box,
    forward: gtk::Box,
    notice: gtk::Label,
    card: gtk::Overlay,
    reactions: gtk::Box,
    reaction_error: gtk::Box,
    reaction_error_label: gtk::Label,
    retry_reaction: gtk::Button,
    dismiss_reaction: gtk::Button,
    replies: gtk::Button,
    thread_context: gtk::Button,
    /// Holds the toolbar: hover shows it, `offers` hides it.
    tools: gtk::Box,
    /// Built when the pointer or keyboard focus first enters the row.
    toolbar: RefCell<Option<Toolbar>>,
    /// The blocked run notice and its Show/Hide.
    blocked: gtk::Box,
    blocked_label: gtk::Label,
    blocked_toggle: gtk::Button,
    row: RefCell<Option<Row>>,
}

/// A row's hover toolbar. It is about a quarter of a row's widgets, and GTK
/// 4.8 lays out every row a page of history adds in one frame, so a row
/// builds it only when the pointer or keyboard focus first enters it; Tab
/// still walks into it, as on web.
struct Toolbar {
    /// Web's "Actions for {author}'s message" group, renamed in `dress`; its
    /// widget name is the author it was last named for.
    group: gtk::Box,
    thread: gtk::Button,
    reply: gtk::Button,
    more: gtk::Button,
}

/// The toolbar as an accessible group named `name`.
fn group(name: &str) -> gtk::Box {
    let xml = format!(
        "<interface><object class=\"GtkBox\" id=\"widget\"><property name=\"accessible-role\">group</property><accessibility><property name=\"label\">{}</property></accessibility></object></interface>",
        glib::markup_escape_text(name)
    );
    let group: gtk::Box = gtk::Builder::from_string(&xml)
        .object("widget")
        .expect("a GTK box");
    group.add_css_class("tools");
    group
}

/// A row's fixed tooltip, filled in when it first shows. GTK 4.8's
/// `set_tooltip_text` also sets an accessible description, and doing that
/// for every row's controls cost more than building the rest of the row.
/// These controls already carry the same text as their name.
fn tooltip(widget: &impl IsA<gtk::Widget>, text: &str) {
    let text = text.to_owned();
    widget.set_has_tooltip(true);
    widget.connect_query_tooltip(move |_, _, _, _, tooltip| {
        tooltip.set_text(Some(&text));
        true
    });
}

fn tool(images: &Images, icon: Icon, label: &str) -> gtk::Button {
    let button = crate::app::named::<gtk::Button>("GtkButton", label);
    // At its own size: a filled 28px button would scale it up.
    let image = images.icon_image(icon, "#b9bcbe", 16);
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    button.set_child(Some(&image));
    tooltip(&button, label);
    button.add_css_class("flat");
    button.add_css_class("tool");
    button
}

/// Recycled message row widgets: each list item builds its widgets once and
/// fills them whenever it is bound to another row.
///
/// GTK 4.8 gives every row a page of history brings into range a new list
/// item, all in the frame that page lands in (about 1 ms each). So torn-down
/// rows' widgets are kept for new items, and idle time builds `spare` ahead.
pub fn factory(images: Rc<Images>, handler: Handler, spare: usize) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let pool: Rc<RefCell<Vec<Rc<Widgets>>>> = Rc::default();
    let live: Rc<RefCell<HashMap<gtk::Widget, Rc<Widgets>>>> = Rc::default();
    let filling = Rc::new(std::cell::Cell::new(false));
    let refill = {
        let (pool, images, handler) = (pool.clone(), images.clone(), handler.clone());
        move || {
            if filling.get() || pool.borrow().len() >= spare {
                return;
            }
            filling.set(true);
            let (pool, images, handler, filling) = (
                pool.clone(),
                images.clone(),
                handler.clone(),
                filling.clone(),
            );
            // One row per pass, after anything else waiting, frames included.
            glib::idle_add_local_full(glib::Priority::LOW, move || {
                let widgets = Rc::new(build_widgets());
                connect(&widgets, &images, &handler);
                pool.borrow_mut().push(widgets);
                if pool.borrow().len() < spare {
                    return glib::ControlFlow::Continue;
                }
                filling.set(false);
                glib::ControlFlow::Break
            });
        }
    };
    refill();
    let setup_live = live.clone();
    let setup_pool = pool.clone();
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        item.set_activatable(false);
        item.set_selectable(false);
        let spare = setup_pool.borrow_mut().pop();
        let widgets = spare.unwrap_or_else(|| {
            let widgets = Rc::new(build_widgets());
            connect(&widgets, &images, &handler);
            widgets
        });
        item.set_child(Some(&widgets.root));
        setup_live
            .borrow_mut()
            .insert(widgets.root.clone().upcast(), widgets.clone());
        refill();
        // GTK 4.12 names list rows; 4.8 has no such property, and the row's
        // labels carry the same text there. The description lists the
        // toolbar, which a row builds once focus or the pointer enters it.
        let labelled = item.find_property("accessible-label").is_some();
        if item.find_property("accessible-description").is_some() {
            item.set_property(
                "accessible-description",
                "Press Tab for reactions, Reply in thread, Reply and Message actions",
            );
        }
        let (images, handler) = (images.clone(), handler.clone());
        item.connect_item_notify(move |item| {
            if let Some(object) = item.item().and_downcast::<glib::BoxedAnyObject>() {
                let row = object.borrow::<Row>().clone();
                if labelled {
                    item.set_property("accessible-label", row.accessible_label());
                }
                fill(&widgets, &images, &row, &handler);
                *widgets.row.borrow_mut() = Some(row);
            }
        });
    });
    // Twice the spares at most: a cleared list hands back every row it had.
    factory.connect_teardown(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(child) = item.child() else {
            return;
        };
        item.set_child(None::<&gtk::Widget>);
        if let Some(widgets) = live.borrow_mut().remove(&child)
            && pool.borrow().len() < spare * 2
        {
            *widgets.row.borrow_mut() = None;
            pool.borrow_mut().push(widgets);
        }
    });
    factory
}

/// One row outside a list (Pins): its widgets live as long as this does.
pub struct Standalone(Rc<Widgets>);

impl Standalone {
    pub fn new(images: &Rc<Images>, row: &Row, handler: &Handler) -> Self {
        let widgets = Rc::new(build_widgets());
        connect(&widgets, images, handler);
        fill(&widgets, images, row, handler);
        *widgets.row.borrow_mut() = Some(row.clone());
        Self(widgets)
    }

    pub fn root(&self) -> &gtk::Box {
        &self.0.root
    }
}

fn build_widgets() -> Widgets {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    // As desktop: the date centered between two rules.
    let day = gtk::Label::builder().css_classes(["day"]).build();
    let day_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    day_row.add_css_class("day-row");
    for (index, _) in [(), ()].iter().enumerate() {
        let rule = gtk::Separator::new(gtk::Orientation::Horizontal);
        rule.set_hexpand(true);
        rule.set_valign(gtk::Align::Center);
        day_row.append(&rule);
        if index == 0 {
            day_row.append(&day);
        }
    }
    let reply = gtk::Box::new(gtk::Orientation::Vertical, 4);
    reply.add_css_class("quote");
    let quote_heading = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    tooltip(&quote_heading, "View original message");
    let quote_avatar = AvatarView::new(16, false);
    quote_heading.append(&quote_avatar.root);
    let quote_author = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .css_classes(["quote-author"])
        .build();
    quote_heading.append(&quote_author);
    let quote_view = gtk::Label::builder()
        .label("View original")
        .hexpand(true)
        .xalign(1.0)
        .css_classes(["quote-view"])
        .build();
    quote_heading.append(&quote_view);
    reply.append(&quote_heading);
    let quote_text = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .css_classes(["quote-text"])
        .build();
    reply.append(&quote_text);
    // A terracotta rule with "New messages" at its end.
    let unread = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    unread.add_css_class("new-messages");
    let rule = gtk::Separator::new(gtk::Orientation::Horizontal);
    rule.set_hexpand(true);
    rule.set_valign(gtk::Align::Center);
    unread.append(&rule);
    unread.append(&gtk::Label::new(Some("New messages")));
    unread.set_visible(false);
    let beginning = gtk::Label::builder()
        .label("Beginning of conversation")
        .css_classes(["history-start"])
        .visible(false)
        .build();
    // As desktop: a 34px avatar, 10px from the text. The avatar overlays
    // the text's margin rather than sitting beside it in a horizontal box:
    // GTK 4.8 measures a horizontal box's wrapping text by searching its
    // width for each height, which made every new page of rows stall.
    let avatar = AvatarView::new(34, false);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.set_hexpand(true);
    content.set_margin_start(44);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = |class: &str| {
        gtk::Label::builder()
            .xalign(0.0)
            .css_classes([class])
            .build()
    };
    let author = label("author");
    let time = label("time");
    let marker = || {
        let button: gtk::Button = crate::app::named("GtkButton", "Message history");
        button.set_label("(edited)");
        tooltip(&button, "Message history");
        button.add_css_class("flat");
        button.add_css_class("edited");
        button
    };
    let edited = marker();
    let edited_inline = marker();
    edited_inline.set_halign(gtk::Align::Start);
    let pinned = label("pinned");
    let body = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .selectable(true)
        .css_classes(["body"])
        .build();
    let notice = label("notice");
    notice.set_label(caper_client::mentions::FEEDBACK_NOTICE);
    let reactions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    reactions.add_css_class("reactions");
    let reaction_error = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    reaction_error.add_css_class("reaction-error");
    let reaction_error_label = label("error");
    let retry_reaction = gtk::Button::with_label("Retry");
    let dismiss_reaction = gtk::Button::with_label("Dismiss");
    for button in [&retry_reaction, &dismiss_reaction] {
        button.add_css_class("flat");
        button.add_css_class("inline");
    }
    reaction_error.append(&reaction_error_label);
    reaction_error.append(&retry_reaction);
    reaction_error.append(&dismiss_reaction);
    let thread_context: gtk::Button =
        crate::app::named("GtkButton", "Replied to a thread · View thread");
    thread_context.set_label("Replied to a thread · View thread");
    thread_context.add_css_class("flat");
    thread_context.add_css_class("thread-context");
    thread_context.set_halign(gtk::Align::Start);
    thread_context.set_visible(false);
    let replies = gtk::Button::new();
    replies.set_child(Some(&gtk::Label::new(None)));
    replies.add_css_class("flat");
    replies.add_css_class("replies");
    replies.set_halign(gtk::Align::Start);
    header.append(&author);
    header.append(&time);
    header.append(&edited);
    header.append(&pinned);
    content.append(&header);
    let extra = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&body);
    content.append(&edited_inline);
    content.append(&extra);
    let forward = gtk::Box::new(gtk::Orientation::Vertical, 4);
    forward.add_css_class("forward-card");
    content.append(&forward);
    content.append(&notice);
    content.append(&reactions);
    content.append(&reaction_error);
    content.append(&replies);

    let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    slot.set_halign(gtk::Align::End);
    slot.set_valign(gtk::Align::Start);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&content));
    overlay.add_overlay(&avatar.root);
    overlay.add_overlay(&slot);
    let blocked = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    blocked.add_css_class("blocked-run");
    let blocked_label = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(["muted", "small"])
        .build();
    let blocked_toggle = gtk::Button::with_label("Show");
    blocked_toggle.add_css_class("flat");
    blocked_toggle.add_css_class("inline");
    blocked.append(&blocked_label);
    blocked.append(&blocked_toggle);
    root.append(&beginning);
    root.append(&blocked);
    root.append(&day_row);
    root.append(&unread);
    // The message itself, below any day rule or divider: what a mention or
    // focus wash covers, edge to edge as desktop.
    let message = gtk::Box::new(gtk::Orientation::Vertical, 0);
    message.add_css_class("body");
    message.append(&reply);
    let quote_more = gtk::Button::with_label("Show more");
    quote_more.add_css_class("flat");
    quote_more.add_css_class("quote-more");
    quote_more.set_halign(gtk::Align::Start);
    quote_more.set_visible(false);
    reply.append(&quote_more);
    message.append(&thread_context);
    message.append(&overlay);
    root.append(&message);
    let footer = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["thread-empty"])
        .visible(false)
        .build();
    root.append(&footer);
    Widgets {
        blocked,
        blocked_label,
        blocked_toggle,
        notice,
        card: overlay.clone(),
        root,
        day,
        day_row,
        unread,
        beginning,
        reply,
        footer,
        quote_heading,
        quote_avatar,
        quote_author,
        quote_view,
        quote_text,
        quote_more,
        avatar,
        header,
        author,
        time,
        edited,
        edited_inline,
        pinned,
        body,
        content: extra,
        forward,
        reactions,
        reaction_error,
        reaction_error_label,
        retry_reaction,
        dismiss_reaction,
        replies,
        thread_context,
        tools: slot,
        toolbar: RefCell::new(None),
        row: RefCell::new(None),
    }
}

/// Builds `widgets`' toolbar if it has none yet, dressed for its row.
fn toolbar(widgets: &Rc<Widgets>, images: &Images, handler: &Handler) {
    if widgets.toolbar.borrow().is_some() {
        return;
    }
    // Web's group, "Actions for {author}'s message" (named per message in
    // `dress`), so a screen reader hears whose tools these are.
    let tools = group("Message actions");
    let quick: Vec<gtk::Button> = QUICK_REACTIONS
        .iter()
        .map(|emoji| {
            let button =
                crate::app::named::<gtk::Button>("GtkButton", &format!("React with {emoji}"));
            // A child label, not `set_label`, which would rename the button.
            button.set_child(Some(&gtk::Label::new(Some(emoji))));
            tooltip(&button, &format!("React with {emoji}"));
            button.add_css_class("flat");
            button.add_css_class("tool");
            button.add_css_class("quick");
            button.set_focus_on_click(false);
            tools.append(&button);
            button
        })
        .collect();
    let pick = tool(images, Icon::SmilePlus, "Add reaction");
    let reply = tool(images, Icon::Reply, "Reply");
    let thread = tool(images, Icon::Thread, "Reply in thread");
    let more = tool(images, Icon::More, "More actions");
    // Desktop's order: thread, reply, add reaction, more.
    for button in [&thread, &reply, &pick, &more] {
        // A click should not leave the toolbar pinned open by focus; keyboard
        // focus still reveals it through `:focus-within`.
        button.set_focus_on_click(false);
        tools.append(button);
    }
    for (button, emoji) in quick.iter().zip(QUICK_REACTIONS) {
        let (weak, handler) = (Rc::downgrade(widgets), handler.clone());
        button.connect_clicked(move |_| {
            let Some(widgets) = weak.upgrade() else {
                return;
            };
            let row = widgets.row.borrow().clone();
            if let Some(row) = row {
                let mine = row
                    .reactions
                    .iter()
                    .any(|chip| chip.emoji == emoji && chip.mine);
                handler(Action::React {
                    id: row.id,
                    emoji: emoji.to_owned(),
                    active: !mine,
                });
            }
        });
    }
    let actions: [(&gtk::Button, Make); 4] = [
        (&pick, |id, anchor| Action::Pick { id, anchor }),
        (&reply, |id, _| Action::Reply(id)),
        (&thread, |id, _| Action::Thread(id)),
        (&more, |id, anchor| Action::Menu { id, anchor }),
    ];
    connect_actions(widgets, handler, &actions);
    widgets.tools.append(&tools);
    let toolbar = Toolbar {
        group: tools,
        thread,
        reply,
        more,
    };
    if let Some(row) = widgets.row.borrow().as_ref() {
        dress(&toolbar, row);
    }
    *widgets.toolbar.borrow_mut() = Some(toolbar);
}

type Make = fn(String, gtk::Widget) -> Action;

/// Each button sends its action for the row it shows now.
fn connect_actions(widgets: &Rc<Widgets>, handler: &Handler, actions: &[(&gtk::Button, Make)]) {
    let id = |widgets: &Widgets| widgets.row.borrow().as_ref().map(|row| row.id.clone());
    for (button, make) in actions {
        let make = *make;
        let (weak, handler) = (Rc::downgrade(widgets), handler.clone());
        button.connect_clicked(move |button| {
            if let Some(id) = weak.upgrade().and_then(|widgets| id(&widgets)) {
                handler(make(id, button.clone().upcast()));
            }
        });
    }
}

fn connect(widgets: &Rc<Widgets>, images: &Rc<Images>, handler: &Handler) {
    let id = |widgets: &Widgets| widgets.row.borrow().as_ref().map(|row| row.id.clone());
    // The toolbar, on first hover or focus.
    let motion = gtk::EventControllerMotion::new();
    let focus = gtk::EventControllerFocus::new();
    for (controller, enter) in [
        (motion.upcast_ref::<gtk::EventController>(), "enter"),
        (focus.upcast_ref(), "enter"),
    ] {
        let (weak, images, handler) = (Rc::downgrade(widgets), images.clone(), handler.clone());
        controller.connect_local(enter, false, move |_| {
            if let Some(widgets) = weak.upgrade() {
                toolbar(&widgets, &images, &handler);
            }
            None
        });
    }
    widgets.root.add_controller(motion);
    widgets.root.add_controller(focus);
    let actions: [(&gtk::Button, Make); 6] = [
        (&widgets.edited, |id, _| Action::History(id)),
        (&widgets.edited_inline, |id, _| Action::History(id)),
        (&widgets.thread_context, |id, _| Action::Thread(id)),
        (&widgets.retry_reaction, |id, _| Action::RetryReaction(id)),
        (&widgets.dismiss_reaction, |id, _| {
            Action::DismissReaction(id)
        }),
        (&widgets.replies, |id, _| Action::Thread(id)),
    ];
    connect_actions(widgets, handler, &actions);
    for target in [
        widgets.author.clone().upcast::<gtk::Widget>(),
        widgets.avatar.root.clone().upcast(),
    ] {
        let click = gtk::GestureClick::new();
        let (weak, handler, anchor) = (Rc::downgrade(widgets), handler.clone(), target.clone());
        click.connect_released(move |_, _, _, _| {
            let person = weak
                .upgrade()
                .and_then(|widgets| widgets.row.borrow().as_ref().map(|row| row.person.clone()));
            if let Some(person) = person.filter(|person| !person.is_guest) {
                handler(Action::Profile {
                    id: person.id.clone(),
                    author: Some(person),
                    anchor: anchor.clone(),
                });
            }
        });
        target.add_controller(click);
        target.set_cursor_from_name(Some("pointer"));
    }
    let (handler_link, anchor) = (handler.clone(), widgets.body.clone());
    widgets.body.connect_activate_link(move |_, uri| {
        if let Some(id) = uri.strip_prefix(crate::markup::PERSON) {
            handler_link(Action::Profile {
                id: id.to_owned(),
                author: None,
                anchor: anchor.clone().upcast(),
            });
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    let click = gtk::GestureClick::new();
    let (weak, jump) = (Rc::downgrade(widgets), handler.clone());
    click.connect_released(move |_, _, _, _| {
        let target = weak
            .upgrade()
            .and_then(|widgets| widgets.row.borrow().as_ref()?.reply_target.clone());
        if let Some((id, root)) = target {
            jump(Action::Quote { id, root });
        }
    });
    widgets.quote_heading.add_controller(click);
    widgets.quote_heading.set_cursor_from_name(Some("pointer"));
    let click = gtk::GestureClick::new();
    let (weak, jump) = (Rc::downgrade(widgets), handler.clone());
    click.connect_released(move |_, _, _, _| {
        let target = weak
            .upgrade()
            .and_then(|widgets| widgets.row.borrow().as_ref()?.reply_target.clone());
        if let Some((id, root)) = target {
            jump(Action::Quote { id, root });
        }
    });
    widgets.quote_text.add_controller(click);
    widgets.quote_text.set_cursor_from_name(Some("pointer"));
    let (weak, toggle) = (Rc::downgrade(widgets), handler.clone());
    widgets.quote_more.connect_clicked(move |_| {
        if let Some(id) = weak.upgrade().and_then(|widgets| id(&widgets)) {
            toggle(Action::ExpandQuote(id));
        }
    });
    let (weak, toggle) = (Rc::downgrade(widgets), handler.clone());
    widgets.blocked_toggle.connect_clicked(move |_| {
        let key = weak.upgrade().and_then(|widgets| {
            widgets
                .row
                .borrow()
                .as_ref()
                .and_then(|row| row.blocked_run.as_ref().map(|(key, _, _)| key.clone()))
        });
        if let Some(key) = key {
            toggle(Action::ToggleBlocked(key));
        }
    });
}

/// The quote card, as desktop: a deleted original names its author and says
/// so; a blocked author's quote says only "Blocked message". Show more
/// appears once the text runs past three lines.
fn fill_quote(widgets: &Widgets, images: &Rc<Images>, row: &Row) {
    let Some(quote) = &row.reply else {
        widgets.reply.set_visible(false);
        return;
    };
    widgets.reply.set_visible(true);
    widgets
        .quote_avatar
        .root
        .set_visible(quote.avatar.is_some());
    if let Some(avatar) = &quote.avatar {
        widgets.quote_avatar.set(images, avatar);
    }
    widgets.quote_author.set_label(&quote.author);
    // Only a quote with an original to show opens it.
    let opens = row.reply_target.is_some() && !quote.deleted;
    widgets.quote_view.set_visible(opens);
    widgets.quote_heading.set_sensitive(opens);
    let text = match &quote.text {
        Some(text) => Some(text.as_str()),
        None if quote.deleted && quote.avatar.is_some() => Some("Original message was deleted."),
        None => None,
    };
    widgets.quote_text.set_visible(text.is_some());
    widgets.quote_text.set_label(text.unwrap_or_default());
    if quote.deleted {
        widgets.quote_text.add_css_class("deleted");
    } else {
        widgets.quote_text.remove_css_class("deleted");
    }
    let expanded = row.reply_expanded;
    // Underlined as desktop draws it; GTK 4.8 ignores CSS decoration here.
    if let Some(label) = widgets.quote_more.child().and_downcast::<gtk::Label>() {
        label.set_markup(if expanded {
            "<u>Show less</u>"
        } else {
            "<u>Show more</u>"
        });
    }
    widgets.quote_more.set_visible(expanded);
    let Some(full) = quote.text.clone().filter(|_| !expanded) else {
        return;
    };
    // As desktop, three rows at most. Pango limits lines per paragraph, so
    // the cut is measured here once the card has a width; until then it
    // shows the first three paragraphs.
    let start: String = full.split('\n').take(3).collect::<Vec<_>>().join("\n");
    widgets.quote_text.set_label(&start);
    let (label, more) = (widgets.quote_text.clone(), widgets.quote_more.clone());
    widgets.quote_text.add_tick_callback(move |_, _| {
        let width = label.width();
        if width <= 0 {
            return glib::ControlFlow::Continue;
        }
        // A recycled row shows another quote now.
        if label.label() != start {
            return glib::ControlFlow::Break;
        }
        let layout = label.create_pango_layout(Some(&full));
        layout.set_width(width * pango::SCALE);
        layout.set_wrap(pango::WrapMode::WordChar);
        let cut = (layout.line_count() > 3)
            .then(|| layout.line_readonly(2))
            .flatten()
            .and_then(|line| usize::try_from(line.start_index() + line.length()).ok());
        match cut {
            Some(end) => {
                let kept = full.get(..end).unwrap_or(&full).trim_end();
                label.set_label(&format!("{kept}…"));
                more.set_visible(true);
            }
            None => label.set_label(&full),
        }
        glib::ControlFlow::Break
    });
}

fn fill(widgets: &Widgets, images: &Rc<Images>, row: &Row, handler: &Handler) {
    let mut classes = vec!["message"];
    if row.grouped {
        classes.push("grouped");
    }
    if row.pending {
        classes.push("pending");
    }
    if row.blocked {
        classes.push("blocked");
    }
    // The wash: the focused message wins, as desktop.
    if row.blocked_run.is_none() {
        if row.focused {
            classes.push("focused");
        } else if row.replied {
            classes.push("replied");
        } else if row.mentioned {
            classes.push("mentioned");
        }
    }
    // Nothing above the message: the wash reaches into the row's top padding.
    if !row.beginning && !row.unread && row.day.is_none() {
        classes.push("flush");
    }
    widgets.root.set_css_classes(&classes);
    widgets.beginning.set_visible(row.beginning);
    widgets.blocked.set_visible(row.blocked_run.is_some());
    widgets.card.set_visible(row.blocked_run.is_none());
    if let Some((_, label, shown)) = &row.blocked_run {
        widgets.blocked_label.set_label(&format!("⊘ {label} —"));
        widgets
            .blocked_toggle
            .set_label(if *shown { "Hide" } else { "Show" });
        widgets.day_row.set_visible(false);
        widgets.reply.set_visible(false);
        widgets.quote_more.set_visible(false);
        return;
    }
    widgets.notice.set_visible(row.team_notice);
    widgets.unread.set_visible(row.unread);
    widgets.day_row.set_visible(row.day.is_some());
    widgets
        .day
        .set_label(row.day.as_deref().unwrap_or_default());
    fill_quote(widgets, images, row);
    widgets.footer.set_visible(row.footer.is_some());
    widgets
        .footer
        .set_label(row.footer.as_deref().unwrap_or_default());
    widgets.header.set_visible(!row.grouped);
    widgets.avatar.root.set_visible(!row.grouped);
    if !row.grouped {
        widgets.avatar.set(images, &row.avatar);
    }
    widgets.author.set_label(&row.author);
    widgets.time.set_label(&row.time);
    widgets.edited.set_visible(row.edited);
    widgets.edited_inline.set_visible(row.edited && row.grouped);
    widgets.pinned.set_visible(row.pinned_by.is_some());
    widgets
        .pinned
        .set_label(row.pinned_by.as_deref().unwrap_or_default());
    widgets.body.set_markup(&row.markup);
    widgets.body.set_visible(!row.markup.is_empty());
    while let Some(child) = widgets.content.first_child() {
        widgets.content.remove(&child);
    }
    if let Some(view) = crate::content::view(images, &row.content, handler) {
        widgets.content.append(&view);
    }
    widgets.content.set_visible(!row.content.is_empty());
    while let Some(child) = widgets.forward.first_child() {
        widgets.forward.remove(&child);
    }
    widgets.forward.set_visible(row.forward.is_some());
    if let Some(original) = &row.forward {
        widgets.forward.append(
            &gtk::Label::builder()
                .label("Forwarded")
                .xalign(0.0)
                .css_classes(["muted", "small"])
                .build(),
        );
        match original {
            Some(original) => {
                widgets
                    .forward
                    .append(&original.view(images, handler, Some(&row.id)))
            }
            None => widgets.forward.append(
                &gtk::Label::builder()
                    .label("Original conversation unavailable.")
                    .xalign(0.0)
                    .css_classes(["muted"])
                    .build(),
            ),
        }
    }
    // Only the hover cards changed (names loaded): update them in place so
    // a card that is showing follows.
    let same_chips = widgets.row.borrow().as_ref().is_some_and(|shown| {
        shown.id == row.id
            && shown.offers.act == row.offers.act
            && shown.reactions.len() == row.reactions.len()
            && shown
                .reactions
                .iter()
                .zip(&row.reactions)
                .all(|(a, b)| (&a.emoji, a.count, a.mine) == (&b.emoji, b.count, b.mine))
    });
    if same_chips {
        let mut child = widgets.reactions.first_child();
        for chip in &row.reactions {
            let Some(button) = child else { break };
            button.set_tooltip_text(Some(&chip.summary));
            child = button.next_sibling();
        }
    } else {
        while let Some(child) = widgets.reactions.first_child() {
            widgets.reactions.remove(&child);
        }
    }
    for chip in row.reactions.iter().filter(|_| !same_chips) {
        let button = gtk::Button::with_label(&format!("{} {}", chip.emoji, chip.count));
        button.add_css_class("reaction");
        if chip.mine {
            button.add_css_class("mine");
        }
        button.set_sensitive(row.offers.act);
        button.set_tooltip_text(Some(&chip.summary));
        // Discord-style: the emoji large, then who reacted. Hovering is
        // what loads the names.
        let (load, id, emoji) = (handler.clone(), row.id.clone(), chip.emoji.clone());
        button.connect_query_tooltip(move |button, _, _, _, tooltip| {
            load(Action::LoadReactors(id.clone()));
            let card = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            card.add_css_class("reactor-card");
            let big = gtk::Label::new(Some(&emoji));
            big.add_css_class("reactor-emoji");
            let summary = gtk::Label::new(button.tooltip_text().as_deref());
            summary.set_wrap(true);
            summary.set_max_width_chars(32);
            summary.set_xalign(0.0);
            card.append(&big);
            card.append(&summary);
            tooltip.set_custom(Some(&card));
            true
        });
        let (handler, id, emoji, active) = (
            handler.clone(),
            row.id.clone(),
            chip.emoji.clone(),
            !chip.mine,
        );
        button.connect_clicked(move |_| {
            handler(Action::React {
                id: id.clone(),
                emoji: emoji.clone(),
                active,
            })
        });
        widgets.reactions.append(&button);
    }
    widgets.reactions.set_visible(!row.reactions.is_empty());
    widgets
        .reaction_error
        .set_visible(row.reaction_error.is_some());
    widgets
        .reaction_error_label
        .set_label(row.reaction_error.as_deref().unwrap_or_default());
    widgets.retry_reaction.set_visible(row.offers.act);
    widgets.replies.set_visible(row.replies.is_some());
    widgets.thread_context.set_visible(row.broadcast);
    if let Some(label) = widgets
        .replies
        .child()
        .and_then(|child| child.downcast::<gtk::Label>().ok())
    {
        label.set_markup(row.replies.as_deref().unwrap_or_default());
    }
    widgets.tools.set_visible(row.offers.act);
    if let Some(toolbar) = widgets.toolbar.borrow().as_ref() {
        dress(toolbar, row);
    }
}

/// The toolbar for `row`: web's labels, renamed when the author changes
/// (rows are recycled; the group's widget name remembers whose they are),
/// and the actions it offers.
fn dress(toolbar: &Toolbar, row: &Row) {
    let author = &row.author;
    if toolbar.group.widget_name() != author.as_str() {
        toolbar.group.set_widget_name(author);
        for (widget, text) in [
            (
                toolbar.group.upcast_ref::<gtk::Widget>(),
                format!("Actions for {author}'s message"),
            ),
            (
                toolbar.thread.upcast_ref(),
                format!("Reply in thread to {author}"),
            ),
            (toolbar.reply.upcast_ref(), format!("Reply to {author}")),
            (
                toolbar.more.upcast_ref(),
                format!("Message actions for {author}"),
            ),
        ] {
            crate::app::rename(widget, &text);
        }
    }
    toolbar.thread.set_visible(row.offers.thread);
    toolbar.more.set_visible(row.offers.act);
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
            person: Author {
                id: "maya".into(),
                avatar_id: Some(31),
                avatar_asset: None,
                avatar_url: None,
                name: "Maya".into(),
                is_guest: false,
                admin: false,
            },
            avatar: crate::images::Avatar::new("Maya", Some(31), &None, &None),
            picture_ready: false,
            time: String::new(),
            markup: text.into(),
            pending: false,
            edited: false,
            pinned_by: None,
            reply: None,
            reply_target: None,
            reply_expanded: false,
            footer: None,
            reactions: Vec::new(),
            reaction_error: None,
            replies: None,
            broadcast: false,
            mentioned: false,
            replied: false,
            focused: false,
            unread: false,
            beginning: false,
            team_notice: false,
            offers: Offers::default(),
            content: Default::default(),
            blocked_run: None,
            blocked: false,
            forward: None,
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
    fn rows_are_named_by_author_time_and_plain_text() {
        let mut message = row("a", "see <b>you</b> at noon &amp; bring <i>snacks</i>");
        message.time = "2:15 AM".into();
        assert_eq!(
            message.accessible_label(),
            "Maya, 2:15 AM, see you at noon & bring snacks"
        );
        let maya = Mention {
            kind: "user".into(),
            id: Some("id".into()),
            username: Some("maya".into()),
        };
        // A mention pill and a link: both `<a>` tags.
        message.markup = crate::markup::message("hi @maya, see www.caper.chat", &[maya]);
        assert!(message.markup.contains("<a href"));
        assert_eq!(
            message.accessible_label(),
            "Maya, 2:15 AM, hi @maya, see www.caper.chat"
        );
        message.markup.clear();
        message.edited = true;
        message.pending = true;
        assert_eq!(message.accessible_label(), "Maya, 2:15 AM, edited, sending");
        message.blocked_run = Some(("k".into(), "2 blocked messages".into(), false));
        assert_eq!(message.accessible_label(), "2 blocked messages");
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
    fn reactions_pictures_and_edits_replace_only_their_row() {
        let old = rows(&["a", "b", "c", "d"]);
        let mut new = old.clone();
        new.insert(2, row("x", ""));
        new[0].reactions.push(Chip {
            emoji: "👍".into(),
            count: 1,
            mine: true,
            summary: "You reacted with :thumbs-up:".into(),
        });
        new[4].markup = "edited".into();
        new[4].picture_ready = true;
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
}
