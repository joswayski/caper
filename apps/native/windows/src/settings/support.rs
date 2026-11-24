//! Team feedback and local-only connection/audio diagnostics on the existing card.
use super::{Builder, Page, SPart, Settings, Style, TIMER_INBOX};
use crate::{App, EV_FOCUS, EV_SCREEN, gfx, live::Live};
use caper_client::{feedback::Report, voice::ConnectionReport};
use windows::{Win32::UI::WindowsAndMessaging::SetTimer, core::Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Snapshot {
    Feedback {
        reports: Vec<Report>,
        archived: bool,
        loaded: bool,
        busy: bool,
        more: bool,
        error: Option<String>,
        action_error: Option<String>,
    },
    Report {
        rows: Vec<(String, String)>,
        json: Option<String>,
        note: String,
        audio: bool,
    },
}

pub fn snapshot(l: &Live, page: Page) -> Option<Snapshot> {
    match page {
        Page::Feedback if l.feedback_team => {
            let f = &l.feedback;
            Some(Snapshot::Feedback {
                reports: f.reports.clone(),
                archived: f.archived,
                loaded: f.loaded,
                busy: f.loading || f.loading_more,
                more: f.cursor.is_some(),
                error: f.error.clone(),
                action_error: f.action_error.clone(),
            })
        }
        Page::Connection => {
            let report = l
                .voice
                .diagnostics
                .as_ref()
                .map(|(stats, _)| ConnectionReport::new(l.voice.join_times.as_ref(), stats));
            Some(Snapshot::Report {
                rows: report.as_ref().map_or_else(Vec::new, |r| {
                    r.rows().into_iter().map(|(k, v)| (k.into(), v)).collect()
                }),
                json: report
                    .as_ref()
                    .and_then(|r| serde_json::to_string_pretty(r).ok()),
                note: if report.is_some() {
                    ""
                } else if matches!(l.voice.state.phase, caper_client::state::Phase::Idle) {
                    "Join voice to see connection details."
                } else {
                    "Waiting for connection statistics…"
                }
                .into(),
                audio: false,
            })
        }
        Page::Diagnostics if l.debug_enabled => {
            let processing = l.voice.audio_processing_report();
            Some(Snapshot::Report {
                rows: vec![],
                note: if processing.is_none() { "No microphone capture started. Open Mic Test or join voice first." } else { "" }.into(),
                json: serde_json::to_string_pretty(&serde_json::json!({"platform": std::env::consts::OS, "processing": processing})).ok(),
                audio: true,
            })
        }
        _ => None,
    }
}

impl Builder<'_> {
    pub(super) fn support(&mut self, s: &Snapshot) -> Result<()> {
        match s {
            Snapshot::Report {
                rows,
                json,
                note,
                audio,
            } => {
                if *audio {
                    self.text("Local diagnostics only. No audio, device identifiers, or credentials. Nothing is uploaded.", Style::Detail)?;
                }
                if !note.is_empty() {
                    self.text(note, Style::Detail)?;
                }
                for (label, value) in rows {
                    self.text(label, Style::Section)?;
                    self.text(value, Style::Label)?;
                    self.y += 8.0;
                }
                if let Some(json) = json {
                    self.button(
                        SPart::CopyReport,
                        if *audio {
                            "Copy diagnostics"
                        } else {
                            "Copy connection details"
                        },
                    )?;
                    if *audio {
                        self.text(json, Style::Code)?;
                    }
                }
            }
            Snapshot::Feedback {
                reports,
                archived,
                loaded,
                busy,
                more,
                error,
                action_error,
            } => {
                self.button(SPart::FeedbackRefresh, "Refresh")?;
                if let Some(e) = action_error {
                    self.text(e, Style::Error)?;
                }
                if let Some(e) = error {
                    self.text(e, Style::Error)?;
                } else if reports.is_empty() {
                    self.text(
                        if !loaded {
                            "Loading feedback…"
                        } else if *archived {
                            "Nothing archived. Feedback you archive moves here, out of the inbox."
                        } else {
                            "No feedback yet. When someone asks the Caper team for help, their conversation shows up here."
                        },
                        Style::Detail,
                    )?;
                }
                for (k, report) in reports.iter().enumerate() {
                    self.separator();
                    self.text(
                        &format!("{} · {}", report.reporter.display_name, report.status()),
                        Style::Title,
                    )?;
                    self.text(&report.preview(), Style::Label)?;
                    self.text(
                        &format!(
                            "{} · {} · {} replies",
                            report.place(),
                            caper_client::feedback::relative_time(
                                &report.created_at,
                                chrono::Utc::now()
                            ),
                            report.reply_count
                        ),
                        Style::Detail,
                    )?;
                    self.button(
                        SPart::FeedbackOpen(k),
                        &format!("Open feedback from {}", report.reporter.display_name),
                    )?;
                    self.button(SPart::FeedbackArchive(k), &report.action_label(*archived))?;
                }
                if *more {
                    self.button(SPart::FeedbackMore, "Load older feedback")?;
                }
                if *busy && !reports.is_empty() {
                    self.text("Loading…", Style::Detail)?;
                }
                self.separator();
                self.button(
                    SPart::FeedbackArchived,
                    if *archived {
                        "Back to feedback"
                    } else {
                        "Show archived"
                    },
                )?;
            }
        }
        Ok(())
    }
}

impl App {
    /// Do Not Disturb's menu items (`super::menu_item`): the presets, after
    /// Turn off while it is on.
    pub fn account_items(&self) -> Vec<usize> {
        let presets = 0..super::MENU_TURN_OFF;
        if self.live.as_ref().is_some_and(|l| l.paused().is_some()) {
            std::iter::once(super::MENU_TURN_OFF)
                .chain(presets)
                .collect()
        } else {
            presets.collect()
        }
    }

    pub fn open_support(&mut self, page: Page) {
        if !self.live.as_ref().is_some_and(|l| match page {
            Page::Feedback => l.feedback_team,
            Page::Diagnostics => l.debug_enabled,
            Page::Connection => l.voice.diagnostics.is_some(),
            _ => false,
        }) {
            return;
        }
        self.close_settings();
        self.close_viewer();
        self.close_pins();
        self.close_profile();
        self.close_dialog();
        self.close_menu();
        self.close_dm_menu();
        self.close_gifs(false);
        self.account_menu = None;
        self.set_composer_focus(false);
        self.set_thread_focus(false);
        self.clear_side_focus();
        self.settings = Some(Settings {
            page,
            ..Settings::new(false, None)
        });
        if page == Page::Feedback
            && let Some(live) = &mut self.live
        {
            live.feedback_shown = true;
            live.load_feedback(false);
        }
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_INBOX, 1000, None);
        }
        self.dirty.sidebar = true;
        self.settings_changed();
        self.events |= EV_SCREEN | EV_FOCUS;
    }

    pub fn support_activate(&mut self, part: SPart) {
        if part == SPart::CopyReport {
            if let Some(s) = &mut self.settings
                && let Some(Snapshot::Report {
                    json: Some(json),
                    audio,
                    ..
                }) = &s.snap.support
            {
                let text = gfx::wide(json);
                crate::set_clipboard(self.hwnd, &text);
                s.notice = if crate::get_clipboard(self.hwnd) != text {
                    "Copy failed; try again."
                } else if *audio {
                    "Copied diagnostics"
                } else {
                    "Copied connection details"
                }
                .into();
            }
        } else if let Some(l) = &mut self.live {
            match part {
                SPart::FeedbackRefresh => l.load_feedback(false),
                SPart::FeedbackMore => l.load_feedback(true),
                SPart::FeedbackArchived => {
                    l.feedback.show_archived(!l.feedback.wanted());
                    l.load_feedback(false);
                    if let Some(s) = &mut self.settings {
                        s.scroll = 0.0;
                    }
                }
                SPart::FeedbackArchive(k) => {
                    if let Some(report) =
                        self.settings.as_ref().and_then(|s| match &s.snap.support {
                            Some(Snapshot::Feedback { reports, .. }) => reports.get(k),
                            _ => None,
                        })
                    {
                        l.archive_feedback(report);
                    }
                }
                SPart::FeedbackOpen(k) => {
                    // Use the displayed report, never a row that moved while a request completed.
                    let target = self.settings.as_ref().and_then(|s| match &s.snap.support {
                        Some(Snapshot::Feedback { reports, .. }) => {
                            reports.get(k).map(Report::target)
                        }
                        _ => None,
                    });
                    if let Some(target) = target {
                        l.open_feedback_report(target);
                        self.close_settings();
                    }
                }
                _ => {}
            }
        }
        self.settings_changed();
    }
}
