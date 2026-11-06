//! Caper for Windows: Win32 window, a DirectComposition visual tree with one
//! retained surface per panel, Direct2D + DirectWrite drawing, UI Automation.
//!
//! Damage model: each panel (rail, sidebar, header, members, composer, hover
//! toolbar) has its own composition surface, redrawn only when its own state
//! changes. The message history is one tall virtual surface: scrolling moves
//! the visual without redrawing, only newly exposed strips are drawn, and
//! hover or selection redraw just the rows they touch.
//!
//! Not shipped yet: kept out of the release workflow and installers until it
//! reaches parity with the egui client. It shows a labelled development mock
//! conversation until the shared client core (`apps/native/client`) is wired
//! in.
#![windows_subsystem = "windows"]

mod actions;
mod bench;
mod card;
mod composer;
mod emoji;
mod overlay;
// The message grammar every client shares.
#[allow(dead_code)]
#[path = "../../../../shared/messages/formatting.rs"]
mod formatting;
mod gfx;
mod history;
mod icons;
#[allow(dead_code)]
#[path = "../../../../shared/messages/links.rs"]
mod links;
mod list;
mod live;
mod model;
mod signin;
mod text;
mod theme;
mod thread;
mod uia;
mod versions;

use bench::{Action, Bench};
use composer::Composer;
use gfx::{Gfx, fill_round, rect, stroke_round};
use list::Rows;
use model::Conversation;
use std::cell::RefCell;
use std::ops::Range;
use std::time::Instant;
use theme::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::Ime::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;
use windows_numerics::Matrix3x2;

/// Pixel row of the anchor (bottom of the startup history) in the virtual surface.
const ORIGIN: f64 = 4_000_000.0;
const VS_H: u32 = 1 << 23;
pub(crate) const EV_TEXT: u8 = 1;
pub(crate) const EV_FOCUS: u8 = 2;
const EV_ROWS: u8 = 4;
/// The sign-in screen appeared, changed step or went away.
pub(crate) const EV_SCREEN: u8 = 8;
const TIMER_BENCH: usize = 1;
pub(crate) const TIMER_CARET: usize = 2;
const TIMER_LIVE: usize = 3;
/// The sign-in screen's spinner and resend countdown.
const TIMER_SIGNIN: usize = 4;
/// A chip's who-reacted card, once the pointer rests on it.
pub(crate) const TIMER_CARD: usize = 5;
/// Re-reads the unread badges, as egui does every 15 s.
const TIMER_UNREAD: usize = 6;
pub(crate) const WHEEL_STEP: f64 = 48.0;
const WM_MOUSELEAVE: u32 = 0x02A3;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app unless it is already borrowed (re-entrant window
/// messages, UIA calls arriving mid-frame) or not created yet.
pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

#[derive(Default, Clone, Copy)]
struct Dirty {
    layout: bool,
    rail: bool,
    sidebar: bool,
    header: bool,
    members: bool,
    composer: bool,
    toolbar: bool,
    signin: bool,
    /// The message menu or a dialog.
    overlay: bool,
    picker: bool,
    reply: bool,
    /// The thread pane, and its reply box.
    thread: bool,
    thread_box: bool,
    /// The who-reacted card.
    card: bool,
}

impl Dirty {
    fn all() -> Self {
        Self {
            layout: true,
            rail: true,
            sidebar: true,
            header: true,
            members: true,
            composer: true,
            toolbar: true,
            signin: true,
            overlay: true,
            picker: true,
            reply: true,
            thread: true,
            thread_box: true,
            card: true,
        }
    }
}

struct Panel {
    visual: IDCompositionVisual2,
    surface: Option<IDCompositionSurface>,
    size: (u32, u32),
}

impl Panel {
    fn new(g: &Gfx) -> Result<Self> {
        Ok(Self {
            visual: g.visual()?,
            surface: None,
            size: (0, 0),
        })
    }

    /// Ensures a surface of this pixel size; true when it was (re)created.
    fn ensure(&mut self, g: &Gfx, w: u32, h: u32) -> Result<bool> {
        if self.surface.is_some() && self.size == (w, h) {
            return Ok(false);
        }
        let s = g.surface(w, h)?;
        unsafe { self.visual.SetContent(&s)? };
        self.surface = Some(s);
        self.size = (w, h);
        Ok(true)
    }

    fn offset(&self, x: f32, y: f32) -> Result<()> {
        unsafe {
            self.visual.SetOffsetX2(x)?;
            self.visual.SetOffsetY2(y)
        }
    }

    fn draw(&self, scale: f32, f: impl FnOnce(&ID2D1DeviceContext) -> Result<()>) -> Result<()> {
        match &self.surface {
            Some(s) => gfx::draw(s, scale, f),
            None => Ok(()),
        }
    }
}

/// A text selection across message rows: (row, UTF-16 position) anchor and focus.
pub type Sel = ((usize, u32), (usize, u32));

pub struct App {
    pub hwnd: HWND,
    pub g: Gfx,
    pub c: Conversation,
    bench: Bench,
    pub rows: Rows,
    pub composer: Composer,
    pub scale: f32,
    pub w: f32,
    pub h: f32,
    _target: IDCompositionTarget,
    background: IDCompositionVisual2,
    rail: Panel,
    sidebar: Panel,
    header: Panel,
    members: Panel,
    composer_p: Panel,
    toolbar: Panel,
    history_clip: IDCompositionVisual2,
    content: IDCompositionVisual2,
    vs: IDCompositionVirtualSurface,
    vs_w: u32,
    draw_w: u32,
    drawn_scale: f32,
    /// Pixel rows [a, b) of the virtual surface that hold current content.
    drawn: Option<(i64, i64)>,
    scroll_top: f64,
    at_end: bool,
    hover_row: Option<usize>,
    hover_space: Option<usize>,
    hover_channel: Option<usize>,
    hover_member: Option<usize>,
    pointer: Option<(f32, f32)>,
    tracking: bool,
    pub sel: Option<Sel>,
    selecting: bool,
    composer_drag: bool,
    redraw: Vec<usize>,
    dirty: Dirty,
    need: bool,
    box_h: f32,
    pending_resize: Option<(f32, f32)>,
    events: u8,
    visible: Range<usize>,
    channel_layouts: Vec<IDWriteTextLayout>,
    member_layouts: Vec<IDWriteTextLayout>,
    live: Option<live::Live>,
    ids: live::Ids,
    /// Shown in the header: the last error (true), or the gateway's
    /// connection state while it is not live (false).
    status: Option<(String, bool)>,
    /// The signed-out screens, over everything else while shown.
    pub signin: Option<signin::SignIn>,
    /// Unsent composer text of channels not on screen, by channel id.
    drafts: std::collections::HashMap<String, Vec<u16>>,
    signin_p: Panel,
    /// The open message menu and dialog, over the conversation.
    pub menu: Option<overlay::Menu>,
    pub dialog: Option<overlay::Dialog>,
    menu_p: Panel,
    dialog_p: Panel,
    /// The reaction picker, the reply above the composer, and the toolbar
    /// control under the pointer and what the toolbar last drew.
    pub picker: Option<actions::Picker>,
    picker_p: Panel,
    pub reply: Option<actions::ReplyBar>,
    reply_p: Panel,
    tool_hover: Option<usize>,
    bar_drawn: Option<actions::BarKey>,
    /// The history's head needs drawing; the pointer is on its button;
    /// the list has reached its newest message since it opened, so reaching
    /// its start loads the previous page (as in the egui client).
    redraw_head: bool,
    head_hover: bool,
    /// The foot (Load newer / Back to latest) needs drawing; the button
    /// under the pointer; View original's highlighted message.
    redraw_foot: bool,
    foot_hover: Option<usize>,
    focused: Option<String>,
    /// The who-reacted card, the chip it follows (message id and emoji),
    /// and its surface.
    card: Option<card::Card>,
    card_target: Option<(String, String)>,
    card_p: Panel,
    /// The history control with keyboard focus (a reaction chip, Add
    /// reaction, Retry or Dismiss), by message id so it survives paging.
    kb: Option<(String, list::Action)>,
    older_armed: bool,
    /// The open thread beside the conversation, its surface and its reply box's.
    pub thread: Option<thread::Pane>,
    thread_p: Panel,
    thread_box_p: Panel,
    quit: bool,
}

impl App {
    fn new(hwnd: HWND, g: Gfx, c: Conversation, bench: Bench) -> Result<Self> {
        unsafe {
            let target = g.dcomp.CreateTargetForHwnd(hwnd, true)?;
            let root = g.visual()?;
            root.SetBorderMode(DCOMPOSITION_BORDER_MODE_HARD)?;
            target.SetRoot(&root)?;
            // A 1×1 surface scaled to the window, behind everything.
            let background = g.visual()?;
            let solid = g.surface(1, 1)?;
            gfx::draw(&solid, 1.0, |dc| {
                dc.Clear(Some(&gfx::color(SURFACE_CONVERSATION, 1.0)));
                Ok(())
            })?;
            background.SetContent(&solid)?;
            background.SetBitmapInterpolationMode(
                DCOMPOSITION_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            )?;
            let rail = Panel::new(&g)?;
            let sidebar = Panel::new(&g)?;
            let header = Panel::new(&g)?;
            let members = Panel::new(&g)?;
            let composer_p = Panel::new(&g)?;
            let toolbar = Panel::new(&g)?;
            let signin_p = Panel::new(&g)?;
            let menu_p = Panel::new(&g)?;
            let dialog_p = Panel::new(&g)?;
            let picker_p = Panel::new(&g)?;
            let reply_p = Panel::new(&g)?;
            let thread_p = Panel::new(&g)?;
            let thread_box_p = Panel::new(&g)?;
            let card_p = Panel::new(&g)?;
            let history_clip = g.visual()?;
            let content = g.visual()?;
            let vs = g.dcomp.CreateVirtualSurface(
                1,
                VS_H,
                windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
                windows::Win32::Graphics::Dxgi::Common::DXGI_ALPHA_MODE_PREMULTIPLIED,
            )?;
            content.SetContent(&vs)?;
            history_clip.AddVisual(&content, false, None)?;
            history_clip.AddVisual(&toolbar.visual, true, &content)?;
            root.AddVisual(&background, false, None)?;
            let mut prev: IDCompositionVisual = background.cast()?;
            for v in [
                &rail.visual,
                &sidebar.visual,
                &header.visual,
                &history_clip,
                &composer_p.visual,
                &reply_p.visual,
                &thread_p.visual,
                &thread_box_p.visual,
                &members.visual,
            ] {
                root.AddVisual(v, true, &prev)?;
                prev = v.cast()?;
            }
            root.AddVisual(&card_p.visual, true, &prev)?;
            root.AddVisual(&menu_p.visual, true, &card_p.visual)?;
            root.AddVisual(&picker_p.visual, true, &menu_p.visual)?;
            root.AddVisual(&dialog_p.visual, true, &picker_p.visual)?;
            root.AddVisual(&signin_p.visual, true, &dialog_p.visual)?;
            let (channel_layouts, member_layouts) = side_layouts(&g, &c)?;
            let n = c.messages.len();
            let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
            let mut rc = RECT::default();
            GetClientRect(hwnd, &mut rc)?;
            Ok(Self {
                hwnd,
                g,
                c,
                bench,
                rows: Rows::new(n),
                composer: Composer::default(),
                scale,
                w: rc.right as f32 / scale,
                h: rc.bottom as f32 / scale,
                _target: target,
                background,
                rail,
                sidebar,
                header,
                members,
                composer_p,
                toolbar,
                history_clip,
                content,
                vs,
                vs_w: 1,
                draw_w: 1,
                drawn_scale: scale,
                drawn: None,
                scroll_top: 0.0,
                at_end: true,
                hover_row: None,
                hover_space: None,
                hover_channel: None,
                hover_member: None,
                pointer: None,
                tracking: false,
                sel: None,
                selecting: false,
                composer_drag: false,
                redraw: Vec::new(),
                dirty: Dirty::all(),
                need: true,
                box_h: 0.0,
                pending_resize: None,
                events: 0,
                visible: 0..0,
                channel_layouts,
                member_layouts,
                live: None,
                ids: live::Ids::default(),
                status: None,
                signin: None,
                drafts: Default::default(),
                signin_p,
                menu: None,
                dialog: None,
                menu_p,
                dialog_p,
                picker: None,
                picker_p,
                reply: None,
                reply_p,
                tool_hover: None,
                bar_drawn: None,
                redraw_head: false,
                head_hover: false,
                redraw_foot: false,
                foot_hover: None,
                focused: None,
                kb: None,
                card: None,
                card_target: None,
                card_p,
                older_armed: false,
                thread: None,
                thread_p,
                thread_box_p,
                quit: false,
            })
        }
    }

    /// Shows another conversation (the live one replacing the mock).
    fn set_conversation(&mut self, c: Conversation) -> Result<()> {
        let (channels, members) = side_layouts(&self.g, &c)?;
        self.rows = Rows::new(c.messages.len());
        self.c = c;
        self.channel_layouts = channels;
        self.member_layouts = members;
        self.sel = None;
        self.hover_row = None;
        self.menu = None;
        self.picker = None;
        self.older_armed = false;
        self.focused = None;
        self.foot_hover = None;
        if self.thread.take().is_some() {
            self.dirty.layout = true;
        }
        self.scroll_top = 0.0;
        self.at_end = true;
        self.drawn = None;
        self.dirty = Dirty::all();
        self.events |= EV_ROWS;
        self.need = true;
        Ok(())
    }

    fn drain_live(&mut self) -> Result<()> {
        let Some(live) = self.live.as_mut() else {
            return Ok(());
        };
        let updates = live.drain();
        let requests = live
            .host
            .requests
            .lock()
            .map(|mut r| std::mem::take(&mut *r))
            .unwrap_or_default();
        unsafe {
            if let Some(delay) = requests.wake_after {
                SetTimer(
                    Some(self.hwnd),
                    TIMER_LIVE,
                    delay.as_millis().clamp(1, u32::MAX as u128) as u32,
                    None,
                );
            }
            if requests.front {
                let _ = ShowWindow(self.hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(self.hwnd);
            }
            if let Some(title) = requests.title {
                let _ = SetWindowTextW(self.hwnd, &HSTRING::from(title));
            }
        }
        for u in updates {
            match u {
                live::Update::SignedOut(warning) => {
                    if let Some(s) = &mut self.signin {
                        s.restoring = false;
                        s.error = warning;
                    }
                    self.signin_screen_changed();
                    self.set_signin_focus(true);
                }
                live::Update::CodeRequested(result) => {
                    if let Some(s) = &mut self.signin {
                        s.code_requested(result);
                    }
                    // The code step's field takes focus, as on web.
                    self.signin_screen_changed();
                    self.set_signin_focus(true);
                }
                live::Update::VerifyFailed(error, attempts) => {
                    if let Some(s) = &mut self.signin {
                        s.verify_failed(error, attempts);
                    }
                    self.signin_screen_changed();
                }
                live::Update::SignedIn => {
                    self.set_signin_focus(false);
                    if let Some(s) = &mut self.signin {
                        s.restoring = true;
                    }
                    self.signin_screen_changed();
                }
                live::Update::Loaded(loaded) => {
                    let (c, ids) = *loaded;
                    // Unsent text stays with its channel, as in the egui client.
                    if let Some(old) = self.ids.channels.get(self.c.channel).cloned() {
                        let text = std::mem::take(&mut self.composer.text);
                        if text.is_empty() {
                            self.drafts.remove(&old);
                        } else {
                            self.drafts.insert(old, text);
                        }
                    }
                    // A reply belongs to the channel it was started in.
                    if self.ids.channels.get(self.c.channel) != ids.channels.get(c.channel) {
                        self.reply = None;
                    }
                    self.ids = ids;
                    self.status = None;
                    self.close_signin()?;
                    self.set_conversation(c)?;
                    let new = self.ids.channels.get(self.c.channel);
                    self.composer.text = new
                        .and_then(|id| self.drafts.remove(id))
                        .unwrap_or_default();
                    self.composer.caret = self.composer.text.len();
                    self.composer.anchor = self.composer.caret;
                    self.composer.comp.clear();
                    self.composer_changed();
                }
                live::Update::Older(older) => {
                    let (c, ids) = *older;
                    self.prepend_history(c, ids)?;
                }
                live::Update::Context(context, anchor) => {
                    let (mut c, ids) = *context;
                    c.presence = self.c.presence;
                    for (new, old) in c.members.iter_mut().zip(&self.c.members) {
                        new.online = old.online;
                    }
                    self.ids = ids;
                    self.set_conversation(c)?;
                    self.focus_message(&anchor)?;
                }
                live::Update::Newer(newer) => {
                    let (c, ids) = *newer;
                    self.append_history(c, ids)?;
                }
                live::Update::Focus(id) => self.focus_message(&id)?,
                // Names for a chip's card arrived; the next frame shows them.
                live::Update::Reactors => self.need = true,
                live::Update::Badges => {
                    self.dirty.rail = true;
                    self.dirty.sidebar = true;
                    self.events |= EV_ROWS;
                    self.need = true;
                }
                live::Update::Unread(id) => self.mark_unread(&id)?,
                live::Update::Thread => self.thread_refresh()?,
                live::Update::OlderState => {
                    self.redraw_head = true;
                    self.redraw_foot = true;
                    self.events |= EV_ROWS;
                    self.need = true;
                }
                live::Update::Message(m) => {
                    self.history_follow(&m);
                    let me = self.live.as_ref().and_then(|l| l.me.clone());
                    let error = self
                        .live
                        .as_ref()
                        .and_then(|l| l.reactions.error(&m.id).map(str::to_owned));
                    if let Some(&i) = self.ids.rows.get(&m.id) {
                        // A shown message changed: rebuild only its row, and
                        // redraw below it when its height changed.
                        let old = &self.c.messages[i];
                        let mut new = live::message(&m, old.author, old.grouped, me.as_deref());
                        new.reaction_error = error;
                        new.date = old.date.clone();
                        new.unread = old.unread;
                        if new == *old {
                            continue;
                        }
                        self.c.messages[i] = new;
                        if self.rows.replace(&self.g, &self.c, i)? {
                            self.drawn = None;
                        } else {
                            self.redraw.push(i);
                        }
                        self.events |= EV_ROWS;
                        self.need = true;
                        continue;
                    }
                    let Some((author, grouped, date)) = self.ids.place(&m) else {
                        continue;
                    };
                    if author >= self.c.members.len() {
                        self.c
                            .members
                            .push(live::person(&m.author.id, &m.author.name));
                    }
                    let mut new = live::message(&m, author, grouped, me.as_deref());
                    new.reaction_error = error;
                    new.date = date;
                    self.c.messages.push(new);
                    self.rows.append(&self.g, &self.c)?;
                    self.need = true;
                }
                live::Update::Edited {
                    request,
                    reloaded,
                    result,
                } => self.edited(request, reloaded, result),
                live::Update::Deleted(request, error) => self.deleted(request, error),
                live::Update::Versions {
                    request,
                    message,
                    result,
                } => self.versions_loaded(request, &message, result),
                live::Update::Presence(online) => {
                    self.c.presence = true;
                    for (person, id) in self.c.members.iter_mut().zip(&self.ids.members) {
                        person.online = online.contains(id);
                    }
                    self.dirty.members = true;
                    self.need = true;
                }
                live::Update::Connection(state) => {
                    if self.status.as_ref().is_none_or(|(_, error)| !error) {
                        self.status = state.map(|s| (s, false));
                        self.dirty.header = true;
                        self.need = true;
                    }
                }
                live::Update::Failed(e) => {
                    if let Some(s) = self.signin.as_mut().filter(|s| s.restoring) {
                        // Signed in, but nothing to open: say why instead of spinning.
                        s.failure = Some(e);
                        self.signin_screen_changed();
                        continue;
                    }
                    self.status = Some((e, true));
                    self.dirty.header = true;
                    self.need = true;
                }
            }
        }
        Ok(())
    }

    // ---- geometry (client DIPs) ----

    /// Keeps row layouts only for the drawn rows and the selection, so a
    /// long history read to its start costs its heights, not its layouts.
    fn trim_rows(&mut self) {
        let Some((d0, d1)) = self.drawn else {
            return;
        };
        let s = self.scale as f64;
        let mut keep = self
            .rows
            .range((d0 as f64 - ORIGIN) / s, (d1 as f64 - ORIGIN) / s);
        if let Some(((a, _), (b, _))) = self.sel {
            keep = keep.start.min(a.min(b))..keep.end.max(a.max(b) + 1);
        }
        self.rows.trim(keep);
    }

    /// Unread state for the rail and sidebar, as egui shows it: per space,
    /// anything new and the mentions of you; per joined channel, anything new.
    pub fn badges(&self) -> (Vec<(bool, u64)>, Vec<bool>) {
        let unread = self
            .live
            .as_ref()
            .map_or(&[][..], |l| l.unread_spaces.as_slice());
        let spaces = self
            .ids
            .spaces
            .iter()
            .map(|id| {
                unread
                    .iter()
                    .filter(|s| s.id == *id)
                    .fold((false, 0), |(any, n), s| {
                        (
                            any || !s.channels.is_empty(),
                            n + s
                                .channels
                                .iter()
                                .filter_map(|c| c.mention_count)
                                .sum::<u64>(),
                        )
                    })
            })
            .collect();
        let channels = self
            .ids
            .channels
            .iter()
            .map(|id| {
                unread
                    .iter()
                    .any(|s| s.channels.iter().any(|c| c.id == *id))
            })
            .collect();
        (spaces, channels)
    }

    pub fn list_x(&self) -> f32 {
        RAIL_W + SIDEBAR_W
    }

    pub fn list_w(&self) -> f32 {
        (self.w - RAIL_W - SIDEBAR_W - MEMBERS_W - self.thread_w()).max(160.0)
    }

    pub fn box_rect(&self) -> (f32, f32, f32, f32) {
        (
            self.list_x() + 18.0,
            self.h - 16.0 - self.box_h,
            self.list_w() - 36.0,
            self.box_h,
        )
    }

    /// Height of the visible history.
    pub fn vh(&self) -> f32 {
        (self.box_rect().1 - 10.0 - HEADER_H - self.reply_h()).max(0.0)
    }

    pub fn visible_rows(&self) -> Range<usize> {
        self.rows
            .range(self.scroll_top, self.scroll_top + self.vh() as f64)
    }

    pub fn composer_text_origin(&self) -> (f32, f32) {
        let (x, y, _, _) = self.box_rect();
        (
            x + composer::PAD,
            y + composer::PAD - self.composer.scroll(),
        )
    }

    /// Client position of row `i`'s message text origin.
    pub fn row_text_origin(&self, i: usize) -> (f32, f32) {
        let ty = self.rows.row(i).map_or(0.0, |r| r.text_y);
        (
            self.list_x() + list::BODY_X,
            HEADER_H
                + (self.rows.top[i] - self.scroll_top) as f32
                + list::content_top(self.c.messages[i].grouped)
                + ty,
        )
    }

    pub fn space_rect(i: usize) -> (f32, f32, f32, f32) {
        (
            (RAIL_W - RAIL_ITEM) / 2.0,
            RAIL_TOP + RAIL_PITCH * i as f32,
            RAIL_ITEM,
            RAIL_ITEM,
        )
    }

    pub fn channel_rect(i: usize) -> (f32, f32, f32, f32) {
        (
            RAIL_W + 12.0,
            CHANNEL_TOP + CHANNEL_H * i as f32,
            SIDEBAR_W - 24.0,
            CHANNEL_H - 2.0,
        )
    }

    pub fn member_rect(&self, i: usize) -> (f32, f32, f32, f32) {
        (
            self.w - MEMBERS_W + 8.0,
            MEMBER_TOP + MEMBER_H * i as f32,
            MEMBERS_W - 16.0,
            MEMBER_H,
        )
    }

    /// The space a row's date divider takes above it.
    pub fn row_lead(&self, i: usize) -> f32 {
        self.rows.row(i).map_or(0.0, |r| r.lead)
    }

    pub fn row_rect(&self, i: usize) -> (f32, f32, f32, f32) {
        (
            self.list_x(),
            HEADER_H + (self.rows.top[i] - self.scroll_top) as f32,
            self.list_w(),
            self.rows.height(i),
        )
    }

    // ---- state changes ----

    pub fn composer_changed(&mut self) {
        self.composer.caret_on = true;
        let (_, _, bw, _) = self.box_rect();
        if let Ok(h) = self.composer.relayout(&self.g, bw - 2.0 * composer::PAD)
            && (h - self.box_h).abs() > 0.01
        {
            self.dirty.layout = true;
        }
        self.dirty.composer = true;
        self.events |= EV_TEXT;
        self.need = true;
        self.ime_position();
    }

    pub fn set_composer_focus(&mut self, focused: bool) {
        // One text field has focus: the reply box in the thread gives it up.
        if focused
            && let Some(t) = &mut self.thread
            && t.composer.focused
        {
            t.composer.focused = false;
            self.dirty.thread_box = true;
        }
        if focused {
            self.set_kb(None);
        }
        if self.composer.focused == focused {
            return;
        }
        self.composer.focused = focused;
        self.composer.caret_on = true;
        unsafe {
            if focused {
                SetTimer(Some(self.hwnd), TIMER_CARET, GetCaretBlinkTime(), None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_CARET);
            }
        }
        self.dirty.composer = true;
        self.events |= EV_FOCUS;
        self.need = true;
    }

    // ---- sign-in ----

    /// The sign-in screen needs drawing and its UIA tree changed.
    fn signin_screen_changed(&mut self) {
        self.arm_signin_timer();
        self.dirty.signin = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    /// Runs the spinner and countdown timer only while one is shown.
    fn arm_signin_timer(&mut self) {
        let Some(s) = &self.signin else { return };
        // The spinner turns and the countdown ticks only while shown.
        let tick = if s.restoring && s.failure.is_none() {
            Some(std::time::Duration::from_millis(33))
        } else {
            s.next_tick()
        };
        unsafe {
            match tick {
                Some(d) => {
                    SetTimer(
                        Some(self.hwnd),
                        TIMER_SIGNIN,
                        d.as_millis().clamp(10, u32::MAX as u128) as u32,
                        None,
                    );
                }
                None => {
                    let _ = KillTimer(Some(self.hwnd), TIMER_SIGNIN);
                }
            }
        }
    }

    pub fn set_signin_focus(&mut self, focused: bool) {
        let hwnd = self.hwnd;
        let Some(s) = &mut self.signin else { return };
        let focused = focused && !s.restoring;
        let field = s.field_mut();
        if field.focused == focused {
            return;
        }
        field.focused = focused;
        field.caret_on = true;
        unsafe {
            if focused {
                SetTimer(Some(hwnd), TIMER_CARET, GetCaretBlinkTime(), None);
            } else {
                let _ = KillTimer(Some(hwnd), TIMER_CARET);
            }
        }
        self.dirty.signin = true;
        self.events |= EV_FOCUS;
        self.need = true;
    }

    /// After the sign-in field's text or caret changed.
    pub fn signin_changed(&mut self, text: bool) {
        let Some(s) = &mut self.signin else { return };
        if text {
            s.edited();
        }
        s.field_mut().caret_on = true;
        if let Err(e) = s.relayout_field(&self.g, self.w) {
            eprintln!("sign-in layout failed: {e:?}");
        }
        self.dirty.signin = true;
        self.events |= EV_TEXT;
        self.need = true;
        self.ime_position();
    }

    /// Clicks, Enter and UIA Invoke on the sign-in form.
    pub fn signin_activate(&mut self, part: Option<signin::Part>) {
        let Some(s) = &mut self.signin else { return };
        let request = match part {
            Some(p) => s.activate(p),
            None => s.submit(),
        };
        if let (Some(request), Some(live)) = (request, &self.live) {
            match request {
                signin::Request::Code(email) => live.request_code(email),
                signin::Request::Verify { challenge, code } => live.verify(challenge, code),
            }
        }
        if part == Some(signin::Part::OtherEmail) {
            self.set_signin_focus(true);
        }
        self.signin_screen_changed();
    }

    /// Signed in and loaded: the conversation replaces the sign-in screen.
    fn close_signin(&mut self) -> Result<()> {
        if self.signin.take().is_none() {
            return Ok(());
        }
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SIGNIN);
            let _ = KillTimer(Some(self.hwnd), TIMER_CARET);
            self.signin_p.visual.SetContent(None)?;
        }
        self.signin_p.surface = None;
        self.signin_p.size = (0, 0);
        self.dirty = Dirty::all();
        self.events |= EV_SCREEN;
        self.need = true;
        Ok(())
    }

    /// The focused text field and its text origin (client DIPs).
    pub fn focused_field(&self) -> Option<(&Composer, (f32, f32))> {
        if let Some(d) = &self.dialog {
            return d.field.focused.then(|| (&d.field, d.text_origin()));
        }
        if let Some(s) = &self.signin {
            return s.field().focused.then(|| (s.field(), s.field_origin));
        }
        if let Some(p) = &self.picker {
            return p.search.focused.then(|| (&p.search, p.text_origin()));
        }
        if let Some(t) = &self.thread
            && t.composer.focused
        {
            return Some((&t.composer, self.thread_text_origin()));
        }
        self.composer
            .focused
            .then(|| (&self.composer, self.composer_text_origin()))
    }

    fn focused_field_mut(&mut self) -> Option<&mut Composer> {
        if let Some(d) = &mut self.dialog {
            return Some(&mut d.field).filter(|f| f.focused && d.kind == overlay::Kind::Edit);
        }
        if let Some(s) = &mut self.signin {
            return Some(s.field_mut()).filter(|f| f.focused);
        }
        if let Some(p) = &mut self.picker {
            return Some(&mut p.search).filter(|f| f.focused);
        }
        if let Some(t) = &mut self.thread
            && t.composer.focused
        {
            return Some(&mut t.composer);
        }
        Some(&mut self.composer).filter(|c| c.focused)
    }

    /// After an edit or caret move in the focused field.
    fn field_changed(&mut self, text: bool) {
        if self.dialog.is_some() {
            self.dialog_changed();
        } else if self.signin.is_some() {
            self.signin_changed(text);
        } else if self.picker.is_some() {
            self.picker_changed(text);
        } else if self.thread.as_ref().is_some_and(|t| t.composer.focused) {
            self.thread_composer_changed();
        } else {
            self.composer_changed();
        }
    }

    pub fn select_space(&mut self, i: usize) {
        if let Some(live) = &mut self.live {
            // Its first joined channel opens once the worker has read it.
            if i != self.c.space
                && let Some(id) = self.ids.spaces.get(i)
            {
                live.navigate(Some(id.clone()), None);
            }
            return;
        }
        if i != self.c.space && i < self.c.spaces.len() {
            self.c.space = i;
            self.dirty.rail = true;
            self.dirty.sidebar = true;
            self.need = true;
        }
    }

    pub fn select_channel(&mut self, i: usize) {
        if let Some(live) = &mut self.live {
            if i != self.c.channel
                && let Some(id) = self.ids.channels.get(i)
            {
                let space = self.ids.spaces.get(self.c.space).cloned();
                live.navigate(space, Some(id.clone()));
            }
            return;
        }
        if i != self.c.channel && i < self.c.channels.len() {
            self.c.channel = i;
            self.dirty.sidebar = true;
            self.dirty.header = true;
            self.dirty.composer = true;
            self.need = true;
        }
    }

    /// The selected UTF-16 range of row `i`'s text, if any.
    pub fn sel_for(sel: Option<Sel>, i: usize, len: u32) -> Option<(u32, u32)> {
        let (a, b) = sel?;
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        if i < a.0 || i > b.0 {
            return None;
        }
        let s = if i == a.0 { a.1 } else { 0 };
        let e = if i == b.0 { b.1 } else { len };
        (e > s).then_some((s, e))
    }

    pub fn row_len(&self, i: usize) -> u32 {
        self.rows.row(i).map_or(0, |r| r.plain16.len() as u32)
    }

    pub fn set_sel(&mut self, new: Option<Sel>) {
        let old = self.sel;
        let span = |s: Option<Sel>| s.map(|((a, _), (b, _))| (a.min(b), a.max(b)));
        let (lo, hi) = match (span(old), span(new)) {
            (Some(a), Some(b)) => (a.0.min(b.0), a.1.max(b.1)),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => return,
        };
        for i in lo..=hi.min(self.rows.len().saturating_sub(1)) {
            let len = self.row_len(i);
            if Self::sel_for(old, i, len) != Self::sel_for(new, i, len) {
                self.redraw.push(i);
            }
        }
        self.sel = new;
        self.need = true;
    }

    fn msg_hit(&self, x: f32, y: f32) -> Option<(usize, u32)> {
        let ay = self.scroll_top + (y - HEADER_H) as f64;
        let vis = self.visible_rows();
        if vis.is_empty() {
            return None;
        }
        let i = self
            .rows
            .row_at(ay)
            .unwrap_or(if ay < self.rows.top[vis.start] {
                vis.start
            } else {
                vis.end - 1
            });
        let row = self.rows.row(i)?;
        let (ox, oy) = self.row_text_origin(i);
        Some((i, gfx::hit_point(&row.text, x - ox, y - oy)))
    }

    fn selected_text(&self) -> Vec<u16> {
        let mut out = Vec::new();
        let Some(((ra, _), (rb, _))) = self.sel else {
            return out;
        };
        for i in ra.min(rb)..=ra.max(rb) {
            if let (Some((s, e)), Some(r)) = (
                Self::sel_for(self.sel, i, self.row_len(i)),
                self.rows.row(i),
            ) {
                if !out.is_empty() {
                    out.extend_from_slice(&gfx::wide("\r\n"));
                }
                out.extend_from_slice(&r.plain16[s as usize..e as usize]);
            }
        }
        out
    }

    fn in_box(&self, x: f32, y: f32) -> bool {
        let (bx, by, bw, bh) = self.box_rect();
        x >= bx && x < bx + bw && y >= by && y < by + bh
    }

    fn in_history(&self, x: f32, y: f32) -> bool {
        x >= self.list_x()
            && x < self.list_x() + self.list_w()
            && y >= HEADER_H
            && y < HEADER_H + self.vh()
    }

    fn update_hover(&mut self) {
        let inside = |(l, t, w, h): (f32, f32, f32, f32), x: f32, y: f32| {
            x >= l && x < l + w && y >= t && y < t + h
        };
        let (mut space, mut channel, mut member) = (None, None, None);
        if let Some((x, y)) = self.pointer {
            space = (0..self.c.spaces.len()).find(|&i| inside(Self::space_rect(i), x, y));
            channel = (0..self.c.channels.len()).find(|&i| inside(Self::channel_rect(i), x, y));
            member = (0..self.c.listed_members).find(|&i| inside(self.member_rect(i), x, y));
        }
        if space != self.hover_space {
            self.hover_space = space;
            self.dirty.rail = true;
            self.need = true;
        }
        if channel != self.hover_channel {
            self.hover_channel = channel;
            self.dirty.sidebar = true;
            self.need = true;
        }
        if member != self.hover_member {
            self.hover_member = member;
            self.dirty.members = true;
            self.need = true;
        }
    }

    fn history_hover(&self) -> Option<usize> {
        let (x, y) = self.pointer?;
        if !self.in_history(x, y) {
            return None;
        }
        // The toolbar reaches above its row: it keeps that row hovered.
        if let Some((i, (tx, ty, tw, th))) = self.toolbar_rect()
            && x >= tx
            && x < tx + tw
            && y >= ty
            && y < ty + th
        {
            return Some(i);
        }
        self.rows.row_at(self.scroll_top + (y - HEADER_H) as f64)
    }

    /// Sends the composer's text. Until the client core is wired in, the
    /// message only joins the local development mock.
    fn send(&mut self) -> Result<()> {
        let text = String::from_utf16_lossy(&self.composer.text)
            .trim()
            .to_string();
        if text.is_empty() {
            return Ok(());
        }
        if let Some(live) = &self.live {
            let reply = self.reply.take().map(|r| r.target.id);
            if reply.is_some() {
                self.dirty.layout = true;
                self.dirty.reply = true;
                self.events |= EV_SCREEN;
            }
            live.send(text, reply, None);
            self.set_focused(None);
        } else {
            self.c.send_local(text);
            self.rows.append(&self.g, &self.c)?;
            self.at_end = true;
        }
        self.composer.text.clear();
        self.composer.caret = 0;
        self.composer.anchor = 0;
        self.composer_changed();
        Ok(())
    }

    // ---- input ----

    fn handle(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let x = (lp.0 & 0xffff) as i16 as f32 / self.scale;
        let y = ((lp.0 >> 16) & 0xffff) as i16 as f32 / self.scale;
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        if self.signin.is_some()
            && let Some(r) = self.signin_input(msg, wp, lp, x, y, ctrl, shift)
        {
            return r;
        }
        if let Some(r) = self.overlay_input(msg, wp, x, y, ctrl, shift) {
            return r;
        }
        if self.picker.is_some()
            && let Some(r) = self.picker_input(msg, wp, x, y, ctrl, shift)
        {
            return r;
        }
        if self.thread.is_some()
            && let Some(r) = self.thread_input(msg, wp, x, y, ctrl, shift)
        {
            return r;
        }
        match msg {
            WM_SIZE => {
                let (pw, ph) = ((lp.0 & 0xffff) as f32, ((lp.0 >> 16) & 0xffff) as f32);
                if pw > 0.0 && ph > 0.0 {
                    self.w = pw / self.scale;
                    self.h = ph / self.scale;
                    self.dirty.layout = true;
                    self.dirty.signin = true;
                    self.close_menu();
                    self.dialog_changed();
                    self.need = true;
                }
            }
            WM_TIMER => match wp.0 {
                TIMER_BENCH if self.bench.tick() => self.quit = true,
                TIMER_LIVE => unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_LIVE);
                    let _ = PostMessageW(Some(self.hwnd), live::WM_LIVE, WPARAM(0), LPARAM(0));
                },
                TIMER_CARET => {
                    if let Some(f) = self.focused_field_mut() {
                        f.caret_on = !f.caret_on;
                    }
                    if self.dialog.is_some() {
                        self.dirty.overlay = true;
                    } else if self.signin.is_some() {
                        self.dirty.signin = true;
                    } else if self.thread.as_ref().is_some_and(|t| t.composer.focused) {
                        self.dirty.thread_box = true;
                    } else {
                        self.dirty.composer = true;
                    }
                    self.need = true;
                }
                TIMER_CARD => self.show_card(),
                TIMER_UNREAD => {
                    if let Some(l) = &mut self.live {
                        l.refresh_unread();
                    }
                }
                TIMER_SIGNIN => {
                    if let Some(s) = &mut self.signin {
                        s.spin = (s.spin + 0.21) % std::f32::consts::TAU;
                    }
                    self.arm_signin_timer();
                    self.dirty.signin = true;
                    self.need = true;
                }
                _ => {}
            },
            WM_MOUSEMOVE => {
                self.track_leave();
                self.pointer = Some((x, y));
                self.update_hover();
                if self.history_hover() != self.hover_row {
                    self.need = true;
                }
                self.track_tool_hover();
                self.track_reply_hover();
                self.track_head_hover();
                self.track_foot_hover();
                self.track_card();
                if self.composer_drag {
                    let (tx, ty) = self.composer_text_origin();
                    let p = self.composer.hit(x - tx, y - ty - self.composer.scroll());
                    if p != self.composer.caret {
                        self.composer.move_to(p, true);
                        self.composer_changed();
                    }
                } else if self.selecting
                    && let (Some(h), Some((a, _))) = (self.msg_hit(x, y), self.sel)
                {
                    self.set_sel(Some((a, h)));
                }
            }
            WM_MOUSELEAVE => {
                self.tracking = false;
                self.pointer = None;
                self.update_hover();
                self.track_tool_hover();
                self.track_reply_hover();
                self.track_card();
                self.need = true;
            }
            WM_LBUTTONDOWN => {
                unsafe {
                    SetCapture(self.hwnd);
                }
                if self.in_box(x, y) {
                    self.set_thread_focus(false);
                    self.set_composer_focus(true);
                    let (tx, ty) = self.composer_text_origin();
                    let p = self.composer.hit(x - tx, y - ty - self.composer.scroll());
                    self.composer.move_to(p, shift);
                    self.composer_drag = true;
                    self.composer_changed();
                } else {
                    self.set_composer_focus(false);
                    self.set_thread_focus(false);
                    self.set_kb(None);
                    if let Some((i, k)) = self.tool_at(x, y) {
                        if let Some(t) = self.tools(i).get(k) {
                            self.tool_activate(i, t.tool);
                        }
                    } else if self.reply_click(x, y) {
                    } else if let Some(i) = self.thread_link_at(x, y) {
                        self.open_thread_of(i);
                    } else if self.on_head_button(x, y) {
                        self.load_older();
                    } else if let Some(k) = self.foot_at(x, y) {
                        self.foot_activate(k);
                    } else if let Some(i) = self.quote_link_at(x, y) {
                        self.view_original_of(i);
                    } else if let Some(i) = self.edited_at(x, y) {
                        self.open_history_of(i);
                    } else if let Some((i, action)) = self.action_at(x, y) {
                        self.row_action(i, action);
                    } else if self.in_history(x, y) {
                        if let Some(h) = self.msg_hit(x, y) {
                            let anchor = if shift {
                                self.sel.map_or(h, |s| s.0)
                            } else {
                                h
                            };
                            self.set_sel(Some((anchor, h)));
                            self.selecting = true;
                        }
                    } else if let Some(i) = self.hover_channel {
                        self.select_channel(i);
                    } else if let Some(i) = self.hover_space {
                        self.select_space(i);
                    }
                }
            }
            WM_LBUTTONDBLCLK if self.in_history(x, y) => {
                // Double-click selects a word.
                if let Some((i, p)) = self.msg_hit(x, y) {
                    let text = self
                        .rows
                        .row(i)
                        .map(|r| r.plain16.clone())
                        .unwrap_or_default();
                    let word = |c: u16| char::from_u32(c as u32).is_some_and(char::is_alphanumeric);
                    let (mut s, mut e) = (p as usize, p as usize);
                    while s > 0 && word(text[s - 1]) {
                        s -= 1;
                    }
                    while e < text.len() && word(text[e]) {
                        e += 1;
                    }
                    self.set_sel(Some(((i, s as u32), (i, e as u32))));
                }
            }
            WM_LBUTTONUP => {
                unsafe {
                    let _ = ReleaseCapture();
                }
                // A click (not a drag-selection) on a link opens it.
                let clicked = self.selecting && self.sel.is_none_or(|(a, b)| a == b);
                self.selecting = false;
                self.composer_drag = false;
                if clicked && let Some(href) = self.link_at(x, y) {
                    self.set_sel(None);
                    open_link(&href);
                }
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xffff) as i16 as f64;
                if self.pointer.is_some_and(|(x, y)| self.in_history(x, y)) {
                    self.scroll_by(-delta / 120.0 * WHEEL_STEP);
                }
            }
            WM_SETCURSOR => {
                if (lp.0 & 0xffff) as u32 != HTCLIENT {
                    return None;
                }
                let beam = self
                    .pointer
                    .is_some_and(|(x, y)| self.in_box(x, y) || self.in_history(x, y));
                let hand = self.hover_channel.is_some()
                    || self.hover_space.is_some()
                    || self.pointer.is_some_and(|(x, y)| {
                        self.link_at(x, y).is_some()
                            || self.action_at(x, y).is_some()
                            || self.tool_at(x, y).is_some()
                            || self.on_head_button(x, y)
                            || self.thread_link_at(x, y).is_some()
                            || self.foot_at(x, y).is_some()
                            || self.quote_link_at(x, y).is_some()
                            || self.edited_at(x, y).is_some()
                            || self.reply_cancel_rect().is_some_and(|(l, t, w, h)| {
                                x >= l && x < l + w && y >= t && y < t + h
                            })
                    });
                unsafe {
                    SetCursor(
                        LoadCursorW(
                            None,
                            if beam {
                                IDC_IBEAM
                            } else if hand {
                                IDC_HAND
                            } else {
                                IDC_ARROW
                            },
                        )
                        .ok(),
                    );
                }
                return Some(LRESULT(1));
            }
            WM_KEYDOWN => {
                return self
                    .key(VIRTUAL_KEY(wp.0 as u16), ctrl, shift)
                    .then_some(LRESULT(0));
            }
            WM_CHAR => {
                let c = wp.0 as u16;
                if self.composer.focused && !ctrl && c >= 0x20 && c != 0x7f {
                    self.composer.insert(&[c]);
                    self.composer_changed();
                }
                return Some(LRESULT(0));
            }
            WM_IME_STARTCOMPOSITION if self.focused_field().is_some() => {
                self.ime_position();
                return Some(LRESULT(0));
            }
            WM_IME_COMPOSITION if self.focused_field().is_some() => {
                self.ime_composition(lp);
                return Some(LRESULT(0));
            }
            WM_IME_ENDCOMPOSITION if self.focused_field().is_some() => {
                if let Some(f) = self.focused_field_mut() {
                    f.comp.clear();
                }
                self.field_changed(false);
                return Some(LRESULT(0));
            }
            _ => return None,
        }
        Some(LRESULT(0))
    }

    fn track_leave(&mut self) {
        if self.tracking {
            return;
        }
        let mut t = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: self.hwnd,
            dwHoverTime: 0,
        };
        unsafe {
            let _ = TrackMouseEvent(&mut t);
        }
        self.tracking = true;
    }

    /// Pointer and keyboard input while the sign-in screen covers the window.
    /// None leaves the message to the normal handler (size, timers, IME).
    fn signin_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        _lp: LPARAM,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) -> Option<Option<LRESULT>> {
        let done = Some(Some(LRESULT(0)));
        match msg {
            WM_MOUSEMOVE => {
                self.track_leave();
                self.pointer = Some((x, y));
                let s = self.signin.as_mut()?;
                let hover = s.hit(x, y);
                let dragging = s.pressed == Some(signin::Part::Field);
                if dragging {
                    let (ox, oy) = s.field_origin;
                    let field = s.field_mut();
                    let p = field.hit(x - ox, y - oy);
                    if p != field.caret {
                        field.move_to(p, true);
                        self.signin_changed(false);
                    }
                } else if hover != s.hover {
                    s.hover = hover;
                    self.dirty.signin = true;
                    self.need = true;
                }
                done
            }
            WM_MOUSELEAVE => {
                self.tracking = false;
                self.pointer = None;
                if let Some(s) = &mut self.signin {
                    s.hover = None;
                }
                self.dirty.signin = true;
                self.need = true;
                done
            }
            WM_LBUTTONDOWN => {
                unsafe {
                    SetCapture(self.hwnd);
                }
                let s = self.signin.as_mut()?;
                let part = s.hit(x, y);
                s.pressed = part;
                if part == Some(signin::Part::Field) {
                    let (ox, oy) = s.field_origin;
                    let field = s.field_mut();
                    let p = field.hit(x - ox, y - oy);
                    field.move_to(p, shift);
                    self.set_signin_focus(true);
                    self.signin_changed(false);
                } else {
                    self.dirty.signin = true;
                    self.need = true;
                }
                done
            }
            WM_LBUTTONUP => {
                unsafe {
                    let _ = ReleaseCapture();
                }
                let s = self.signin.as_mut()?;
                let pressed = s.pressed.take();
                let released = s.hit(x, y);
                self.dirty.signin = true;
                self.need = true;
                if let Some(p) =
                    pressed.filter(|p| Some(*p) == released && *p != signin::Part::Field)
                {
                    self.signin_activate(Some(p));
                }
                done
            }
            WM_LBUTTONDBLCLK | WM_MOUSEWHEEL => done,
            WM_SETCURSOR => {
                let s = self.signin.as_ref()?;
                let cursor = match self.pointer.and_then(|(x, y)| s.hit(x, y)) {
                    Some(signin::Part::Field) => IDC_IBEAM,
                    Some(_) => IDC_HAND,
                    None => IDC_ARROW,
                };
                unsafe {
                    SetCursor(LoadCursorW(None, cursor).ok());
                }
                Some(Some(LRESULT(1)))
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                let hwnd = self.hwnd;
                let s = self.signin.as_mut()?;
                let editable = s.editable();
                let field = s.field_mut();
                if !field.focused {
                    if vk == VK_TAB || vk == VK_RETURN {
                        self.set_signin_focus(true);
                    }
                    return done;
                }
                if ctrl && vk == VIRTUAL_KEY('C' as u16) {
                    let (a, b) = field.selection();
                    if b > a {
                        set_clipboard(hwnd, &field.text[a..b]);
                    }
                    return done;
                }
                match vk {
                    VK_RETURN => self.signin_activate(None),
                    VK_ESCAPE => self.set_signin_focus(false),
                    // Movement and selection work while read-only; edits do not.
                    VK_LEFT | VK_RIGHT | VK_HOME | VK_END => {
                        field.edit_key(hwnd, vk, ctrl, shift, false);
                        self.signin_changed(false);
                    }
                    _ if ctrl && vk == VIRTUAL_KEY('A' as u16) => {
                        field.edit_key(hwnd, vk, ctrl, shift, false);
                        self.signin_changed(false);
                    }
                    _ if editable && field.edit_key(hwnd, vk, ctrl, shift, false) => {
                        self.signin_changed(true);
                    }
                    _ => return Some(None),
                }
                done
            }
            WM_CHAR => {
                let c = wp.0 as u16;
                let s = self.signin.as_mut()?;
                if s.editable() && s.field().focused && !ctrl && c >= 0x20 && c != 0x7f {
                    s.field_mut().insert(&[c]);
                    self.signin_changed(true);
                }
                done
            }
            _ => None,
        }
    }

    /// The reaction chip or Retry/Dismiss button under client point (x, y),
    /// on a live message you can react to.
    fn action_at(&self, x: f32, y: f32) -> Option<(usize, list::Action)> {
        if !self.in_history(x, y) {
            return None;
        }
        let ay = self.scroll_top + (y - HEADER_H) as f64;
        let i = self.rows.row_at(ay)?;
        let ry = (ay - self.rows.top[i]) as f32;
        let action = self.row_controls(i).into_iter().find(|(_, r)| {
            let x = x - self.list_x();
            x >= r.left && x < r.right && ry >= r.top && ry < r.bottom
        })?;
        Some((i, action.0))
    }

    /// Row `i`'s reaction chips and Retry/Dismiss, from the row's top in
    /// list coordinates; none in the mock or on a deleted message.
    pub fn row_controls(&self, i: usize) -> Vec<(list::Action, Common::D2D_RECT_F)> {
        let m = &self.c.messages[i];
        if self.live.is_none() || m.deleted {
            return Vec::new();
        }
        self.rows.controls(i, m.grouped)
    }

    /// Clicks on a row's reaction chip (toggle yours) or its error's Retry
    /// or Dismiss.
    pub fn row_action(&mut self, i: usize, action: list::Action) {
        let m = &self.c.messages[i];
        let id = m.id.clone();
        let Some(live) = &mut self.live else {
            return;
        };
        match action {
            list::Action::Chip(k) => {
                let r = &m.reactions[k];
                live.react(&id, &r.emoji, !r.mine);
            }
            list::Action::Add => return self.open_add_picker(i),
            list::Action::Retry => live.retry_reaction(&id),
            list::Action::Dismiss => live.dismiss_reaction(&id),
        }
        if let Err(e) = self.drain_live() {
            eprintln!("reaction update failed: {e:?}");
        }
    }

    /// The history control with keyboard focus, while its row still has it.
    pub fn kb_control(&self) -> Option<(usize, list::Action)> {
        let (id, action) = self.kb.as_ref()?;
        let &i = self.ids.rows.get(id)?;
        (i < self.c.messages.len() && self.row_controls(i).iter().any(|(a, _)| a == action))
            .then_some((i, *action))
    }

    /// Moves keyboard focus to a history control (or off them), keeping it
    /// in view; the composer gives up focus to it.
    pub fn set_kb(&mut self, to: Option<(usize, list::Action)>) {
        let old = self.kb_control();
        if old == to && self.kb.is_some() == to.is_some() {
            return;
        }
        self.redraw.extend(old.map(|(i, _)| i));
        self.kb = to.map(|(i, a)| (self.c.messages[i].id.clone(), a));
        if let Some((i, action)) = to {
            self.set_composer_focus(false);
            self.set_thread_focus(false);
            self.redraw.push(i);
            if let Some((_, r)) = self.row_controls(i).into_iter().find(|(a, _)| *a == action) {
                let (top, bottom) = (
                    self.rows.top[i] + r.top as f64,
                    self.rows.top[i] + r.bottom as f64,
                );
                let vh = self.vh() as f64;
                if top < self.scroll_top + 8.0 {
                    self.scroll_top = top - 8.0;
                    self.at_end = false;
                } else if bottom > self.scroll_top + vh - 8.0 {
                    self.scroll_top = bottom - vh + 8.0;
                    self.at_end = false;
                }
            }
        }
        self.events |= EV_FOCUS;
        self.need = true;
    }

    /// The control after (or with `back`, before) `from` among the measured
    /// rows; without `from`, the first (or last) one on screen.
    fn next_control(
        &mut self,
        from: Option<(usize, list::Action)>,
        back: bool,
    ) -> Option<(usize, list::Action)> {
        let n = self.rows.len();
        let (start, skip) = match from {
            Some((i, action)) => (i, Some(action)),
            None => {
                let v = self.visible_rows();
                if v.is_empty() {
                    return None;
                }
                (if back { v.end - 1 } else { v.start }, None)
            }
        };
        let mut i = start;
        let mut skip = skip;
        loop {
            if i < self.rows.lo || i >= n {
                return None;
            }
            if self.rows.ensure(&self.g, &self.c, i).is_err() {
                return None;
            }
            let mut controls: Vec<list::Action> =
                self.row_controls(i).into_iter().map(|(a, _)| a).collect();
            if back {
                controls.reverse();
            }
            let at = match skip.take() {
                Some(a) => controls.iter().position(|c| *c == a).map_or(0, |p| p + 1),
                None => 0,
            };
            if let Some(a) = controls.get(at) {
                return Some((i, *a));
            }
            if back {
                i = i.checked_sub(1)?;
            } else {
                i += 1;
            }
        }
    }

    /// Tab, Shift+Tab, Enter, Space and Escape for the history's controls:
    /// Shift+Tab from the composer reaches the last one on screen, Tab past
    /// the newest returns to the composer.
    fn history_key(&mut self, vk: VIRTUAL_KEY, shift: bool) -> bool {
        if let Some((i, action)) = self.kb_control() {
            match vk {
                VK_TAB => match self.next_control(Some((i, action)), shift) {
                    Some(next) => self.set_kb(Some(next)),
                    None if !shift => self.set_composer_focus(true),
                    None => {}
                },
                VK_RETURN | VK_SPACE => self.row_action(i, action),
                VK_ESCAPE => self.set_composer_focus(true),
                _ => return false,
            }
            return true;
        }
        let in_thread = self.thread.as_ref().is_some_and(|t| t.composer.focused);
        if vk != VK_TAB || in_thread || (self.composer.focused && !shift) {
            return false;
        }
        let back = self.composer.focused || shift;
        match self.next_control(None, back) {
            Some(next) => {
                self.set_kb(Some(next));
                true
            }
            None => false,
        }
    }

    /// The address of the link under client point (x, y) in the history.
    fn link_at(&self, x: f32, y: f32) -> Option<String> {
        if !self.in_history(x, y) {
            return None;
        }
        let (i, p) = self.msg_hit(x, y)?;
        self.rows.row(i)?.links.iter().find_map(|(s, l, href)| {
            // A hit at a link's end position is just past its last character.
            (p >= *s && p < s + l).then(|| href.clone())
        })
    }

    pub fn scroll_by(&mut self, dy: f64) {
        self.scroll_top += dy;
        self.at_end = false;
        self.need = true;
    }

    fn key(&mut self, vk: VIRTUAL_KEY, ctrl: bool, shift: bool) -> bool {
        if ctrl && vk == VIRTUAL_KEY('C' as u16) {
            let text = if self.composer.focused {
                let (a, b) = self.composer.selection();
                self.composer.text[a..b].to_vec()
            } else {
                self.selected_text()
            };
            if !text.is_empty() {
                set_clipboard(self.hwnd, &text);
            }
            return true;
        }
        if self.history_key(vk, shift) {
            return true;
        }
        if !self.composer.focused {
            let page = self.vh() as f64 * 0.9;
            match vk {
                VK_PRIOR => self.scroll_by(-page),
                VK_NEXT => self.scroll_by(page),
                VK_END if ctrl => {
                    self.at_end = true;
                    self.need = true;
                }
                // As on web, Escape closes the thread when nothing else takes it.
                VK_ESCAPE if self.thread.is_some() => self.close_thread(),
                _ => return false,
            }
            return true;
        }
        match vk {
            VK_RETURN if shift => self.composer.insert(&['\n' as u16]),
            VK_RETURN => {
                if let Err(e) = self.send() {
                    eprintln!("send failed: {e:?}");
                }
                return true;
            }
            VK_ESCAPE => {
                self.set_composer_focus(false);
                return true;
            }
            _ if self.composer.edit_key(self.hwnd, vk, ctrl, shift, true) => {}
            _ => return false,
        }
        self.composer_changed();
        true
    }

    fn ime_position(&self) {
        let Some((field, (tx, ty))) = self.focused_field() else {
            return;
        };
        let (cx, cy, ch) = field.caret_pos(field.display_caret());
        let s = self.scale;
        let p = POINT {
            x: ((tx + cx) * s) as i32,
            y: ((ty + cy) * s) as i32,
        };
        unsafe {
            let himc = ImmGetContext(self.hwnd);
            if himc.is_invalid() {
                return;
            }
            let form = COMPOSITIONFORM {
                dwStyle: CFS_POINT,
                ptCurrentPos: p,
                rcArea: RECT::default(),
            };
            let _ = ImmSetCompositionWindow(himc, &form);
            let cand = CANDIDATEFORM {
                dwIndex: 0,
                dwStyle: CFS_EXCLUDE,
                ptCurrentPos: p,
                rcArea: RECT {
                    left: p.x,
                    top: p.y,
                    right: p.x + 2,
                    bottom: p.y + (ch * s) as i32,
                },
            };
            let _ = ImmSetCandidateWindow(himc, &cand);
            let _ = ImmReleaseContext(self.hwnd, himc);
        }
    }

    fn ime_composition(&mut self, lp: LPARAM) {
        let flags = lp.0 as u32;
        unsafe {
            let himc = ImmGetContext(self.hwnd);
            if himc.is_invalid() {
                return;
            }
            let read = |kind: IME_COMPOSITION_STRING| -> Vec<u16> {
                let bytes = ImmGetCompositionStringW(himc, kind, None, 0);
                if bytes <= 0 {
                    return Vec::new();
                }
                let mut buf = vec![0u16; bytes as usize / 2];
                ImmGetCompositionStringW(himc, kind, Some(buf.as_mut_ptr().cast()), bytes as u32);
                buf
            };
            let editable = self.signin.as_ref().is_none_or(signin::SignIn::editable);
            if let Some(field) = self.focused_field_mut().filter(|_| editable) {
                if flags & GCS_RESULTSTR.0 != 0 {
                    let s = read(GCS_RESULTSTR);
                    field.comp.clear();
                    field.insert(&s);
                }
                if flags & GCS_COMPSTR.0 != 0 {
                    field.comp = read(GCS_COMPSTR);
                    let cur = ImmGetCompositionStringW(himc, GCS_CURSORPOS, None, 0);
                    field.comp_cursor = (cur.max(0) as usize).min(field.comp.len());
                } else if flags & GCS_RESULTSTR.0 != 0 {
                    field.comp.clear();
                }
            }
            let _ = ImmReleaseContext(self.hwnd, himc);
        }
        self.field_changed(flags & GCS_RESULTSTR.0 != 0);
    }

    // ---- frame ----

    fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
        self.dirty = Dirty::all();
        self.need = true;
    }

    fn apply_layout(&mut self) -> Result<()> {
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let (w, h, lx, lw) = (self.w, self.h, self.list_x(), self.list_w());
        unsafe {
            self.background.SetTransform2(&Matrix3x2 {
                M11: w * s,
                M12: 0.0,
                M21: 0.0,
                M22: h * s,
                M31: 0.0,
                M32: 0.0,
            })?;
        }
        self.dirty.rail |= self.rail.ensure(&self.g, px(RAIL_W), px(h))?;
        self.dirty.sidebar |= self.sidebar.ensure(&self.g, px(SIDEBAR_W), px(h))?;
        self.sidebar.offset((RAIL_W * s).round(), 0.0)?;
        self.dirty.header |= self.header.ensure(&self.g, px(lw), px(HEADER_H))?;
        self.header.offset((lx * s).round(), 0.0)?;
        self.dirty.members |= self.members.ensure(&self.g, px(MEMBERS_W), px(h))?;
        self.members.offset(((w - MEMBERS_W) * s).round(), 0.0)?;
        let bw = lw - 36.0;
        self.box_h = self.composer.relayout(&self.g, bw - 2.0 * composer::PAD)?;
        let (bx, by, _, _) = self.box_rect();
        self.composer_p.ensure(&self.g, px(bw), px(self.box_h))?;
        self.composer_p.offset((bx * s).round(), (by * s).round())?;
        self.dirty.composer = true;
        self.relayout_reply();
        self.dirty.reply = true;
        let vh = self.vh();
        unsafe {
            self.history_clip.SetOffsetX2((lx * s).round())?;
            self.history_clip.SetOffsetY2((HEADER_H * s).round())?;
            self.history_clip
                .SetClip2(&rect(0.0, 0.0, (lw * s).round(), (vh * s).round()))?;
        }
        let body = Rows::body_width(lw);
        if (body - self.rows.width).abs() > 0.01 {
            // Text rewraps: keep the top visible row in place unless at the live end.
            let keep = (!self.at_end)
                .then(|| {
                    self.rows
                        .row_at(self.scroll_top)
                        .map(|i| (i, self.scroll_top - self.rows.top[i]))
                })
                .flatten();
            self.rows.set_width(body);
            if let Some((i, frac)) = keep {
                while self.rows.lo > i {
                    let y = if self.rows.lo < self.rows.len() {
                        self.rows.top[self.rows.lo] - 1.0
                    } else {
                        0.0
                    };
                    self.rows.measure_up_to(&self.g, &self.c, y)?;
                }
                self.scroll_top = self.rows.top[i] + frac;
            }
            self.drawn = None;
        }
        // The surface only grows, in 512 px steps: resizing it every frame of
        // a window drag would reallocate its tiles. The clip hides the rest.
        if px(lw) > self.vs_w {
            self.vs_w = px(lw).next_multiple_of(512);
            unsafe { self.vs.Resize(self.vs_w, VS_H)? };
            self.drawn = None;
        }
        if px(lw) != self.draw_w || self.drawn_scale != s {
            self.draw_w = px(lw);
            self.drawn_scale = s;
            self.drawn = None;
        }
        Ok(())
    }

    fn row_px(&self, y: f64) -> f64 {
        ORIGIN + y * self.scale as f64
    }

    /// Draws pixel rows [y0, y1) of the history surface.
    fn draw_strip(&mut self, y0: i64, y1: i64) -> Result<()> {
        if y1 <= y0 {
            return Ok(());
        }
        let s = self.scale;
        let r = RECT {
            left: 0,
            top: y0 as i32,
            right: self.draw_w as i32,
            bottom: y1 as i32,
        };
        let range = self.rows.range(
            (y0 as f64 - ORIGIN) / s as f64,
            (y1 as f64 - ORIGIN) / s as f64,
        );
        // Rows trimmed since they were last drawn get their layouts back.
        for i in range.clone() {
            self.rows.ensure(&self.g, &self.c, i)?;
        }
        let lw = self.list_w();
        let (hover, sel) = (self.hover_row, self.sel);
        let tops: Vec<(usize, f32, f32, Option<(u32, u32)>)> = range
            .map(|i| {
                (
                    i,
                    ((self.row_px(self.rows.top[i]) - y0 as f64) / s as f64) as f32,
                    self.rows.height(i),
                    Self::sel_for(sel, i, self.row_len(i)),
                )
            })
            .collect();
        let head = self
            .head_top()
            .zip(self.live.as_ref().map(live::Live::older))
            .map(|(top, state)| (((self.row_px(top) - y0 as f64) / s as f64) as f32, state))
            .filter(|(y, _)| *y < (y1 - y0) as f32 / s && *y + list::HEAD_H > 0.0);
        let head_hover = self.head_hover;
        let foot: Vec<(f32, list::Head, bool)> =
            if self.foot_h() > 0.0 && self.rows.lo < self.rows.len() {
                (0..2)
                    .map(|k| {
                        let top = self.rows.bottom() + (k as f32 * list::HEAD_H) as f64;
                        (
                            ((self.row_px(top) - y0 as f64) / s as f64) as f32,
                            self.foot_head(k),
                            self.foot_hover == Some(k),
                        )
                    })
                    .filter(|(y, ..)| *y < (y1 - y0) as f32 / s && *y + list::HEAD_H > 0.0)
                    .collect()
            } else {
                Vec::new()
            };
        let focused = self
            .focused
            .as_ref()
            .and_then(|id| self.ids.rows.get(id).copied());
        let kb = self.kb_control();
        let (g, c, rows) = (&self.g, &self.c, &mut self.rows);
        gfx::draw_local(&self.vs, r, s, |dc| unsafe {
            dc.Clear(Some(&gfx::color(SURFACE_CONVERSATION, 1.0)));
            if let Some((y, state)) = head {
                list::draw_head(g, dc, state, lw, y, head_hover)?;
            }
            for (y, foot, lit) in &foot {
                list::draw_head_row(g, dc, foot, lw, *y, *lit)?;
            }
            for (i, y, h, rs) in tops {
                // The date divider above a row is not part of its hover.
                let lead = rows.row(i).map_or(0.0, |r| r.lead);
                if focused == Some(i) {
                    dc.FillRectangle(&rect(0.0, y + lead, lw, y + h), &g.brushes.focused);
                }
                if hover == Some(i) {
                    dc.FillRectangle(&rect(0.0, y + lead, lw, y + h), &g.brushes.hover);
                }
                list::draw_row(g, dc, c, rows, i, y, rs)?;
                // The keyboard-focused control's terracotta ring.
                if let Some((_, action)) = kb.filter(|(k, _)| *k == i)
                    && let Some((_, r)) = rows
                        .controls(i, c.messages[i].grouped)
                        .into_iter()
                        .find(|(a, _)| *a == action)
                {
                    let r = rect(
                        r.left - 2.0,
                        y + r.top - 2.0,
                        r.right + 2.0,
                        y + r.bottom + 2.0,
                    );
                    stroke_round(dc, r, 10.0, &g.brushes.terracotta);
                }
            }
            Ok(())
        })
    }

    fn update_history(&mut self) -> Result<()> {
        let s = self.scale as f64;
        let vh = self.vh() as f64;
        // At startup and after a rewrap only the viewport is measured and
        // drawn this frame; the margins fill in on the following frames.
        let fresh = self.drawn.is_none();
        let margin = if fresh { 0.0 } else { vh };
        let foot_h = self.foot_h() as f64;
        if self.at_end {
            self.scroll_top = self.rows.bottom() + foot_h - vh;
        }
        self.rows
            .measure_up_to(&self.g, &self.c, self.scroll_top - margin)?;
        let min = if self.rows.lo == 0 {
            self.rows.top[0] - self.head_h() as f64
        } else {
            f64::NEG_INFINITY
        };
        let max = (self.rows.bottom() + foot_h - vh).max(min);
        self.scroll_top = self.scroll_top.clamp(min, max);
        self.at_end = self.scroll_top >= max - 0.5;
        self.older_armed |= self.at_end;
        // Reaching the start loads the previous page, as on web; a failed
        // page waits for Retry instead of looping.
        if self.older_armed
            && self.rows.lo == 0
            && self.scroll_top <= min + 1.0
            && self
                .live
                .as_mut()
                .is_some_and(|l| l.older() == live::Older::More && l.load_older())
        {
            self.redraw_head = true;
        }
        // Snap to device pixels so the composition offset and the drawn rows agree.
        let off = self.row_px(self.scroll_top).round();
        self.scroll_top = (off - ORIGIN) / s;
        unsafe { self.content.SetOffsetY2(-(off as f32))? };
        let visible = self.visible_rows();
        if visible != self.visible {
            self.visible = visible;
            self.events |= EV_ROWS;
        }
        let hover = self.history_hover();
        if hover != self.hover_row {
            self.redraw.extend(self.hover_row);
            self.redraw.extend(hover);
            self.hover_row = hover;
        }

        // Keep a quarter viewport drawn beyond each edge; when that runs out,
        // draw ahead in half-viewport chunks so most scroll frames only move
        // the composition offset.
        let known_top = if self.rows.lo < self.rows.len() {
            self.rows.top[self.rows.lo]
                - if self.rows.lo == 0 {
                    self.head_h() as f64
                } else {
                    0.0
                }
        } else {
            self.scroll_top
        };
        let (lim0, lim1) = (
            self.row_px(known_top).floor() as i64,
            self.row_px(self.rows.bottom() + foot_h).ceil() as i64,
        );
        let (above, below) = if fresh { (0.0, 1.0) } else { (0.25, 1.25) };
        let n0 = (self.row_px(self.scroll_top - vh * above).floor() as i64).max(lim0);
        let n1 = (self.row_px(self.scroll_top + vh * below).ceil() as i64)
            .min(lim1)
            .max(n0);
        let chunk = (vh * 0.5 * s) as i64;
        match self.drawn {
            Some((d0, d1)) if n0 < d1 && n1 > d0 => {
                let n0 = if n0 < d0 { (n0 - chunk).max(lim0) } else { n0 };
                let n1 = if n1 > d1 { (n1 + chunk).min(lim1) } else { n1 };
                if n0 < d0 {
                    self.draw_strip(n0, d0)?;
                }
                if n1 > d1 {
                    self.draw_strip(d1, n1)?;
                }
                let (m0, m1) = (n0.min(d0), n1.max(d1));
                let span = (vh * s) as i64;
                if m1 - m0 > span * 6 {
                    let (k0, k1) = ((n0 - span * 2).max(m0), (n1 + span * 2).min(m1));
                    unsafe {
                        self.vs.Trim(Some(&[RECT {
                            left: 0,
                            top: k0 as i32,
                            right: self.draw_w as i32,
                            bottom: k1 as i32,
                        }]))?
                    };
                    self.drawn = Some((k0, k1));
                    self.trim_rows();
                } else {
                    self.drawn = Some((m0, m1));
                }
            }
            _ => {
                self.draw_strip(n0, n1)?;
                unsafe {
                    self.vs.Trim(Some(&[RECT {
                        left: 0,
                        top: n0 as i32,
                        right: self.draw_w as i32,
                        bottom: n1 as i32,
                    }]))?
                };
                self.drawn = Some((n0, n1));
                self.trim_rows();
            }
        }

        if std::mem::take(&mut self.redraw_head)
            && let (Some((d0, d1)), Some(top)) = (self.drawn, self.head_top())
        {
            let y0 = (self.row_px(top).floor() as i64).max(d0);
            let y1 = (self.row_px(top + list::HEAD_H as f64).ceil() as i64).min(d1);
            self.draw_strip(y0, y1)?;
        }
        if std::mem::take(&mut self.redraw_foot)
            && foot_h > 0.0
            && let Some((d0, d1)) = self.drawn
        {
            let b = self.rows.bottom();
            let y0 = (self.row_px(b).floor() as i64).max(d0);
            let y1 = (self.row_px(b + foot_h).ceil() as i64).min(d1);
            if y1 > y0 {
                self.draw_strip(y0, y1)?;
            }
        }
        // Rows whose hover or selection changed.
        let mut redraw = std::mem::take(&mut self.redraw);
        redraw.sort_unstable();
        redraw.dedup();
        if let Some((d0, d1)) = self.drawn {
            for i in redraw {
                if i < self.rows.lo || self.rows.row(i).is_none() {
                    continue;
                }
                let y0 = (self.row_px(self.rows.top[i]).floor() as i64).max(d0);
                let y1 = (self
                    .row_px(self.rows.top[i] + self.rows.height(i) as f64)
                    .ceil() as i64)
                    .min(d1);
                self.draw_strip(y0, y1)?;
            }
        }
        match self.bar_row() {
            Some(i) => self.toolbar.offset(
                ((self.list_w() - actions::bar_width(self.tools(i).len()) - 16.0) * self.scale)
                    .round(),
                (((self.rows.top[i] - self.scroll_top) as f32 + self.row_lead(i) - 12.0).max(0.0)
                    * self.scale)
                    .round(),
            )?,
            None => self.toolbar.offset(-10_000.0, 0.0)?,
        }
        Ok(())
    }

    fn draw_signin(&mut self) -> Result<()> {
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let resized = self.signin_p.ensure(&self.g, px(self.w), px(self.h))?;
        if !(self.dirty.signin || resized) {
            return Ok(());
        }
        self.dirty.signin = false;
        let (g, w, h) = (&self.g, self.w, self.h);
        let Some(form) = self.signin.as_mut() else {
            return Ok(());
        };
        self.signin_p.draw(s, |dc| form.draw(g, dc, w, h))
    }

    fn draw_panels(&mut self) -> Result<()> {
        let s = self.scale;
        let (space_badges, channel_badges) = self.badges();
        let g = &self.g;
        let c = &self.c;
        let d = std::mem::take(&mut self.dirty);
        if d.rail {
            let hover = self.hover_space;
            self.rail.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(BLACKOUT, 1.0)));
                for (i, space) in c.spaces.iter().enumerate() {
                    let (x, y, w, h) = Self::space_rect(i);
                    let r = rect(x, y, x + w, y + h);
                    fill_round(
                        dc,
                        r,
                        12.0,
                        if hover == Some(i) {
                            &g.brushes.raised
                        } else {
                            &g.brushes.composer
                        },
                    );
                    if i == c.space {
                        stroke_round(
                            dc,
                            rect(r.left - 2.0, r.top - 2.0, r.right + 2.0, r.bottom + 2.0),
                            14.0,
                            &g.brushes.terracotta,
                        );
                    }
                    let initial: String = space
                        .name
                        .chars()
                        .next()
                        .map(String::from)
                        .unwrap_or_default();
                    let l = g.layout(&gfx::wide(&initial), &g.formats.rail, w)?;
                    l.SetMaxHeight(h)?;
                    text::draw(dc, &l, r.left, r.top, &g.brushes.text);
                    // egui's unread dot in the rail margin, and the
                    // terracotta mention count on the tile's corner.
                    let (unread, mentions) = space_badges.get(i).copied().unwrap_or_default();
                    let cy = r.top + h / 2.0;
                    if unread {
                        fill_round(
                            dc,
                            rect(r.left - 7.0, cy - 3.0, r.left - 1.0, cy + 3.0),
                            3.0,
                            &g.brushes.text,
                        );
                    }
                    if mentions > 0 {
                        let label = if mentions > 99 {
                            "99+".to_owned()
                        } else {
                            mentions.to_string()
                        };
                        let n = g.layout(&gfx::wide(&label), &g.formats.badge, 100.0)?;
                        let bw =
                            (gfx::metrics(&n).widthIncludingTrailingWhitespace + 10.0).max(20.0);
                        let b = rect(
                            r.right + 3.0 - bw,
                            r.bottom - 17.0,
                            r.right + 3.0,
                            r.bottom + 3.0,
                        );
                        fill_round(dc, b, 10.0, &g.solid(BLACKOUT));
                        let inner = rect(b.left + 2.0, b.top + 2.0, b.right - 2.0, b.bottom - 2.0);
                        fill_round(dc, inner, 8.0, &g.brushes.terracotta);
                        n.SetMaxWidth(inner.right - inner.left)?;
                        n.SetMaxHeight(inner.bottom - inner.top)?;
                        text::draw(dc, &n, inner.left, inner.top, &g.brushes.text);
                    }
                }
                Ok(())
            })?;
        }
        if d.sidebar {
            let (hover, layouts) = (self.hover_channel, &self.channel_layouts);
            self.sidebar.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(SURFACE_SIDEBAR, 1.0)));
                let t = g.layout(
                    &gfx::wide(&c.spaces[c.space].name),
                    &g.formats.space,
                    10_000.0,
                )?;
                text::draw(dc, &t, 20.0, 16.0, &g.brushes.text);
                dc.FillRectangle(
                    &rect(0.0, HEADER_H - 1.0, SIDEBAR_W, HEADER_H),
                    &g.brushes.border,
                );
                let label = g.layout(
                    &gfx::wide(&format!("Channels  {}", c.channels.len())),
                    &g.formats.section,
                    10_000.0,
                )?;
                text::draw(dc, &label, 20.0, SECTION_TOP + 4.0, &g.brushes.muted);
                for (i, l) in layouts.iter().enumerate() {
                    let (_, y, w, h) = Self::channel_rect(i);
                    let r = rect(12.0, y, 12.0 + w, y + h);
                    if i == c.channel {
                        fill_round(dc, r, RADIUS, &g.brushes.selected);
                    } else if hover == Some(i) {
                        fill_round(dc, r, RADIUS, &g.brushes.hover);
                    }
                    // A channel with anything new: egui's small text-coloured dot.
                    if channel_badges.get(i).copied().unwrap_or(false) {
                        let cy = y + h / 2.0;
                        fill_round(
                            dc,
                            rect(r.right - 17.0, cy - 3.0, r.right - 11.0, cy + 3.0),
                            3.0,
                            &g.brushes.text,
                        );
                    }
                    let mark = g.layout(
                        &gfx::wide(if c.channels[i].private { "🔒" } else { "#" }),
                        &g.formats.channel,
                        40.0,
                    )?;
                    emoji::apply(
                        &mark,
                        if c.channels[i].private { "🔒" } else { "#" },
                        15.0,
                        0.9,
                    )?;
                    text::draw(
                        dc,
                        &mark,
                        24.0,
                        y + 5.0,
                        if i == c.channel {
                            &g.brushes.terracotta
                        } else {
                            &g.brushes.muted
                        },
                    );
                    text::draw(
                        dc,
                        l,
                        46.0,
                        y + 5.0,
                        if i == c.channel {
                            &g.brushes.text
                        } else {
                            &g.brushes.muted
                        },
                    );
                }
                Ok(())
            })?;
        }
        if d.header {
            let name = c
                .channels
                .get(c.channel)
                .map_or_else(String::new, |ch| ch.name.clone());
            let lw = self.list_w();
            let status = self.status.as_ref().map(|(s, e)| (s.as_str(), *e));
            self.header.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(SURFACE_CONVERSATION, 1.0)));
                let hash = g.layout(&gfx::wide("#"), &g.formats.header, 40.0)?;
                text::draw(dc, &hash, 24.0, 16.0, &g.brushes.muted);
                let l = g.layout(&gfx::wide(&name), &g.formats.header, 10_000.0)?;
                text::draw(dc, &l, 40.0, 16.0, &g.brushes.text);
                if let Some((status, error)) = status {
                    let l =
                        g.layout(&gfx::wide(status), &g.formats.time, (lw - 200.0).max(40.0))?;
                    let brush = if error {
                        &g.brushes.terracotta
                    } else {
                        &g.brushes.muted
                    };
                    text::draw(dc, &l, 200.0, 21.0, brush);
                }
                dc.FillRectangle(&rect(0.0, HEADER_H - 1.0, lw, HEADER_H), &g.brushes.border);
                Ok(())
            })?;
        }
        if d.members {
            let (hover, layouts) = (self.hover_member, &self.member_layouts);
            self.members.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(SURFACE_SIDEBAR, 1.0)));
                dc.FillRectangle(&rect(0.0, 0.0, 1.0, 100_000.0), &g.brushes.border);
                let t = g.layout(
                    &gfx::wide(&format!("Members  {}", c.listed_members)),
                    &g.formats.section,
                    10_000.0,
                )?;
                text::draw(dc, &t, 20.0, 18.0, &g.brushes.text);
                dc.FillRectangle(
                    &rect(0.0, HEADER_H - 1.0, MEMBERS_W, HEADER_H),
                    &g.brushes.border,
                );
                let online = c.members[..c.listed_members]
                    .iter()
                    .filter(|m| m.online)
                    .count();
                let label = if c.presence {
                    format!("Online  {online}")
                } else {
                    "Everyone".to_owned()
                };
                let o = g.layout(&gfx::wide(&label), &g.formats.section, 10_000.0)?;
                text::draw(dc, &o, 12.0, SECTION_TOP + 4.0, &g.brushes.muted);
                for (i, l) in layouts.iter().enumerate() {
                    let top = MEMBER_TOP + MEMBER_H * i as f32;
                    if hover == Some(i) {
                        fill_round(
                            dc,
                            rect(8.0, top, MEMBERS_W - 8.0, top + MEMBER_H - 2.0),
                            RADIUS,
                            &g.brushes.hover,
                        );
                    }
                    let m = &c.members[i];
                    let av = rect(16.0, top + 5.0, 46.0, top + 35.0);
                    fill_round(dc, av, 15.0, &g.solid(m.color));
                    if m.online {
                        let e = D2D1_ELLIPSE {
                            point: windows_numerics::Vector2 {
                                X: av.right - 3.0,
                                Y: av.bottom - 3.0,
                            },
                            radiusX: 4.5,
                            radiusY: 4.5,
                        };
                        dc.FillEllipse(&e, &g.brushes.blackout);
                        let e = D2D1_ELLIPSE {
                            radiusX: 3.5,
                            radiusY: 3.5,
                            ..e
                        };
                        dc.FillEllipse(&e, &g.brushes.caper);
                    }
                    text::draw(
                        dc,
                        l,
                        56.0,
                        top + 11.0,
                        if m.online {
                            &g.brushes.text
                        } else {
                            &g.brushes.muted
                        },
                    );
                }
                Ok(())
            })?;
        }
        if d.composer {
            let (_, _, bw, bh) = self.box_rect();
            let placeholder = format!("Message #{}", c.channels[c.channel].name);
            let comp = &self.composer;
            self.composer_p
                .draw(s, |dc| comp.draw(g, dc, bw, bh, &placeholder))?;
        }
        Ok(())
    }

    fn frame(&mut self) -> Result<()> {
        let started = Instant::now();
        match self.bench.frame() {
            Action::ScrollUp(dy) => self.scroll_by(-(dy as f64)),
            Action::Append => {
                self.c.push_demo();
                self.rows.append(&self.g, &self.c)?;
            }
            Action::Resize(w, h) => self.pending_resize = Some((w, h)),
            Action::Type(ch) => {
                let mut b = [0u16; 2];
                let s = ch.encode_utf16(&mut b).to_vec();
                self.composer.insert(&s);
                self.composer_changed();
            }
            Action::Quit => self.quit = true,
            Action::None => {}
        }
        if self.signin.is_some() {
            // The sign-in screen covers everything; the rest waits until it closes.
            self.draw_signin()?;
            unsafe { self.g.dcomp.Commit()? };
            self.bench.log_frame(started.elapsed());
            return Ok(());
        }
        if self.dirty.layout {
            self.dirty.layout = false;
            self.apply_layout()?;
        }
        self.update_history()?;
        // Before the panels, which take the rest of the damage flags.
        self.draw_actions()?;
        self.draw_overlays()?;
        self.draw_thread()?;
        self.track_card();
        self.draw_card()?;
        self.draw_panels()?;
        unsafe { self.g.dcomp.Commit()? };
        if let Some(live) = &mut self.live {
            // Renews the worker's lease on the open conversation while focused.
            live.showing(unsafe { GetForegroundWindow() } == self.hwnd);
        }
        self.bench.log_frame(started.elapsed());
        Ok(())
    }
}

/// Text layouts of the channel and member names.
fn side_layouts(
    g: &Gfx,
    c: &Conversation,
) -> Result<(Vec<IDWriteTextLayout>, Vec<IDWriteTextLayout>)> {
    let channels = c
        .channels
        .iter()
        .map(|ch| g.layout(&gfx::wide(&ch.name), &g.formats.channel, 10_000.0))
        .collect::<Result<_>>()?;
    let members = c.members[..c.listed_members]
        .iter()
        .map(|m| {
            let l = g.layout(&gfx::wide(&m.name), &g.formats.member, 10_000.0)?;
            emoji::apply(&l, &m.name, 14.0, 1.0)?;
            Ok(l)
        })
        .collect::<Result<_>>()?;
    Ok((channels, members))
}

fn set_clipboard(hwnd: HWND, text: &[u16]) {
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, (text.len() + 1) * 2) {
            let p = GlobalLock(h).cast::<u16>();
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(text.as_ptr(), p, text.len());
                *p.add(text.len()) = 0;
                let _ = GlobalUnlock(h);
                let _ = SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(h.0)));
            }
        }
        let _ = CloseClipboard();
    }
}

fn get_clipboard(hwnd: HWND) -> Vec<u16> {
    let mut out = Vec::new();
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return out;
        }
        if let Ok(h) = GetClipboardData(CF_UNICODETEXT.0 as u32) {
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g).cast::<u16>();
            if !p.is_null() {
                let mut i = 0;
                while *p.add(i) != 0 {
                    out.push(*p.add(i));
                    i += 1;
                }
                let _ = GlobalUnlock(g);
            }
        }
        let _ = CloseClipboard();
    }
    out
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_GETOBJECT => {
                // Outside the app borrow: UIA may call straight back into the providers.
                if lp.0 as i32 == UiaRootObjectId {
                    return UiaReturnRawElementProvider(hwnd, wp, lp, &uia::node(uia::Id::Root));
                }
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            WM_DESTROY => {
                let _ = UiaReturnRawElementProvider(hwnd, WPARAM(0), LPARAM(0), None);
                PostQuitMessage(0);
                return LRESULT(0);
            }
            // An Alt tap must not enter menu mode (there is no menu bar).
            WM_SYSCOMMAND if (wp.0 & 0xFFF0) as u32 == SC_KEYMENU => return LRESULT(0),
            WM_ERASEBKGND => return LRESULT(1),
            live::WM_LIVE => {
                if let Some(Err(e)) = with_app(App::drain_live) {
                    eprintln!("live update failed: {e:?}");
                }
                return LRESULT(0);
            }
            WM_PAINT => {
                let _ = ValidateRect(Some(hwnd), None);
                return LRESULT(0);
            }
            WM_DPICHANGED => {
                let dpi = (wp.0 & 0xffff) as f32;
                with_app(|a| a.set_scale(dpi / 96.0));
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                return LRESULT(0);
            }
            // The composition is drawn inline in the composer.
            WM_IME_SETCONTEXT => {
                return DefWindowProcW(
                    hwnd,
                    msg,
                    wp,
                    LPARAM(lp.0 & !(ISC_SHOWUICOMPOSITIONWINDOW as isize)),
                );
            }
            _ => {}
        }
        if let Some(Some(r)) = with_app(|a| a.handle(msg, wp, lp)) {
            return r;
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

/// Opens a message link in the default browser. `links::segments` only
/// yields web addresses.
fn open_link(href: &str) {
    unsafe {
        let _ = windows::Win32::UI::Shell::ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(href),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

fn resize_window(hwnd: HWND, w: f32, h: f32) {
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        let s = dpi as f32 / 96.0;
        let mut r = RECT {
            left: 0,
            top: 0,
            right: (w * s).round() as i32,
            bottom: (h * s).round() as i32,
        };
        let _ = AdjustWindowRectExForDpi(
            &mut r,
            WS_OVERLAPPEDWINDOW,
            false,
            WS_EX_NOREDIRECTIONBITMAP,
            dpi,
        );
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn main() -> Result<()> {
    let process_start = Instant::now();
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: w!("CaperWindows"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            w!("CaperWindows"),
            w!("Caper"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1440,
            900,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        resize_window(hwnd, 1440.0, 900.0);
        let messages = std::env::var("MESSAGES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3000);
        let bench = Bench::from_env(process_start);
        let active = bench.active();
        let app = App::new(hwnd, Gfx::new()?, Conversation::demo(messages), bench)?;
        APP.with(|a| *a.borrow_mut() = Some(app));
        with_app(|a| {
            a.live = live::start(hwnd);
            if a.live.is_some() {
                // Restoring the stored session; the sign-in form follows if there is none.
                a.signin = Some(signin::SignIn::new());
                a.signin_screen_changed();
            }
        });
        if active {
            SetTimer(Some(hwnd), TIMER_BENCH, 50, None);
        }
        if with_app(|a| a.live.is_some()) == Some(true) {
            SetTimer(Some(hwnd), TIMER_UNREAD, 15_000, None);
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        let mut clock = true;
        loop {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return Ok(());
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let state = with_app(|a| {
                let render = !a.quit && (a.need || a.bench.wants_frames());
                if render {
                    a.need = false;
                    if let Err(e) = a.frame() {
                        eprintln!("frame failed: {e:?}");
                    }
                }
                (
                    render,
                    a.quit,
                    a.pending_resize.take(),
                    std::mem::take(&mut a.events),
                )
            });
            let Some((rendered, quit, resize, events)) = state else {
                continue;
            };
            if let Some((w, h)) = resize {
                resize_window(hwnd, w, h);
            }
            if events & EV_TEXT != 0 {
                uia::raise_composer_events();
            }
            if events & EV_FOCUS != 0 {
                uia::raise_focus();
            }
            if events & EV_ROWS != 0 {
                uia::raise_rows_changed();
            }
            if events & EV_SCREEN != 0 {
                uia::raise_screen_changed();
            }
            if quit {
                let _ = DestroyWindow(hwnd);
                continue;
            }
            if rendered {
                // One frame per compositor tick (display refresh).
                if clock && DCompositionWaitForCompositorClock(None, 100) == u32::MAX {
                    clock = false;
                }
                if !clock {
                    let _ = DwmFlush();
                }
            } else {
                let _ = WaitMessage();
            }
        }
    }
}
