//! Account-wide notifications. Acknowledging a row never accepts an invitation.
pub use caper_client::activity::{Filter, Item, Page, Read};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct Inbox {
    pub shown: bool,
    pub items: Vec<Item>,
    pub cursor: Option<String>,
    pub unread_count: u64,
    pub filter: Filter,
    pub unread_only: bool,
    pub loading: bool,
    /// The row whose open is in flight; only it is disabled, and a second open waits.
    pub opening: Option<String>,
    /// Optimistic mark-reads in flight, with each row's original position for undo.
    pub reads: Vec<(String, Option<usize>)>,
    /// After a filter change the previous rows stay, dimmed and inert, until the new
    /// page arrives: emptying the list first collapsed it and reset the scroll.
    pub stale: bool,
    pub error: Option<String>,
    pub request: u64,
    pub refreshed: Option<Instant>,
    pub expanded: HashMap<String, Item>,
    pub targets: HashMap<String, Item>,
    pub reacting: Option<Item>,
    pub reaction_request: u64,
    pub gif_picker: Option<(Item, crate::klipy::Picker)>,
    pub media: HashMap<String, crate::klipy::Choice>,
}
impl Default for Inbox {
    fn default() -> Self {
        Self {
            shown: false,
            items: Vec::new(),
            cursor: None,
            unread_count: 0,
            filter: Filter::All,
            unread_only: true,
            loading: false,
            opening: None,
            reads: Vec::new(),
            stale: false,
            error: None,
            request: 0,
            refreshed: None,
            expanded: HashMap::new(),
            targets: HashMap::new(),
            reacting: None,
            reaction_request: 0,
            gif_picker: None,
            media: HashMap::new(),
        }
    }
}
impl Inbox {
    pub fn visible_items(&self) -> Vec<Item> {
        let mut items = self.items.clone();
        let pinned = self
            .expanded
            .values()
            .filter(|item| !items.iter().any(|row| row.id == item.id))
            .cloned()
            .collect::<Vec<_>>();
        items.extend(pinned);
        items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        items
    }
    pub fn due(&self) -> bool {
        self.shown
            && !self.loading
            && self.reads.is_empty()
            && self.opening.is_none()
            && self
                .refreshed
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(15))
    }
    fn set_read(&mut self, id: &str) {
        for row in self.items.iter_mut().chain(self.expanded.values_mut()) {
            if row.id == id {
                row.unread = false;
            }
        }
        if self.unread_only {
            self.items.retain(|row| row.id != id);
        }
    }
    /// Marks an unread row read at once; `read_finished` keeps or reverts it.
    pub fn mark_read(&mut self, item: &Item) {
        if !item.unread {
            return;
        }
        let index = self.items.iter().position(|row| row.id == item.id);
        self.reads.push((item.id.clone(), index));
        self.set_read(&item.id);
        self.unread_count = self.unread_count.saturating_sub(1);
    }
    /// `item` is the row as it was when acted on. Returns whether the read succeeded.
    pub fn read_finished(&mut self, item: &Item, result: Result<Read, String>) -> bool {
        let pending = item
            .unread
            .then(|| self.reads.iter().position(|(id, _)| *id == item.id))
            .flatten()
            .map(|at| self.reads.remove(at).1);
        match result {
            Ok(read) => {
                self.set_read(&item.id);
                // With several reads in flight, only the last reply's count includes them all.
                if self.reads.is_empty() {
                    self.unread_count = read.unread_count;
                }
                true
            }
            Err(error) => {
                if let Some(index) = pending {
                    if let Some(row) = self.items.iter_mut().find(|row| row.id == item.id) {
                        row.unread = true;
                    } else if let Some(index) = index {
                        self.items.insert(index.min(self.items.len()), item.clone());
                    }
                    if let Some(row) = self.expanded.get_mut(&item.id) {
                        row.unread = true;
                    }
                    self.unread_count += 1;
                }
                self.error = Some(error);
                false
            }
        }
    }
    pub fn finish(&mut self, request: u64, more: bool, result: Result<Page, String>) {
        if request != self.request {
            return;
        }
        self.loading = false;
        let stale = std::mem::take(&mut self.stale);
        match result {
            Ok(page) => {
                if !more {
                    self.items.clear();
                }
                for item in page.items {
                    if !self.items.iter().any(|existing| existing.id == item.id) {
                        self.items.push(item);
                    }
                }
                self.cursor = page.next_cursor;
                self.unread_count = page.unread_count;
            }
            Err(error) => {
                // Rows kept from the previous filter must not stand in for this one.
                if stale {
                    self.items.clear();
                    self.cursor = None;
                }
                self.error = Some(error);
            }
        }
    }
}

impl crate::CaperApp {
    pub(crate) fn send_activity_reply(&mut self, item: &Item) {
        let Some(token) = self.token.clone() else {
            return;
        };
        if !item.can_reply() || self.profile_sending() || self.profile_picker.is_some() {
            return;
        }
        let person = item.draft_key();
        let media = self.activity.media.get(&person);
        let draft = self.profile_drafts.entry(person.clone()).or_default();
        let client_id = if media.is_some() {
            draft.sending = true;
            draft.error = None;
            draft
                .retry_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
        } else {
            match draft.begin() {
                Ok(id) => id,
                Err(error) => {
                    draft.error = Some(error);
                    return;
                }
            }
        };
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.profile_send_cancel = Some(cancel.clone());
        self.worker.send(crate::worker::Command::SendActivity {
            epoch: self.account_epoch,
            item: item.clone(),
            token,
            gif: media.map(|media| media.gif.clone()),
            send: Box::new(crate::profile_dm::Send {
                person,
                username: item.actor_name.clone(),
                name: self
                    .account
                    .as_ref()
                    .and_then(|account| account.display_name.clone())
                    .unwrap_or_default(),
                client_id,
                text: if media.is_some() {
                    String::new()
                } else {
                    draft.text.clone()
                },
                files: if media.is_some() {
                    Vec::new()
                } else {
                    draft.files.clone()
                },
                settings: self.uploads.clone(),
                cancel,
                retrying: draft.retry_id.is_some(),
                open_only: false,
                voice_only: false,
                direct: None,
            }),
        });
    }

    pub(crate) fn react_to_activity(&mut self, item: Item, emoji: String) {
        let Some(token) = self.token.clone() else {
            return;
        };
        self.activity.reaction_request += 1;
        self.activity.error = None;
        self.worker.send(crate::worker::Command::ReactActivity {
            epoch: self.account_epoch,
            request: self.activity.reaction_request,
            item,
            token,
            emoji,
            name: self
                .account
                .as_ref()
                .and_then(|account| account.display_name.clone())
                .unwrap_or_default(),
        });
    }

    pub(crate) fn load_activity(&mut self, more: bool) {
        self.load_activity_page(more, false);
    }

    fn load_activity_page(&mut self, more: bool, poll: bool) {
        let Some(token) = self.token.clone() else {
            return;
        };
        if poll && (self.activity.loading || !self.activity.reads.is_empty())
            || more && (self.activity.loading || self.activity.cursor.is_none())
        {
            return;
        }
        self.activity.request += 1;
        self.activity.loading = true;
        self.activity.error = None;
        self.activity.refreshed = Some(Instant::now());
        if !more && !poll {
            self.activity.stale = !self.activity.items.is_empty();
        }
        self.worker.send(crate::worker::Command::LoadActivity {
            epoch: self.account_epoch,
            request: self.activity.request,
            token,
            more,
            filter: self.activity.filter,
            unread: self.activity.unread_only,
            cursor: if more {
                self.activity.cursor.clone()
            } else {
                None
            },
            through: if poll {
                self.activity.items.last().cloned()
            } else {
                None
            },
        });
    }

    pub(crate) fn open_activity_item(&mut self, item: Item) {
        self.activity.shown = false;
        self.navigation_open = false;
        if item.kind == "space.invitation" || item.kind == "channel.invitation" {
            if let (Some(token), Some(space)) = (self.token.clone(), item.space_id) {
                self.navigation += 1;
                self.worker.send(crate::worker::Command::OpenInvitation {
                    epoch: self.account_epoch,
                    navigation: self.navigation,
                    token,
                    space,
                    channel: item.channel_id,
                });
            } else {
                self.error = Some("This invitation is no longer available.".into());
            }
            return;
        }
        let Some(channel) = item.conversation_id.or(item.channel_id) else {
            self.error = Some("This notification is no longer available.".into());
            return;
        };
        let target = item.message_id.map(|message| crate::feedback::VisitTarget {
            channel: channel.clone(),
            message,
            thread: item.thread_root_id,
            reporter: None,
        });
        if self.selected_channel.as_ref() == Some(&channel) {
            if let Some(target) = target {
                self.focus_report(&target);
            }
        } else {
            self.navigate(crate::NavigationTarget {
                space: item.space_id,
                channel: Some(channel),
            });
            self.report_focus = target.map(|target| (self.navigation, target));
        }
    }

    pub(crate) fn activity_dialog(&mut self, context: &eframe::egui::Context) {
        use eframe::egui;
        if !self.activity.shown || self.attachment_editor.is_some() {
            return;
        }
        context.request_repaint_after(Duration::from_secs(1));
        if self.activity.due() {
            self.load_activity_page(false, true);
        }
        let mut action = None;
        let picker_open = egui::Popup::is_any_open(context)
            || self.activity.reacting.is_some()
            || self.profile_emoji_open()
            || self.activity.gif_picker.is_some();
        let response = egui::Modal::new(egui::Id::unique("activity-inbox")).show(context, |ui| {
            ui.set_width((context.content_rect().width() - 48.0).clamp(260.0, 600.0));
            ui.horizontal(|ui| {
                ui.heading("Notifications");
                if ui.button("Done").clicked() {
                    self.activity.shown = false;
                }
            });
            let mut changed = false;
            ui.horizontal_wrapped(|ui| {
                for filter in Filter::ALL {
                    changed |= ui
                        .selectable_value(&mut self.activity.filter, filter, filter.label())
                        .changed();
                }
            });
            changed |= ui
                .scope(|ui| {
                    ui.set_max_width(210.0);
                    crate::settings_switch(
                        ui,
                        &mut self.activity.unread_only,
                        &format!("Unread only ({})", self.activity.unread_count),
                        "",
                    )
                    .changed()
                })
                .inner;
            if changed {
                self.load_activity(false);
            }
            if let Some(error) = &self.activity.error {
                ui.colored_label(crate::TERRACOTTA, error);
                if ui
                    .add_enabled(
                        !self.activity.loading && self.activity.reads.is_empty(),
                        egui::Button::new("Retry"),
                    )
                    .clicked()
                {
                    self.load_activity_page(false, true);
                }
            }
            let visible = self.activity.visible_items();
            if self.activity.loading && visible.is_empty() {
                ui.label("Loading notifications…");
            }
            if !self.activity.loading && visible.is_empty() && self.activity.error.is_none() {
                ui.label("No notifications match these filters.");
            }
            egui::ScrollArea::vertical()
                .min_scrolled_height((context.content_rect().height() - 240.0).clamp(160.0, 480.0))
                .max_height((context.content_rect().height() - 200.0).max(160.0))
                .show(ui, |ui| {
                    if self.activity.stale {
                        ui.visuals_mut().disabled_alpha = 0.5;
                        ui.disable();
                    }
                    for item in &visible {
                        let opening = self.activity.opening.as_ref() == Some(&item.id);
                        ui.push_id(&item.id, |ui| {
                            let title = format!(
                                "{}{}",
                                if item.unread { "● " } else { "" },
                                item.actor_name
                            );
                            ui.horizontal(|ui| {
                                ui.allocate_ui_with_layout(
                                    egui::vec2((ui.available_width() - 104.0).max(80.0), 32.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add_enabled(
                                                !opening,
                                                egui::Button::new(
                                                    egui::RichText::new(title).strong(),
                                                )
                                                .frame(false)
                                                .wrap(),
                                            )
                                            .clicked()
                                        {
                                            action = Some((item.clone(), true));
                                        }
                                    },
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if item.unread
                                            && crate::drawn_icon_button(
                                                ui,
                                                crate::NavIcon::Check,
                                                &format!(
                                                    "Mark notification from {} read",
                                                    item.actor_name
                                                ),
                                            )
                                            .clicked()
                                        {
                                            action = Some((item.clone(), false));
                                        }
                                        ui.small(crate::feedback::relative_time(
                                            &item.created_at,
                                            chrono::Utc::now(),
                                        ));
                                    },
                                );
                            });
                            let description = match item.kind.as_str() {
                                "reaction" => Some(format!(
                                    "Reacted {} to your message",
                                    item.emoji.as_deref().unwrap_or_default()
                                )),
                                "direct.request" => Some("Wants to send you messages".into()),
                                "space.invitation" => Some("Invited you to a space".into()),
                                "channel.invitation" => {
                                    Some("Invited you to a private channel".into())
                                }
                                _ => None,
                            };
                            if let Some(description) = description {
                                crate::emoji::name_label(
                                    ui,
                                    egui::RichText::new(description)
                                        .size(12.0)
                                        .color(crate::MUTED),
                                    egui::Sense::hover(),
                                );
                            }
                            if ui
                                .add_enabled(
                                    !opening,
                                    egui::Label::new(&item.body).sense(egui::Sense::click()),
                                )
                                .clicked()
                            {
                                action = Some((item.clone(), true));
                            }
                            ui.small(
                                [item.space_name.as_deref(), item.channel_name.as_deref()]
                                    .into_iter()
                                    .flatten()
                                    .collect::<Vec<_>>()
                                    .join(" · "),
                            );
                            let writable = item.can_reply()
                                && !self.directs.iter().any(|direct| {
                                    Some(&direct.id) == item.conversation_id.as_ref()
                                        && (direct.blocked
                                            || direct.status
                                                == crate::model::DirectStatus::Incoming)
                                });
                            let key = item.draft_key();
                            if writable {
                                ui.horizontal(|ui| {
                                    let expanded = self.activity.expanded.contains_key(&item.id);
                                    if ui
                                        .small_button(if expanded {
                                            "Close reply"
                                        } else {
                                            "Reply"
                                        })
                                        .clicked()
                                    {
                                        if expanded {
                                            self.activity.expanded.remove(&item.id);
                                        } else {
                                            self.activity
                                                .expanded
                                                .insert(item.id.clone(), item.clone());
                                            self.activity.targets.insert(key.clone(), item.clone());
                                        }
                                    }
                                    let react = ui.small_button("React");
                                    if react.clicked() {
                                        self.composer_emoji = None;
                                        self.activity.reacting = Some(item.clone());
                                        self.reaction_picker = item.message_id.clone();
                                        self.reaction_picker_anchor =
                                            Some(react.rect.left_bottom());
                                    }
                                });
                            }
                            if self.activity.expanded.contains_key(&item.id) {
                                let progress = crate::motion::entrance(
                                    context,
                                    egui::Id::unique(("inbox-reply", &item.id)),
                                    crate::motion::FAST,
                                );
                                crate::motion::arrive(ui, progress, egui::vec2(0.0, 4.0), |ui| {
                                    ui.small(item.reply_label());
                                    if self.profile_composer(
                                        ui,
                                        &key,
                                        &item.actor_name,
                                        !writable,
                                        false,
                                    ) {
                                        self.send_activity_reply(item);
                                    }
                                    if ui
                                        .add_enabled(
                                            writable
                                                && self.klipy.is_some()
                                                && !self.profile_sending()
                                                && self
                                                    .profile_drafts
                                                    .get(&key)
                                                    .is_none_or(crate::profile_dm::Draft::editable),
                                            egui::Button::new("GIF").small(),
                                        )
                                        .clicked()
                                    {
                                        self.activity.gif_picker = Some((
                                            item.clone(),
                                            crate::klipy::Picker::new(
                                                None,
                                                context.input(|input| input.pointer.interact_pos()),
                                                context.cumulative_frame_nr(),
                                            ),
                                        ));
                                    }
                                });
                            }
                            ui.separator();
                        });
                    }
                    if self.activity.cursor.is_some()
                        && ui
                            .add_enabled(!self.activity.loading, egui::Button::new("Load more"))
                            .clicked()
                    {
                        self.load_activity(true);
                    }
                });
            context.set_sublayer(
                ui.layer_id(),
                egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("emoji-picker")),
            );
            self.emoji_picker(context);
            context.set_sublayer(
                ui.layer_id(),
                egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("gif-picker")),
            );
            let choice = self.activity.gif_picker.as_mut().map(|(item, picker)| {
                (
                    item.clone(),
                    picker.show(context, self.klipy.as_ref(), &mut self.media),
                )
            });
            if let Some((item, choice)) = choice {
                match choice {
                    Ok(Some(choice)) => {
                        self.activity.gif_picker = None;
                        let key = item.draft_key();
                        if !self.profile_sending()
                            && self
                                .profile_drafts
                                .get(&key)
                                .is_none_or(crate::profile_dm::Draft::editable)
                        {
                            self.activity.media.insert(key, choice);
                            self.send_activity_reply(&item);
                        }
                    }
                    Err(()) => {
                        self.activity.gif_picker = None;
                    }
                    Ok(None) => {}
                }
            }
        });
        if response.should_close()
            && !picker_open
            && self.profile_picker.is_none()
            && self.voice_recording.is_none()
        {
            self.activity.shown = false;
        }
        if !self.activity.shown {
            self.activity.gif_picker = None;
            self.close_emoji_picker(context, false);
        }
        if let Some((item, open)) = action
            && let Some(token) = self.token.clone()
            && !(open && self.activity.opening.is_some())
        {
            self.unread_revision += 1;
            // A refresh already in flight could bring this row back unread; a filter
            // reload replacing stale rows keeps going.
            if !self.activity.stale {
                self.activity.request += 1;
                self.activity.loading = false;
            }
            if open {
                self.activity.opening = Some(item.id.clone());
            }
            self.activity.error = None;
            self.activity.mark_read(&item);
            self.worker.send(crate::worker::Command::ReadActivity {
                epoch: self.account_epoch,
                navigation: self.navigation,
                token,
                item,
                open,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(id: &str) -> Item {
        serde_json::from_value(serde_json::json!({
            "id": id, "kind": "message", "actorName": "Test", "body": "Preview",
            "createdAt": "2026-11-04T12:00:00Z", "unread": true,
            "conversationId": "dm0000000001", "messageId": "message00000001"
        }))
        .unwrap()
    }

    #[test]
    fn reply_targets_exclude_invitations_and_keep_drafting_rows_out_of_pagination() {
        let mut row = item("opaque:/id");
        assert!(row.can_reply());
        assert_eq!(row.quote_id(), row.message_id.as_deref());
        let key = row.draft_key();
        row.thread_root_id = Some("root".into());
        assert_ne!(key, row.draft_key());
        assert_eq!(row.quote_id(), None);
        assert_eq!(row.reply_label(), "Reply in thread");
        let mut inbox = Inbox::default();
        inbox.expanded.insert(row.id.clone(), row.clone());
        inbox.finish(
            0,
            false,
            Ok(Page {
                items: vec![],
                next_cursor: None,
                unread_count: 0,
            }),
        );
        assert!(
            inbox.items.is_empty(),
            "retained drafts must not extend pagination polling"
        );
        assert_eq!(inbox.visible_items()[0].id, row.id);
        for kind in ["direct.request", "space.invitation", "channel.invitation"] {
            row.kind = kind.into();
            assert!(!row.can_reply());
        }
    }

    #[test]
    fn pagination_keeps_refreshing_and_replaces_the_loaded_window() {
        let mut inbox = Inbox {
            shown: true,
            ..Default::default()
        };
        inbox.finish(
            0,
            false,
            Ok(Page {
                items: vec![item("removed"), item("kept")],
                next_cursor: Some("page2".into()),
                unread_count: 3,
            }),
        );
        inbox.finish(
            0,
            true,
            Ok(Page {
                items: vec![item("older")],
                next_cursor: Some("page3".into()),
                unread_count: 3,
            }),
        );
        assert!(
            inbox.due(),
            "older pages must not disable automatic updates"
        );
        inbox.finish(
            0,
            false,
            Ok(Page {
                items: vec![item("new"), item("kept"), item("displaced"), item("older")],
                next_cursor: Some("page3".into()),
                unread_count: 4,
            }),
        );
        assert_eq!(
            inbox
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["new", "kept", "displaced", "older"]
        );
        assert_eq!(inbox.cursor.as_deref(), Some("page3"));
        assert_eq!(inbox.unread_count, 4);
    }

    #[test]
    fn stale_pages_do_not_replace_current_filter_or_clear_loading() {
        // A filter change keeps the previous rows (dimmed) until its own page answers.
        let mut inbox = Inbox {
            request: 2,
            loading: true,
            stale: true,
            items: vec![item("previous")],
            cursor: Some("previous".into()),
            ..Default::default()
        };
        inbox.finish(
            1,
            false,
            Ok(Page {
                items: vec![],
                next_cursor: Some("old".into()),
                unread_count: 9,
            }),
        );
        assert!(inbox.loading && inbox.stale);
        assert_eq!(inbox.items[0].id, "previous");
        assert_eq!(inbox.cursor.as_deref(), Some("previous"));
        assert_eq!(inbox.unread_count, 0);
        inbox.finish(2, false, Err("offline".into()));
        assert!(!inbox.loading && !inbox.stale);
        assert!(inbox.items.is_empty() && inbox.cursor.is_none());
        assert_eq!(inbox.error.as_deref(), Some("offline"));
    }

    #[test]
    fn mark_read_is_optimistic_and_restores_the_row_and_count_on_failure() {
        let mut inbox = Inbox {
            shown: true,
            items: vec![item("a"), item("b"), item("c")],
            unread_count: 5,
            ..Default::default()
        };
        let (b, c) = (item("b"), item("c"));
        inbox.mark_read(&b);
        inbox.mark_read(&c);
        let ids = |inbox: &Inbox| {
            inbox
                .items
                .iter()
                .map(|row| row.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&inbox), ["a"]);
        assert_eq!(inbox.unread_count, 3);
        assert!(!inbox.due(), "polling waits for reads in flight");
        assert!(!inbox.read_finished(&b, Err("offline".into())));
        assert_eq!(ids(&inbox), ["a", "b"]);
        assert!(inbox.items[1].unread);
        assert_eq!(inbox.unread_count, 4);
        assert_eq!(inbox.error.as_deref(), Some("offline"));
        assert!(inbox.read_finished(&c, Ok(Read { unread_count: 2 })));
        assert!(inbox.reads.is_empty() && inbox.due());
        assert_eq!(
            inbox.unread_count, 2,
            "the last read takes the server count"
        );
    }
}
