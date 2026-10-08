//! Author editing and retained history. HTTP snapshots are cursor-neutral.
use crate::{
    BORDER, CAPER, CaperApp, ERROR, MUTED, RAISED, TERRACOTTA, TEXT, egui, model, worker::Command,
};
use model::{Message, MessageVersion, MessageVersions};
use similar::{ChangeTag, TextDiff};

pub struct Editor {
    pub original: Message,
    pub draft: String,
    pub busy: bool,
    pub error: Option<String>,
    pub request: u64,
    pub focus: bool,
}

pub struct History {
    pub message: Message,
    pub versions: Vec<MessageVersion>,
    pub selected: Option<u32>,
    pub loading: bool,
    pub older: bool,
    pub more: bool,
    pub error: Option<String>,
    pub request: u64,
    pub requested_revision: u32,
}

pub fn valid_text(text: &str) -> bool {
    !text.trim().is_empty()
        && text.chars().count() <= 4000
        && !text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}

type DiffTokens = Vec<(String, bool)>;

pub fn diff(before: &str, after: &str) -> (DiffTokens, DiffTokens) {
    let mut old = Vec::new();
    let mut new = Vec::new();
    for change in TextDiff::from_words(before, after).iter_all_changes() {
        let token = (change.value().to_owned(), change.tag() != ChangeTag::Equal);
        match change.tag() {
            ChangeTag::Equal => {
                old.push(token.clone());
                new.push(token);
            }
            ChangeTag::Delete => old.push(token),
            ChangeTag::Insert => new.push(token),
        }
    }
    (old, new)
}

impl CaperApp {
    pub(crate) fn can_edit(&self, message: &Message) -> bool {
        message.forward.is_none()
            && self.selected_channel.as_deref() == Some(&message.channel_id)
            && self.selected_is_joined()
            && self.session.as_ref().is_some_and(|session| {
                !session.author.is_guest && session.author.id == message.author.id
            })
    }

    pub(crate) fn open_editor(&mut self, message: &Message) {
        if !self.can_edit(message) {
            return;
        }
        self.edit_request += 1;
        self.edit_history = None;
        self.message_editor = Some(Editor {
            original: message.clone(),
            draft: message.content.text.clone(),
            busy: false,
            error: None,
            request: self.edit_request,
            focus: true,
        });
    }

    pub(crate) fn open_edit_history(&mut self, message: &Message) {
        if message.forward.is_some() {
            return;
        }
        self.message_editor = None;
        self.edit_history = Some(History {
            message: message.clone(),
            versions: Vec::new(),
            selected: None,
            loading: false,
            older: false,
            more: false,
            error: None,
            request: 0,
            requested_revision: message.revision,
        });
        self.load_versions(false);
    }

    fn load_versions(&mut self, older: bool) {
        let Some(history) = &mut self.edit_history else {
            return;
        };
        self.edit_request += 1;
        history.request = self.edit_request;
        history.loading = true;
        history.older = older;
        history.error = None;
        self.worker.send(Command::MessageVersions {
            generation: self.generation,
            request: history.request,
            token: self.token.clone(),
            channel: history.message.channel_id.clone(),
            message: history.message.id.clone(),
            before: if older {
                history.versions.last().map(|version| version.revision)
            } else {
                None
            },
        });
    }

    pub(crate) fn accept_versions(
        &mut self,
        request: u64,
        message: &str,
        result: Result<MessageVersions, String>,
    ) {
        let Some(history) = self
            .edit_history
            .as_mut()
            .filter(|history| history.request == request && history.message.id == message)
        else {
            return;
        };
        history.loading = false;
        match result {
            Ok(page) => {
                history.more = page.has_more;
                if history.older {
                    for version in page.versions {
                        if !history
                            .versions
                            .iter()
                            .any(|old| old.revision == version.revision)
                        {
                            history.versions.push(version);
                        }
                    }
                } else {
                    history.versions = page.versions;
                    history.selected = None;
                }
            }
            Err(error) => history.error = Some(error),
        }
    }

    pub(crate) fn accept_edit(
        &mut self,
        request: u64,
        message: &str,
        reloaded: bool,
        result: Result<Box<Message>, String>,
    ) {
        if self
            .message_editor
            .as_ref()
            .is_some_and(|editor| !self.can_edit(&editor.original))
        {
            self.mutations.edits.remove(message);
            self.message_editor = None;
            return;
        }
        let Some(editor) = self
            .message_editor
            .as_mut()
            .filter(|editor| editor.request == request && editor.original.id == message)
        else {
            return;
        };
        editor.busy = false;
        self.mutations.edits.remove(message);
        match result {
            Ok(snapshot) if snapshot.author.id == editor.original.author.id => {
                if reloaded {
                    editor.draft = snapshot.content.text.clone();
                    editor.original = *snapshot.clone();
                    editor.focus = true;
                } else {
                    self.message_editor = None;
                }
                if self.timeline.merge_edit_ack(*snapshot).is_err() {
                    self.reload_channel();
                }
            }
            Ok(_) => editor.error = Some("Message author mismatch.".into()),
            Err(error) => {
                editor.error = Some(error);
                editor.focus = true;
            }
        }
    }

    fn submit_edit(&mut self, reload: bool) {
        let Some(editor) = &mut self.message_editor else {
            return;
        };
        if editor.busy || (!reload && !valid_text(&editor.draft)) {
            return;
        }
        self.edit_request += 1;
        editor.request = self.edit_request;
        editor.busy = true;
        editor.error = None;
        if reload {
            self.worker.send(Command::ReloadMessage {
                generation: self.generation,
                request: editor.request,
                token: self.token.clone(),
                channel: editor.original.channel_id.clone(),
                message: editor.original.id.clone(),
            });
        } else if let Some(session) = &self.session {
            self.mutations.edits.insert(
                editor.original.id.clone(),
                (editor.draft.clone(), editor.original.revision),
            );
            self.worker.send(Command::EditMessage {
                generation: self.generation,
                request: editor.request,
                token: self.token.clone(),
                chat_token: session.token.clone(),
                original: Box::new(editor.original.clone()),
                text: editor.draft.clone(),
            });
        }
    }

    pub(crate) fn message_edit_dialogs(&mut self, context: &egui::Context) {
        if let Some(mut editor) = self.message_editor.take() {
            if !self.can_edit(&editor.original) {
                return;
            }
            let mut save = false;
            let mut reload = false;
            let mut close = false;
            let id = egui::Id::new("message-editor");
            let previous_size =
                context.memory(|memory| memory.area_rect(id).map(|rect| rect.size()));
            let modal = egui::Modal::new(id).show(context, |ui| {
                ui.set_width(500.0_f32.min(context.content_rect().width() - 48.0));
                ui.heading("Edit message");
                ui.colored_label(
                    MUTED,
                    "Previous versions remain visible to people who can read this message.",
                );
                let input = ui.add_enabled(
                    !editor.busy,
                    egui::TextEdit::multiline(&mut editor.draft)
                        .id(egui::Id::new("edit-message-text"))
                        .desired_width(f32::INFINITY)
                        .desired_rows(7),
                );
                if editor.focus {
                    input.request_focus();
                    editor.focus = false;
                }
                ui.colored_label(MUTED, format!("{} / 4,000", editor.draft.chars().count()));
                if let Some(error) = &editor.error {
                    ui.colored_label(ERROR, error);
                    reload = ui
                        .add_enabled(
                            !editor.busy,
                            egui::Button::new("Discard draft and load latest"),
                        )
                        .clicked();
                }
                ui.horizontal(|ui| {
                    close = ui
                        .add_enabled(!editor.busy, egui::Button::new("Cancel"))
                        .clicked();
                    save = ui
                        .add_enabled(
                            !editor.busy && valid_text(&editor.draft),
                            egui::Button::new(if editor.busy {
                                "Saving…"
                            } else {
                                "Save changes"
                            })
                            .fill(TERRACOTTA),
                        )
                        .clicked();
                });
                save |= !editor.busy
                    && valid_text(&editor.draft)
                    && ui.input_mut(|input| {
                        input.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
                    });
            });
            if previous_size != Some(modal.response.rect.size()) {
                context.request_repaint(); // Recenter after draft/error or viewport size changes.
            }
            if close || (modal.should_close() && !editor.busy) {
                return;
            }
            self.message_editor = Some(editor);
            if save || reload {
                self.submit_edit(reload);
            }
        }
        if let Some(history) = &self.edit_history {
            let current = self
                .timeline
                .messages()
                .chain(self.timeline.pinned_messages())
                .find(|message| message.id == history.message.id)
                .map_or(history.requested_revision, |message| message.revision);
            if current > history.requested_revision {
                self.edit_history.as_mut().unwrap().requested_revision = current;
                self.load_versions(false);
            }
        }
        if let Some(mut history) = self.edit_history.take() {
            let mut close = false;
            let mut load = false;
            let id = egui::Id::new("message-history");
            let previous_size =
                context.memory(|memory| memory.area_rect(id).map(|rect| rect.size()));
            let previous_selection = history.selected;
            let modal = egui::Modal::new(id).show(context, |ui| {
                ui.set_width(840.0_f32.min(context.content_rect().width() - 48.0));
                ui.horizontal(|ui| {
                    ui.heading("Message history");
                    close = ui.button("Close").clicked();
                });
                ui.colored_label(
                    MUTED,
                    format!(
                        "{} · Previous versions are retained.",
                        history.message.author.name
                    ),
                );
                egui::ScrollArea::vertical()
                    .max_height((context.content_rect().height() - 150.0).clamp(120.0, 560.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if history.versions.len() > 1 {
                            comparison(ui, &history.versions[1], &history.versions[0], true);
                        } else if let Some(original) = history.versions.first() {
                            ui.label("Original version");
                            ui.label(&original.content.text);
                        }
                        if history.versions.len() > 1 {
                            ui.separator();
                            ui.label("View previous versions");
                            egui::ComboBox::from_id_salt("older-message-version")
                                .selected_text(history.selected.map_or_else(
                                    || "Choose an earlier version…".into(),
                                    |revision| format!("Version {revision}"),
                                ))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut history.selected,
                                        None,
                                        "Choose an earlier version…",
                                    );
                                    for version in history.versions.iter().skip(1) {
                                        ui.selectable_value(
                                            &mut history.selected,
                                            Some(version.revision),
                                            version_label(version),
                                        );
                                    }
                                });
                            if let Some(after) = history
                                .versions
                                .iter()
                                .find(|version| Some(version.revision) == history.selected)
                            {
                                if let Some(before) = history
                                    .versions
                                    .iter()
                                    .find(|version| version.revision + 1 == after.revision)
                                {
                                    comparison(ui, before, after, false);
                                } else {
                                    ui.label(if after.revision == 1 {
                                        "Original version".into()
                                    } else {
                                        format!("Version {}", after.revision)
                                    });
                                    ui.monospace(&after.content.text);
                                    if after.revision > 1 && history.more {
                                        ui.colored_label(
                                            MUTED,
                                            "Load older versions to compare this change.",
                                        );
                                    }
                                }
                            }
                        }
                        if history.loading {
                            ui.label("Loading versions…");
                        }
                        if let Some(error) = &history.error {
                            ui.colored_label(ERROR, error);
                            load = ui
                                .add_enabled(!history.loading, egui::Button::new("Retry"))
                                .clicked();
                        }
                        if history.more && history.error.is_none() {
                            load = ui
                                .add_enabled(
                                    !history.loading,
                                    egui::Button::new("Load older versions"),
                                )
                                .clicked();
                        }
                    });
            });
            if previous_selection != history.selected
                || previous_size != Some(modal.response.rect.size())
            {
                context.request_repaint(); // Settle scroll height and recenter the changed modal.
            }
            if close || modal.should_close() {
                return;
            }
            let older = !history.versions.is_empty();
            self.edit_history = Some(history);
            if load {
                self.load_versions(older);
            }
        }
    }
}

fn version_label(version: &MessageVersion) -> String {
    let date = chrono::DateTime::parse_from_rfc3339(&version.created_at)
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%b %-d · %-I:%M %p")
                .to_string()
        })
        .unwrap_or_else(|_| version.created_at.clone());
    format!(
        "{} · {date}",
        if version.revision == 1 {
            "Original version".into()
        } else {
            format!("Version {}", version.revision)
        }
    )
}

fn comparison(ui: &mut egui::Ui, before: &MessageVersion, after: &MessageVersion, current: bool) {
    let (old, new) = diff(&before.content.text, &after.content.text);
    ui.columns(2, |columns| {
        for (index, (version, tokens)) in [(before, old), (after, new)].into_iter().enumerate() {
            let column = &mut columns[index];
            column.label(
                egui::RichText::new(if current {
                    if index == 0 {
                        "Previous version"
                    } else {
                        "Current version"
                    }
                } else if index == 0 {
                    "Before"
                } else {
                    "After"
                })
                .strong(),
            );
            column.colored_label(MUTED, version_label(version));
            egui::Frame::new()
                .fill(RAISED)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .inner_margin(8)
                .show(column, |ui| {
                    let mut job = egui::text::LayoutJob::default();
                    for (text, changed) in tokens {
                        job.append(
                            &text,
                            0.0,
                            egui::TextFormat {
                                font_id: egui::FontId::monospace(12.0),
                                color: TEXT,
                                background: if changed {
                                    if index == 0 {
                                        TERRACOTTA.gamma_multiply(0.3)
                                    } else {
                                        CAPER.gamma_multiply(0.3)
                                    }
                                } else {
                                    egui::Color32::TRANSPARENT
                                },
                                ..Default::default()
                            },
                        );
                    }
                    ui.add(egui::Label::new(job).wrap());
                });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separated_changes_preserve_unicode_and_whitespace() {
        let before = "Meet Friday 🙂\nKeep this unchanged\nAt 9";
        let after = "Meet Saturday 🚀\nKeep this unchanged\nAt 11";
        let (old, new) = diff(before, after);
        assert_eq!(
            old.iter()
                .map(|(text, _)| text.as_str())
                .collect::<String>(),
            before
        );
        assert_eq!(
            new.iter()
                .map(|(text, _)| text.as_str())
                .collect::<String>(),
            after
        );
        assert!(
            old.iter()
                .filter(|(text, _)| text.contains("Keep"))
                .all(|(_, changed)| !changed)
        );
        assert!(
            old.iter()
                .filter(|(_, changed)| *changed)
                .map(|(text, _)| text.as_str())
                .collect::<String>()
                .contains("Friday")
        );
        assert!(
            new.iter()
                .filter(|(_, changed)| *changed)
                .map(|(text, _)| text.as_str())
                .collect::<String>()
                .contains("Saturday")
        );
        assert!(valid_text(&"🙂".repeat(4000)));
        assert!(!valid_text(&"🙂".repeat(4001)));
        assert!(!valid_text(" \n\t"));
    }
}
