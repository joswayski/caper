//! The reaction picker, as on the egui desktop: the shared Twemoji catalog
//! by category, searchable, in web's order (`caper_client::emoji`).

use crate::app::named;
use crate::images::Images;
use crate::ui::label;
use caper_client::emoji::{self, Entry};
use gtk::{gio, glib, prelude::*};
use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

/// Opens the picker at `anchor`, titled "Add a reaction" or, for a
/// composer, "Insert emoji". `pick` gets the emoji and whether it adds
/// your reaction (`false`: you had reacted with it, so it comes off).
pub fn show(
    images: &Rc<Images>,
    anchor: &gtk::Widget,
    title: &str,
    mine: HashSet<String>,
    pick: impl Fn(String, bool) + 'static,
) {
    let popover = gtk::Popover::new();
    popover.add_css_class("emoji-picker");
    let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
    root.set_size_request(352, 430);
    root.append(&label(title, &["picker-title"]));

    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tabs.set_homogeneous(true);
    root.append(&tabs);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search emoji"));
    root.append(&search);
    let section = label(emoji::CATEGORIES[0].1, &["muted", "small"]);
    root.append(&section);
    let empty = label("No results. Try another search.", &["muted", "small"]);
    empty.set_visible(false);
    root.append(&empty);

    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("factory items are list items");
        let image = gtk::Image::new();
        image.set_pixel_size(30);
        item.set_child(Some(&image));
    });
    let images_ref = images.clone();
    let mine_ref = Rc::new(mine);
    let mine_bind = mine_ref.clone();
    factory.connect_bind(move |_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("factory items are list items");
        let (Some(object), Some(image)) = (
            item.item().and_downcast::<glib::BoxedAnyObject>(),
            item.child().and_downcast::<gtk::Image>(),
        ) else {
            return;
        };
        let entry = *object.borrow::<&'static Entry>();
        image.set_paintable(images_ref.emoji(entry).as_ref());
        image.set_tooltip_text(Some(&entry.name));
        if mine_bind.contains(&entry.emoji) {
            image.add_css_class("mine");
        } else {
            image.remove_css_class("mine");
        }
    });
    let grid: gtk::GridView = named("GtkGridView", "Emoji");
    grid.set_model(Some(&gtk::NoSelection::new(Some(store.clone()))));
    grid.set_factory(Some(&factory));
    grid.set_max_columns(8);
    grid.set_min_columns(8);
    grid.set_single_click_activate(true);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&grid)
        .build();
    root.append(&scroller);
    popover.set_child(Some(&root));

    let category = Rc::new(Cell::new(0_usize));
    let tab_buttons: Rc<Vec<gtk::Button>> = Rc::new(
        emoji::CATEGORIES
            .iter()
            .map(|(_, name, icon)| {
                let button: gtk::Button = named("GtkButton", name);
                button.add_css_class("flat");
                button.add_css_class("emoji-tab");
                button.set_tooltip_text(Some(name));
                if let Some(entry) = emoji::find(icon) {
                    button.set_child(Some(&images.emoji_image(entry, 22)));
                }
                tabs.append(&button);
                button
            })
            .collect(),
    );
    let fill = {
        let (store, search, section, empty, category, tabs, scroller) = (
            store.clone(),
            search.clone(),
            section.clone(),
            empty.clone(),
            category.clone(),
            tab_buttons.clone(),
            scroller.clone(),
        );
        Rc::new(move || {
            let query = search.text().trim().to_lowercase();
            let (id, name, _) = emoji::CATEGORIES[category.get()];
            section.set_label(if query.is_empty() {
                name
            } else {
                "Search results"
            });
            for (index, tab) in tabs.iter().enumerate() {
                if query.is_empty() && index == category.get() {
                    tab.add_css_class("selected");
                } else {
                    tab.remove_css_class("selected");
                }
            }
            let choices = emoji::picker_choices(id, &query);
            let objects: Vec<glib::BoxedAnyObject> = choices
                .iter()
                .map(|entry| glib::BoxedAnyObject::new(*entry))
                .collect();
            store.splice(0, store.n_items(), &objects);
            empty.set_visible(choices.is_empty());
            scroller.vadjustment().set_value(0.0);
        })
    };
    for (index, tab) in tab_buttons.iter().enumerate() {
        let (fill, category, search) = (fill.clone(), category.clone(), search.clone());
        tab.connect_clicked(move |_| {
            category.set(index);
            if search.text().is_empty() {
                fill();
            } else {
                search.set_text("");
            }
        });
    }
    let refill = fill.clone();
    search.connect_search_changed(move |_| refill());
    let (popover_ref, pick) = (popover.downgrade(), Rc::new(pick));
    let activate = move |position: u32| {
        let Some(entry) = store
            .item(position)
            .and_downcast::<glib::BoxedAnyObject>()
            .map(|object| *object.borrow::<&'static Entry>())
        else {
            return;
        };
        pick(entry.emoji.clone(), !mine_ref.contains(&entry.emoji));
        if let Some(popover) = popover_ref.upgrade() {
            popover.popdown();
        }
    };
    grid.connect_activate(move |_, position| activate(position));
    fill();
    crate::app::popup(&popover, anchor);
    search.grab_focus();
}
