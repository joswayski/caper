//! The Caper team's feedback inbox and report visits, as on the egui
//! desktop. Team accounts (`feedbackTeam`) get "Feedback" under the
//! notifications, with the count of reports waiting for a reply. It opens
//! the inbox in the conversation's place: each `@feedback` report with who,
//! where and when, Waiting or Answered, and Archive (Restore in the archived
//! view), then "Load older feedback" and "Show archived". A report opens its
//! conversation at the report, without joining it: a visit, titled "#plans ·
//! Studio" or "Alice & Bob", where the team replies as themselves. "Back to
//! feedback" returns to the inbox, or Close to the conversation a
//! notification interrupted. Paging is `caper_client::feedback`. A child of
//! `app`.

use super::{App, State, clear};
use crate::images::{Avatar, AvatarView, Icon};
use crate::ui::label;
use caper_client::feedback::{Origin, Report, Visit, VisitRequest, VisitTarget};
use caper_client::worker::Command;
use gtk::prelude::*;
use std::rc::Rc;

/// The inbox page's widgets.
pub(super) struct Page {
    browse: gtk::Button,
    title: gtk::Label,
    opening: gtk::Label,
    refresh: gtk::Button,
    list: gtk::Box,
    /// The sidebar row's accessible name, with the waiting count.
    row_name: String,
    /// What the list shows, so it redraws only when that changes.
    shown: Option<Shown>,
}

#[derive(PartialEq)]
struct Shown {
    reports: Vec<Report>,
    archived: bool,
    loaded: bool,
    error: Option<String>,
    action_error: Option<String>,
    more: Option<bool>,
    /// Relative times move on each minute.
    minute: i64,
}

impl State {
    fn feedback_team(&self) -> bool {
        self.account
            .as_ref()
            .is_some_and(|account| account.feedback_team)
    }

    /// The open conversation when it is a feedback visit.
    pub(super) fn visiting(&self) -> Option<&Visit> {
        self.visit
            .as_ref()
            .filter(|visit| self.channel.as_deref() == Some(visit.channel.as_str()))
    }

    /// Where Back returns from a visit opened now.
    fn report_origin(&self) -> Origin {
        if let Some(visit) = self.visiting() {
            return visit.origin.clone();
        }
        match &self.channel {
            Some(channel) => Origin::Conversation {
                space: if self.direct.is_some() {
                    None
                } else {
                    self.space.clone()
                },
                channel: channel.clone(),
            },
            None => Origin::Nowhere,
        }
    }
}

impl App {
    pub(super) fn connect_feedback(self: &Rc<Self>) {
        let ui = &self.ui;
        let app = self.clone();
        ui.feedback
            .connect_row_activated(move |_, _| app.open_feedback());
        let app = self.clone();
        ui.visit_back.connect_clicked(move |_| app.leave_visit());

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("header");
        let browse: gtk::Button = crate::app::named("GtkButton", "Browse");
        browse.set_child(Some(&self.images.icon_image(Icon::Menu, "#b9bcbe", 18)));
        browse.set_tooltip_text(Some("Browse"));
        browse.add_css_class("flat");
        browse.set_valign(gtk::Align::Center);
        browse.set_visible(false);
        let app = self.clone();
        browse.connect_clicked(move |_| app.set_browsing(true));
        header.append(&browse);
        let title = label("Feedback", &["channel-title"]);
        header.append(&title);
        let opening = label("Opening message…", &["muted", "status"]);
        opening.set_hexpand(true);
        opening.set_xalign(0.0);
        header.append(&opening);
        let refresh = gtk::Button::with_label("Refresh");
        refresh.set_valign(gtk::Align::Center);
        let app = self.clone();
        refresh.connect_clicked(move |_| {
            let mut state = app.state.borrow_mut();
            app.load_feedback(&mut state, false);
            drop(state);
            app.render();
        });
        header.append(&refresh);
        ui.feedback_page.append(&header);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("feedback-list");
        ui.feedback_page.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&list)
                .build(),
        );
        *self.feedback_page.borrow_mut() = Some(Page {
            browse,
            title,
            opening,
            refresh,
            list,
            row_name: String::new(),
            shown: None,
        });
    }

    /// Shows the inbox over the conversation and refreshes it.
    pub(super) fn open_feedback(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            if !state.feedback_team() {
                return;
            }
            // Returning to the inbox cancels a report on its way.
            if state.visit_request.take().is_some() || state.report_thread.is_some() {
                state.navigation += 1;
                state.request += 1;
                state.context_loading = false;
            }
            state.report_thread = None;
            state.report_focus = None;
            state.feedback_open = true;
            self.load_feedback(&mut state, false);
        }
        self.browsing.set(false);
        self.render();
    }

    fn load_feedback(&self, state: &mut State, more: bool) {
        let Some(token) = state.token.clone().filter(|_| state.feedback_team()) else {
            return;
        };
        if let Some((request, before)) = state.feedback.start(more) {
            self.worker.send(Command::LoadFeedback {
                epoch: state.epoch,
                request,
                token,
                before,
                archived: state.feedback.archived,
            });
        }
    }

    /// A reply in a visit can answer a report: the list and badge catch up.
    pub(super) fn sent_in_visit(&self, state: &mut State) {
        if state.visiting().is_some() {
            self.load_feedback(state, false);
            self.refresh_unread(state);
        }
    }

    /// Archives (or, in the archived view, restores) a report's thread for
    /// the team. It leaves the list at once; only a failure reloads it.
    fn archive_report(self: &Rc<Self>, report: &Report) {
        {
            let mut state = self.state.borrow_mut();
            let Some(token) = state.token.clone().filter(|_| state.feedback_team()) else {
                return;
            };
            let archived = !state.feedback.archived;
            state
                .feedback
                .remove_thread(&report.channel.id, &report.thread_root_id);
            self.worker.send(Command::ArchiveFeedback {
                epoch: state.epoch,
                token,
                channel: report.channel.id.clone(),
                thread_root: report.thread_root_id.clone(),
                archived,
            });
        }
        self.render();
    }

    pub(super) fn feedback_archive_failed(&self, state: &mut State, error: String) {
        self.load_feedback(state, false);
        state.feedback.action_error = Some(error);
    }

    /// Opens a report's conversation by channel id, even one outside the
    /// space and DM lists, at the reported message.
    pub(super) fn open_feedback_report(self: &Rc<Self>, target: VisitTarget, from_inbox: bool) {
        {
            let mut state = self.state.borrow_mut();
            if state.token.is_none() {
                return;
            }
            let origin = if from_inbox {
                Origin::Inbox
            } else {
                state.report_origin()
            };
            if !from_inbox {
                state.feedback_open = false;
            }
            state.report_thread = None;
            state.report_focus = None;
            if state.channel.as_deref() == Some(target.channel.as_str()) {
                // Already on its way: it lands on this report instead.
                if let Some(request) = state.visit_request.as_mut() {
                    *request = VisitRequest { target, origin };
                    drop(state);
                    self.render();
                    return;
                }
                // Already open, as a visit or as a member: land on the report.
                if let Some(visit) = state
                    .visit
                    .as_mut()
                    .filter(|visit| visit.channel == target.channel)
                {
                    visit.origin = origin;
                    if target.reporter.is_some() {
                        visit.reporter.clone_from(&target.reporter);
                    }
                }
                self.focus_report(&mut state, &target);
            } else {
                // A visit is DM-shaped: it never changes the open space.
                self.leave_conversation(&mut state);
                let channel = target.channel.clone();
                state.channel = Some(channel.clone());
                state.direct = Some(channel.clone());
                state.channel_name.clear();
                state.restore_draft = Some(state.drafts.remove(&channel).unwrap_or_default());
                state.visit_request = Some(VisitRequest { target, origin });
                self.prepare(&mut state, None, Some(channel));
            }
        }
        self.browsing.set(false);
        self.render();
    }

    /// History around the report; only a reported reply opens its thread.
    pub(super) fn focus_report(&self, state: &mut State, target: &VisitTarget) {
        let (anchor, thread) = target.plan();
        state.report_thread = thread.map(|thread| (anchor.clone(), thread));
        self.load_context(state, anchor, false);
    }

    /// The report's context is in: a reported reply's thread opens at it,
    /// and the inbox gives way once nothing is still loading.
    pub(super) fn report_context_loaded(&self, state: &mut State, anchor: &str) {
        if state.error.is_none()
            && let Some((_, thread)) = state
                .report_thread
                .take_if(|(expected, _)| expected == anchor)
        {
            state.forward_thread = Some((thread.root, thread.anchor));
        }
        if !state.context_loading && state.visit_request.is_none() {
            state.feedback_open = false;
        }
    }

    /// The navigation for a visit is in: a visitor's conversation becomes
    /// the visit; a team member who is in the channel opens it the normal
    /// way. Returns `false` when it went elsewhere instead.
    pub(super) fn visit_prepared(
        &self,
        state: &mut State,
        request: Option<VisitRequest>,
        history: &caper_client::model::History,
    ) -> bool {
        let Some(request) = request else {
            return true;
        };
        if history.visitor {
            let visit = Visit::new(request.clone(), history);
            state.channel_name.clone_from(&visit.name);
            state.visit = Some(visit);
        } else if !history.space.id.is_empty() {
            let open = state.feedback_open;
            self.navigate(
                state,
                Some(history.space.id.clone()),
                Some(history.channel.id.clone()),
            );
            state.feedback_open = open;
            state.report_focus = Some((state.navigation, request.target));
            return false;
        }
        true
    }

    /// Back from a visit: the inbox, or the conversation a notification
    /// interrupted.
    fn leave_visit(self: &Rc<Self>) {
        let origin = self
            .state
            .borrow()
            .visiting()
            .map(|visit| visit.origin.clone());
        match origin {
            Some(Origin::Conversation {
                space: Some(space),
                channel,
            }) => {
                {
                    let mut state = self.state.borrow_mut();
                    self.navigate(&mut state, Some(space), Some(channel));
                }
                self.render();
            }
            Some(Origin::Conversation {
                space: None,
                channel,
            }) => self.open_direct(channel),
            Some(Origin::Inbox | Origin::Nowhere) => self.open_feedback(),
            None => {}
        }
    }

    /// The sidebar row, the header's way back, and the inbox page.
    pub(super) fn render_feedback(self: &Rc<Self>, state: &State) {
        let ui = &self.ui;
        let team = state.feedback_team();
        ui.feedback.set_visible(team);
        let waiting = state.feedback_waiting;
        ui.feedback_waiting.set_visible(waiting > 0);
        ui.feedback_waiting.set_label(&waiting.to_string());
        let name = if waiting > 0 {
            format!("Feedback, {waiting} feedback conversations waiting for a reply")
        } else {
            "Feedback".into()
        };
        let mut page = self.feedback_page.borrow_mut();
        let Some(page) = page.as_mut() else {
            return;
        };
        // The row's accessible name carries the count, set as it's built.
        if page.row_name != name
            && let Some(old) = ui.feedback.row_at_index(0)
        {
            let line = old.child();
            old.set_child(None::<&gtk::Widget>);
            ui.feedback.remove(&old);
            let row: gtk::ListBoxRow = crate::app::named("GtkListBoxRow", &name);
            row.set_child(line.as_ref());
            ui.feedback.append(&row);
            page.row_name = name;
        }
        if let Some(row) = ui.feedback.row_at_index(0) {
            // The inbox or a visit owns the sidebar highlight.
            if state.feedback_open || state.visiting().is_some() {
                ui.feedback.select_row(Some(&row));
                ui.channels.unselect_all();
                ui.directs.unselect_all();
            } else {
                ui.feedback.unselect_all();
            }
        }
        let visit = state.visiting();
        ui.visit_back
            .set_visible(visit.is_some() && !state.feedback_open);
        ui.visit_back
            .set_label(match visit.map(|visit| &visit.origin) {
                Some(Origin::Conversation { .. }) => "Close",
                _ => "Back to feedback",
            });
        let open = team && state.feedback_open;
        ui.feedback_page.set_visible(open);
        ui.conversation.set_visible(!open);
        page.browse.set_visible(self.narrow.get());
        page.title.set_label(if state.feedback.archived {
            "Archived feedback"
        } else {
            "Feedback"
        });
        page.opening
            .set_opacity(if state.visit_request.is_some() || state.context_loading {
                1.0
            } else {
                0.0
            });
        page.refresh.set_sensitive(!state.feedback.loading);
        if !open {
            return;
        }
        let inbox = &state.feedback;
        let shown = Shown {
            reports: inbox.reports.clone(),
            archived: inbox.archived,
            loaded: inbox.loaded,
            error: inbox.error.clone(),
            action_error: inbox.action_error.clone(),
            more: inbox.cursor.as_ref().map(|_| inbox.loading_more),
            minute: chrono::Utc::now().timestamp() / 60,
        };
        if page.shown.as_ref() == Some(&shown) {
            return;
        }
        clear(&page.list);
        if let Some(error) = &shown.action_error {
            let line = label(error, &["error", "feedback-note"]);
            line.set_wrap(true);
            page.list.append(&line);
        }
        if shown.reports.is_empty() {
            let empty = gtk::Box::new(gtk::Orientation::Vertical, 8);
            empty.add_css_class("empty-state");
            empty.set_halign(gtk::Align::Center);
            empty.set_margin_top(48);
            if let Some(error) = &shown.error {
                empty.append(&label(error, &["muted"]));
                let retry = gtk::Button::with_label("Try again");
                retry.set_halign(gtk::Align::Center);
                let app = self.clone();
                retry.connect_clicked(move |_| {
                    let mut state = app.state.borrow_mut();
                    app.load_feedback(&mut state, false);
                    drop(state);
                    app.render();
                });
                empty.append(&retry);
            } else if !shown.loaded {
                empty.append(&label("Loading…", &["muted"]));
            } else if shown.archived {
                empty.append(&label("Nothing archived.", &["muted"]));
            } else {
                empty.append(&label("No feedback yet.", &["empty-title"]));
                empty.append(&label(
                    "Messages that mention @feedback anywhere appear here.",
                    &["muted"],
                ));
            }
            // Centered, as the conversation's empty states.
            let mut child = empty.first_child();
            while let Some(widget) = child {
                if let Some(text) = widget.downcast_ref::<gtk::Label>() {
                    text.set_xalign(0.5);
                    text.set_justify(gtk::Justification::Center);
                }
                child = widget.next_sibling();
            }
            page.list.append(&empty);
        } else {
            let now = chrono::Utc::now();
            for report in &shown.reports {
                page.list
                    .append(&self.feedback_row(report, now, shown.archived));
            }
            if let Some(error) = shown.error.as_ref() {
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                line.add_css_class("feedback-note");
                line.append(&label(error, &["error"]));
                let retry = gtk::Button::with_label("Retry");
                retry.add_css_class("small-action");
                let app = self.clone();
                retry.connect_clicked(move |_| {
                    let mut state = app.state.borrow_mut();
                    let more = state.feedback.cursor.is_some();
                    app.load_feedback(&mut state, more);
                    drop(state);
                    app.render();
                });
                line.append(&retry);
                page.list.append(&line);
            }
            if let Some(loading) = shown.more {
                let more = gtk::Button::with_label("Load older feedback");
                more.add_css_class("history-button");
                more.set_halign(gtk::Align::Center);
                more.set_sensitive(!loading);
                let app = self.clone();
                more.connect_clicked(move |_| {
                    let mut state = app.state.borrow_mut();
                    app.load_feedback(&mut state, true);
                    drop(state);
                    app.render();
                });
                page.list.append(&more);
            }
        }
        // The quiet way to the archive and back.
        let archived = shown.archived;
        let toggle = gtk::Button::with_label(if archived {
            "Back to feedback"
        } else {
            "Show archived"
        });
        toggle.add_css_class("history-button");
        toggle.set_halign(gtk::Align::Center);
        let app = self.clone();
        toggle.connect_clicked(move |_| {
            let mut state = app.state.borrow_mut();
            state.feedback.show_archived(!archived);
            app.load_feedback(&mut state, false);
            drop(state);
            app.render();
        });
        page.list.append(&toggle);
        page.shown = Some(shown);
    }

    /// Who, where and when, the report, its status and replies, and
    /// Archive or Restore beside it.
    fn feedback_row(
        self: &Rc<Self>,
        report: &Report,
        now: chrono::DateTime<chrono::Utc>,
        archived_view: bool,
    ) -> gtk::Widget {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("feedback-row");
        let status = if archived_view {
            "archived"
        } else {
            report.status()
        };
        let open: gtk::Button = crate::app::named(
            "GtkButton",
            &format!(
                "{}, {}, {status}",
                report.reporter.display_name,
                report.place()
            ),
        );
        open.add_css_class("flat");
        open.add_css_class("feedback-open");
        open.set_hexpand(true);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let avatar = AvatarView::new(34, false);
        avatar.set(
            &self.images,
            &Avatar::new(
                &report.reporter.display_name,
                report.reporter.avatar_id,
                &report.reporter.avatar_asset,
                &report.reporter.avatar_url,
            ),
        );
        avatar.root.set_valign(gtk::Align::Start);
        line.append(&avatar.root);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
        text.set_hexpand(true);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        top.append(&label(&report.reporter.display_name, &["author"]));
        let place = label(&report.place(), &["muted", "small"]);
        place.set_hexpand(true);
        place.set_xalign(1.0);
        place.set_ellipsize(gtk::pango::EllipsizeMode::End);
        top.append(&place);
        top.append(&label(
            &caper_client::feedback::relative_time(&report.created_at, now),
            &["muted", "small"],
        ));
        text.append(&top);
        let preview = report.preview();
        if !preview.is_empty() {
            let preview = label(&preview, &["feedback-preview"]);
            preview.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text.append(&preview);
        }
        let replies = match report.reply_count {
            0 => String::new(),
            1 => "1 reply".into(),
            count => format!("{count} replies"),
        };
        // Archived threads carry no Waiting/Answered status.
        let state_line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        if !archived_view {
            state_line.append(&label(
                report.status(),
                &[if report.answered {
                    "feedback-answered"
                } else {
                    "feedback-waiting"
                }],
            ));
        }
        if !replies.is_empty() {
            let replies = if archived_view {
                replies
            } else {
                format!(" · {replies}")
            };
            state_line.append(&label(&replies, &["muted", "small"]));
        }
        if state_line.first_child().is_some() {
            text.append(&state_line);
        }
        line.append(&text);
        open.set_child(Some(&line));
        let app = self.clone();
        let target = report.target();
        open.connect_clicked(move |_| app.open_feedback_report(target.clone(), true));
        row.append(&open);
        let (icon, tip) = if archived_view {
            (Icon::Restore, "Restore to the inbox")
        } else {
            (Icon::Archive, "Archive")
        };
        let action: gtk::Button =
            crate::app::named("GtkButton", &report.action_label(archived_view));
        action.set_child(Some(&self.images.icon_image(icon, "#b9bcbe", 16)));
        action.set_tooltip_text(Some(tip));
        action.add_css_class("flat");
        action.set_valign(gtk::Align::Center);
        let app = self.clone();
        let report = report.clone();
        action.connect_clicked(move |_| app.archive_report(&report));
        row.append(&action);
        row.upcast()
    }
}
