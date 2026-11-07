//! The member list, as on the egui desktop: "Members" with the count, a
//! search, then Online, Offline and Status unavailable sections. A private
//! channel's readers lead in their own group ("In #name"), then "Others in
//! this space". A row opens the person's profile card. A child of `app`.

use super::{App, State, clear_list};
use crate::images::{Avatar, AvatarView};
use crate::ui::label;
use caper_client::model::Member;
use gtk::prelude::*;
use std::rc::Rc;

/// What the list shows, so it redraws only when this changes.
#[derive(Default, PartialEq, Eq)]
pub(super) struct Shown {
    groups: Vec<(String, Vec<(Member, usize)>)>,
    query: String,
}

/// A member's section: online (or idle), offline, or unknown.
fn section(state: &State, id: &str) -> usize {
    match state.presence.get(id).map(String::as_str) {
        Some("online" | "idle") => 0,
        Some("offline") => 1,
        _ => 2,
    }
}

impl App {
    pub(super) fn render_members(self: &Rc<Self>) {
        let state = self.state.borrow();
        let ui = &self.ui;
        let detail = state
            .detail
            .as_ref()
            .filter(|detail| state.space.as_ref() == Some(&detail.space.id));
        let shown =
            state.direct.is_none() && detail.is_some_and(|detail| !detail.members.is_empty());
        ui.members_toggle.set_visible(shown);
        let Some(detail) = detail.filter(|_| shown) else {
            *self.members_shown.borrow_mut() = Shown::default();
            return;
        };
        let query = ui.member_search.text().trim().to_lowercase();
        let matches = |member: &&Member| {
            member.display_name.to_lowercase().contains(&query)
                || member.username.to_lowercase().contains(&query)
        };
        let sectioned = |members: Vec<&Member>| -> Vec<(Member, usize)> {
            members
                .into_iter()
                .filter(matches)
                .map(|member| (member.clone(), section(&state, &member.id)))
                .collect()
        };
        let groups = match detail.member_groups(state.channel.as_deref()) {
            Some((readers, others)) => {
                let name = detail
                    .channels
                    .iter()
                    .find(|channel| Some(&channel.id) == state.channel.as_ref())
                    .map_or_else(String::new, |channel| channel.name.clone());
                vec![
                    (format!("In #{name}"), sectioned(readers)),
                    ("Others in this space".to_owned(), sectioned(others)),
                ]
            }
            None => vec![(String::new(), sectioned(detail.members.iter().collect()))],
        };
        let next = Shown { groups, query };
        ui.members_title.set_label("Members");
        ui.members_count
            .set_label(&detail.members.len().to_string());
        if *self.members_shown.borrow() == next {
            return;
        }
        clear_list(&ui.members);
        let empty = next.groups.iter().all(|(_, members)| members.is_empty());
        if empty {
            let note = label(
                if next.query.is_empty() {
                    "No members to show."
                } else {
                    "No members match your search."
                },
                &["muted", "small", "members-empty"],
            );
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&note));
            row.set_activatable(false);
            ui.members.append(&row);
        }
        let grouped = next.groups.len() > 1;
        for (index, (title, members)) in next.groups.iter().enumerate() {
            if members.is_empty() {
                continue;
            }
            if grouped {
                let heading = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                heading.add_css_class("member-group");
                heading.append(&label(
                    title,
                    &[if index == 0 {
                        "member-heading"
                    } else {
                        "member-heading-quiet"
                    }],
                ));
                heading.append(&label(&members.len().to_string(), &["muted", "small"]));
                ui.members
                    .append(&heading_row(&heading, title, members.len()));
            }
            for (section, title) in ["Online", "Offline", "Status unavailable"]
                .into_iter()
                .enumerate()
            {
                let rows: Vec<&Member> = members
                    .iter()
                    .filter(|(_, at)| *at == section)
                    .map(|(member, _)| member)
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                let heading = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                heading.add_css_class("member-section");
                heading.append(&label(title, &["member-heading-quiet"]));
                heading.append(&label(&rows.len().to_string(), &["muted", "small"]));
                ui.members.append(&heading_row(&heading, title, rows.len()));
                for member in rows {
                    ui.members.append(
                        &self.member_row(
                            member,
                            state
                                .presence
                                .get(&member.id)
                                .map_or("unknown", String::as_str),
                            section == 1,
                            grouped && index == 1,
                        ),
                    );
                }
            }
        }
        *self.members_shown.borrow_mut() = next;
    }

    fn member_row(
        self: &Rc<Self>,
        member: &Member,
        status: &str,
        offline: bool,
        quiet: bool,
    ) -> gtk::ListBoxRow {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        line.add_css_class("member");
        if offline {
            line.add_css_class("offline");
        }
        let view = AvatarView::new(30, true);
        view.set(
            &self.images,
            &Avatar::new(
                &member.display_name,
                member.avatar_id,
                &member.avatar_asset,
                &member.avatar_url,
            ),
        );
        view.set_presence(status);
        line.append(&view.root);
        let name = label(
            &member.display_name,
            if quiet || offline {
                &["member-name-quiet"]
            } else {
                &["member-name"]
            },
        );
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.set_hexpand(true);
        line.append(&name);
        if member.owner {
            line.append(&label("Owner", &["muted", "badge"]));
        }
        // As desktop: "Open profile for …", by click or by Enter (the
        // list's activation, `open_member`), the row's name its member id.
        let action = format!("Open profile for {}", member.display_name);
        line.set_tooltip_text(Some(&action));
        line.set_cursor_from_name(Some("pointer"));
        let row: gtk::ListBoxRow = super::named("GtkListBoxRow", &action);
        row.set_widget_name(&member.id);
        row.set_child(Some(&line));
        row
    }

    /// The Members list's activation: the member's profile, beside the row.
    pub(super) fn open_member(self: &Rc<Self>, row: &gtk::ListBoxRow) {
        if row.is_activatable() {
            self.open_profile(row.widget_name().to_string(), None, row.upcast_ref());
        }
    }
}

/// A section's heading, named for assistive technologies ("Online, 3").
/// It stays focusable: a list whose first row can't take focus can't be
/// entered with Tab.
fn heading_row(heading: &gtk::Box, title: &str, count: usize) -> gtk::ListBoxRow {
    let row: gtk::ListBoxRow = super::named("GtkListBoxRow", &format!("{title}, {count}"));
    row.set_child(Some(heading));
    row.set_activatable(false);
    row
}
