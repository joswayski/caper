//! Account-wide notifications. Acknowledging a row never accepts an invitation.
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub kind: String,
    pub actor_name: String,
    pub body: String,
    pub created_at: String,
    pub unread: bool,
    pub space_id: Option<String>,
    pub space_name: Option<String>,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub thread_root_id: Option<String>,
    pub emoji: Option<String>,
}

impl Item {
    pub fn channel(&self) -> Option<&str> {
        self.conversation_id
            .as_deref()
            .or(self.channel_id.as_deref())
    }
    pub fn can_reply(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "message" | "mention" | "reply" | "reaction"
        ) && self.message_id.as_ref().is_some_and(|id| !id.is_empty())
            && self.channel().is_some_and(|id| !id.is_empty())
    }
    pub fn draft_key(&self) -> String {
        format!(
            "inbox:{}",
            [
                self.id.as_str(),
                self.channel().unwrap_or_default(),
                self.message_id.as_deref().unwrap_or_default(),
                self.thread_root_id.as_deref().unwrap_or_default()
            ]
            .map(|part| format!("{}:{part}", part.len()))
            .join("")
        )
    }
    pub fn quote_id(&self) -> Option<&str> {
        self.thread_root_id
            .is_none()
            .then_some(self.message_id.as_deref())
            .flatten()
    }
    pub fn reply_label(&self) -> String {
        if self.thread_root_id.is_some() {
            "Reply in thread".into()
        } else if self.conversation_id.is_some() {
            "Reply in DM".into()
        } else {
            format!(
                "Reply in #{}",
                self.channel_name.as_deref().unwrap_or("channel")
            )
        }
    }
    /// Only existing destinations. Never resolve the actor's username or accept a request.
    pub fn destination(&self, api: &crate::api::Api, token: &str) -> Result<String, String> {
        if !self.can_reply() {
            return Err("This notification cannot be replied to.".into());
        }
        if let Some(id) = &self.conversation_id {
            let directs = api
                .direct_conversations(token)
                .map_err(|error| error.to_string())?;
            if !directs.conversations.iter().any(|direct| {
                &direct.id == id
                    && !direct.blocked
                    && direct.status != crate::model::DirectStatus::Incoming
            }) {
                return Err("This conversation is not available for replies. Open it to review access or requests.".into());
            }
        }
        Ok(self.channel().unwrap().to_owned())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub items: Vec<Item>,
    pub next_cursor: Option<String>,
    pub unread_count: u64,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Read {
    pub unread_count: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Dms,
    Mentions,
    Threads,
    Reactions,
    Invitations,
}
impl Filter {
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::Dms,
        Self::Mentions,
        Self::Threads,
        Self::Reactions,
        Self::Invitations,
    ];
    pub fn wire(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Dms => "dms",
            Self::Mentions => "mentions",
            Self::Threads => "threads",
            Self::Reactions => "reactions",
            Self::Invitations => "invitations",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Dms => "DMs",
            Self::Mentions => "Mentions",
            Self::Threads => "Threads",
            Self::Reactions => "Reactions",
            Self::Invitations => "Invitations",
        }
    }
}

pub fn path(filter: Filter, unread: bool, cursor: Option<&str>) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query
        .append_pair("filter", filter.wire())
        .append_pair("unread", if unread { "true" } else { "false" });
    if let Some(cursor) = cursor {
        query.append_pair("cursor", cursor);
    }
    format!("api/notifications/inbox?{}", query.finish())
}

pub struct Inbox {
    pub shown: bool,
    pub items: Vec<Item>,
    pub cursor: Option<String>,
    pub unread_count: u64,
    pub filter: Filter,
    pub unread_only: bool,
    pub loading: bool,
    pub reading: Option<String>,
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
            reading: None,
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
            && self.reading.is_none()
            && self
                .refreshed
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(15))
    }
    pub fn finish(&mut self, request: u64, more: bool, result: Result<Page, String>) {
        if request != self.request {
            return;
        }
        self.loading = false;
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
            Err(error) => self.error = Some(error),
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
        if self.activity.reading.is_some()
            || poll && self.activity.loading
            || more && (self.activity.loading || self.activity.cursor.is_none())
        {
            return;
        }
        self.activity.request += 1;
        self.activity.loading = true;
        self.activity.error = None;
        self.activity.refreshed = Some(Instant::now());
        if !more && !poll {
            self.activity.items.clear();
            self.activity.cursor = None;
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
        let response = egui::Modal::new(egui::Id::new("activity-inbox")).show(context, |ui| {
            ui.set_width((context.content_rect().width() - 48.0).clamp(260.0, 600.0));
            ui.horizontal(|ui| {
                ui.heading("Notifications");
                if ui.button("Done").clicked() {
                    self.activity.shown = false;
                }
            });
            ui.add_enabled_ui(self.activity.reading.is_none(), |ui| {
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
            });
            if let Some(error) = &self.activity.error {
                ui.colored_label(crate::TERRACOTTA, error);
                if ui
                    .add_enabled(
                        !self.activity.loading && self.activity.reading.is_none(),
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
                    for item in &visible {
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
                                                self.activity.reading.is_none(),
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
                                            && ui
                                                .add_enabled_ui(
                                                    self.activity.reading.is_none(),
                                                    |ui| {
                                                        crate::drawn_icon_button(
                                                            ui,
                                                            crate::NavIcon::Check,
                                                            &format!(
                                                                "Mark notification from {} read",
                                                                item.actor_name
                                                            ),
                                                        )
                                                    },
                                                )
                                                .inner
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
                                    self.activity.reading.is_none(),
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
                                    egui::Id::new(("inbox-reply", &item.id)),
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
                            .add_enabled(
                                !self.activity.loading && self.activity.reading.is_none(),
                                egui::Button::new("Load more"),
                            )
                            .clicked()
                    {
                        self.load_activity(true);
                    }
                });
            context.set_sublayer(
                ui.layer_id(),
                egui::LayerId::new(egui::Order::Foreground, egui::Id::new("emoji-picker")),
            );
            self.emoji_picker(context);
            context.set_sublayer(
                ui.layer_id(),
                egui::LayerId::new(egui::Order::Foreground, egui::Id::new("gif-picker")),
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
        {
            self.unread_revision += 1;
            self.activity.request += 1;
            self.activity.loading = false;
            self.activity.reading = Some(item.id.clone());
            self.activity.error = None;
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
    fn opaque_cursor_is_encoded_and_filters_are_explicit() {
        assert_eq!(
            path(Filter::Dms, true, Some("a+/=&?")),
            "api/notifications/inbox?filter=dms&unread=true&cursor=a%2B%2F%3D%26%3F"
        );
        assert_eq!(
            Filter::ALL.map(Filter::wire),
            [
                "all",
                "dms",
                "mentions",
                "threads",
                "reactions",
                "invitations"
            ]
        );
    }
    #[test]
    fn stale_pages_do_not_replace_current_filter_or_clear_loading() {
        let mut inbox = Inbox {
            request: 2,
            loading: true,
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
        assert!(inbox.loading);
        assert!(inbox.cursor.is_none());
        assert_eq!(inbox.unread_count, 0);
        inbox.finish(2, false, Err("offline".into()));
        assert!(!inbox.loading);
        assert_eq!(inbox.error.as_deref(), Some("offline"));
    }
}
