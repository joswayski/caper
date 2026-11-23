//! Recipient-scoped files and voice recording in the profile card. The shared
//! DM sender owns upload safety and retries; closing the card retains files
//! but discards an unfinished recording or file-dialog result.
use super::*;
use caper_client::profile_dm::File;
use caper_client::voice_note::{self, LiveMeter, Recorder, RecorderEvent};
use std::sync::Arc;
use std::time::Instant;

pub const TIMER_PROFILE_RECORDING: usize = 43;
const FILE_H: f32 = 42.0;
pub(super) const VISIBLE_FILES: usize = 3;

pub(super) struct Recording {
    recorder: Recorder,
    pub started: Option<Instant>,
    pub finishing: bool,
    meter: LiveMeter,
}

impl App {
    pub(super) fn profile_draft(&self) -> Option<&crate::live::ProfileDraft> {
        self.live
            .as_ref()?
            .profile_drafts
            .get(&self.profile.as_ref()?.id)
    }

    pub fn profile_recording_active(&self) -> bool {
        self.profile.as_ref().is_some_and(|c| c.recording.is_some())
    }

    pub(super) fn profile_can_attach(&self) -> bool {
        self.profile_view().is_some_and(|v| v.can_message)
            && self.live.as_ref().is_some_and(|l| l.uploads.is_some())
            && self
                .profile
                .as_ref()
                .is_some_and(|c| c.file_dialog.is_none())
            && self
                .profile_draft()
                .is_none_or(|d| d.files.len() < caper_client::uploads::MAX_ATTACHMENTS)
    }

    /// Whether the card offers GIFs (KLIPY answered for this account).
    pub(super) fn profile_gif_shown(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(crate::live::Live::can_profile_gif)
    }

    pub(super) fn profile_can_record(&self) -> bool {
        self.profile_can_attach()
            && self.rec.is_none()
            && self.gifs.is_none()
            && self
                .profile_draft()
                .is_none_or(|d| d.files.iter().all(|f| f.voice.is_none()))
    }

    /// Called by the worker wake, including file-dialog and eager-upload news.
    pub fn poll_profile_composer(&mut self) {
        let paths = self
            .profile
            .as_ref()
            .and_then(|c| c.file_dialog.as_ref())
            .and_then(|rx| rx.try_recv().ok());
        if let Some(paths) = paths {
            if let Some(c) = &mut self.profile {
                c.file_dialog = None;
            }
            self.add_profile_files(paths.into_iter().map(File::picked).collect());
        }
        if let (Some(c), Some(v), Some(live)) = (&self.profile, self.profile_view(), &self.live)
            && v.editable
            && live.profile_drafts.get(&c.id).is_some_and(|d| {
                d.files
                    .iter()
                    .any(|f| f.upload.is_none() && f.attachment.is_none())
            })
        {
            let id = c.id.clone();
            if let Some(username) = v.profile.username {
                self.live
                    .as_mut()
                    .unwrap()
                    .upload_profile_files(&id, &username);
            }
        }
        if self.profile.is_some() {
            self.profile_changed();
        }
    }

    pub(super) fn paste_profile_files(&mut self) -> bool {
        if !self.profile_part_enabled(PPart::Attach) {
            return false;
        }
        let files = crate::uploads::clipboard_files(self.hwnd);
        if files.is_empty() {
            return false;
        }
        self.add_profile_files(
            files
                .into_iter()
                .map(|(path, scratch)| File::staged(path, scratch))
                .collect(),
        );
        true
    }

    fn add_profile_files(&mut self, files: Vec<File>) {
        if !self
            .profile_view()
            .is_some_and(|v| v.editable && v.can_message)
        {
            return;
        }
        let (Some(c), Some(live)) = (&self.profile, &mut self.live) else {
            return;
        };
        let draft = live.profile_drafts.entry(c.id.clone()).or_default();
        let room = caper_client::uploads::MAX_ATTACHMENTS.saturating_sub(draft.files.len());
        draft.error = (files.len() > room).then(|| {
            format!(
                "You can attach up to {} files.",
                caper_client::uploads::MAX_ATTACHMENTS
            )
        });
        draft.files.extend(files.into_iter().take(room));
        if let Some(username) = self.profile_view().and_then(|v| v.profile.username) {
            let id = self.profile.as_ref().unwrap().id.clone();
            self.live
                .as_mut()
                .unwrap()
                .upload_profile_files(&id, &username);
        }
        self.profile_changed();
    }

    pub(super) fn profile_compose_parts(
        &self,
        parts: &mut Vec<(PPart, Rect)>,
        y: &mut f32,
        inner: f32,
    ) {
        if let Some(draft) = self.profile_draft() {
            let start = self.profile.as_ref().map_or(0, |c| c.file_start);
            for (k, file) in draft
                .files
                .iter()
                .enumerate()
                .skip(start)
                .take(VISIBLE_FILES)
            {
                parts.push((PPart::File(k), (PAD, *y, inner - 92.0, FILE_H - 4.0)));
                let right = PAD + inner;
                parts.push((PPart::Remove(k), (right - 28.0, *y + 5.0, 28.0, 28.0)));
                if file.voice.is_none() {
                    parts.push((PPart::Spoiler(k), (right - 58.0, *y + 5.0, 28.0, 28.0)));
                }
                if file.upload_status().1 {
                    parts.push((PPart::RetryFile(k), (right - 88.0, *y + 5.0, 28.0, 28.0)));
                }
                *y += FILE_H;
            }
            if draft.files.len() > VISIBLE_FILES {
                parts.push((PPart::PreviousFiles, (PAD, *y, inner / 2.0 - 3.0, 28.0)));
                parts.push((
                    PPart::NextFiles,
                    (PAD + inner / 2.0 + 3.0, *y, inner / 2.0 - 3.0, 28.0),
                ));
                *y += 34.0;
            }
            if draft.retry.is_some() {
                parts.push((PPart::DiscardSend, (PAD, *y, inner, 28.0)));
                *y += 34.0;
            }
        }
        if self.profile.as_ref().is_some_and(|c| c.recording.is_some()) {
            parts.push((PPart::DiscardRecording, (PAD, *y, 28.0, 32.0)));
            parts.push((PPart::RecordingStatus, (PAD + 34.0, *y, inner - 68.0, 44.0)));
            parts.push((PPart::Stop, (PAD + inner - 28.0, *y, 28.0, 32.0)));
            *y += 50.0;
        }
    }

    pub fn profile_part_name(&self, part: PPart) -> String {
        if let PPart::File(k) | PPart::Remove(k) | PPart::Spoiler(k) | PPart::RetryFile(k) = part
            && let Some(file) = self.profile_draft().and_then(|d| d.files.get(k))
        {
            let action = match part {
                PPart::File(_) if file.voice.is_some() => "Play or pause",
                PPart::File(_) => "Preview",
                PPart::Remove(_) => "Remove",
                PPart::Spoiler(_) if file.spoiler => "Unmark spoiler",
                PPart::Spoiler(_) => "Mark spoiler",
                _ => "Retry upload",
            };
            // A ready file has no status text.
            return format!("{action} {} {}", file.name(), file.upload_status().0)
                .trim_end()
                .to_owned();
        }
        if part == PPart::Send && self.profile_draft().is_some_and(|d| d.retry.is_some()) {
            return "Retry message with the same ID".into();
        }
        if part == PPart::RecordingStatus
            && let Some(r) = self.profile.as_ref().and_then(|c| c.recording.as_ref())
        {
            return if r.finishing {
                "Preparing voice message…".into()
            } else if let Some(started) = r.started {
                format!(
                    "Recording {}",
                    voice_note::duration_label(started.elapsed().as_millis() as u64)
                )
            } else {
                "Starting microphone…".into()
            };
        }
        self.profile_view()
            .map_or_else(String::new, |v| v.part_name(part))
    }

    pub(super) fn profile_compose_activate(&mut self, part: PPart) {
        if !self.profile_part_enabled(part) {
            return;
        }
        match part {
            PPart::PreviousFiles | PPart::NextFiles => {
                if let Some(c) = &mut self.profile {
                    c.file_start = if part == PPart::NextFiles {
                        c.file_start + VISIBLE_FILES
                    } else {
                        c.file_start.saturating_sub(VISIBLE_FILES)
                    };
                }
            }
            PPart::Attach => {
                self.profile.as_mut().unwrap().file_dialog = Some(crate::uploads::pick(self.hwnd));
            }
            PPart::Emoji => self.toggle_profile_emoji(),
            PPart::Gif => self.toggle_profile_gifs(),
            PPart::Mic => {
                self.playback.stop();
                self.set_profile_focus(false);
                let p = &self.live.as_ref().unwrap().voice.preferences;
                let recorder = Recorder::start(
                    p.input.clone(),
                    p.input_percent,
                    p.processing_strength,
                    || {},
                );
                self.profile.as_mut().unwrap().recording = Some(Recording {
                    recorder,
                    started: None,
                    finishing: false,
                    meter: LiveMeter::default(),
                });
                unsafe {
                    SetTimer(Some(self.hwnd), TIMER_PROFILE_RECORDING, 33, None);
                }
            }
            PPart::Stop => {
                if let Some(r) = self.profile.as_mut().and_then(|c| c.recording.as_mut()) {
                    r.finishing = true;
                    r.recorder.finish();
                }
            }
            PPart::DiscardRecording => {
                self.profile.as_mut().unwrap().recording = None;
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_PROFILE_RECORDING);
                }
                self.set_profile_focus(true);
            }
            PPart::File(k) => {
                if let Some(file) = self.profile_draft().and_then(|d| d.files.get(k)).cloned() {
                    if let Some(voice) = file.voice {
                        self.play_staged(&voice);
                    } else if let Some(asset) = file.attachment.or_else(|| {
                        file.upload
                            .as_ref()
                            .and_then(|u| u.result.get())
                            .and_then(|r| r.as_ref().ok())
                            .map(|(_, a, _)| a.clone())
                    }) {
                        self.open_viewer(vec![asset], 0);
                    }
                }
            }
            PPart::Remove(k) | PPart::Spoiler(k) | PPart::RetryFile(k) => {
                let id = self.profile.as_ref().unwrap().id.clone();
                let live = self.live.as_mut().unwrap();
                if let Some(draft) = live.profile_drafts.get_mut(&id)
                    && let Some(file) = draft.files.get_mut(k)
                {
                    match part {
                        PPart::Remove(_) => {
                            let file = draft.files.remove(k);
                            live.cancel_assets(file.uploaded_id().into_iter().collect());
                            if file.voice.is_some() {
                                self.playback.stop();
                            }
                            let remaining = live.profile_drafts[&id].files.len();
                            if let Some(c) = &mut self.profile {
                                c.file_start = c.file_start.min(
                                    remaining.saturating_sub(1) / VISIBLE_FILES * VISIBLE_FILES,
                                );
                            }
                        }
                        PPart::Spoiler(_) => file.spoiler = !file.spoiler,
                        _ => file.upload = None,
                    }
                }
                self.poll_profile_composer();
            }
            PPart::DiscardSend => {
                let id = self.profile.as_ref().unwrap().id.clone();
                let mut clear = false;
                if let Some(d) = self
                    .live
                    .as_mut()
                    .and_then(|l| l.profile_drafts.get_mut(&id))
                {
                    // Do not delete assets from a write the server may have accepted.
                    clear = d.retry.take().is_some_and(|s| !s.voice_only);
                    d.error = None;
                    if clear {
                        d.files.clear();
                    }
                }
                if clear {
                    self.profile.as_mut().unwrap().field.set_text(Vec::new());
                }
                if clear {
                    self.profile.as_mut().unwrap().file_start = 0;
                }
                self.set_profile_focus(true);
            }
            _ => {}
        }
        self.profile_changed();
    }

    pub fn profile_recording_tick(&mut self) {
        loop {
            let event = self
                .profile
                .as_ref()
                .and_then(|c| c.recording.as_ref())
                .and_then(|r| r.recorder.poll());
            let Some(event) = event else { break };
            if matches!(event, RecorderEvent::Started) {
                if let Some(r) = self.profile.as_mut().and_then(|c| c.recording.as_mut()) {
                    r.started = Some(Instant::now());
                }
                continue;
            }
            let Some(recording) = self.profile.as_mut().and_then(|c| c.recording.take()) else {
                break;
            };
            let id = self.profile.as_ref().unwrap().id.clone();
            let lost = recording.recorder.input_lost();
            match event {
                RecorderEvent::Ready(prepared) => {
                    if let Some(username) = self
                        .profile_view()
                        .filter(|v| v.can_message)
                        .and_then(|v| v.profile.username)
                        && let Some(live) = &mut self.live
                    {
                        live.profile_voice(&id, username, File::voice(Arc::new(*prepared)));
                    }
                }
                RecorderEvent::Failed(error) => {
                    if let Some(live) = &mut self.live {
                        live.profile_drafts.entry(id.clone()).or_default().error = Some(error);
                    }
                }
                _ => {}
            }
            if lost && let Some(live) = &mut self.live {
                live.profile_drafts.entry(id).or_default().error =
                    Some(voice_note::INPUT_LOST.into());
            }
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_PROFILE_RECORDING);
            }
            self.set_profile_focus(true);
        }
        if let Some(r) = self.profile.as_mut().and_then(|c| c.recording.as_mut()) {
            r.meter
                .sample(voice_note::perceptual(r.recorder.level()), Instant::now());
            if r.started
                .is_some_and(|s| s.elapsed() >= voice_note::MAX_DURATION)
            {
                r.finishing = true;
            }
        } else {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_PROFILE_RECORDING);
            }
        }
        self.profile_changed();
    }

    pub(super) fn draw_profile_compose(
        &self,
        g: &crate::gfx::Gfx,
        dc: &ID2D1DeviceContext,
        part: PPart,
        (x, y, w, h): Rect,
        enabled: bool,
        hover: bool,
    ) -> Result<()> {
        if hover {
            fill_round(dc, rect(x, y, x + w, y + h), 6.0, &g.brushes.hover);
        }
        let icon = match part {
            PPart::Gif => {
                let open = self.profile_gifs_open();
                let color = if !enabled {
                    crate::theme::border()
                } else if hover || open {
                    crate::theme::text()
                } else {
                    crate::theme::text_muted()
                };
                return crate::uploads::gif_button(g, dc, rect(x, y, x + w, y + h), color);
            }
            PPart::Attach => Icon::Plus,
            PPart::Emoji => Icon::Smile,
            PPart::Mic => Icon::Mic,
            PPart::Stop => Icon::Check,
            PPart::DiscardRecording | PPart::Remove(_) => Icon::Trash,
            PPart::RetryFile(_) => Icon::Retry,
            PPart::Spoiler(k) => {
                if self
                    .profile_draft()
                    .and_then(|d| d.files.get(k))
                    .is_some_and(|f| f.spoiler)
                {
                    Icon::EyeOff
                } else {
                    Icon::Eye
                }
            }
            PPart::File(k) => {
                if let Some(file) = self.profile_draft().and_then(|d| d.files.get(k)) {
                    let title = layout(g, &file.name(), &g.formats.direct_note, w - 8.0)?;
                    unsafe {
                        dc.PushAxisAlignedClip(
                            &rect(x, y, x + w, y + h),
                            D2D1_ANTIALIAS_MODE_ALIASED,
                        );
                    }
                    crate::text::draw(dc, &title, x + 4.0, y + 2.0, &g.brushes.text);
                    let (status, failed) = file.upload_status();
                    let status = if status.is_empty() {
                        if file.voice.is_some() {
                            "Play / pause voice message"
                        } else {
                            "Ready to send"
                        }
                    } else {
                        status
                    };
                    let note = layout(g, status, &g.formats.direct_note, w - 8.0)?;
                    crate::text::draw(
                        dc,
                        &note,
                        x + 4.0,
                        y + 20.0,
                        &g.solid(if failed {
                            crate::theme::error()
                        } else {
                            crate::theme::text_muted()
                        })?,
                    );
                    unsafe {
                        dc.PopAxisAlignedClip();
                    }
                }
                return Ok(());
            }
            PPart::RecordingStatus
            | PPart::DiscardSend
            | PPart::PreviousFiles
            | PPart::NextFiles => {
                let label = match part {
                    PPart::PreviousFiles => "Previous files".into(),
                    PPart::NextFiles => "Next files".into(),
                    _ => self.profile_part_name(part),
                };
                let l = layout(g, &label, &g.formats.direct_note, w - 8.0)?;
                crate::text::draw(dc, &l, x + 4.0, y + 4.0, &g.brushes.muted);
                if part == PPart::RecordingStatus
                    && let Some(r) = self.profile.as_ref().and_then(|c| c.recording.as_ref())
                {
                    crate::recording::live_levels(
                        dc,
                        rect(x + 4.0, y + 24.0, x + w - 4.0, y + h),
                        &r.meter.bars.iter().copied().collect::<Vec<_>>(),
                        r.meter.pending(),
                        if crate::files::motion() {
                            r.meter.scroll(Instant::now())
                        } else {
                            0.0
                        },
                    )?;
                }
                return Ok(());
            }
            _ => return Ok(()),
        };
        icons::draw(
            dc,
            icon,
            rect(
                x + (w - 18.0) / 2.0,
                y + (h - 18.0) / 2.0,
                x + (w + 18.0) / 2.0,
                y + (h + 18.0) / 2.0,
            ),
            if hover {
                crate::theme::text()
            } else {
                crate::theme::text_muted()
            },
            if enabled { 1.0 } else { 0.4 },
        );
        Ok(())
    }
}
