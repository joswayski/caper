//! A username field with the people it matches listed below it, as the egui
//! desktop's new DM, space invite and private channel grant fields do. Text
//! is normalized as typed (`caper_client::names::normalize_username`); the
//! rows rank as `@` suggestions do (`caper_client::mentions`).

use crate::images::{Avatar, AvatarView, Images};
use crate::ui::label;
use caper_client::mentions::{self, Person};
use gtk::{gdk, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type Changed = Box<dyn Fn(&str)>;

pub struct PeopleField {
    pub root: gtk::Box,
    pub entry: gtk::Entry,
    list: gtk::Box,
    images: Rc<Images>,
    people: RefCell<Vec<Person>>,
    shown: RefCell<Vec<String>>,
    rows: RefCell<Vec<gtk::Box>>,
    selected: Cell<usize>,
    /// Escape or a pick hides the rows until the text changes again.
    hidden: Cell<bool>,
    changed: RefCell<Option<Changed>>,
}

impl PeopleField {
    pub fn new(images: Rc<Images>, name: &str) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let entry: gtk::Entry = crate::app::named("GtkEntry", name);
        entry.set_placeholder_text(Some(name));
        entry.add_css_class("people-entry");
        root.append(&entry);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("people-list");
        list.set_visible(false);
        root.append(&list);
        let field = Rc::new(Self {
            root,
            entry,
            list,
            images,
            people: RefCell::default(),
            shown: RefCell::default(),
            rows: RefCell::default(),
            selected: Cell::new(0),
            hidden: Cell::new(false),
            changed: RefCell::default(),
        });
        let weak = Rc::downgrade(&field);
        field.entry.connect_changed(move |entry| {
            let Some(field) = weak.upgrade() else {
                return;
            };
            let text = entry.text().to_string();
            let normalized = caper_client::names::normalize_username(&text);
            if normalized != text {
                // Rewrites the text; this handler runs again for it.
                let entry = entry.clone();
                glib::idle_add_local_once(move || {
                    entry.set_text(&normalized);
                    entry.set_position(-1);
                });
                return;
            }
            field.hidden.set(false);
            field.refresh();
            if let Some(changed) = field.changed.borrow().as_ref() {
                changed(&normalized);
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&field);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(field) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let count = field.shown.borrow().len();
            if count == 0 {
                return glib::Propagation::Proceed;
            }
            let selected = field.selected.get();
            match key {
                gdk::Key::Down | gdk::Key::KP_Down => field.select((selected + 1) % count),
                gdk::Key::Up | gdk::Key::KP_Up => field.select((selected + count - 1) % count),
                gdk::Key::Return | gdk::Key::KP_Enter => field.choose(selected),
                gdk::Key::Escape => {
                    field.hidden.set(true);
                    field.refresh();
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        field.entry.add_controller(keys);
        field
    }

    pub fn text(&self) -> String {
        self.entry.text().to_string()
    }

    /// Calls `changed` with the normalized text after each edit.
    pub fn connect_changed(&self, changed: impl Fn(&str) + 'static) {
        *self.changed.borrow_mut() = Some(Box::new(changed));
    }

    pub fn set_people(self: &Rc<Self>, people: Vec<Person>) {
        *self.people.borrow_mut() = people;
        self.refresh();
    }

    fn refresh(self: &Rc<Self>) {
        let text = self.text();
        let people = self.people.borrow();
        let rows: Vec<&Person> = if self.hidden.get() {
            Vec::new()
        } else {
            mentions::field_suggestions(&people, &text)
        };
        let usernames: Vec<String> = rows.iter().map(|person| person.username.clone()).collect();
        if *self.shown.borrow() == usernames {
            return;
        }
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let views: Vec<gtk::Box> = rows
            .iter()
            .enumerate()
            .map(|(index, person)| {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                row.add_css_class("suggestion");
                row.set_size_request(-1, 44);
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
                let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
                text.set_valign(gtk::Align::Center);
                let detail = if person.invited {
                    format!("@{} · Already invited", person.username)
                } else {
                    format!("@{}", person.username)
                };
                for (value, classes) in [
                    (person.display_name.as_str(), &[][..]),
                    (detail.as_str(), &["muted", "small"][..]),
                ] {
                    let line = label(value, classes);
                    line.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    text.append(&line);
                }
                row.append(&text);
                let click = gtk::GestureClick::new();
                let weak = Rc::downgrade(self);
                click.connect_released(move |_, _, _, _| {
                    if let Some(field) = weak.upgrade() {
                        field.choose(index);
                    }
                });
                row.add_controller(click);
                let motion = gtk::EventControllerMotion::new();
                let weak = Rc::downgrade(self);
                motion.connect_motion(move |_, _, _| {
                    if let Some(field) = weak.upgrade() {
                        field.select(index);
                    }
                });
                row.add_controller(motion);
                self.list.append(&row);
                row
            })
            .collect();
        self.list.set_visible(!views.is_empty());
        *self.rows.borrow_mut() = views;
        *self.shown.borrow_mut() = usernames;
        self.select(0);
    }

    fn select(&self, index: usize) {
        self.selected.set(index);
        for (position, row) in self.rows.borrow().iter().enumerate() {
            if position == index {
                row.add_css_class("selected");
            } else {
                row.remove_css_class("selected");
            }
        }
    }

    fn choose(self: &Rc<Self>, index: usize) {
        let Some(username) = self.shown.borrow().get(index).cloned() else {
            return;
        };
        let field = self.clone();
        glib::idle_add_local_once(move || {
            field.entry.set_text(&username);
            field.entry.set_position(-1);
            field.entry.grab_focus_without_selecting();
            field.hidden.set(true);
            field.refresh();
        });
    }
}
