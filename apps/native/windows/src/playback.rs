//! Voice messages play in the app, as on the egui desktop: a card with
//! Play or Pause, the message's waveform (click or drag to seek) and its
//! time (the length, or what remains once started), then its transcript
//! ("Transcribing…" until it lands; three lines until "View transcript").
//! One voice message plays at a time, app-wide, through the bundled FFmpeg
//! (`caper_client::player`) and a loopback route to its media
//! (`caper_client::media_proxy`); reaching the end, or its message leaving
//! the conversation, returns it to idle.
use crate::files::File;
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::theme::*;
use crate::{App, EV_ROWS};
use caper_client::media_proxy::MediaProxy;
use caper_client::model::{Attachment, AttachmentStatus, TranscriptView};
use caper_client::player::{Player, State};
use caper_client::voice_note;
use std::collections::HashSet;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
use windows::core::Result;

/// Moves the playing card's clock and waveform, as egui's repaint.
pub const TIMER_PLAYBACK: usize = 14;
/// The player bar's height; its width is the row's, from 180 to 320.
pub const BAR_H: f32 = 48.0;
const MIN_W: f32 = 180.0;
const MAX_W: f32 = 320.0;
/// Waveform bars: 3 px wide, 5 px apart.
const WAVE_BAR: f32 = 3.0;
const WAVE_PITCH: f32 = 5.0;
/// The transcript sits right of its 2 px rule.
const RULE_PAD: f32 = 12.0;
const TOGGLE_H: f32 = 18.0;
const LINES: usize = 3;

/// A ready voice message plays in place (egui's `voice_message_card`);
/// others, and spoilers, keep the file card.
pub fn is_voice(a: &Attachment) -> bool {
    a.voice
        && !a.spoiler
        && !a.unavailable
        && a.status == AttachmentStatus::Ready
        && a.url.is_some()
}

/// The key in a message's `revealed` set that opens its transcript.
pub fn expanded_key(id: &str) -> String {
    format!("transcript:{id}")
}

/// A voice card's transcript, laid out once per row build.
pub struct Transcript {
    /// The words, or "Transcribing…" while pending.
    pub layout: IDWriteTextLayout,
    pub pending: bool,
    /// Longer than three lines: "View transcript" shows the rest.
    pub long: bool,
    /// Its toggle's width, the wider of its two labels.
    pub toggle_w: f32,
}

pub fn card_width(width: f32) -> f32 {
    width.clamp(MIN_W, MAX_W).min(width.max(1.0))
}

/// The transcript's layout, `None` when it has none to show.
pub fn transcript(g: &Gfx, a: &Attachment) -> Result<Option<Transcript>> {
    let (text, pending) = match a.transcript_view() {
        TranscriptView::Hidden => return Ok(None),
        TranscriptView::Pending => ("Transcribing\u{2026}", true),
        TranscriptView::Text(text) => (text, false),
    };
    let format = if pending {
        &g.formats.direct_note
    } else {
        &g.formats.picker
    };
    let layout = g.layout(&gfx::wide(text), format, MAX_W - RULE_PAD)?;
    if !pending {
        crate::emoji::apply(&layout, text, 12.5, 1.0)?;
        unsafe {
            let sign = g.dwrite.CreateEllipsisTrimmingSign(format)?;
            layout.SetTrimming(
                &DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_WORD,
                    ..Default::default()
                },
                &sign,
            )?;
        }
    }
    let mut toggle_w: f32 = 0.0;
    for label in ["View transcript", "Hide transcript"] {
        let l = g.layout(&gfx::wide(label), &g.formats.menu_title, 1e6)?;
        toggle_w = toggle_w.max(gfx::metrics(&l).widthIncludingTrailingWhitespace);
    }
    Ok(Some(Transcript {
        layout,
        pending,
        long: false,
        toggle_w,
    }))
}

fn line_heights(l: &IDWriteTextLayout) -> Vec<f32> {
    let mut count = 0;
    unsafe {
        let _ = l.GetLineMetrics(None, &mut count);
        let mut lines = vec![DWRITE_LINE_METRICS::default(); count as usize];
        let _ = l.GetLineMetrics(Some(&mut lines), &mut count);
        lines.iter().map(|m| m.height).collect()
    }
}

/// Fits a voice card `w` wide, its transcript open or clamped; its height.
pub fn place(f: &mut File, w: f32, expanded: bool) -> f32 {
    let Some(t) = &mut f.transcript else {
        return BAR_H;
    };
    let l = &t.layout;
    unsafe {
        let _ = l.SetMaxWidth((w - RULE_PAD).max(1.0));
        let _ = l.SetMaxHeight(f32::MAX);
    }
    let heights = line_heights(l);
    let long = !t.pending && heights.len() > LINES;
    t.long = long;
    let shown: f32 = if long && !expanded {
        let h = heights[..LINES].iter().sum();
        unsafe {
            let _ = l.SetMaxHeight(h);
        }
        h
    } else {
        heights.iter().sum()
    };
    let toggle = if long { 4.0 + TOGGLE_H } else { 0.0 };
    BAR_H + 6.0 + 2.0 + shown + toggle + 2.0
}

/// The card's parts under (x, y), relative to its rectangle `r`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Part {
    Play,
    /// How far along the waveform (0–1).
    Seek(f32),
    Toggle,
}

fn play_rect(r: D2D_RECT_F) -> D2D_RECT_F {
    let (cx, cy) = (r.left + 26.0, r.top + BAR_H / 2.0);
    rect(cx - 16.0, cy - 16.0, cx + 16.0, cy + 16.0)
}

/// The waveform's rectangle, left of the time (`time_w` wide).
fn wave_rect(r: D2D_RECT_F, time_w: f32) -> D2D_RECT_F {
    let play = play_rect(r);
    let time_left = r.right - 12.0 - time_w.max(28.0);
    rect(
        play.right + 12.0,
        r.top + 10.0,
        (time_left - 10.0).max(play.right + 13.0),
        r.top + BAR_H - 10.0,
    )
}

/// The time's widest label ("0:00"), so the waveform doesn't move.
const TIME_W: f32 = 28.0;

fn toggle_rect(f: &File, r: D2D_RECT_F) -> Option<D2D_RECT_F> {
    let t = f.transcript.as_ref().filter(|t| t.long)?;
    let x = r.left + RULE_PAD;
    let y = r.bottom - 2.0 - TOGGLE_H;
    Some(rect(x, y, x + t.toggle_w, y + TOGGLE_H))
}

pub fn part_at(f: &File, r: D2D_RECT_F, x: f32, y: f32) -> Option<Part> {
    let inside = |b: &D2D_RECT_F| x >= b.left && x < b.right && y >= b.top && y < b.bottom;
    if inside(&play_rect(r)) {
        return Some(Part::Play);
    }
    let wave = wave_rect(r, TIME_W);
    // The waveform's band, its full height, seeks.
    if inside(&rect(wave.left, r.top, wave.right, r.top + BAR_H)) {
        let w = (wave.right - wave.left).max(1.0);
        return Some(Part::Seek(((x - wave.left) / w).clamp(0.0, 1.0)));
    }
    if toggle_rect(f, r).is_some_and(|t| inside(&t)) {
        return Some(Part::Toggle);
    }
    None
}

/// What a card shows of the one voice message loaded.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct View {
    pub id: String,
    pub playing: bool,
    pub position: Option<u64>,
    pub error: Option<String>,
    /// A drag across its waveform, shown instead of the position.
    pub scrub: Option<f32>,
}

/// Draws a voice card (and its transcript) in `r`.
pub fn draw(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    f: &File,
    a: &Attachment,
    view: Option<&View>,
    revealed: &HashSet<String>,
    r: D2D_RECT_F,
) -> Result<()> {
    let view = view.filter(|v| v.id == a.id);
    let playing = view.is_some_and(|v| v.playing);
    let duration = a.duration_ms.unwrap_or(0);
    let bar = rect(r.left, r.top, r.right, r.top + BAR_H);
    fill_round(dc, bar, 8.0, &g.brushes.surface);
    stroke_round(dc, bar, 8.0, &g.brushes.border);
    let play = play_rect(r);
    let (cx, cy) = (
        (play.left + play.right) / 2.0,
        (play.top + play.bottom) / 2.0,
    );
    let disc = D2D1_ELLIPSE {
        point: windows_numerics::Vector2 { X: cx, Y: cy },
        radiusX: 16.0,
        radiusY: 16.0,
    };
    unsafe { dc.FillEllipse(&disc, &g.solid(ACTION)) };
    let nudge = if playing { 0.0 } else { 1.0 };
    icons::draw(
        dc,
        if playing { Icon::Pause } else { Icon::Play },
        rect(
            play.left + 9.0 + nudge,
            play.top + 9.0,
            play.right - 9.0 + nudge,
            play.bottom - 9.0,
        ),
        BLACKOUT,
        1.0,
    );
    // Idle shows the length; started, what remains.
    let position = view.and_then(|v| v.position);
    let shown = duration.saturating_sub(position.unwrap_or(0));
    let time = g.layout(
        &gfx::wide(&voice_note::duration_label(shown)),
        &g.formats.direct_note,
        1e6,
    )?;
    let tm = gfx::metrics(&time);
    crate::text::draw(
        dc,
        &time,
        r.right - 12.0 - tm.widthIncludingTrailingWhitespace,
        r.top + (BAR_H - tm.height) / 2.0,
        &g.brushes.muted,
    );
    let wave = wave_rect(r, TIME_W);
    match view.and_then(|v| v.error.as_deref()) {
        // Why it can't play takes the waveform's place.
        Some(error) => {
            let l = g.layout(
                &gfx::wide(error),
                &g.formats.file_detail,
                wave.right - wave.left,
            )?;
            let m = gfx::metrics(&l);
            crate::text::draw(
                dc,
                &l,
                wave.left,
                r.top + (BAR_H - m.height) / 2.0,
                &g.brushes.error,
            );
        }
        None => {
            let played = view.and_then(|v| v.scrub).unwrap_or(match position {
                Some(p) if duration > 0 => p as f32 / duration as f32,
                _ => 0.0,
            });
            waveform(g, dc, wave, a.waveform.as_deref(), played)?;
        }
    }
    let Some(t) = &f.transcript else {
        return Ok(());
    };
    let top = r.top + BAR_H + 6.0;
    let bottom = r.bottom;
    unsafe {
        dc.FillRectangle(&rect(r.left, top, r.left + 2.0, bottom), &g.brushes.border);
    }
    crate::text::draw(
        dc,
        &t.layout,
        r.left + RULE_PAD,
        top + 2.0,
        &g.brushes.muted,
    );
    if let Some(tr) = toggle_rect(f, r) {
        let open = revealed.contains(&expanded_key(&a.id));
        let label = if open {
            "Hide transcript"
        } else {
            "View transcript"
        };
        let l = g.layout(&gfx::wide(label), &g.formats.menu_title, 1e6)?;
        let m = gfx::metrics(&l);
        crate::text::draw(
            dc,
            &l,
            tr.left,
            tr.top + (TOGGLE_H - m.height) / 2.0,
            &g.brushes.text,
        );
    }
    Ok(())
}

/// The bars that fit `r`, the played part (0–1) in text colour over the
/// muted rest.
fn waveform(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    r: D2D_RECT_F,
    peaks: Option<&str>,
    played: f32,
) -> Result<()> {
    let width = r.right - r.left;
    let height = r.bottom - r.top;
    let count = (((width + WAVE_PITCH - WAVE_BAR) / WAVE_PITCH).floor() as usize).max(1);
    let levels = voice_note::bars(peaks, count);
    let split = r.left + width * played.clamp(0.0, 1.0);
    let rest = unsafe { dc.CreateSolidColorBrush(&gfx::color(TEXT_MUTED, 0.45), None)? };
    for (k, level) in levels.iter().enumerate() {
        let x = r.left + k as f32 * WAVE_PITCH;
        // Never shorter than its own width: silence reads as dots.
        let h = (level.clamp(0.0, 1.0) * height).max(WAVE_BAR);
        let bar = rect(
            x,
            r.top + (height - h) / 2.0,
            x + WAVE_BAR,
            r.top + (height + h) / 2.0,
        );
        fill_round(dc, bar, WAVE_BAR / 2.0, &rest);
        // The boundary bar is lit as far as it has played.
        if split > x {
            unsafe {
                dc.PushAxisAlignedClip(
                    &rect(x, bar.top - 1.0, split.min(x + WAVE_BAR), bar.bottom + 1.0),
                    D2D1_ANTIALIAS_MODE_ALIASED,
                );
            }
            fill_round(dc, bar, WAVE_BAR / 2.0, &g.brushes.text);
            unsafe { dc.PopAxisAlignedClip() };
        }
    }
    Ok(())
}

/// The one voice message loaded (playing, paused, or failed to start).
struct Current {
    id: String,
    player: Option<Player>,
    /// Its loopback route, removed when it stops.
    local: Option<String>,
    error: Option<String>,
}

#[derive(Default)]
pub struct Playback {
    current: Option<Current>,
    proxy: Option<MediaProxy>,
    /// A drag across a waveform: its row, attachment and how far.
    pub scrub: Option<(usize, usize, f32)>,
}

impl Playback {
    /// Plays or pauses `a` (or plays it from `at` ms), stopping any other
    /// voice message first.
    fn play(&mut self, a: &Attachment, at: Option<u64>) {
        if let Some(player) = self
            .current
            .as_mut()
            .filter(|c| c.id == a.id)
            .and_then(|c| c.player.as_mut())
        {
            match at {
                Some(ms) => player.seek(ms, true),
                None => player.toggle(),
            }
            return;
        }
        self.stop();
        let started = (|| {
            let binary = caper_client::ffmpeg::binary()
                .ok_or("Voice playback needs the FFmpeg that ships with Caper.")?;
            let unplayable = "This voice message can\u{2019}t be played here.";
            let url = a
                .url
                .as_deref()
                .filter(|url| caper_client::api::media_url(url).is_some())
                .ok_or(unplayable)?;
            if self.proxy.is_none() {
                self.proxy = MediaProxy::start().ok();
            }
            let local = self.proxy.as_ref().ok_or(unplayable)?.route(url);
            let mut player = Player::audio(binary, local.clone(), a.duration_ms);
            match at {
                Some(ms) => player.seek(ms, true),
                None => player.play(),
            }
            Ok::<_, &str>((player, local))
        })();
        self.current = Some(match started {
            Ok((player, local)) => Current {
                id: a.id.clone(),
                player: Some(player),
                local: Some(local),
                error: None,
            },
            Err(error) => Current {
                id: a.id.clone(),
                player: None,
                local: None,
                error: Some(error.into()),
            },
        });
    }

    pub fn stop(&mut self) {
        if let Some(Current {
            local: Some(local), ..
        }) = self.current.take()
            && let Some(proxy) = &self.proxy
        {
            proxy.remove(&local);
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.current.as_ref().map(|c| c.id.as_str())
    }

    /// Whether the clock moves: playing or starting.
    fn running(&self) -> bool {
        self.current
            .as_ref()
            .and_then(|c| c.player.as_ref())
            .is_some_and(|p| matches!(p.state(), State::Playing | State::Loading))
    }

    pub fn view(&self) -> Option<View> {
        let c = self.current.as_ref()?;
        let state = c.player.as_ref().map(|p| (p.state(), p.position_ms()));
        Some(View {
            id: c.id.clone(),
            playing: matches!(state, Some((State::Playing | State::Loading, _))),
            position: state.map(|(_, position)| position),
            error: c
                .error
                .clone()
                .or_else(|| c.player.as_ref().and_then(|p| p.error().map(str::to_owned))),
            scrub: None,
        })
    }
}

impl App {
    /// The rows showing attachment `id`.
    fn voice_message_rows(&self, id: &str) -> Vec<usize> {
        self.c
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.attachments.iter().any(|a| a.id == id))
            .map(|(i, _)| i)
            .collect()
    }

    fn voice_message_redraw(&mut self, id: Option<String>) {
        if let Some(id) = id {
            let rows = self.voice_message_rows(&id);
            self.redraw.extend(rows);
            self.events |= EV_ROWS;
            self.need = true;
        }
    }

    /// The clock runs only while a voice message plays or starts.
    fn arm_playback(&self) {
        unsafe {
            if self.playback.running() {
                SetTimer(Some(self.hwnd), TIMER_PLAYBACK, 33, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_PLAYBACK);
            }
        }
    }

    /// A click on a voice card's part (`k` of row `i`): Play or Pause,
    /// seek, or open or close the transcript.
    pub fn voice_message_activate(&mut self, i: usize, k: usize, part: Part) {
        let Some(a) = self
            .c
            .messages
            .get(i)
            .and_then(|m| m.attachments.get(k))
            .cloned()
        else {
            return;
        };
        match part {
            Part::Toggle => {
                let key = expanded_key(&a.id);
                let Some(m) = self.c.messages.get_mut(i) else {
                    return;
                };
                if !m.revealed.remove(&key) {
                    m.revealed.insert(key);
                }
                match self.rows.replace(&self.g, &self.c, i) {
                    Ok(true) => self.drawn = None,
                    Ok(false) => self.redraw.push(i),
                    Err(e) => eprintln!("transcript layout failed: {e:?}"),
                }
                self.events |= EV_ROWS;
                self.need = true;
                return;
            }
            Part::Play => self.playback.play(&a, None),
            Part::Seek(at) => {
                let duration = a.duration_ms.unwrap_or(0);
                if duration == 0 {
                    return;
                }
                self.playback.play(&a, Some((at * duration as f32) as u64));
            }
        }
        self.voice_message_redraw(Some(a.id));
        self.arm_playback();
    }

    /// The clock's step: the player advances; the end, or its message
    /// leaving the conversation, returns it to idle.
    pub fn playback_tick(&mut self) {
        let ended = self
            .playback
            .current
            .as_mut()
            .and_then(|c| c.player.as_mut())
            .is_some_and(|p| {
                p.update(|_, _| {});
                p.state() == State::Ended
            });
        let id = self.playback.id().map(str::to_owned);
        let gone = id
            .as_deref()
            .is_some_and(|id| self.voice_message_rows(id).is_empty());
        if ended || gone {
            self.playback.stop();
        }
        self.voice_message_redraw(id);
        self.arm_playback();
    }

    /// The conversation changed (another channel, sign-out): a voice
    /// message no longer shown stops.
    pub fn playback_follow(&mut self) {
        if let Some(id) = self.playback.id().map(str::to_owned)
            && self.voice_message_rows(&id).is_empty()
        {
            self.playback.stop();
            self.arm_playback();
        }
    }

    /// Row `i`'s voice card `k` and its part under client point (x, y).
    pub fn voice_message_at(&self, x: f32, y: f32) -> Option<(usize, usize, Part)> {
        let (i, k, _) = self.file_at(x, y)?;
        let m = self.c.messages.get(i)?;
        if !is_voice(m.attachments.get(k)?) {
            return None;
        }
        let f = self.rows.row(i)?.files.list.iter().find(|f| f.index == k)?;
        let (rx, ry, rw, rh) = self.file_rect(i, k)?;
        part_at(f, rect(rx, ry, rx + rw, ry + rh), x, y).map(|p| (i, k, p))
    }

    /// A drag across a voice card's waveform follows the pointer; letting
    /// go seeks there.
    pub fn voice_message_scrub(&mut self, x: f32, done: bool) {
        let Some((i, k, _)) = self.playback.scrub else {
            return;
        };
        let at = self.file_rect(i, k).and_then(|(rx, ry, rw, _)| {
            let f = self.rows.row(i)?.files.list.iter().find(|f| f.index == k)?;
            match part_at(
                f,
                rect(rx, ry, rx + rw, ry + BAR_H),
                x.clamp(rx, rx + rw - 1.0),
                ry + BAR_H / 2.0,
            )? {
                Part::Seek(at) => Some(at),
                // Past either end of the waveform.
                _ if x < rx + 58.0 => Some(0.0),
                _ => Some(1.0),
            }
        });
        let id = self
            .c
            .messages
            .get(i)
            .and_then(|m| m.attachments.get(k))
            .map(|a| a.id.clone());
        if done {
            self.playback.scrub = None;
            if let Some(at) = at {
                return self.voice_message_activate(i, k, Part::Seek(at));
            }
        } else if let Some(at) = at {
            self.playback.scrub = Some((i, k, at));
        }
        self.voice_message_redraw(id);
    }

    /// What the cards show, with any drag on the playing one's waveform.
    pub fn voice_message_view(&self) -> Option<View> {
        let mut view = self.playback.view();
        if let Some((i, k, at)) = self.playback.scrub
            && let Some(a) = self.c.messages.get(i).and_then(|m| m.attachments.get(k))
        {
            let v = view.get_or_insert_with(|| View {
                id: a.id.clone(),
                ..View::default()
            });
            if v.id == a.id {
                v.scrub = Some(at);
            }
        }
        view
    }
}
