//! A composer's `:` emoji and `@` mention suggestions: one popup above the
//! composer, as on the egui desktop. Arrows move the highlight, Enter or Tab
//! accepts it without sending, Escape dismisses it for that token, and a
//! click accepts a row. The grammar and ranking live in `caper_client`.

use crate::images::{Avatar, AvatarView, Images};
use crate::ui::{Composer, label};
use caper_client::emoji::{self, Entry};
use caper_client::mentions::{self, Candidate, Person, Specials};
use gtk::{gdk, glib, prelude::*};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

/// The composer token that opens suggestions; at most one kind is active.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Emoji(emoji::Token),
    Mention(mentions::Token),
}

impl Token {
    fn at(text: &str, caret: usize) -> Option<Self> {
        emoji::token(text, caret)
            .map(Self::Emoji)
            .or_else(|| mentions::token(text, caret).map(Self::Mention))
    }

    fn end(&self) -> usize {
        match self {
            Self::Emoji(token) => token.end,
            Self::Mention(token) => token.end,
        }
    }

    fn insert(&self, text: &str, choice: &Choice) -> Option<(String, usize)> {
        match (self, choice) {
            (Self::Emoji(token), Choice::Emoji(entry)) => emoji::insert(text, token, &entry.emoji),
            (Self::Mention(token), Choice::Mention(candidate)) => {
                mentions::insert(text, token, candidate.name())
            }
            _ => None,
        }
    }
}

#[derive(Clone)]
enum Choice {
    Emoji(&'static Entry),
    Mention(Candidate),
}

#[derive(Default)]
struct State {
    token: Option<Token>,
    choices: Vec<Choice>,
    selected: usize,
    /// Escape hides the token ending at this caret in this text.
    dismissed: Option<(String, usize)>,
    rows: Vec<gtk::Box>,
}

/// One composer's popup. `people` answers who `@` can suggest now.
pub struct Popup {
    popover: gtk::Popover,
    list: gtk::Box,
    view: gtk::TextView,
    field: gtk::Widget,
    images: Rc<Images>,
    /// `None` while the app is busy: the popup refreshes once it is idle.
    people: Box<dyn Fn() -> Option<(Vec<Person>, Specials)>>,
    state: RefCell<State>,
    /// The composer's highlighted complete mentions.
    tag: gtk::TextTag,
}

impl Popup {
    pub fn new(
        composer: &Composer,
        images: Rc<Images>,
        people: impl Fn() -> Option<(Vec<Person>, Specials)> + 'static,
    ) -> Rc<Self> {
        let popover = gtk::Popover::new();
        popover.set_autohide(false);
        popover.set_has_arrow(false);
        popover.set_can_focus(false);
        popover.set_position(gtk::PositionType::Top);
        popover.add_css_class("suggestions");
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        popover.set_child(Some(&list));
        popover.set_parent(&composer.root);
        let tag = gtk::TextTag::builder().name("mention").build();
        tag.set_foreground(Some("#F3F4F5"));
        tag.set_background(Some("rgba(182, 77, 50, 0.28)"));
        composer.view.buffer().tag_table().add(&tag);
        let popup = Rc::new(Self {
            popover,
            list,
            view: composer.view.clone(),
            field: composer
                .view
                .parent()
                .and_then(|scroller| scroller.parent())
                .unwrap_or_else(|| composer.view.clone().upcast()),
            images,
            people: Box::new(people),
            state: RefCell::default(),
            tag,
        });
        let buffer = composer.view.buffer();
        let weak = Rc::downgrade(&popup);
        buffer.connect_changed(move |_| {
            if let Some(popup) = weak.upgrade() {
                popup.refresh();
            }
        });
        let weak = Rc::downgrade(&popup);
        buffer.connect_notify_local(Some("cursor-position"), move |_, _| {
            if let Some(popup) = weak.upgrade() {
                popup.refresh();
            }
        });
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(&popup);
        focus.connect_leave(move |_| {
            if let Some(popup) = weak.upgrade() {
                popup.hide();
            }
        });
        composer.view.add_controller(focus);
        popup
    }

    fn text(&self) -> String {
        let buffer = self.view.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), true)
            .to_string()
    }

    pub fn hide(&self) {
        self.state.borrow_mut().token = None;
        self.popover.popdown();
    }

    /// Recomputes the token at the caret and redraws the popup.
    pub fn refresh(self: &Rc<Self>) {
        let buffer = self.view.buffer();
        let text = self.text();
        let caret = usize::try_from(buffer.cursor_position()).unwrap_or_default();
        let Some((people, specials)) = (self.people)() else {
            let popup = self.clone();
            glib::idle_add_local_once(move || popup.refresh());
            return;
        };
        self.highlight(&text, &people, specials);
        let mut state = self.state.borrow_mut();
        if let Some((dismissed, at)) = &state.dismissed
            && (*dismissed != text || *at != caret || buffer.has_selection())
        {
            state.dismissed = None;
        }
        let token = (!buffer.has_selection() && self.view.has_focus())
            .then(|| Token::at(&text, caret))
            .flatten()
            .filter(|token| state.dismissed.as_ref() != Some(&(text.clone(), token.end())));
        let choices: Vec<Choice> = match &token {
            Some(Token::Emoji(token)) => emoji::suggestions(&token.query)
                .into_iter()
                .map(Choice::Emoji)
                .collect(),
            Some(Token::Mention(token)) => mentions::suggestions(&people, specials, &token.query)
                .into_iter()
                .map(Choice::Mention)
                .collect(),
            None => Vec::new(),
        };
        if token != state.token {
            state.selected = 0;
        }
        let mention = matches!(token, Some(Token::Mention(_)));
        state.token = token;
        if choices.is_empty() {
            state.choices.clear();
            drop(state);
            self.popover.popdown();
            return;
        }
        state.selected = state.selected.min(choices.len() - 1);
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let parent = self
            .popover
            .parent()
            .map(|parent| parent.width())
            .unwrap_or(400);
        let width = if mention {
            parent - 12
        } else {
            parent.min(272) - 12
        };
        state.rows = choices
            .iter()
            .enumerate()
            .map(|(index, choice)| {
                let row = self.row(choice, width);
                let click = gtk::GestureClick::new();
                let weak = Rc::downgrade(self);
                click.connect_released(move |_, _, _, _| {
                    if let Some(popup) = weak.upgrade() {
                        popup.accept(Some(index));
                    }
                });
                row.add_controller(click);
                let motion = gtk::EventControllerMotion::new();
                let weak = Rc::downgrade(self);
                motion.connect_motion(move |_, _, _| {
                    if let Some(popup) = weak.upgrade() {
                        popup.select(index);
                    }
                });
                row.add_controller(motion);
                self.list.append(&row);
                row
            })
            .collect();
        state.choices = choices;
        let selected = state.selected;
        drop(state);
        self.select(selected);
        // Mentions span the composer; emoji start at the field's left edge.
        let left = if mention {
            0
        } else {
            self.field
                .compute_point(
                    &self.popover.parent().expect("popover has a parent"),
                    &gtk::graphene::Point::zero(),
                )
                .map_or(0, |point| point.x() as i32)
        };
        self.popover
            .set_pointing_to(Some(&gdk::Rectangle::new(left, 0, width + 12, 1)));
        crate::ui::hands(&self.popover);
        crate::ui::arrive(&self.popover);
        self.popover.popup();
    }

    fn row(&self, choice: &Choice, width: i32) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("suggestion");
        row.set_size_request(width, 44);
        let (title, detail) = match choice {
            Choice::Emoji(entry) => {
                row.append(&self.images.emoji_image(entry, 24));
                let name = label(&format!(":{}:", entry.name.replace(' ', "_")), &[]);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                row.append(&name);
                return row;
            }
            Choice::Mention(Candidate::Person(person)) => {
                let view = AvatarView::new(24, false);
                view.set(
                    &self.images,
                    &Avatar::new(
                        &person.display_name,
                        person.avatar_id,
                        &person.avatar_asset,
                        &person.avatar_url,
                    ),
                );
                view.root.set_valign(gtk::Align::Center);
                row.append(&view.root);
                (
                    person.display_name.clone(),
                    if person.invited {
                        format!("@{} · Already invited", person.username)
                    } else {
                        format!("@{}", person.username)
                    },
                )
            }
            Choice::Mention(Candidate::Everyone) => {
                row.append(&at_sign());
                ("@everyone".into(), "Everyone in this channel".into())
            }
            Choice::Mention(Candidate::Here) => {
                row.append(&at_sign());
                ("@here".into(), "Everyone online in this channel".into())
            }
            Choice::Mention(Candidate::Feedback(name)) => {
                row.append(&at_sign());
                (format!("@{name}"), mentions::FEEDBACK_LABEL.into())
            }
        };
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        for (value, classes) in [(title, &[][..]), (detail, &["muted", "small"][..])] {
            let line = label(&value, classes);
            line.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text.append(&line);
        }
        row.append(&text);
        row
    }

    fn select(&self, index: usize) {
        let mut state = self.state.borrow_mut();
        state.selected = index;
        for (position, row) in state.rows.iter().enumerate() {
            if position == index {
                row.add_css_class("selected");
            } else {
                row.remove_css_class("selected");
            }
        }
    }

    /// The composer's key, while the popup shows: whether it was handled.
    pub fn key(self: &Rc<Self>, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
        let (count, selected) = {
            let state = self.state.borrow();
            (state.choices.len(), state.selected)
        };
        if count == 0 || !self.popover.is_visible() || !modifiers.is_empty() {
            return false;
        }
        match key {
            gdk::Key::Down | gdk::Key::KP_Down => self.select((selected + 1) % count),
            gdk::Key::Up | gdk::Key::KP_Up => self.select((selected + count - 1) % count),
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::Tab => self.accept(None),
            gdk::Key::Escape => {
                let text = self.text();
                let mut state = self.state.borrow_mut();
                state.dismissed = state.token.as_ref().map(|token| (text, token.end()));
                drop(state);
                self.hide();
            }
            _ => return false,
        }
        true
    }

    /// Inserts the chosen row (`None`: the highlighted one).
    fn accept(self: &Rc<Self>, index: Option<usize>) {
        let text = self.text();
        let inserted = {
            let state = self.state.borrow();
            let choice = state.choices.get(index.unwrap_or(state.selected));
            state
                .token
                .as_ref()
                .zip(choice)
                .and_then(|(token, choice)| token.insert(&text, choice))
        };
        let Some((value, caret)) = inserted else {
            return;
        };
        let popup = self.clone();
        // Not inside the click's or key's own handler: the buffer change
        // re-enters `refresh`.
        glib::idle_add_local_once(move || {
            let buffer = popup.view.buffer();
            buffer.set_text(&value);
            let caret = i32::try_from(caret).unwrap_or(i32::MAX);
            buffer.place_cursor(&buffer.iter_at_offset(caret));
            popup.view.grab_focus();
            popup.hide();
        });
    }

    /// Tints complete mentions of people this composer knows.
    fn highlight(&self, text: &str, people: &[Person], specials: Specials) {
        let buffer = self.view.buffer();
        buffer.remove_tag(&self.tag, &buffer.start_iter(), &buffer.end_iter());
        for Range { start, end } in mentions::draft_highlights(text, people, specials) {
            let offset = |byte: usize| i32::try_from(text[..byte].chars().count()).unwrap_or(0);
            buffer.apply_tag(
                &self.tag,
                &buffer.iter_at_offset(offset(start)),
                &buffer.iter_at_offset(offset(end)),
            );
        }
    }
}

fn at_sign() -> gtk::Label {
    let at = label("@", &["muted"]);
    at.set_size_request(24, 24);
    at
}
