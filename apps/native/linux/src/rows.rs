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
    /// The quoted message: its author and a short plain preview.
    pub reply: Option<(String, String)>,
    pub reactions: Vec<Chip>,
    /// Your reaction failed: shown under the row with Retry and Dismiss.
    pub reaction_error: Option<String>,
    /// "3 replies", on a thread root in the main list.
    pub replies: Option<String>,
    /// It mentions you or replies to you: a terracotta-tinted row.
    pub mentioned: bool,
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
        revealed: &std::collections::HashSet<String>,
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
                    attachments: message
                        .content
                        .attachments
                        .iter()
                        .map(|attachment| {
                            (
                                images.media.resolve(attachment),
                                attachment.spoiler && !revealed.contains(&attachment.id),
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
    pub fn view(&self, images: &Images, handler: &Handler, open: Option<&str>) -> gtk::Box {
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
            reactions: Vec::new(),
            reaction_error: None,
            replies: None,
            mentioned: false,
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
    pub revealed: &'a std::collections::HashSet<String>,
    /// People you blocked, and the blocked runs you chose to show.
    pub blocked: &'a std::collections::BTreeSet<String>,
    pub shown_blocked: &'a std::collections::HashSet<String>,
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
        // A reply quote or pin keeps its header, like desktop.
        let header = row.reply.is_some() || row.pinned_by.is_some();
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
            reply: message.reply_to.as_ref().map(|reply| {
                if viewer.blocked.contains(&reply.author.id) {
                    return ("Blocked message".to_owned(), String::new());
                }
                let text = if reply.deleted {
                    "Original message was deleted".to_owned()
                } else if reply.text.trim().is_empty() && reply.has_attachments {
                    caper_client::model::attachments_label(&reply.attachments).to_owned()
                } else {
                    preview(&reply.text)
                };
                (reply.author.name.clone(), text)
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
                        emoji: reaction.emoji,
                    })
                    .collect()
            },
            reaction_error: viewer.reactions.error(&message.id).map(str::to_owned),
            replies: message
                .thread
                .as_ref()
                .filter(|thread| !viewer.in_thread && thread.reply_count > 0)
                .map(|thread| match thread.reply_count {
                    1 => "1 reply".to_owned(),
                    count => format!("{count} replies"),
                }),
            // As desktop: a pinned row keeps its pin wash instead.
            mentioned: !deleted
                && message.pin.is_none()
                && (replies_to_me
                    || caper_client::mentions::mentions_me(
                        &message.content.mentions,
                        &message.author.id,
                        viewer.me,
                    )),
            team_notice: !deleted
                && caper_client::mentions::notifies_team(&message.content.mentions),
            content: if deleted {
                crate::content::Content::default()
            } else {
                crate::content::Content {
                    attachments: message
                        .content
                        .attachments
                        .iter()
                        .map(|attachment| {
                            (
                                viewer.images.media.resolve(attachment),
                                attachment.spoiler && !viewer.revealed.contains(&attachment.id),
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
                        Original::new(original, viewer.images, viewer.revealed, true)
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
            reactions: Vec::new(),
            reaction_error: None,
            replies: None,
            mentioned: false,
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

/// A reply quote: the first line, at most 120 characters.
fn preview(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    let mut short: String = line.chars().take(120).collect();
    if short.len() < line.len() || text.lines().nth(1).is_some() {
        short.push('…');
    }
    short
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
    Reply(String),
    Thread(String),
    /// Edit, delete and pin, anchored to `anchor`.
    Menu {
        id: String,
        anchor: gtk::Widget,
    },
    /// Show this spoiler attachment.
    Reveal(String),
    /// Show these images in the viewer, starting at the index.
    View(Vec<caper_client::model::Attachment>, usize),
    /// Open the full file in the browser or the system's player.
    Open(caper_client::model::Attachment),
    /// Open an http(s) link.
    OpenUrl(String),
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
    reply: gtk::Label,
    avatar: AvatarView,
    gap: gtk::Box,
    header: gtk::Box,
    author: gtk::Label,
    time: gtk::Label,
    edited: gtk::Label,
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
    tools: gtk::Box,
    quick: Vec<gtk::Button>,
    pick: gtk::Button,
    reply_button: gtk::Button,
    thread_button: gtk::Button,
    more: gtk::Button,
    /// The blocked run notice and its Show/Hide.
    blocked: gtk::Box,
    blocked_label: gtk::Label,
    blocked_toggle: gtk::Button,
    row: RefCell<Option<Row>>,
}

fn tool(images: &Images, icon: Icon, label: &str) -> gtk::Button {
    let button = crate::app::named::<gtk::Button>("GtkButton", label);
    button.set_child(Some(&images.icon_image(icon, "#b9bcbe", 16)));
    button.set_tooltip_text(Some(label));
    button.add_css_class("flat");
    button.add_css_class("tool");
    button
}

/// Recycled message row widgets: each list item builds its widgets once and
/// fills them whenever it is bound to another row.
pub fn factory(images: Rc<Images>, handler: Handler) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        item.set_activatable(false);
        item.set_selectable(false);
        let widgets = Rc::new(build_widgets(&images));
        connect(&widgets, &handler);
        item.set_child(Some(&widgets.root));
        let (images, handler) = (images.clone(), handler.clone());
        item.connect_item_notify(move |item| {
            if let Some(object) = item.item().and_downcast::<glib::BoxedAnyObject>() {
                let row = object.borrow::<Row>().clone();
                fill(&widgets, &images, &row, &handler);
                *widgets.row.borrow_mut() = Some(row);
            }
        });
    });
    factory
}

/// One row outside a list (Pins): its widgets live as long as this does.
pub struct Standalone(Rc<Widgets>);

impl Standalone {
    pub fn new(images: &Images, row: &Row, handler: &Handler) -> Self {
        let widgets = Rc::new(build_widgets(images));
        connect(&widgets, handler);
        fill(&widgets, images, row, handler);
        *widgets.row.borrow_mut() = Some(row.clone());
        Self(widgets)
    }

    pub fn root(&self) -> &gtk::Box {
        &self.0.root
    }
}

fn build_widgets(images: &Images) -> Widgets {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let day = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(["day"])
        .build();
    let reply = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .css_classes(["quote"])
        .build();
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let avatar = AvatarView::new(36, false);
    let gap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    gap.set_size_request(36, 1);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.set_hexpand(true);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = |class: &str| {
        gtk::Label::builder()
            .xalign(0.0)
            .css_classes([class])
            .build()
    };
    let author = label("author");
    let time = label("time");
    let edited = label("time");
    edited.set_label("(edited)");
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
    let replies = gtk::Button::new();
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
    content.append(&extra);
    let forward = gtk::Box::new(gtk::Orientation::Vertical, 4);
    forward.add_css_class("forward-card");
    content.append(&forward);
    content.append(&notice);
    content.append(&reactions);
    content.append(&reaction_error);
    content.append(&replies);
    line.append(&avatar.root);
    line.append(&gap);
    line.append(&content);

    let tools = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tools.add_css_class("tools");
    tools.set_halign(gtk::Align::End);
    tools.set_valign(gtk::Align::Start);
    let quick: Vec<gtk::Button> = QUICK_REACTIONS
        .iter()
        .map(|emoji| {
            let button = crate::app::named::<gtk::Button>("GtkButton", &format!("React {emoji}"));
            button.set_label(emoji);
            button.add_css_class("flat");
            button.add_css_class("tool");
            button.set_focus_on_click(false);
            tools.append(&button);
            button
        })
        .collect();
    let pick = tool(images, Icon::SmilePlus, "Add reaction");
    let reply_button = tool(images, Icon::Reply, "Reply");
    let thread_button = tool(images, Icon::Thread, "Reply in thread");
    let more = tool(images, Icon::More, "More actions");
    for button in [&pick, &reply_button, &thread_button, &more] {
        // A click should not leave the toolbar pinned open by focus; keyboard
        // focus still reveals it through `:focus-within`.
        button.set_focus_on_click(false);
        tools.append(button);
    }
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&line));
    overlay.add_overlay(&tools);
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
    root.append(&blocked);
    root.append(&day);
    root.append(&reply);
    root.append(&overlay);
    Widgets {
        blocked,
        blocked_label,
        blocked_toggle,
        notice,
        card: overlay.clone(),
        root,
        day,
        reply,
        avatar,
        gap,
        header,
        author,
        time,
        edited,
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
        tools,
        quick,
        pick,
        reply_button,
        thread_button,
        more,
        row: RefCell::new(None),
    }
}

fn connect(widgets: &Rc<Widgets>, handler: &Handler) {
    let id = |widgets: &Widgets| widgets.row.borrow().as_ref().map(|row| row.id.clone());
    for (button, emoji) in widgets.quick.iter().zip(QUICK_REACTIONS) {
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
    type Make = fn(String, gtk::Widget) -> Action;
    let actions: [(&gtk::Button, Make); 7] = [
        (&widgets.retry_reaction, |id, _| Action::RetryReaction(id)),
        (&widgets.dismiss_reaction, |id, _| {
            Action::DismissReaction(id)
        }),
        (&widgets.pick, |id, anchor| Action::Pick { id, anchor }),
        (&widgets.reply_button, |id, _| Action::Reply(id)),
        (&widgets.thread_button, |id, _| Action::Thread(id)),
        (&widgets.replies, |id, _| Action::Thread(id)),
        (&widgets.more, |id, anchor| Action::Menu { id, anchor }),
    ];
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
    for (button, action) in actions {
        let (weak, handler) = (Rc::downgrade(widgets), handler.clone());
        button.connect_clicked(move |button| {
            if let Some(id) = weak.upgrade().and_then(|widgets| id(&widgets)) {
                handler(action(id, button.clone().upcast()));
            }
        });
    }
}

fn fill(widgets: &Widgets, images: &Images, row: &Row, handler: &Handler) {
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
    widgets.root.set_css_classes(&classes);
    widgets.blocked.set_visible(row.blocked_run.is_some());
    widgets.card.set_visible(row.blocked_run.is_none());
    if let Some((_, label, shown)) = &row.blocked_run {
        widgets.blocked_label.set_label(&format!("⊘ {label} —"));
        widgets
            .blocked_toggle
            .set_label(if *shown { "Hide" } else { "Show" });
        widgets.day.set_visible(false);
        widgets.reply.set_visible(false);
        return;
    }
    if row.mentioned {
        widgets.card.add_css_class("mentioned");
    } else {
        widgets.card.remove_css_class("mentioned");
    }
    widgets.notice.set_visible(row.team_notice);
    widgets.day.set_visible(row.day.is_some());
    widgets
        .day
        .set_label(row.day.as_deref().unwrap_or_default());
    match &row.reply {
        Some((author, text)) => {
            widgets.reply.set_markup(&format!(
                "↱ <b>{}</b>  {}",
                glib::markup_escape_text(author),
                glib::markup_escape_text(text)
            ));
            widgets.reply.set_visible(true);
        }
        None => widgets.reply.set_visible(false),
    }
    widgets.header.set_visible(!row.grouped);
    widgets.avatar.root.set_visible(!row.grouped);
    widgets.gap.set_visible(row.grouped);
    if !row.grouped {
        widgets.avatar.set(images, &row.avatar);
    }
    widgets.author.set_label(&row.author);
    widgets.time.set_label(&row.time);
    widgets.edited.set_visible(row.edited);
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
    while let Some(child) = widgets.reactions.first_child() {
        widgets.reactions.remove(&child);
    }
    for chip in &row.reactions {
        let button = gtk::Button::with_label(&format!("{} {}", chip.emoji, chip.count));
        button.add_css_class("reaction");
        if chip.mine {
            button.add_css_class("mine");
        }
        button.set_sensitive(row.offers.act);
        button.set_tooltip_text(Some(if chip.mine {
            "Remove your reaction"
        } else {
            "Add this reaction"
        }));
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
    widgets
        .replies
        .set_label(row.replies.as_deref().unwrap_or_default());
    let offers = row.offers;
    widgets.tools.set_visible(offers.act);
    widgets.thread_button.set_visible(offers.thread);
    widgets.more.set_visible(offers.act);
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
            reactions: Vec::new(),
            reaction_error: None,
            replies: None,
            mentioned: false,
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

    #[test]
    fn reply_previews_keep_one_short_line() {
        assert_eq!(preview("short"), "short");
        assert_eq!(preview("first\nsecond"), "first…");
        assert_eq!(preview(&"a".repeat(130)).chars().count(), 121);
    }
}
