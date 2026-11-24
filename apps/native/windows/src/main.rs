//! Caper for Windows: Win32 window, a DirectComposition visual tree with one
//! retained surface per panel, Direct2D + DirectWrite drawing, UI Automation.
//!
//! Damage model: each panel (rail, sidebar, header, members, composer, hover
//! toolbar) has its own composition surface, redrawn only when its own state
//! changes. The message history is one tall virtual surface: scrolling moves
//! the visual without redrawing, only newly exposed strips are drawn, and
//! hover or selection redraw just the rows they touch.
//!
//! The Windows release ships it as Caper.exe (`apps/native/desktop/build.ps1`),
//! on the shared client core (`apps/native/client`). Development builds
//! without a server show a labelled mock conversation (`live::endpoint`).
#![windows_subsystem = "windows"]

mod actions;
mod app_link;
mod attachment_editor;
mod autoscroll;
mod avatars;
mod bench;
mod browse;
mod call;
mod card;
mod composer;
mod custom_links;
mod directs;
mod drop_overlay;
mod embeds;
mod emoji;
mod files;
mod forward;
mod gifs;
mod invite_people;
mod invites;
mod jump;
mod manage;
mod overlay;
mod playback;
mod preview;
mod profile;
mod reactors;
mod recording;
mod settings;
mod space_settings;
mod suggest;
mod tips;
mod tray;
mod typing;
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
mod member_search;
mod model;
mod motion;
mod narrow;
mod panes;
mod pins;
mod signin;
mod text;
mod theme;
mod thread;
mod uia;
mod updates;
mod uploads;
mod versions;
mod video;
mod viewer;
mod web_player;
mod window_state;

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
use windows::Win32::Graphics::Dwm::{
    DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmFlush, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::{CF_UNICODETEXT, OleInitialize, OleUninitialize, RevokeDragDrop};
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
pub(crate) const EV_ROWS: u8 = 4;
/// The sign-in screen appeared, changed step or went away.
pub(crate) const EV_SCREEN: u8 = 8;
/// Who is typing changed.
const EV_TYPING: u8 = 16;
/// Settings' version now reads "Copied".
pub(crate) const EV_VERSION_COPIED: u8 = 32;
const TIMER_BENCH: usize = 1;
pub(crate) const TIMER_CARET: usize = 2;
pub(crate) const TIMER_LIVE: usize = 3;
/// The sign-in screen's spinner and resend countdown.
const TIMER_SIGNIN: usize = 4;
/// A chip's who-reacted card, once the pointer rests on it.
pub(crate) const TIMER_CARD: usize = 5;
/// The next frame of a shown GIF.
const TIMER_MEDIA: usize = 7;
/// Re-reads the unread badges, as egui does every 15 s.
const TIMER_UNREAD: usize = 6;
/// The GIF picker's search, once typing pauses, and its next GIF frame.
pub(crate) const TIMER_GIFS: usize = 8;
pub(crate) const TIMER_GIF_FRAMES: usize = 9;
/// Redraws the DM rows when the earliest timed mute ends, as egui does.
const TIMER_MUTE: usize = 10;
/// Who is typing: their dots' bounce, their entries' expiry and the end of
/// your own typing.
const TIMER_TYPING: usize = 19;
/// Hides the zoom notice.
const TIMER_ZOOM: usize = 22;
pub(crate) const WHEEL_STEP: f64 = 48.0;
pub(crate) const WM_MOUSELEAVE: u32 = 0x02A3;

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
    narrow: bool,
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
    /// The GIF picker.
    gifs: bool,
    /// Settings, or the account footer's menu.
    settings: bool,
    reply: bool,
    /// The thread pane, and its reply box.
    thread: bool,
    thread_box: bool,
    /// The who-reacted card.
    card: bool,
    /// A conversation's options menu.
    dm_menu: bool,
    /// A person's profile card.
    profile: bool,
    /// A composer's emoji and mention suggestions.
    suggest: bool,
    tip: bool,
    /// The row naming who is typing, above the composer.
    typing: bool,
    /// The Pins dialog.
    pins: bool,
    /// The zoom notice.
    zoom: bool,
}

impl Dirty {
    fn all() -> Self {
        Self {
            layout: true,
            narrow: true,
            rail: true,
            sidebar: true,
            header: true,
            members: true,
            composer: true,
            toolbar: true,
            signin: true,
            overlay: true,
            picker: true,
            gifs: true,
            settings: true,
            reply: true,
            thread: true,
            thread_box: true,
            card: true,
            dm_menu: true,
            profile: true,
            suggest: true,
            tip: true,
            typing: true,
            pins: true,
            zoom: true,
        }
    }
}

struct Panel {
    visual: IDCompositionVisual2,
    surface: Option<IDCompositionSurface>,
    size: (u32, u32),
    entrance: Option<motion::Entrance>,
}

impl Panel {
    fn new(g: &Gfx) -> Result<Self> {
        Ok(Self {
            visual: g.visual()?,
            surface: None,
            size: (0, 0),
            entrance: None,
        })
    }

    fn entering(mut self, g: &Gfx, kind: motion::Kind) -> Self {
        self.entrance = Some(motion::Entrance::new(&g.dcomp, kind));
        self
    }

    /// Ensures a surface of this pixel size; true when it was (re)created.
    fn ensure(&mut self, g: &Gfx, w: u32, h: u32) -> Result<bool> {
        if self.surface.is_some() && self.size == (w, h) {
            return Ok(false);
        }
        // Resizing an already shown panel must never animate its geometry.
        if self.surface.is_some()
            && let Some(entrance) = &self.entrance
        {
            entrance.settle(&self.visual)?;
        }
        let s = g.surface(w, h)?;
        unsafe { self.visual.SetContent(&s)? };
        self.surface = Some(s);
        self.size = (w, h);
        Ok(true)
    }

    fn offset(&self, x: f32, y: f32) -> Result<()> {
        if let Some(entrance) = &self.entrance {
            return entrance.place(&self.visual, x, y);
        }
        unsafe {
            self.visual.SetOffsetX2(x)?;
            self.visual.SetOffsetY2(y)
        }
    }

    /// Moves a closed panel off-screen and frees its surface, so an overlay
    /// (dialog, viewer, Settings, Pins, pickers, cards, the thread pane)
    /// holds no GPU memory while closed; the next `ensure` makes a new one.
    fn hide(&mut self) -> Result<()> {
        self.offset(-100_000.0, 0.0)?;
        if self.surface.take().is_some() {
            self.size = (0, 0);
            unsafe { self.visual.SetContent(None)? };
        }
        Ok(())
    }

    fn draw(&self, scale: f32, f: impl FnOnce(&ID2D1DeviceContext) -> Result<()>) -> Result<()> {
        match &self.surface {
            Some(s) => {
                gfx::draw(s, scale, f)?;
                if let Some(entrance) = &self.entrance {
                    entrance.start(&self.visual, scale)?;
                }
                Ok(())
            }
            None => Ok(()),
        }
    }
}

/// A text selection across message rows: (row, UTF-16 position) anchor and focus.
pub type Sel = ((usize, u32), (usize, u32));

/// The sidebar row that has the keyboard: the channel or DM picked last,
/// or the account panel's Notifications or Feedback row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Channel(usize),
    Direct(usize),
    Inbox(usize),
}

pub struct App {
    pub hwnd: HWND,
    pub g: Gfx,
    pub c: Conversation,
    bench: Bench,
    pub rows: Rows,
    pub composer: Composer,
    /// DIPs to pixels: the monitor's DPI scale times the zoom.
    pub scale: f32,
    /// The monitor's DPI scale; Ctrl +/−/0 zoom on top of it, as egui's,
    /// and whether its notice shows.
    dpi: f32,
    zoom: f32,
    zoom_notice: bool,
    zoom_p: Panel,
    pub w: f32,
    pub h: f32,
    panes: panes::State,
    narrow: narrow::State,
    navigation_layer: IDCompositionVisual2,
    conversation_layer: IDCompositionVisual2,
    thread_layer: IDCompositionVisual2,
    narrow_p: Panel,
    narrow_menu_p: Panel,
    members_backdrop: Panel,
    _target: IDCompositionTarget,
    background: IDCompositionVisual2,
    rail: Panel,
    sidebar: Panel,
    header: Panel,
    members: Panel,
    composer_p: Panel,
    toolbar: Panel,
    history_clip: IDCompositionVisual2,
    history_y_drawn: f32,
    /// The narrow voice error's band when last laid out, which reflows the
    /// history and composer when it comes or goes.
    voice_error_drawn: f32,
    /// A row whose reaction just failed, announced after the frame.
    reaction_alert: Option<usize>,
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
    /// The row currently drawn with the hover highlight (active menu/picker, else hovered).
    lit_row: Option<usize>,
    hover_space: Option<usize>,
    hover_channel: Option<usize>,
    /// Whether the pointer is on the hovered channel's options button, or
    /// the sidebar header's space options button.
    hover_channel_options: bool,
    space_options_hover: bool,
    hover_member: Option<usize>,
    /// The sidebar's DM row under the pointer.
    hover_direct: Option<directs::Spot>,
    /// The account panel's Notifications or Feedback row under the pointer.
    hover_inbox: Option<usize>,
    /// The invitation tile, Join with invite link or channel
    /// invitation under the pointer.
    hover_invite: Option<invites::Spot>,
    /// A channel row's Join or a call card button under the pointer.
    voice_hover: Option<call::VPart>,
    call_ui: call::People,
    /// The request bar's hovered and pressed buttons.
    bar_hover: Option<directs::BarPart>,
    bar_pressed: Option<directs::BarPart>,
    /// The channel composer's hovered control (Attach, GIF, Emoji, Dictate).
    box_hover: Option<uploads::Part>,
    /// Settings, the account footer's Do Not Disturb menu and whether its
    /// gear or your avatar is hovered, and their surface.
    pub settings: Option<settings::Settings>,
    pub account_menu: Option<settings::AccountMenu>,
    me_hover: bool,
    /// Edit profile opened from Settings: closing it returns there, to
    /// this part.
    pub settings_back: Option<settings::SPart>,
    /// A conversation's open options menu, and its surface.
    pub dm_menu: Option<directs::DmMenu>,
    /// A rail menu's item waiting for its space ("id") to open.
    pub rail_action: Option<(String, directs::MenuItem)>,
    dm_menu_p: Panel,
    gear_hover: bool,
    /// The header's member-list toggle: the list is hidden, and the
    /// pointer is over the toggle.
    pub members_hidden: bool,
    /// An open thread hid the member list, and closing it shows it again.
    pub members_thread_hid: bool,
    members_toggle_hover: bool,
    /// The open Pins dialog, its panel, and whether the header's Pins
    /// button is hovered.
    pub pins: Option<pins::Pins>,
    /// The member list's search, and whether its heading button is hovered.
    pub member_search: Option<member_search::MemberSearch>,
    member_search_hover: bool,
    pins_p: Panel,
    pins_hover: bool,
    /// Self-updates, the version "Update available" offers, and whether
    /// it is hovered (`updates`).
    pub updates: caper_client::updates::Updates,
    pub update_dock: Option<String>,
    pub update_dock_hover: bool,
    /// When the shine's sweeps started, and whether the last sidebar frame
    /// drew one (so the frame after it clears it).
    pub update_shine: Option<Instant>,
    pub shine_drawn: bool,
    /// `caper://` invite codes from later launches, and the one waiting
    /// for sign-in (`app_link`).
    pub app_links: Option<std::sync::mpsc::Receiver<String>>,
    pub pending_invite: Option<String>,
    settings_p: Panel,
    /// `settings_p` last drew Settings rather than the gear's menu.
    settings_shown: bool,
    pub side_focus: Option<Side>,
    /// Its ring shows after keyboard or Narrator moves, not a pointer click.
    pub side_ring: bool,
    pointer: Option<(f32, f32)>,
    tracking: bool,
    autoscroll: Option<autoscroll::Run>,
    autoscroll_release: Option<u32>,
    autoscroll_p: Panel,
    pub sel: Option<Sel>,
    selecting: bool,
    composer_drag: bool,
    redraw: Vec<usize>,
    /// Rows whose GIFs play, redrawn when their next frame is due.
    gif_rows: Vec<usize>,
    /// When the first of their frames is due, while the timer waits.
    gif_due: Option<Instant>,
    /// The voice message playing or paused.
    playback: playback::Playback,
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
    /// Unsent composer text by channel id and thread replies by
    /// `thread_key`, kept on disk for the signed-in account.
    drafts: caper_client::saved_drafts::SavedDrafts,
    signin_p: Panel,
    /// The open message menu and dialog, over the conversation.
    pub menu: Option<overlay::Menu>,
    pub dialog: Option<overlay::Dialog>,
    pub attachment_editor: Option<attachment_editor::Editor>,
    /// The media viewer over the conversation, and its media's panel.
    pub viewer: Option<viewer::Viewer>,
    /// The link preview player that is playing (`web_player`).
    pub web_player: Option<web_player::Player>,
    viewer_p: Panel,
    /// Files handed to another app must outlive the editor and its draft.
    opened_attachments: Vec<std::sync::Arc<caper_client::uploads::Scratch>>,
    menu_p: Panel,
    dialog_p: Panel,
    /// Dialog content rises separately from its stationary scrim.
    dialog_card_p: Panel,
    /// The reaction picker, the reply above the composer, and the toolbar
    /// control under the pointer and what the toolbar last drew.
    pub picker: Option<actions::Picker>,
    picker_p: Panel,
    /// The composer's GIF picker.
    pub gifs: Option<gifs::Picker>,
    gifs_p: Panel,
    /// Dictation or a voice note in the composer's place, and its hovered
    /// button.
    pub rec: Option<recording::Rec>,
    pub rec_hover: Option<recording::RPart>,
    /// An empty conversation's "No messages yet." and what it last drew.
    empty_p: Panel,
    empty_drawn: Option<((Option<&'static str>, String), Option<usize>, f32, f32)>,
    pub reply: Option<actions::ReplyBar>,
    reply_p: Panel,
    /// "Waiting for @x to accept" above your own request's composer: its
    /// text and height.
    pub note: Option<(String, f32)>,
    note_p: Panel,
    /// Who is typing, in the 16 px band above the composer, reply and note.
    typing_p: Panel,
    /// The composer's lip: a shade over the bottom of the history.
    lip_p: Panel,
    /// "You're viewing older messages" (`jump`): its state, panel, what
    /// the panel last drew, whether the pointer is on its Jump to latest
    /// button or the button has the keyboard, and its glide (from where,
    /// since when).
    pub jump: caper_client::jump::Jump,
    jump_p: Panel,
    jump_drawn: Option<jump::Drawn>,
    jump_hover: bool,
    pub jump_kb: bool,
    jump_glide: Option<(f64, Instant)>,
    thread_jump_p: Panel,
    thread_jump_drawn: Option<jump::Drawn>,
    tool_hover: Option<usize>,
    bar_drawn: Option<actions::BarKey>,
    /// The history's head needs drawing; the pointer is on its button;
    /// the list has reached its newest message since it opened, so reaching
    /// its start loads the previous page (as in the egui client).
    redraw_head: bool,
    /// The open "Upload a file" dialog, a send waiting for the composer's
    /// files (its text and files), and why files could not be attached.
    file_dialog: Option<std::sync::mpsc::Receiver<Vec<std::path::PathBuf>>>,
    send_waiting: Option<(Vec<u16>, Vec<u64>)>,
    attach_error: Option<String>,
    head_hover: bool,
    /// The foot (Load newer messages) needs drawing; the pointer is on its
    /// button; View original's highlighted message.
    redraw_foot: bool,
    foot_hover: bool,
    focused: Option<String>,
    /// When the latest jump started its highlight's hold and fade.
    focus_at: Option<Instant>,
    /// The who-reacted card, the chip it follows (message id and emoji),
    /// and its surface.
    card: Option<card::Card>,
    card_target: Option<(String, String)>,
    card_p: Panel,
    /// A person's profile card and its surface.
    profile: Option<profile::ProfileCard>,
    profile_p: Panel,
    /// Each person's unsent card message, kept until sent or sign-out.
    profile_drafts: std::collections::HashMap<String, Vec<u16>>,
    /// The focused composer's `:`/`@` suggestions, the token Escape hid
    /// (field, text and caret), and the popup's surface.
    suggest: Option<suggest::Suggest>,
    suggest_dismissed: Option<(suggest::Field, Vec<u16>, usize)>,
    suggest_p: Panel,
    /// The tooltip under the pointer, and its surface.
    tip: tips::State,
    tip_p: Panel,
    /// The history control with keyboard focus (a reaction chip, Add
    /// reaction, Retry or Dismiss), by message id so it survives paging.
    kb: Option<(String, list::Action)>,
    older_armed: bool,
    /// The open thread beside the conversation, its surface and its reply box's.
    pub thread: Option<thread::Pane>,
    thread_p: Panel,
    thread_box_p: Panel,
    drop_overlay: Option<drop_overlay::Open>,
    drop_p: Panel,
    /// The caret stopped blinking: the window is minimized or lost the
    /// keyboard, or input paused for the caret timeout. Input, focus or a
    /// restore starts it again.
    caret_paused: bool,
    /// The last key, click or focus, which the caret timeout counts from.
    caret_input: Instant,
    /// GIFs stopped on their frame while nobody could watch them
    /// (`resume_gifs`).
    gifs_paused: bool,
    /// The worker updated something since drafts were last tinted.
    tint_due: bool,
    /// Inside Windows' modal move/size loop, which runs instead of ours.
    sizing: bool,
    /// The notification-area icon that holds Caper while its window is
    /// closed.
    tray: tray::Tray,
    quit: bool,
}

/// Syncs the Win32 title bar's dark/light chrome and caption color with the active theme.
fn apply_window_theme(hwnd: HWND) {
    let dark = BOOL::from(!theme::is_light());
    let rgb = theme::blackout();
    // Win32 COLORREF is 0x00BBGGRR.
    let caption = ((rgb & 0xff) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 0xff);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const dark).cast(),
            std::mem::size_of::<BOOL>() as u32,
        );
        // Windows 11 tints the title bar to match the frame surface; Windows 10 ignores this attribute.
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            (&raw const caption).cast(),
            std::mem::size_of::<u32>() as u32,
        );
    }
}

impl App {
    /// Switches the live palette between Obsidian (dark) and Paper (light),
    /// recolors shared Direct2D brushes and the Win32 title bar, invalidates
    /// cached message rows and surfaces, and schedules a full repaint.
    pub fn apply_theme(&mut self, light: bool) {
        theme::set_light(light);
        self.g.apply_theme();
        apply_window_theme(self.hwnd);
        if let Ok(solid) = self.g.surface(1, 1) {
            let _ = gfx::draw(&solid, 1.0, |dc| {
                unsafe { dc.Clear(Some(&gfx::color(theme::surface_conversation(), 1.0))) };
                Ok(())
            });
            unsafe {
                let _ = self.background.SetContent(&solid);
            }
        }
        self.rows.trim(0..0);
        if let Some(t) = &mut self.thread {
            t.rows.trim(0..0);
            for i in 0..t.rows.len() {
                let _ = t.rows.ensure(&self.g, &t.c, i);
            }
        }
        if let Some(s) = &mut self.signin {
            s.clear_wordmark();
        }
        self.drawn = None;
        self.empty_drawn = None;
        self.jump_drawn = None;
        self.thread_jump_drawn = None;
        self.bar_drawn = None;
        self.redraw_head = true;
        self.redraw_foot = true;
        self.dirty = Dirty::all();
        self.need = true;
    }

    fn new(hwnd: HWND, g: Gfx, c: Conversation, bench: Bench) -> Result<Self> {
        apply_window_theme(hwnd);
        unsafe {
            // Below child windows: the link preview player shows above it.
            let target = g.dcomp.CreateTargetForHwnd(hwnd, false)?;
            let root = g.visual()?;
            root.SetBorderMode(DCOMPOSITION_BORDER_MODE_HARD)?;
            target.SetRoot(&root)?;
            // A 1×1 surface scaled to the window, behind everything.
            let background = g.visual()?;
            let solid = g.surface(1, 1)?;
            gfx::draw(&solid, 1.0, |dc| {
                dc.Clear(Some(&gfx::color(theme::surface_conversation(), 1.0)));
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
            let menu_p = Panel::new(&g)?.entering(&g, motion::Kind::Menu);
            let dialog_p = Panel::new(&g)?.entering(&g, motion::Kind::Fade);
            let dialog_card_p = Panel::new(&g)?.entering(&g, motion::Kind::Dialog);
            let viewer_p = Panel::new(&g)?;
            let pins_p = Panel::new(&g)?.entering(&g, motion::Kind::Fade);
            let picker_p = Panel::new(&g)?.entering(&g, motion::Kind::Menu);
            let gifs_p = Panel::new(&g)?.entering(&g, motion::Kind::Menu);
            let empty_p = Panel::new(&g)?;
            let reply_p = Panel::new(&g)?;
            let note_p = Panel::new(&g)?;
            let typing_p = Panel::new(&g)?;
            let lip_p = Panel::new(&g)?;
            let jump_p = Panel::new(&g)?.entering(&g, motion::Kind::Menu);
            let thread_jump_p = Panel::new(&g)?.entering(&g, motion::Kind::Menu);
            let thread_p = Panel::new(&g)?.entering(&g, motion::Kind::Thread);
            let thread_box_p = Panel::new(&g)?.entering(&g, motion::Kind::Thread);
            let drop_p = Panel::new(&g)?;
            let card_p = Panel::new(&g)?.entering(&g, motion::Kind::Menu);
            let profile_p = Panel::new(&g)?.entering(&g, motion::Kind::Menu);
            let suggest_p = Panel::new(&g)?.entering(&g, motion::Kind::Menu);
            let tip_p = Panel::new(&g)?.entering(&g, motion::Kind::Menu);
            let zoom_p = Panel::new(&g)?;
            let autoscroll_p = Panel::new(&g)?;
            let settings_p = Panel::new(&g)?.entering(&g, motion::Kind::Fade);
            let dm_menu_p = Panel::new(&g)?.entering(&g, motion::Kind::Menu);
            let narrow_p = Panel::new(&g)?;
            let narrow_menu_p = Panel::new(&g)?.entering(&g, motion::Kind::Menu);
            let members_backdrop = Panel::new(&g)?;
            let navigation_layer = g.visual()?;
            let conversation_layer = g.visual()?;
            let thread_layer = g.visual()?;
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
            history_clip.AddVisual(&lip_p.visual, true, &content)?;
            root.AddVisual(&background, false, None)?;
            navigation_layer.AddVisual(&rail.visual, false, None)?;
            navigation_layer.AddVisual(&sidebar.visual, true, &rail.visual)?;
            thread_layer.AddVisual(&thread_p.visual, false, None)?;
            thread_layer.AddVisual(&thread_box_p.visual, true, &thread_p.visual)?;
            thread_layer.AddVisual(&thread_jump_p.visual, true, &thread_p.visual)?;
            let mut prev: Option<IDCompositionVisual> = None;
            for v in [
                &header.visual,
                &history_clip,
                &empty_p.visual,
                &composer_p.visual,
                &reply_p.visual,
                &note_p.visual,
                &typing_p.visual,
            ] {
                conversation_layer.AddVisual(v, true, prev.as_ref())?;
                prev = Some(v.cast()?);
            }
            // Over the history, unclipped so its shadow and focus ring
            // show, below the composer and everything else.
            conversation_layer.AddVisual(&jump_p.visual, true, &history_clip)?;
            let mut prev: IDCompositionVisual = background.cast()?;
            for v in [
                &navigation_layer,
                &conversation_layer,
                &thread_layer,
                &narrow_p.visual,
                &members_backdrop.visual,
                &members.visual,
                &narrow_menu_p.visual,
                &drop_p.visual,
            ] {
                root.AddVisual(v, true, &prev)?;
                prev = v.cast()?;
            }
            root.AddVisual(&card_p.visual, true, &prev)?;
            root.AddVisual(&profile_p.visual, true, &card_p.visual)?;
            root.AddVisual(&menu_p.visual, true, &profile_p.visual)?;
            root.AddVisual(&suggest_p.visual, true, &menu_p.visual)?;
            root.AddVisual(&picker_p.visual, true, &suggest_p.visual)?;
            root.AddVisual(&gifs_p.visual, true, &picker_p.visual)?;
            root.AddVisual(&dm_menu_p.visual, true, &gifs_p.visual)?;
            root.AddVisual(&dialog_p.visual, true, &dm_menu_p.visual)?;
            root.AddVisual(&dialog_card_p.visual, true, &dialog_p.visual)?;
            root.AddVisual(&pins_p.visual, true, &dialog_card_p.visual)?;
            root.AddVisual(&viewer_p.visual, true, &pins_p.visual)?;
            root.AddVisual(&settings_p.visual, true, &viewer_p.visual)?;
            root.AddVisual(&signin_p.visual, true, &settings_p.visual)?;
            root.AddVisual(&tip_p.visual, true, &signin_p.visual)?;
            root.AddVisual(&zoom_p.visual, true, &tip_p.visual)?;
            root.AddVisual(&autoscroll_p.visual, true, &zoom_p.visual)?;
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
                composer: Composer::formatted(),
                scale,
                dpi: scale,
                zoom: 1.0,
                zoom_notice: false,
                zoom_p,
                panes: panes::State::default(),
                narrow: narrow::State::default(),
                navigation_layer,
                conversation_layer,
                thread_layer,
                narrow_p,
                narrow_menu_p,
                members_backdrop,
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
                history_y_drawn: HEADER_H,
                voice_error_drawn: 0.0,
                reaction_alert: None,
                content,
                vs,
                vs_w: 1,
                draw_w: 1,
                drawn_scale: scale,
                drawn: None,
                scroll_top: 0.0,
                at_end: true,
                hover_row: None,
                lit_row: None,
                hover_space: None,
                hover_channel: None,
                hover_channel_options: false,
                voice_hover: None,
                call_ui: Default::default(),
                space_options_hover: false,
                hover_member: None,
                hover_direct: None,
                hover_inbox: None,
                hover_invite: None,
                bar_hover: None,
                bar_pressed: None,
                box_hover: None,
                settings: None,
                account_menu: None,
                me_hover: false,
                settings_back: None,
                dm_menu: None,
                rail_action: None,
                dm_menu_p,
                gear_hover: false,
                members_hidden: false,
                members_thread_hid: false,
                members_toggle_hover: false,
                pins: None,
                member_search: None,
                member_search_hover: false,
                pins_p,
                pins_hover: false,
                updates: caper_client::updates::Updates::start(|| {}, false),
                update_dock: None,
                update_dock_hover: false,
                update_shine: None,
                shine_drawn: false,
                app_links: None,
                pending_invite: None,
                settings_p,
                settings_shown: false,
                side_focus: None,
                side_ring: false,
                pointer: None,
                tracking: false,
                sel: None,
                selecting: false,
                composer_drag: false,
                autoscroll: None,
                autoscroll_release: None,
                autoscroll_p,
                redraw: Vec::new(),
                gif_rows: Vec::new(),
                gif_due: None,
                playback: playback::Playback::default(),
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
                attachment_editor: None,
                viewer: None,
                web_player: None,
                viewer_p,
                opened_attachments: Vec::new(),
                menu_p,
                dialog_p,
                dialog_card_p,
                picker: None,
                picker_p,
                gifs: None,
                gifs_p,
                rec: None,
                rec_hover: None,
                empty_p,
                empty_drawn: None,
                reply: None,
                reply_p,
                note: None,
                note_p,
                typing_p,
                lip_p,
                jump: Default::default(),
                jump_p,
                jump_drawn: None,
                jump_hover: false,
                jump_kb: false,
                jump_glide: None,
                thread_jump_p,
                thread_jump_drawn: None,
                tool_hover: None,
                bar_drawn: None,
                redraw_head: false,
                file_dialog: None,
                send_waiting: None,
                attach_error: None,
                head_hover: false,
                redraw_foot: false,
                foot_hover: false,
                focused: None,
                focus_at: None,
                kb: None,
                card: None,
                card_target: None,
                card_p,
                profile: None,
                profile_p,
                profile_drafts: std::collections::HashMap::new(),
                suggest: None,
                suggest_dismissed: None,
                suggest_p,
                tip: tips::State::default(),
                tip_p,
                older_armed: false,
                thread: None,
                thread_p,
                thread_box_p,
                drop_overlay: None,
                drop_p,
                caret_paused: false,
                caret_input: Instant::now(),
                gifs_paused: false,
                tint_due: true,
                sizing: false,
                tray: tray::Tray::new(hwnd),
                quit: false,
            })
        }
    }

    /// Shows another conversation (the live one replacing the mock).
    fn set_conversation(&mut self, c: Conversation) -> Result<()> {
        self.stop_autoscroll();
        self.close_attachment_editor();
        self.close_viewer();
        let (channels, members) = side_layouts(&self.g, &c)?;
        self.rows = Rows::new(c.messages.len());
        self.c = c;
        self.playback.scrub = None;
        self.playback_follow();
        self.rec_follow();
        self.channel_layouts = channels;
        self.member_layouts = members;
        self.sel = None;
        self.hover_row = None;
        self.lit_row = None;
        self.menu = None;
        self.picker = None;
        self.gifs = None;
        self.older_armed = false;
        self.focused = None;
        self.foot_hover = false;
        if self.thread.take().is_some() {
            self.thread_members(false);
            self.dirty.layout = true;
        }
        self.scroll_top = 0.0;
        self.at_end = true;
        self.reset_jump();
        self.drawn = None;
        self.dirty = Dirty::all();
        self.events |= EV_ROWS;
        self.need = true;
        Ok(())
    }

    fn drain_live(&mut self) -> Result<()> {
        self.poll_file_dialog();
        self.poll_profile_editor();
        self.poll_attachment_editor();
        self.poll_viewer();
        self.poll_menu_copy();
        self.gifs_poll();
        self.poll_updates();
        self.poll_app_links();
        if avatars::poll() {
            self.avatars_changed();
        }
        let showing =
            self.chat_shown() && !self.narrow.menu && !(self.is_narrow() && self.members_w() > 0.0);
        let Some(live) = self.live.as_mut() else {
            return Ok(());
        };
        // A toast asks whether its conversation is being read by waking the
        // window; renewing the lease here answers without drawing a frame.
        live.showing(
            showing
                && unsafe {
                    // The tray menu makes a hidden window the foreground one.
                    GetForegroundWindow() == self.hwnd
                        && !windows::Win32::UI::WindowsAndMessaging::IsIconic(self.hwnd).as_bool()
                        && IsWindowVisible(self.hwnd).as_bool()
                },
        );
        let updates = live.drain();
        let images = live.pump_media(&mut files::D2d(self.g.dc.clone()));
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
            if let Some(title) = requests.title {
                let _ = SetWindowTextW(self.hwnd, &HSTRING::from(title));
            }
        }
        if requests.front {
            self.show_window();
        }
        if images {
            self.media_changed();
        }
        // A link card's answer arrives through the same wake.
        self.preview_tick();
        // Who `@` can name changes only through the worker's updates.
        self.tint_due |= !updates.is_empty();
        for u in updates {
            match u {
                live::Update::SignedOut(warning) => {
                    avatars::clear();
                    self.close_viewer();
                    self.close_pins();
                    self.stop_web_player();
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
                    if let Some(old) = self.ids.open.clone() {
                        let text = std::mem::take(&mut self.composer.text);
                        self.drafts.set(&old, &String::from_utf16_lossy(&text));
                    }
                    // The account's saved drafts, from before a restart or update too.
                    if let Some(account) = self.live.as_ref().and_then(|l| l.account_id.as_deref())
                    {
                        self.drafts.open_for(drafts_dir().as_deref(), account);
                    }
                    let text: Vec<u16> = ids
                        .open
                        .as_ref()
                        .map(|id| self.drafts.get(id).encode_utf16().collect())
                        .unwrap_or_default();
                    // A reply and Pins belong to the channel they were opened in.
                    if self.ids.open != ids.open {
                        self.reply = None;
                        self.close_pins();
                    }
                    self.ids = ids;
                    self.status = None;
                    self.close_signin()?;
                    self.set_conversation(c)?;
                    if !self.c.has_conversation() {
                        self.set_composer_focus(false);
                    }
                    // Restoring the channel's draft isn't typing.
                    if let Some(live) = &mut self.live {
                        live.typing.adopt(&String::from_utf16_lossy(&text));
                    }
                    self.composer.set_text(text);
                    self.composer_changed();
                    self.member_search_follow();
                    self.bar_changed();
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
                live::Update::DirectHead => {
                    self.direct_head_changed();
                    self.bar_changed();
                }
                live::Update::DirectFound { request, result } => self.direct_found(request, result),
                live::Update::Blocked(error) => self.blocked(error),
                live::Update::Left(error) => self.left(error),
                live::Update::Joining => self.bar_changed(),
                live::Update::InviteLink { request, result } => self.invite_link(request, result),
                live::Update::Invitation(invitation) => {
                    self.open_dialog(overlay::Dialog::invitation(invitation));
                }
                live::Update::Invitations => self.invitations_changed(),
                live::Update::InvitePeople(answer) => self.invite_people_answer(*answer),
                live::Update::ChannelSettings(id) => self.open_channel_settings(&id),
                live::Update::Settings => {
                    if self.settings.is_some() {
                        self.settings_changed();
                    }
                    self.mutes_changed();
                }
                live::Update::DirectCreated(error) => self.direct_created(error),
                live::Update::Badges => {
                    self.dirty.rail = true;
                    self.dirty.sidebar = true;
                    // The narrow account panel's Feedback count.
                    self.dirty.narrow = true;
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
                        if error.is_some() && error != old.reaction_error {
                            self.reaction_alert = Some(i);
                        }
                        new.reaction_error = error;
                        new.date = old.date.clone();
                        new.unread = old.unread;
                        new.revealed = old.revealed.clone();
                        new.forward_revealed.clone_from(&old.forward_revealed);
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
                        self.c.members.push(live::person(
                            &m.author.id,
                            &m.author.name,
                            crate::avatars::Face::avatar(&m.author),
                        ));
                    }
                    let watching = self.at_end
                        && self.watching_arrivals()
                        && self.chat_shown()
                        && self.live.as_ref().is_some_and(|l| !l.has_newer());
                    let mut new = live::message(&m, author, grouped, me.as_deref());
                    new.reaction_error = error;
                    new.date = date;
                    self.c.messages.push(new);
                    // Others' messages are counted while the toast shows.
                    if me.as_deref() != Some(m.author.id.as_str())
                        && self
                            .live
                            .as_ref()
                            .is_some_and(|l| !l.has_newer() && !l.is_blocked(&m.author.id))
                    {
                        self.jump_arrived();
                    }
                    self.rows.arrivals.received(&m.id, watching, Instant::now());
                    self.arm_arrivals();
                    self.rows.append(&self.g, &self.c)?;
                    self.need = true;
                }
                live::Update::Edited {
                    request,
                    reloaded,
                    result,
                } => self.edited(request, reloaded, result),
                live::Update::Deleted(request, error) => self.deleted(request, error),
                live::Update::Media => self.media_changed(),
                live::Update::Profile(person) => self.profile_finished(person),
                live::Update::Drafts => self.drafts_changed(),
                live::Update::AttachmentEdited(key, error) => {
                    self.attachment_edit_finished(key, error)
                }
                live::Update::Dictated { key, result } => self.dictated(key, result),
                live::Update::Open(url) => open_link(&url),
                live::Update::Versions {
                    request,
                    message,
                    result,
                } => self.versions_loaded(request, &message, result),
                live::Update::Presence(online) => {
                    // A snapshot that changes nobody's dot redraws nothing.
                    let mut changed = !std::mem::replace(&mut self.c.presence, true);
                    for (person, id) in self.c.members.iter_mut().zip(&self.ids.members) {
                        let now = online.contains(id);
                        changed |= std::mem::replace(&mut person.online, now) != now;
                    }
                    if changed {
                        self.dirty.members = true;
                        self.need = true;
                    }
                }
                live::Update::Connection(state) => {
                    if self.status.as_ref().is_none_or(|(_, error)| !error) {
                        self.status = state.map(|s| (s, false));
                        self.dirty.header = true;
                        self.need = true;
                    }
                }
                live::Update::Inbox => self.inbox_changed(),
                live::Update::Switcher => self.switcher_changed(),
                live::Update::Typing => {
                    self.dirty.typing = true;
                    self.need = true;
                    self.events |= EV_TYPING;
                    self.arm_typing();
                }
                live::Update::Voice => self.voice_changed(),
                live::Update::Notified => {
                    self.close_attachment_editor();
                    self.close_viewer();
                    self.close_pins();
                    self.close_dialog();
                    self.close_settings();
                    self.close_dm_menu();
                }
                live::Update::Warning(e) => {
                    self.status = Some((e, true));
                    self.dirty.header = true;
                    self.need = true;
                }
                live::Update::SendConfirmed => {
                    if self
                        .status
                        .as_ref()
                        .is_some_and(|(s, error)| *error && s.ends_with(live::RESEND_NOTE))
                    {
                        self.status = None;
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
        self.refresh_reactors();
        self.poll_forward();
        self.refresh_pins();
        self.poll_profile_composer();
        self.run_rail_action();
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

    /// Who is in voice in joined channel `i`: their count and one line of
    /// names, as egui's room cards word it.
    pub fn channel_voice(&self, i: usize) -> Option<(usize, String)> {
        let names = self.live.as_ref()?.voice_names(self.ids.channels.get(i)?);
        (!names.is_empty()).then(|| (names.len(), caper_client::voice::room_names(&names)))
    }

    pub fn list_x(&self) -> f32 {
        if self.is_narrow() {
            0.0
        } else {
            RAIL_W + self.sidebar_w()
        }
    }

    pub fn list_w(&self) -> f32 {
        self.chat_width().max(160.0)
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
        // 16px above the composer (and its reply), matching the 16px below it,
        // so the composer sits centered in its band.
        (self.box_rect().1
            - 16.0
            - self.history_y()
            - self.reply_h()
            - self.note_h()
            - self.voice_error_h())
        .max(0.0)
    }

    pub fn visible_rows(&self) -> Range<usize> {
        if !self.chat_shown() {
            return 0..0;
        }
        self.rows
            .range(self.scroll_top, self.scroll_top + self.vh() as f64)
    }

    pub fn composer_text_origin(&self) -> (f32, f32) {
        let (x, y, _, _) = self.box_rect();
        let (left, top) = self.composer.inset;
        (
            x + composer::PAD + left,
            y + composer::PAD + top - self.composer.scroll(),
        )
    }

    /// Client position of row `i`'s message text origin.
    pub fn row_text_origin(&self, i: usize) -> (f32, f32) {
        let ty = self.rows.row(i).map_or(0.0, |r| r.text_y);
        (
            self.list_x() + list::BODY_X,
            self.history_y()
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

    pub fn channel_rect(&self, i: usize) -> (f32, f32, f32, f32) {
        (
            RAIL_W + 12.0,
            CHANNEL_TOP + CHANNEL_H * i as f32,
            self.sidebar_w() - 24.0,
            CHANNEL_H - 2.0,
        )
    }

    /// Member `i`'s row, at its place among the shown members; empty
    /// while a search hides it.
    pub fn member_rect(&self, i: usize) -> (f32, f32, f32, f32) {
        let shown = self.shown_members();
        let online = self.shown_online(&shown);
        shown
            .iter()
            .position(|&k| k == i)
            .map_or((0.0, 0.0, 0.0, 0.0), |slot| {
                self.member_slot_rect(slot, online)
            })
    }

    /// The `slot`th shown member's row, with `online` of them online.
    pub fn member_slot_rect(&self, slot: usize, online: usize) -> (f32, f32, f32, f32) {
        (
            self.members_x() + 8.0,
            self.members_y() + self.member_slot_top(slot, online),
            self.member_width() - 16.0,
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
            self.history_y() + (self.rows.top[i] - self.scroll_top) as f32,
            self.list_w(),
            self.rows.height(i),
        )
    }

    /// The band above the composer (and its reply and note) where the
    /// history's 16 px gap is: who is typing shows there, as in egui.
    pub fn typing_rect(&self) -> (f32, f32, f32, f32) {
        let (bx, by, bw, _) = self.box_rect();
        let above = self.reply_h() + self.note_h() + self.voice_error_h();
        (bx, by - 16.0 - above, bw, 16.0)
    }

    /// What the typing band says, such as "Maya is typing…", while
    /// someone types.
    pub fn typing_label(&self) -> Option<String> {
        let names = self.live.as_ref()?.typing.names();
        (!names.is_empty()).then(|| typing::label(&names))
    }

    /// The dots bounce while someone types, unless animations are off or
    /// the window is in the background, as in egui.
    fn typing_animates(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| !l.typing.names().is_empty())
            && files::motion()
            && unsafe { GetForegroundWindow() } == self.hwnd
    }

    /// Keeps the typing timer running while it has work: a frame for the
    /// bounce, else a slower tick to expire entries and end your typing.
    fn arm_typing(&self) {
        let busy = self.live.as_ref().is_some_and(|l| l.typing.busy());
        unsafe {
            if self.typing_animates() {
                SetTimer(Some(self.hwnd), TIMER_TYPING, 33, None);
            } else if busy {
                SetTimer(Some(self.hwnd), TIMER_TYPING, 250, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_TYPING);
            }
        }
    }

    fn typing_tick(&mut self) {
        let changed = self.live.as_mut().is_some_and(|l| l.typing_tick());
        if changed {
            self.events |= EV_TYPING;
        }
        if changed || self.typing_animates() {
            self.dirty.typing = true;
            self.need = true;
        }
        self.arm_typing();
    }

    /// The header's member-list toggle (client DIPs), in a space's channel
    /// with members, as in egui.
    pub fn members_toggle_rect(&self) -> Option<(f32, f32, f32, f32)> {
        (!self.is_narrow()
            && self.c.has_conversation()
            && self.c.direct.is_none()
            && !self.c.members.is_empty())
        .then(|| {
            (
                self.list_x() + self.list_w() - 18.0 - 28.0,
                (HEADER_H - 28.0) / 2.0,
                28.0,
                28.0,
            )
        })
    }

    pub fn on_members_toggle(&self, x: f32, y: f32) -> bool {
        self.members_toggle_rect()
            .is_some_and(|(rx, ry, rw, rh)| x >= rx && x < rx + rw && y >= ry && y < ry + rh)
    }

    /// Shows or hides the member list; the conversation takes its width.
    /// The reader's choice outlasts a thread that hid the list.
    pub fn toggle_members(&mut self) {
        let thread_hid = std::mem::take(&mut self.members_thread_hid);
        if self.is_narrow() {
            self.narrow.members = !self.narrow.members;
            // As on the web, the wide column the thread hid comes back.
            self.members_hidden &= !thread_hid;
        } else {
            self.members_hidden = !self.members_hidden;
        }
        self.hover_member = None;
        self.member_search_follow();
        self.dirty.narrow = true;
        self.dirty.layout = true;
        self.dirty.header = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    // ---- state changes ----

    pub fn composer_changed(&mut self) {
        self.composer.caret_on = true;
        // Kept on every change, so quitting or updating never loses it.
        if let Some(open) = &self.ids.open {
            self.drafts
                .set(open, &String::from_utf16_lossy(&self.composer.text));
        }
        self.preview_tick();
        if let Some(live) = &mut self.live {
            live.typed(&String::from_utf16_lossy(&self.composer.text));
            self.arm_typing();
        }
        let (_, _, bw, _) = self.box_rect();
        if let Ok(h) = self.box_height(bw)
            && (h - self.box_h).abs() > 0.01
        {
            self.dirty.layout = true;
            self.events |= EV_SCREEN;
        }
        self.dirty.composer = true;
        self.events |= EV_TEXT;
        self.need = true;
        self.ime_position();
    }

    pub fn set_composer_focus(&mut self, focused: bool) {
        // A request's bar stands in for the composer.
        let focused =
            focused && self.c.has_conversation() && self.dm_bar().is_none() && self.rec().is_none();
        // One text field has focus: the reply box in the thread gives it up.
        if focused {
            self.jump_kb = false;
            self.narrow.focus = None;
            self.clear_pane_focus();
            self.set_thread_focus(false);
            self.clear_side_focus();
        }
        if focused
            && let Some(p) = &mut self.gifs
            && p.search.focused
        {
            p.search.focused = false;
            self.dirty.gifs = true;
        }
        if focused
            && let Some(s) = &mut self.member_search
            && s.field.focused
        {
            s.field.focused = false;
            s.field.end();
            self.dirty.members = true;
        }
        if focused {
            self.set_kb(None);
        }
        if self.composer.focused == focused {
            return;
        }
        self.composer.focused = focused;
        if !focused {
            self.composer.end();
        }
        self.composer.caret_on = true;
        unsafe {
            if focused {
                SetTimer(Some(self.hwnd), TIMER_CARET, GetCaretBlinkTime(), None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_CARET);
            }
        }
        self.composer_changed();
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
    /// Mutes and failed changes show in the DM rows and menu; the rows
    /// redraw again when the earliest timed mute ends.
    fn mutes_changed(&mut self) {
        self.dirty.rail = true;
        self.dirty.sidebar = true;
        self.dirty.dm_menu = true;
        // The account footer's Do Not Disturb, wide and narrow, and its menu.
        self.dirty.narrow = true;
        self.dirty.settings = true;
        self.events |= EV_ROWS;
        self.need = true;
        let wait = self.live.as_ref().and_then(|l| l.next_mute_expiry());
        unsafe {
            match wait {
                // Rounded up, so the mute has ended when it fires.
                Some(wait) => {
                    SetTimer(
                        Some(self.hwnd),
                        TIMER_MUTE,
                        (wait.as_millis() + 1).min(u32::MAX as u128) as u32,
                        None,
                    );
                }
                None => {
                    let _ = KillTimer(Some(self.hwnd), TIMER_MUTE);
                }
            }
        }
    }

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
        if let Some(e) = &self.attachment_editor {
            return (e.field.focused && !e.busy).then(|| (&e.field, e.text_origin()));
        }
        if let Some(d) = &self.dialog {
            if let Some(f) = &d.forwarding {
                return (forward::field(f.focus) && d.forward_enabled(f.focus))
                    .then(|| (d.field_for(f.focus), d.forward_origin(f.focus)));
            }
            if let Some(e) = &d.profile_edit {
                return matches!(e.focus, overlay::Part::Field | overlay::Part::Number(0))
                    .then(|| (d.field_for(e.focus), d.profile_field_origin(e.focus)))
                    .filter(|_| d.profile_enabled(e.focus));
            }
            return d.field.focused.then(|| (&d.field, d.text_origin()));
        }
        if let Some(s) = &self.signin {
            return s.field().focused.then(|| (s.field(), s.field_origin));
        }
        if let Some(c) = &self.profile
            && c.field.focused
        {
            return Some((&c.field, self.profile_text_origin()));
        }
        if let Some(p) = &self.picker {
            return p.search.focused.then(|| (&p.search, p.text_origin()));
        }
        if let Some(p) = &self.gifs
            && p.search.focused
        {
            return Some((&p.search, p.text_origin()));
        }
        if let Some(t) = &self.thread
            && t.composer.focused
        {
            return Some((&t.composer, self.thread_text_origin()));
        }
        if let Some(s) = &self.member_search
            && s.field.focused
        {
            return Some((&s.field, self.member_search_text_origin()));
        }
        self.composer
            .focused
            .then(|| (&self.composer, self.composer_text_origin()))
    }

    fn focused_field_mut(&mut self) -> Option<&mut Composer> {
        let profile_editable = self.profile_part_enabled(profile::PPart::Field);
        if let Some(e) = &mut self.attachment_editor {
            return Some(&mut e.field).filter(|f| f.focused && !e.busy);
        }
        if let Some(d) = &mut self.dialog {
            if let Some(f) = &d.forwarding {
                let p = f.focus;
                return (forward::field(p) && d.forward_enabled(p)).then(|| d.field_for_mut(p));
            }
            if let Some(e) = &d.profile_edit {
                let p = e.focus;
                return (matches!(p, overlay::Part::Field | overlay::Part::Number(0))
                    && d.profile_enabled(p))
                .then(|| d.field_for_mut(p));
            }
            if d.number_focus().is_some() {
                return d.number_mut();
            }
            return Some(&mut d.field).filter(|f| {
                f.focused
                    && matches!(
                        d.kind,
                        overlay::Kind::Edit
                            | overlay::Kind::StartDirect
                            | overlay::Kind::Browse
                            | overlay::Kind::JoinInvite
                            | overlay::Kind::CreateSpace
                            | overlay::Kind::CreateChannel
                            | overlay::Kind::InvitePeople
                    )
            });
        }
        if let Some(s) = &mut self.signin {
            return Some(s.field_mut()).filter(|f| f.focused);
        }
        if let Some(c) = &mut self.profile
            && c.field.focused
        {
            return profile_editable.then_some(&mut c.field);
        }
        if let Some(p) = &mut self.picker {
            return Some(&mut p.search).filter(|f| f.focused);
        }
        if let Some(p) = &mut self.gifs
            && p.search.focused
        {
            return Some(&mut p.search);
        }
        if let Some(t) = &mut self.thread
            && t.composer.focused
        {
            return Some(&mut t.composer);
        }
        if let Some(s) = &mut self.member_search
            && s.field.focused
        {
            return Some(&mut s.field);
        }
        Some(&mut self.composer).filter(|c| c.focused)
    }

    /// Redraws the panel holding the focused field, for its caret.
    fn caret_dirty(&mut self) {
        if self.dialog.is_some() || self.attachment_editor.is_some() {
            self.dirty.overlay = true;
        } else if self.signin.is_some() {
            self.dirty.signin = true;
        } else if self.profile.as_ref().is_some_and(|c| c.field.focused) {
            self.dirty.profile = true;
        } else if self.picker.as_ref().is_some_and(|p| p.search.focused) {
            self.dirty.picker = true;
        } else if self.gifs.as_ref().is_some_and(|p| p.search.focused) {
            self.dirty.gifs = true;
        } else if self.thread.as_ref().is_some_and(|t| t.composer.focused) {
            self.dirty.thread_box = true;
        } else if self.member_search.as_ref().is_some_and(|s| s.field.focused) {
            self.dirty.members = true;
        } else {
            self.dirty.composer = true;
        }
        self.need = true;
    }

    /// Blinks the focused field's caret. Like Windows' own edit controls,
    /// it hides while the window is in the background or minimized, and
    /// rests, shown, once input pauses for the system's caret timeout; the
    /// timer stops either way until input, focus or a restore.
    fn caret_tick(&mut self, focus_lost: bool) {
        let away =
            focus_lost || unsafe { IsIconic(self.hwnd).as_bool() || GetFocus() != self.hwnd };
        let rested = caret_timeout().is_some_and(|t| self.caret_input.elapsed() >= t);
        if away || rested {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_CARET);
            }
            self.caret_paused = true;
        }
        if let Some(f) = self.focused_field_mut() {
            let on = if away { false } else { rested || !f.caret_on };
            if f.caret_on != on {
                f.caret_on = on;
                self.caret_dirty();
            }
        }
    }

    /// After input or focus: the caret timeout starts over, and a paused
    /// caret shows and blinks again.
    fn resume_caret(&mut self) {
        self.caret_input = Instant::now();
        if !std::mem::take(&mut self.caret_paused) {
            return;
        }
        let hwnd = self.hwnd;
        if let Some(f) = self.focused_field_mut() {
            f.caret_on = true;
            unsafe {
                SetTimer(Some(hwnd), TIMER_CARET, GetCaretBlinkTime(), None);
            }
            self.caret_dirty();
        }
    }

    /// After an edit or caret move in the focused field.
    fn field_changed(&mut self, text: bool) {
        if self.attachment_editor.is_some() {
            self.attachment_editor_changed();
        } else if self.dialog.is_some() {
            self.dialog_changed();
        } else if self.signin.is_some() {
            self.signin_changed(text);
        } else if self.profile.as_ref().is_some_and(|c| c.field.focused) {
            self.profile_changed();
        } else if self.picker.is_some() {
            self.picker_changed(text);
        } else if self.gifs.as_ref().is_some_and(|p| p.search.focused) {
            self.gifs_changed(text);
        } else if self.thread.as_ref().is_some_and(|t| t.composer.focused) {
            self.thread_composer_changed();
        } else if self.member_search.as_ref().is_some_and(|s| s.field.focused) {
            self.member_search_changed(text);
        } else {
            self.composer_changed();
        }
    }

    /// The keyboard navigation every client has: Ctrl+K opens the quick
    /// switcher, Ctrl+/ the shortcuts sheet, Alt+Up/Down steps through the
    /// sidebar's conversations (with Shift, the unread ones), and
    /// Ctrl+Shift+M and +D mute and deafen while in a call. Other surfaces
    /// (Settings, viewers, dialogs) keep their keys.
    fn navigation_key(&mut self, wp: WPARAM, ctrl: bool, shift: bool) -> Option<LRESULT> {
        let alt = unsafe { GetKeyState(VK_MENU.0 as i32) } < 0;
        let vk = VIRTUAL_KEY(wp.0 as u16);
        if !self.c.is_live() || self.live.as_ref().is_none_or(|l| l.account_id.is_none()) {
            return None;
        }
        let covered = self.settings.is_some()
            || self.pins.is_some()
            || self.profile.is_some()
            || self.signin.is_some()
            || self.viewer.is_some()
            || self.attachment_editor.is_some();
        let other_dialog = self.dialog.as_ref().is_some_and(|d| !d.switcher());
        match vk {
            VK_K if ctrl && !alt && !shift && !covered && !other_dialog => self.open_switcher(),
            VK_OEM_2 if ctrl && !alt && !covered && !other_dialog => self.open_shortcut_help(),
            VK_UP | VK_DOWN if alt && !ctrl && !covered && self.dialog.is_none() => {
                if let Some(live) = &mut self.live {
                    live.step_conversation(vk == VK_DOWN, shift);
                }
                self.set_browsing(false);
            }
            VK_M | VK_D
                if ctrl && shift && !alt && self.live.as_ref().is_some_and(|l| l.in_call()) =>
            {
                if let Some(live) = &mut self.live {
                    if vk == VK_M {
                        live.toggle_mute();
                    } else {
                        live.toggle_deafen();
                    }
                }
                self.voice_changed();
            }
            _ => return None,
        }
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), live::WM_LIVE, WPARAM(0), LPARAM(0));
        }
        self.need = true;
        Some(LRESULT(0))
    }

    /// Escape in the conversation, once nothing else used it: marks it read
    /// through its newest message.
    pub fn mark_open_read(&mut self) -> bool {
        let read = self.live.as_mut().is_some_and(|l| l.mark_open_read());
        if read {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), live::WM_LIVE, WPARAM(0), LPARAM(0));
            }
            self.need = true;
        }
        read
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

    /// A pointer click, Enter or Narrator's Invoke on a channel row: as on
    /// web, the row it picked keeps the keyboard.
    pub fn pick_channel(&mut self, i: usize) {
        self.select_channel(i);
        self.focus_side(Side::Channel(i));
    }

    /// The same for a DM row.
    pub fn pick_direct(&mut self, k: usize) {
        self.open_direct_row(k);
        self.focus_side(Side::Direct(k));
    }

    pub fn focus_side(&mut self, side: Side) {
        // Narrow navigation closes as it picks, so its rows can't keep focus.
        if self.is_narrow() {
            return;
        }
        self.set_composer_focus(false);
        self.set_thread_focus(false);
        self.set_member_search_focus(false);
        self.clear_pane_focus();
        self.set_kb(None);
        self.narrow.focus = None;
        self.side_focus = Some(side);
        self.side_ring = false;
        self.dirty.sidebar = true;
        self.events |= EV_FOCUS;
        self.need = true;
    }

    /// As web's page order, Tab past the composer (or the thread's reply
    /// box) wraps to the sidebar: the open channel's or DM's row, with its
    /// ring. False in a narrow window, whose sidebar is behind Browse.
    pub fn tab_to_sidebar(&mut self) -> bool {
        let direct = self
            .direct_rows()
            .iter()
            .position(|row| row.active && row.kind == crate::directs::RowKind::Direct);
        let side = match direct {
            Some(k) if self.c.direct.is_some() => Side::Direct(k),
            _ if self.c.channel < self.c.channels.len() => Side::Channel(self.c.channel),
            _ if !self.c.channels.is_empty() => Side::Channel(0),
            _ if !self.direct_rows().is_empty() => Side::Direct(0),
            _ => return false,
        };
        self.focus_side(side);
        if self.side_focus.is_none() {
            return false;
        }
        self.side_ring = true;
        true
    }

    pub fn clear_side_focus(&mut self) {
        if self.side_focus.take().is_some() {
            self.dirty.sidebar = true;
            self.events |= EV_FOCUS;
            self.need = true;
        }
    }

    /// Narrator moved focus to the message list: nothing else keeps the keys.
    pub fn focus_history(&mut self) {
        self.set_composer_focus(false);
        self.set_thread_focus(false);
        self.set_member_search_focus(false);
        self.clear_pane_focus();
        self.set_kb(None);
        self.narrow.focus = None;
        self.clear_side_focus();
        self.events |= EV_FOCUS;
        self.need = true;
    }

    /// Up and Down move between rows of the focused list (the account
    /// panel's rows follow the DM list), Enter and Space pick, Tab goes on
    /// to the splitters, and Shift+Tab or Escape go to the composer.
    fn side_key(&mut self, side: Side, vk: VIRTUAL_KEY, shift: bool) -> bool {
        let (k, n) = match side {
            Side::Channel(i) => (i, self.c.channels.len()),
            Side::Direct(k) => (k, self.direct_rows().len()),
            Side::Inbox(k) => (k, self.inbox_rows().len()),
        };
        let to = |k: usize| match side {
            Side::Channel(_) => Side::Channel(k),
            Side::Direct(_) => Side::Direct(k),
            Side::Inbox(_) => Side::Inbox(k),
        };
        let directs = self.direct_rows().len();
        match vk {
            VK_UP if k > 0 => self.focus_side(to(k - 1)),
            VK_DOWN if k + 1 < n => self.focus_side(to(k + 1)),
            VK_DOWN if matches!(side, Side::Direct(_)) && !self.inbox_rows().is_empty() => {
                self.focus_side(Side::Inbox(0))
            }
            VK_UP if matches!(side, Side::Inbox(_)) && directs > 0 => {
                self.focus_side(Side::Direct(directs - 1))
            }
            VK_UP | VK_DOWN => {}
            VK_RETURN | VK_SPACE => match side {
                Side::Channel(i) => self.pick_channel(i),
                Side::Direct(k) => self.pick_direct(k),
                Side::Inbox(k) => self.pick_inbox(k),
            },
            // Tab goes on to the splitters, then the conversation; Shift+Tab
            // wraps back to its end, the composer, as web's page order does.
            VK_TAB if !shift && self.pane_rect(crate::panes::Pane::Sidebar).is_some() => {
                self.clear_side_focus();
                self.focus_pane(crate::panes::Pane::Sidebar);
                return true;
            }
            VK_TAB | VK_ESCAPE => self.set_composer_focus(true),
            _ => return false,
        }
        self.side_ring = true;
        true
    }

    pub fn select_channel(&mut self, i: usize) {
        self.set_browsing(false);
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
        let ay = self.scroll_top + (y - self.history_y()) as f64;
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

    /// Whether (`x`, `y`) is on a shown message's text.
    fn over_text(&self, x: f32, y: f32) -> bool {
        let ay = self.scroll_top + (y - self.history_y()) as f64;
        let Some(i) = self.rows.row_at(ay) else {
            return false;
        };
        let Some(row) = self.rows.row(i) else {
            return false;
        };
        let (ox, oy) = self.row_text_origin(i);
        gfx::on_text(&row.text, x - ox, y - oy)
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
        self.chat_shown()
            && self.c.has_conversation()
            && x < self.list_x() + self.chat_width()
            && x >= bx
            && x < bx + bw
            && y >= by
            && y < by + bh
    }

    pub(crate) fn in_history(&self, x: f32, y: f32) -> bool {
        self.chat_shown()
            && !self.on_member_overlay(x, y)
            && x >= self.list_x()
            && x < self.list_x() + self.chat_width()
            && y >= self.history_y()
            && y < self.history_y() + self.vh()
    }

    fn update_hover(&mut self) {
        let inside = |(l, t, w, h): (f32, f32, f32, f32), x: f32, y: f32| {
            x >= l && x < l + w && y >= t && y < t + h
        };
        let (mut space, mut channel, mut member, mut direct) = (None, None, None, None);
        let (mut channel_options, mut space_options) = (false, false);
        if let Some((x, y)) = self.pointer {
            space = (0..self.c.spaces.len()).find(|&i| inside(Self::space_rect(i), x, y));
            channel = (0..self.c.channels.len()).find(|&i| inside(self.channel_rect(i), x, y));
            // A hidden member list leaves its column to the conversation.
            if self.members_w() > 0.0 {
                let shown = self.shown_members();
                let online = self.shown_online(&shown);
                member = (0..shown.len())
                    .find(|&k| inside(self.member_slot_rect(k, online), x, y))
                    .map(|k| shown[k]);
            }
            // The call card covers the DM rows under it.
            direct = if self.on_call(x, y) {
                None
            } else {
                self.direct_at(x, y)
            };
            channel_options = channel.is_some_and(|i| {
                self.channel_scope(i).is_some() && inside(self.channel_options_rect(i), x, y)
            });
            space_options = self.space_scope().is_some() && inside(self.space_options_rect(), x, y);
            // The account panel covers the sidebar rows under it.
            if self.on_account_panel(x, y) {
                channel = None;
                direct = None;
                channel_options = false;
            }
        }
        if !self.navigation_shown() {
            space = None;
            channel = None;
            direct = None;
            channel_options = false;
            space_options = false;
        }
        if direct != self.hover_direct {
            self.hover_direct = direct;
            self.dirty.sidebar = true;
            self.need = true;
        }
        let invite = self
            .pointer
            .filter(|&(x, y)| self.navigation_shown() && !self.on_account_panel(x, y))
            .and_then(|(x, y)| self.invite_at(x, y));
        if invite != self.hover_invite {
            self.hover_invite = invite;
            self.dirty.rail = true;
            self.dirty.sidebar = true;
            self.need = true;
        }
        self.track_bar_hover();
        self.track_rec_hover();
        self.track_voice_hover();
        let box_hover = self.pointer.and_then(|(x, y)| self.box_part_at(x, y));
        if box_hover != self.box_hover {
            self.box_hover = box_hover;
            self.dirty.composer = true;
            self.need = true;
        }
        let toggle = self
            .pointer
            .is_some_and(|(x, y)| self.on_members_toggle(x, y));
        if toggle != self.members_toggle_hover {
            self.members_toggle_hover = toggle;
            self.dirty.header = true;
            self.need = true;
        }
        let pins = self.pointer.is_some_and(|(x, y)| self.on_pins_button(x, y));
        if pins != self.pins_hover {
            self.pins_hover = pins;
            self.dirty.header = true;
            self.need = true;
        }
        let gear = self.pointer.is_some_and(|(x, y)| self.on_gear(x, y));
        if gear != self.gear_hover {
            self.gear_hover = gear;
            self.dirty.sidebar = true;
            self.need = true;
        }
        let me = self.pointer.is_some_and(|(x, y)| self.on_me(x, y));
        if me != self.me_hover {
            self.me_hover = me;
            self.dirty.sidebar = true;
            self.dirty.narrow = true;
            self.need = true;
        }
        self.track_inbox_hover();
        let dock = self.pointer.is_some_and(|(x, y)| self.on_update_dock(x, y));
        if dock != self.update_dock_hover {
            self.update_dock_hover = dock;
            self.dirty.sidebar = true;
            self.need = true;
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
        if (channel_options, space_options)
            != (self.hover_channel_options, self.space_options_hover)
        {
            self.hover_channel_options = channel_options;
            self.space_options_hover = space_options;
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
        // Settings covers the history, and the toast floats over it.
        if !self.in_history(x, y) || self.settings.is_some() || self.jump_at(x, y) {
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
        self.rows
            .row_at(self.scroll_top + (y - self.history_y()) as f64)
    }

    /// Sends the composer's text and files (to the local development mock
    /// without a server). With files still uploading it goes when they finish.
    fn send(&mut self) -> Result<()> {
        let text = String::from_utf16_lossy(&self.composer.text)
            .trim()
            .to_string();
        let files = self
            .live
            .as_ref()
            .is_some_and(|l| !l.open_drafts().is_empty());
        if text.is_empty() && !files {
            return Ok(());
        }
        if self.live.is_some() {
            if files && !self.send_files(text.clone()) {
                return Ok(());
            }
            let reply = self.reply.take().map(|r| r.target.id);
            if reply.is_some() {
                self.dirty.layout = true;
                self.dirty.reply = true;
                self.events |= EV_SCREEN;
            }
            if !files && let Some(live) = &self.live {
                live.send(text, reply, None);
            }
            // Like web, a send (a reply to an old message too) jumps to
            // the latest message wherever the reader had scrolled.
            self.at_end = true;
            self.need = true;
            self.set_focused(None);
        } else {
            self.c.send_local(text);
            self.rows.append(&self.g, &self.c)?;
            self.at_end = true;
        }
        self.composer.set_text(Vec::new());
        self.composer_changed();
        Ok(())
    }

    // ---- input ----

    fn handle(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let (x, y) = if msg == WM_SETCURSOR {
            // WM_SETCURSOR carries no position and arrives before the
            // WM_MOUSEMOVE it precedes: read where the pointer is now, so
            // every handler picks the cursor for this spot, not the last.
            let mut p = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut p);
                let _ = ScreenToClient(self.hwnd, &mut p);
            }
            let at = (p.x as f32 / self.scale, p.y as f32 / self.scale);
            if (lp.0 & 0xffff) as u32 == HTCLIENT {
                self.pointer = Some(at);
            }
            at
        } else if msg == WM_CONTEXTMENU {
            // Screen coordinates, or (-1, -1) from the keyboard.
            let mut p = POINT {
                x: (lp.0 & 0xffff) as i16 as i32,
                y: ((lp.0 >> 16) & 0xffff) as i16 as i32,
            };
            if (p.x, p.y) != (-1, -1) {
                unsafe {
                    let _ = ScreenToClient(self.hwnd, &mut p);
                }
            }
            (p.x as f32 / self.scale, p.y as f32 / self.scale)
        } else {
            (
                (lp.0 & 0xffff) as i16 as f32 / self.scale,
                ((lp.0 >> 16) & 0xffff) as i16 as f32 / self.scale,
            )
        };
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        if matches!(
            msg,
            WM_KEYDOWN
                | WM_SYSKEYDOWN
                | WM_CHAR
                | WM_MOUSEMOVE
                | WM_LBUTTONDOWN
                | WM_RBUTTONDOWN
                | WM_MBUTTONDOWN
                | WM_MOUSEWHEEL
                | WM_MOUSEHWHEEL
        ) && let Some(live) = &mut self.live
        {
            live.active();
        }
        if matches!(
            msg,
            WM_KEYDOWN
                | WM_SYSKEYDOWN
                | WM_CHAR
                | WM_LBUTTONDOWN
                | WM_RBUTTONDOWN
                | WM_MBUTTONDOWN
                | WM_IME_COMPOSITION
                | WM_SETFOCUS
        ) {
            self.resume_caret();
        }
        // Brought to the front: GIFs that paused behind other windows play.
        if msg == WM_ACTIVATE && wp.0 & 0xffff != 0 {
            self.resume_gifs();
        }
        if let Some(r) = self.autoscroll_input(msg, wp, x, y) {
            return Some(r);
        }
        // Ctrl +/−/0 zoom everywhere, as in egui; no field types these.
        if msg == WM_KEYDOWN && ctrl {
            let steps = match VIRTUAL_KEY(wp.0 as u16) {
                VK_OEM_PLUS | VK_ADD => Some(1),
                VK_OEM_MINUS | VK_SUBTRACT => Some(-1),
                VK_0 | VK_NUMPAD0 => Some(0),
                _ => None,
            };
            if let Some(steps) = steps {
                self.zoom_by(steps);
                return Some(LRESULT(0));
            }
        }
        // Ctrl+K, Ctrl+/, Alt+Up/Down and Ctrl+Shift+M/D, as on web.
        if matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN)
            && let Some(r) = self.navigation_key(wp, ctrl, shift)
        {
            return Some(r);
        }
        // A click Caper saw was outside the player: keys come back here.
        if matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN)
            && let Some(player) = &self.web_player
        {
            player.blur();
        }
        // Always release the shell's HDROP, including while a modal surface
        // is open. The shared pane hit test rejects covered destinations.
        if msg == WM_DROPFILES {
            self.dropped(windows::Win32::UI::Shell::HDROP(wp.0 as _));
            return Some(LRESULT(0));
        }
        if let Some(r) = self.attachment_editor_input(msg, wp, x, y, ctrl, shift) {
            return r;
        }
        if let Some(r) = self.viewer_input(msg, wp, x, y, shift) {
            return r;
        }
        if self.pins.is_some()
            && let Some(r) = self.pins_input(msg, wp, x, y)
        {
            return r;
        }
        if self.signin.is_some()
            && let Some(r) = self.signin_input(msg, wp, lp, x, y, ctrl, shift)
        {
            return r;
        }
        if self.settings.is_some()
            && let Some(r) = self.settings_input(msg, wp, x, y)
        {
            return r;
        }
        if self.dm_menu.is_some()
            && let Some(r) = self.dm_menu_input(msg, wp, x, y)
        {
            return r;
        }
        if self.account_menu.is_some()
            && let Some(r) = self.account_menu_input(msg, wp, x, y)
        {
            return r;
        }
        if let Some(r) = self.suggest_input(msg, wp, x, y, ctrl, shift) {
            return r;
        }
        if self.profile_emoji_open()
            && let Some(r) = self.picker_input(msg, wp, x, y, ctrl, shift)
        {
            return r;
        }
        if self.profile_gifs_open()
            && let Some(r) = self.gifs_input(msg, wp, x, y, ctrl, shift)
        {
            return r;
        }
        if self.profile.is_some()
            && let Some(r) = self.profile_input(msg, wp, x, y, ctrl, shift)
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
        if self.gifs.is_some()
            && let Some(r) = self.gifs_input(msg, wp, x, y, ctrl, shift)
        {
            return r;
        }
        // The account footer is behind Browse in the narrow shell, but the
        // account's Settings shortcut remains available from either view.
        if msg == WM_KEYDOWN
            && ctrl
            && wp.0 == VK_OEM_COMMA.0 as usize
            && self.c.is_live()
            && self.live.as_ref().is_some_and(|l| l.account_id.is_some())
        {
            self.open_settings();
            return Some(LRESULT(0));
        }
        if let Some(r) = self.pane_input(msg, wp, x, y, shift) {
            return Some(r);
        }
        if let Some(r) = self.narrow_input(msg, wp, x, y, ctrl, shift) {
            return Some(r);
        }
        if (self.navigation_shown() || self.inline_voice_error())
            && let Some(r) = self.call_input(msg, wp, x, y, shift)
        {
            return Some(r);
        }
        if let Some(r) = self.video_input(msg, wp, x, y, ctrl, shift) {
            return Some(r);
        }
        if self.thread_shown()
            && let Some(r) = self.thread_input(msg, wp, x, y, ctrl, shift)
        {
            return r;
        }
        if self.members_w() > 0.0
            && let Some(r) = self.member_search_input(msg, wp, x, y, ctrl, shift)
        {
            return r;
        }
        if self.is_narrow()
            && (!self.chat_shown() || self.narrow.menu || self.members_w() > 0.0)
            && matches!(
                msg,
                WM_KEYDOWN
                    | WM_CHAR
                    | WM_LBUTTONDOWN
                    | WM_LBUTTONDBLCLK
                    | WM_CONTEXTMENU
                    | WM_MOUSEWHEEL
            )
        {
            return Some(LRESULT(0));
        }
        match msg {
            WM_SIZE => {
                let (pw, ph) = ((lp.0 & 0xffff) as f32, ((lp.0 >> 16) & 0xffff) as f32);
                self.resized(pw, ph);
                if wp.0 != SIZE_MINIMIZED as usize {
                    self.resume_caret();
                    self.resume_gifs();
                    self.arm_call_timer();
                }
                // Nothing was drawn while minimized; GIFs resume with this frame.
                self.need = true;
            }
            WM_ENTERSIZEMOVE => self.sizing = true,
            WM_EXITSIZEMOVE => self.sizing = false,
            WM_SETTINGCHANGE => self.motion_changed(),
            WM_TIMER => match wp.0 {
                TIMER_BENCH if self.bench.tick() => self.quit = true,
                TIMER_LIVE => unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_LIVE);
                    let _ = PostMessageW(Some(self.hwnd), live::WM_LIVE, WPARAM(0), LPARAM(0));
                },
                TIMER_CARET => self.caret_tick(false),
                TIMER_CARD => self.show_card(),
                TIMER_ZOOM => {
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), TIMER_ZOOM);
                    }
                    self.zoom_notice = false;
                    self.dirty.zoom = true;
                    self.need = true;
                }
                invite_people::TIMER_COPIED => self.copied_tick(),
                TIMER_MEDIA => self.next_gif_frame(),
                drop_overlay::TIMER_DROP => self.need = true,
                autoscroll::TIMER_AUTOSCROLL => self.autoscroll_tick(),
                motion::TIMER_ARRIVAL => self.arrivals_tick(),
                jump::TIMER_JUMP => self.jump_tick(),
                history::TIMER_FOCUS => self.focus_tick(),
                video::TIMER_VIDEO => self.inline_video_tick(),
                thread::TIMER_MEDIA => self.thread_media_tick(),
                pins::TIMER_PINS => self.pins_tick(),
                viewer::TIMER_VIEWER => self.viewer_tick(),
                TIMER_MUTE => {
                    // Do Not Disturb's status in Settings ends with it.
                    if self.settings.is_some() {
                        self.settings_changed();
                    }
                    self.mutes_changed();
                }
                updates::TIMER_SHINE => self.shine_tick(),
                updates::TIMER_RELEASE => {
                    self.dirty.overlay = true;
                    self.need = true;
                }
                tips::TIMER_TIP => self.tip_tick(),
                web_player::TIMER_WEB_PLAYER => self.need = true,
                // Speaking rings, their release and the call's running time;
                // a minimized window stops the step until it is restored.
                call::TIMER_CALL => {
                    if unsafe { IsIconic(self.hwnd).as_bool() } {
                        self.arm_call_timer();
                    } else {
                        self.call_tick();
                    }
                }
                settings::TIMER_INBOX => self.inbox_tick(),
                settings::TIMER_VERSION_COPIED => self.version_copied_tick(),
                TIMER_TYPING => self.typing_tick(),
                settings::audio::TIMER_AUDIO => self.audio_tick(),
                playback::TIMER_PLAYBACK => self.playback_tick(),
                recording::TIMER_RECORDING => self.recording_tick(),
                profile::TIMER_PROFILE_RECORDING => self.profile_recording_tick(),
                preview::TIMER_PREVIEW => self.preview_tick(),
                TIMER_GIFS => self.gifs_tick(),
                TIMER_GIF_FRAMES => self.next_picker_frame(),
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
                self.track_quote_hover();
                self.track_head_hover();
                self.track_foot_hover();
                self.track_jump_hover();
                self.track_card();
                if self.playback.scrub.is_some() {
                    self.voice_message_scrub(x, false);
                } else if self.composer_drag {
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
                self.track_jump_hover();
                self.track_tool_hover();
                self.track_reply_hover();
                self.track_quote_hover();
                self.track_card();
                self.need = true;
            }
            WM_LBUTTONDOWN => {
                unsafe {
                    SetCapture(self.hwnd);
                }
                if !self.jump_button_at(x, y) {
                    self.set_jump_kb(false);
                }
                if self.jump_button_at(x, y) {
                    self.jump_activate();
                } else if self.jump_at(x, y) {
                    // The rest of the toast keeps the click off the rows.
                } else if self.on_gear(x, y) {
                    self.open_settings();
                } else if self.on_me(x, y) {
                    self.toggle_account_menu();
                } else if let Some(k) = self.inbox_at(x, y) {
                    self.pick_inbox(k);
                } else if self.on_members_toggle(x, y) {
                    self.toggle_members();
                } else if self.on_pins_button(x, y) {
                    self.open_pins();
                } else if self.on_update_dock(x, y) {
                    self.open_update_review();
                } else if let Some(part) = self.bar_at(x, y) {
                    self.bar_pressed = Some(part);
                    self.dirty.composer = true;
                    self.need = true;
                } else if let Some(part) = self.rec_at(x, y) {
                    self.rec_activate(part);
                } else if (self.dm_bar().is_some() || self.rec().is_some()) && self.in_box(x, y) {
                    self.set_composer_focus(false);
                } else if let Some(part) = self.box_part_at(x, y) {
                    self.box_activate(part);
                } else if self.in_box(x, y) {
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
                    self.clear_side_focus();
                    if let Some((i, k)) = self.tool_at(x, y) {
                        if let Some(t) = self.tools(i).get(k) {
                            self.tool_activate(i, t.tool);
                        }
                    } else if self.reply_click(x, y) {
                    } else if let Some(i) = self.thread_link_at(x, y) {
                        self.open_thread_of(i);
                    } else if self.on_head_button(x, y) {
                        self.load_older();
                    } else if self.foot_at(x, y) {
                        self.foot_activate();
                    } else if let Some(i) = self.quote_link_at(x, y) {
                        self.view_original_of(i);
                    } else if let Some((i, part)) = self.forward_at(x, y) {
                        self.forward_click(false, i, part, x, y);
                    } else if let Some(i) = self.edited_at(x, y) {
                        self.open_history_of(i);
                    } else if let Some((i, k, part)) = self.voice_message_at(x, y) {
                        if let playback::Part::Seek(at) = part {
                            self.playback.scrub = Some((i, k, at));
                            self.voice_message_scrub(x, false);
                        } else {
                            self.voice_message_activate(i, k, part);
                        }
                    } else if let Some((i, k, hide)) = self.file_at(x, y)
                        && !self.c.messages[i]
                            .attachments
                            .get(k)
                            .is_some_and(playback::is_voice)
                    {
                        self.file_activate(i, k, hide);
                    } else if let Some((i, k)) = self.embed_at(x, y) {
                        self.embed_activate(i, k);
                    } else if let Some((i, action)) = self.action_at(x, y) {
                        self.row_action(i, action);
                    } else if let Some((i, k)) = self.spoiler_at(x, y) {
                        self.reveal_spoiler(i, k);
                    } else if let Some((id, entry, author)) = self.person_at(x, y) {
                        self.open_profile(id, entry, author, x, y);
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
                    } else if let Some(p) = self.voice_at(x, y) {
                        self.voice_activate(p);
                    } else if self.on_call(x, y) {
                    } else if let Some(spot) = self.hover_invite {
                        self.invite_activate(spot);
                    } else if self.space_options_hover {
                        let scope = self.space_scope();
                        self.toggle_scope_menu(scope);
                    } else if let Some(i) = self.hover_channel {
                        if self.hover_channel_options {
                            let scope = self.channel_scope(i);
                            self.toggle_scope_menu(scope);
                        } else {
                            self.pick_channel(i);
                        }
                    } else if let Some(directs::Spot::Row(k)) = self.hover_direct {
                        self.pick_direct(k);
                    } else if let Some(spot) = self.hover_direct {
                        self.direct_click(spot);
                    } else if let Some(i) = self.hover_space {
                        self.rail_action = None;
                        self.select_space(i);
                    } else if let Some(i) = self.hover_member {
                        self.open_member_profile(i, x, y);
                    }
                }
            }
            // Right-click (sent on release) or Shift+F10 and the menu key.
            WM_CONTEXTMENU
                if self.settings.is_none()
                    && self.viewer.is_none()
                    && self.attachment_editor.is_none()
                    && self.dialog.is_none() =>
            {
                if lp.0 & 0xffff_ffff == 0xffff_ffff {
                    if let Some((i, _)) = self.kb_control() {
                        self.open_menu(i);
                    }
                } else if self.in_history(x, y)
                    && let Some(i) = self
                        .rows
                        .row_at(self.scroll_top + (y - self.history_y()) as f64)
                    && i < self.c.messages.len()
                {
                    self.open_context_menu(i, x, y);
                } else if self.in_thread(x, y)
                    && let Some(thread::Part::Row(i)) = self.thread_hit(x, y)
                {
                    self.open_thread_context_menu(i, x, y);
                } else if let Some(i) = self.hover_space {
                    // Right-click, or press and hold on a touch screen: the
                    // space name's menu, out from the tile.
                    self.toggle_rail_menu(i);
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
                if self.playback.scrub.is_some() {
                    self.voice_message_scrub(x, true);
                }
                if let Some(part) = self.bar_pressed.take() {
                    self.dirty.composer = true;
                    self.need = true;
                    if self.bar_at(x, y) == Some(part) {
                        self.bar_activate(part);
                    }
                }
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
                // Hover state follows the pointer read above.
                self.update_hover();
                let part = self.box_part_at(x, y).is_some()
                    || self.bar_at(x, y).is_some()
                    || self.rec_at(x, y).is_some();
                let bar = self.dm_bar().is_some() || self.rec().is_some();
                let hand = part
                    || self.voice_hover.is_some()
                    || self.hover_member.is_some()
                    || self.on_call(x, y)
                    || self.voice_message_at(x, y).is_some()
                    || self.hover_channel.is_some()
                    || self.space_options_hover
                    || self
                        .hover_direct
                        .is_some_and(|s| s != directs::Spot::Heading)
                    || self.hover_space.is_some()
                    || self.hover_invite.is_some()
                    || self.pointer.is_some_and(|(x, y)| {
                        self.link_at(x, y).is_some()
                            || self.action_at(x, y).is_some()
                            || self.tool_at(x, y).is_some()
                            || self.on_head_button(x, y)
                            || self.on_gear(x, y)
                            || self.on_me(x, y)
                            || self.inbox_at(x, y).is_some()
                            || self.on_members_toggle(x, y)
                            || self.on_pins_button(x, y)
                            || self.on_update_dock(x, y)
                            || self.thread_link_at(x, y).is_some()
                            || self.foot_at(x, y)
                            || self.jump_button_at(x, y)
                            || self.quote_link_at(x, y).is_some()
                            || self.forward_at(x, y).is_some()
                            || self.edited_at(x, y).is_some()
                            || self.file_at(x, y).is_some()
                            || self.embed_at(x, y).is_some()
                            || self.spoiler_at(x, y).is_some()
                            || self.person_at(x, y).is_some()
                            || self.reply_cancel_rect().is_some_and(|(l, t, w, h)| {
                                x >= l && x < l + w && y >= t && y < t + h
                            })
                    });
                // Text takes the I-beam: the message box, and message text
                // (or any spot while a selection is being dragged).
                let beam = !hand
                    && ((self.in_box(x, y) && !bar)
                        || (self.in_history(x, y)
                            && (self.selecting || (!self.jump_at(x, y) && self.over_text(x, y)))));
                unsafe {
                    SetCursor(
                        LoadCursorW(
                            None,
                            if hand {
                                IDC_HAND
                            } else if beam {
                                IDC_IBEAM
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
                // Type-to-compose: a letter typed with nothing else focused (or
                // the history after a click) starts a message. Space still
                // belongs to the history.
                if !self.composer.focused
                    && !ctrl
                    && c > 0x20
                    && c != 0x7f
                    && self.picker.is_none()
                    && self.gifs.is_none()
                    && matches!(uia::focused(self), None | Some(uia::Id::History))
                {
                    self.set_composer_focus(true);
                }
                if self.composer.focused && !ctrl && c >= 0x20 && c != 0x7f {
                    self.composer.insert(&[c]);
                    self.composer_changed();
                }
                return Some(LRESULT(0));
            }
            WM_IME_STARTCOMPOSITION if self.focused_field().is_some() => {
                if let Some(f) = self.focused_field_mut() {
                    f.end();
                    f.composing = true;
                }
                self.field_changed(false);
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
                    f.composing = false;
                }
                self.field_changed(false);
                return Some(LRESULT(0));
            }
            WM_KILLFOCUS => {
                if let Some(f) = self.focused_field_mut() {
                    f.end();
                }
                self.caret_tick(true);
                return None;
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
        let ay = self.scroll_top + (y - self.history_y()) as f64;
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

    /// Row `i`'s failed-reaction text and its rectangle from the row's top
    /// in list coordinates, while it shows.
    pub fn row_error(&self, i: usize) -> Option<(String, Common::D2D_RECT_F)> {
        let m = self.c.messages.get(i)?;
        if self.live.is_none() || m.deleted {
            return None;
        }
        let r = self.rows.error_rect(i, m.grouped)?;
        Some((m.reaction_error.clone()?, r))
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
            self.jump_kb = false;
            self.set_composer_focus(false);
            self.set_thread_focus(false);
            self.clear_side_focus();
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
        // The toast's Jump to latest sits between the history's controls
        // and the composer in the Tab order.
        if self.jump_kb && self.jump_rect().is_some() {
            match vk {
                VK_TAB if shift => {
                    if let Some(next) = self.next_control(None, true) {
                        self.set_kb(Some(next));
                    }
                }
                VK_TAB | VK_ESCAPE => self.set_composer_focus(true),
                VK_RETURN | VK_SPACE => self.jump_activate(),
                _ => return false,
            }
            return true;
        }
        if let Some((i, action)) = self.kb_control() {
            match vk {
                VK_TAB => match self.next_control(Some((i, action)), shift) {
                    Some(next) => self.set_kb(Some(next)),
                    None if !shift && self.jump_rect().is_some() => self.set_jump_kb(true),
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
        if self.composer.focused && shift && self.jump_rect().is_some() {
            self.set_jump_kb(true);
            return true;
        }
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
        self.stop_glide();
        self.scroll_top += dy;
        self.at_end = false;
        self.need = true;
    }

    fn key(&mut self, vk: VIRTUAL_KEY, ctrl: bool, shift: bool) -> bool {
        if ctrl && vk == VIRTUAL_KEY('C' as u16) {
            let text = if self.composer.focused {
                self.composer.copied()
            } else {
                self.selected_text()
            };
            if !text.is_empty() {
                set_clipboard(self.hwnd, &text);
            }
            return true;
        }
        if let Some(side) = self.side_focus
            && matches!(
                uia::focused(self),
                Some(uia::Id::Channel(_) | uia::Id::Direct(_))
            )
            && self.side_key(side, vk, shift)
        {
            return true;
        }
        if self.history_key(vk, shift) {
            return true;
        }
        // The recording bar: Escape discards and Enter finishes.
        if (vk == VK_ESCAPE || vk == VK_RETURN) && self.rec_key(vk == VK_RETURN) {
            return true;
        }
        if !self.composer.focused {
            let page = self.vh() as f64 * 0.9;
            match vk {
                VK_PRIOR => self.scroll_by(-page),
                VK_NEXT => self.scroll_by(page),
                VK_END => {
                    self.stop_autoscroll();
                    if self.live.as_ref().is_some_and(live::Live::has_newer) {
                        self.load_latest();
                    }
                    self.at_end = true;
                    self.need = true;
                }
                // As on web, Escape closes the thread when nothing else takes it.
                VK_ESCAPE if self.thread.is_some() => self.dismiss_thread(),
                // Otherwise it marks the conversation read, as on web.
                VK_ESCAPE if !ctrl && !shift => return self.mark_open_read(),
                _ => return false,
            }
            return true;
        }
        if self.composer.composing || !self.composer.comp.is_empty() {
            // The IME owns Enter and navigation while composing.
            return false;
        }
        match vk {
            // As web's textarea: Tab leaves for the sidebar (suggestions
            // took it first when open); Shift+Tab went to the history.
            VK_TAB if !ctrl && !shift => return self.tab_to_sidebar(),
            VK_UP if !ctrl && !shift && self.edit_last(None) => return true,
            VK_RETURN if shift => self.composer.insert(&['\n' as u16]),
            VK_RETURN => {
                if let Err(e) = self.send() {
                    eprintln!("send failed: {e:?}");
                }
                return true;
            }
            // Escape marks the conversation read, as on web, and leaves the
            // message box.
            VK_ESCAPE => {
                self.mark_open_read();
                self.set_composer_focus(false);
                return true;
            }
            // Copied files or a picture attach; text pastes below.
            _ if ctrl && vk == VIRTUAL_KEY('V' as u16) && self.paste_files() => return true,
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
                    field.commit(&s);
                }
                if flags & GCS_COMPSTR.0 != 0 {
                    field.end();
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

    fn set_scale(&mut self, dpi: f32) {
        self.dpi = dpi;
        self.scale = dpi * self.zoom;
        self.dirty = Dirty::all();
        self.need = true;
    }

    /// The client area is `pw` × `ph` pixels: everything is laid out again.
    fn resized(&mut self, pw: f32, ph: f32) {
        if pw <= 0.0 || ph <= 0.0 {
            return;
        }
        let was_narrow = self.is_narrow();
        self.w = pw / self.scale;
        self.h = ph / self.scale;
        if was_narrow != self.is_narrow() {
            self.narrow = narrow::State::default();
            self.stop_autoscroll();
            self.clear_pane_focus();
            self.set_composer_focus(false);
            self.set_thread_focus(false);
            self.member_search_follow();
            self.events |= EV_SCREEN;
        }
        self.dirty.narrow = true;
        self.dirty.layout = true;
        self.dirty.signin = true;
        self.dirty.profile = true;
        self.dirty.zoom = true;
        self.close_menu();
        self.dialog_changed();
        if self.attachment_editor.is_some() {
            self.attachment_editor_changed();
        }
        self.viewer_changed();
        if self.pins.is_some() {
            self.pins_changed();
        }
        if self.settings.is_some() {
            self.settings_changed();
        }
        self.dirty.settings = true;
        self.need = true;
    }

    /// Ctrl+Plus (1), Ctrl+Minus (−1) or Ctrl+0 (0): egui's zoom, shown in
    /// a notice at the top right for two seconds when it changes.
    fn zoom_by(&mut self, steps: i32) {
        let zoom = window_state::zoomed(self.zoom, steps);
        if zoom == self.zoom {
            return;
        }
        self.zoom = zoom;
        self.set_scale(self.dpi);
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        self.resized((rc.right - rc.left) as f32, (rc.bottom - rc.top) as f32);
        self.zoom_notice = true;
        self.events |= EV_SCREEN;
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_ZOOM, 2000, None);
        }
    }

    fn draw_zoom(&mut self) -> Result<()> {
        if !std::mem::take(&mut self.dirty.zoom) {
            return Ok(());
        }
        if !self.zoom_notice {
            return self.zoom_p.offset(-100_000.0, 0.0);
        }
        let g = &self.g;
        let label = format!("Zoom {:.0}%", self.zoom * 100.0);
        let l = g.layout(&gfx::wide(&label), &g.formats.label, 1e6)?;
        let m = gfx::metrics(&l);
        let (w, h) = (
            m.widthIncludingTrailingWhitespace.ceil() + 24.0,
            m.height.ceil() + 24.0,
        );
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        self.zoom_p.ensure(g, px(w), px(h))?;
        self.zoom_p
            .offset(((self.w - 16.0 - w) * s).round(), (16.0 * s).round())?;
        self.zoom_p.draw(s, |dc| unsafe {
            dc.Clear(Some(
                &windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F::default(),
            ));
            let r = rect(0.5, 0.5, w - 0.5, h - 0.5);
            fill_round(dc, r, RADIUS, &g.brushes.surface);
            stroke_round(dc, r, RADIUS, &g.brushes.border);
            text::draw(dc, &l, 12.0, 12.0, &g.brushes.text);
            Ok(())
        })
    }

    /// Remembers the window's normal rectangle and whether it is maximized
    /// (a minimized window by how it restores), as egui does.
    pub fn save_placement(&mut self) {
        let Some(live) = &mut self.live else { return };
        let mut wp = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        if unsafe { GetWindowPlacement(self.hwnd, &mut wp) }.is_err() {
            return;
        }
        let r = wp.rcNormalPosition;
        let maximized = wp.showCmd == SW_SHOWMAXIMIZED.0 as u32
            || (wp.showCmd == SW_SHOWMINIMIZED.0 as u32
                && wp.flags.contains(WPF_RESTORETOMAXIMIZED));
        let placement = window_state::Placement {
            rect: (r.left, r.top, r.right, r.bottom),
            maximized,
        };
        live.preferences.set(window_state::KEY, placement.format());
    }

    fn apply_layout(&mut self) -> Result<()> {
        self.history_y_drawn = self.history_y();
        self.voice_error_drawn = self.voice_error_h();
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let (w, h, lx, lw) = (self.w, self.h, self.list_x(), self.list_w());
        let (sw, mw) = (self.sidebar_w(), self.member_width());
        self.relayout_thread_width()?;
        if let Some(search) = &mut self.member_search {
            search
                .field
                .relayout_with(&self.g, &self.g.formats.composer, 15.0, mw - 68.0)?;
        }
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
        self.dirty.sidebar |= self.sidebar.ensure(&self.g, px(sw), px(h))?;
        self.dirty.narrow = true;
        unsafe {
            self.navigation_layer
                .SetOffsetX2(if self.navigation_shown() {
                    0.0
                } else {
                    -100_000.0
                })?;
            self.conversation_layer.SetOffsetX2(if self.chat_shown() {
                0.0
            } else {
                -100_000.0
            })?;
            self.conversation_layer.SetClip2(&rect(
                lx * s,
                0.0,
                (lx + self.chat_width()) * s,
                h * s,
            ))?;
            self.thread_layer
                .SetOffsetX2(if self.thread_shown() { 0.0 } else { -100_000.0 })?;
        }
        self.sidebar.offset((RAIL_W * s).round(), 0.0)?;
        self.dirty.header |= self.header.ensure(&self.g, px(lw), px(HEADER_H))?;
        self.header.offset((lx * s).round(), 0.0)?;
        // A DM, a hidden list or the narrow layout's closed sheet has no
        // member list: the column frees its surface, so presence updates
        // draw nothing until it shows again.
        if self.members_w() > 0.0 {
            self.dirty.members |= self.members.ensure(&self.g, px(mw), px(self.members_h()))?;
            self.members.offset(
                (self.members_x() * s).round(),
                (self.members_y() * s).round(),
            )?;
        } else {
            self.members.hide()?;
        }
        if self.is_narrow() && self.members_w() > 0.0 {
            self.members_backdrop
                .ensure(&self.g, px(w), px(h - HEADER_H))?;
            self.members_backdrop.offset(0.0, (HEADER_H * s).round())?;
            self.members_backdrop.draw(s, |dc| {
                unsafe {
                    dc.Clear(Some(&gfx::color(0, 0.25)));
                }
                Ok(())
            })?;
        } else {
            self.members_backdrop.hide()?;
        }
        let bw = lw - 36.0;
        self.box_h = self.box_height(bw)?;
        let (bx, by, _, _) = self.box_rect();
        self.composer_p.ensure(&self.g, px(bw), px(self.box_h))?;
        if self.c.has_conversation() {
            self.composer_p.offset((bx * s).round(), (by * s).round())?;
        } else {
            self.composer_p.offset(-10_000.0, 0.0)?;
        }
        self.dirty.composer = true;
        self.relayout_reply();
        self.dirty.reply = true;
        self.layout_note(bw)?;
        match self.note_rect() {
            Some((nx, ny, nw, nh)) => {
                self.note_p.ensure(&self.g, px(nw), px(nh))?;
                self.note_p.offset((nx * s).round(), (ny * s).round())?;
            }
            None => self.note_p.offset(-10_000.0, 0.0)?,
        }
        let (tx, ty, tw, th) = self.typing_rect();
        self.typing_p.ensure(&self.g, px(tw), px(th))?;
        self.dirty.typing = true;
        if self.c.has_conversation() {
            self.typing_p.offset((tx * s).round(), (ty * s).round())?;
        } else {
            self.typing_p.offset(-10_000.0, 0.0)?;
        }
        let vh = self.vh();
        unsafe {
            self.history_clip.SetOffsetX2((lx * s).round())?;
            self.history_clip
                .SetOffsetY2((self.history_y() * s).round())?;
            self.history_clip
                .SetClip2(&rect(0.0, 0.0, (lw * s).round(), (vh * s).round()))?;
        }
        if self.lip_p.ensure(&self.g, px(lw), px(LIP_H))? {
            self.lip_p.draw(s, |dc| {
                unsafe { dc.Clear(Some(&gfx::color(0x000000, 0.0))) };
                draw_lip(dc, 0.0, 0.0, lw);
                Ok(())
            })?;
        }
        self.lip_p.offset(0.0, ((vh - LIP_H) * s).round())?;
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
        let (hover, sel) = (self.bar_row(), self.sel);
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
        let foot: Option<(f32, list::Head, bool)> = (self.foot_h() > 0.0
            && self.rows.lo < self.rows.len())
        .then(|| {
            (
                ((self.row_px(self.rows.bottom()) - y0 as f64) / s as f64) as f32,
                self.foot_head(),
                self.foot_hover,
            )
        })
        .filter(|(y, ..)| *y < (y1 - y0) as f32 / s && *y + list::HEAD_H > 0.0);
        // The jump highlight, while its fade has strength left.
        let strength = self.focus_strength();
        let focused = self
            .focused
            .as_ref()
            .and_then(|id| self.ids.rows.get(id).copied())
            .filter(|_| strength > 0.0);
        let kb = self.kb_control();
        let voice = self.voice_message_view();
        let video = self.inline_video_view();
        let video_focus = self.video_focus_view();
        let web_player = self.web_player_spot();
        let (g, c, rows) = (&self.g, &self.c, &mut self.rows);
        let mut tex = files::D2d(g.dc.clone());
        let mut media = self.live.as_mut().map(|l| files::Ctx {
            voice,
            video,
            video_focus,
            web_player,
            ..files::Ctx::new(&mut l.media, &mut tex)
        });
        let result = gfx::draw_local(&self.vs, r, s, |dc| unsafe {
            dc.Clear(Some(&gfx::color(theme::surface_conversation(), 1.0)));
            if let Some((y, state)) = head {
                list::draw_head(g, dc, state, lw, y, head_hover)?;
            }
            if let Some((y, foot, lit)) = &foot {
                list::draw_head_row(g, dc, foot, lw, *y, *lit)?;
            }
            for (i, y, h, rs) in tops {
                // Date dividers and inter-group gaps stay outside the highlight.
                let top = rows
                    .row(i)
                    .map_or(0.0, |r| list::highlight_top(r, c.messages[i].grouped));
                if focused == Some(i) {
                    g.brushes.focused.SetOpacity(strength);
                    dc.FillRectangle(&rect(0.0, y + top, lw, y + h), &g.brushes.focused);
                    g.brushes.focused.SetOpacity(1.0);
                }
                if hover == Some(i) {
                    dc.FillRectangle(&rect(0.0, y + top, lw, y + h), &g.brushes.hover);
                }
                // Web's 3px accent bar at the highlight's left edge.
                if focused == Some(i) {
                    g.brushes.terracotta.SetOpacity(strength);
                    dc.FillRectangle(&rect(0.0, y + top, 3.0, y + h), &g.brushes.terracotta);
                    g.brushes.terracotta.SetOpacity(1.0);
                }
                list::draw_row(g, dc, c, rows, i, y, rs, media.as_mut())?;
                if hover == Some(i) || focused == Some(i) {
                    list::draw_grouped_time(g, dc, c, rows, i, y)?;
                }
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
        });
        let drawn = media.map(|m| (m.next, m.animated, m.media.busy()));
        if let Some((next, animated, busy)) = drawn {
            self.media_drawn(next, animated, busy);
        }
        result
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
            // An empty conversation has no first row.
            self.rows.top.first().copied().unwrap_or(0.0) - self.head_h() as f64
        } else {
            f64::NEG_INFINITY
        };
        let max = (self.rows.bottom() + foot_h - vh).max(min);
        self.scroll_top = self.scroll_top.clamp(min, max);
        self.at_end = self.scroll_top >= max - 0.5;
        self.older_armed |= self.at_end;
        // Rows that start below the viewport; [lo, len) is measured from the bottom up.
        let fold = self.scroll_top + vh;
        let below = (self.rows.lo..self.rows.len())
            .rev()
            .take_while(|&i| self.rows.top[i] >= fold)
            .count();
        self.sync_jump(below);
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
            self.hover_row = hover;
        }
        // Scrolling carries the quote panels under a still pointer too.
        self.track_quote_hover();
        let lit = self.bar_row();
        if lit != self.lit_row {
            self.redraw.extend(self.lit_row);
            self.redraw.extend(lit);
            self.lit_row = lit;
            actions::settle_quick();
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

    /// Draws the panels the damage flags name, taking them. A failed draw
    /// (such as a lost GPU device) keeps them, so the next frame tries again.
    fn draw_panels(&mut self) -> Result<()> {
        let d = std::mem::take(&mut self.dirty);
        let drawn = self.draw_dirty_panels(d);
        if drawn.is_err() {
            // Nothing below sets flags while drawing: these are all of them.
            self.dirty = d;
        }
        drawn
    }

    fn draw_dirty_panels(&mut self, d: Dirty) -> Result<()> {
        let s = self.scale;
        let preview_thumb = self.preview_thumb(false);
        let (space_badges, channel_badges) = self.badges();
        let space_muted: Vec<bool> = (0..self.c.spaces.len())
            .map(|i| self.space_muted(i).is_some())
            .collect();
        let g = &self.g;
        let c = &self.c;
        // Invitations: the rail's tiles and Join with invite link (while
        // signed in), and the sidebar's private channel invitations.
        let (rail_invites, side_invites) = self.invite_names();
        let rail_invites = self
            .invite_spots()
            .contains(&invites::Spot::Join)
            .then_some(rail_invites);
        let (invite_hover, invites_top) = (self.hover_invite, self.invites_top());
        let create_space = self.invite_enabled(invites::Spot::Create);
        let channel_plus = self
            .invite_spots()
            .contains(&invites::Spot::NewChannel)
            .then(|| self.invite_enabled(invites::Spot::NewChannel));
        if d.rail {
            let hover = self.hover_space;
            self.rail.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(theme::blackout(), 1.0)));
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
                            rect(r.left - 3.0, r.top - 3.0, r.right + 3.0, r.bottom + 3.0),
                            15.5,
                            &g.brushes.terracotta,
                        );
                    }
                    // An uploaded icon fills the tile in place of the initial.
                    let icon = avatars::draw(dc, &space.icon, r, 13.0);
                    // A hovered space shows a quiet offset neutral ring that upgrades
                    // seamlessly to botanical green when selected.
                    let lifted = hover == Some(i) && i != c.space;
                    if lifted {
                        stroke_round(
                            dc,
                            rect(r.left - 3.0, r.top - 3.0, r.right + 3.0, r.bottom + 3.0),
                            15.5,
                            &dc.CreateSolidColorBrush(&gfx::color(theme::text(), 0.26), None)?,
                        );
                    }
                    // egui's initial: a whole grapheme, an emoji as Twemoji.
                    let initial = if icon {
                        String::new()
                    } else {
                        caper_client::space_initial(&space.name)
                    };
                    let l = g.layout(&gfx::wide(&initial), &g.formats.rail, w)?;
                    emoji::apply(&l, &initial, 13.0, 1.0)?;
                    l.SetMaxHeight(h)?;
                    // egui dims a muted space's tile and badges its corner
                    // with the bell-slash.
                    let muted = space_muted.get(i).copied().unwrap_or(false);
                    if muted {
                        if icon {
                            fill_round(
                                dc,
                                r,
                                13.0,
                                &dc.CreateSolidColorBrush(
                                    &gfx::color(theme::blackout(), 0.5),
                                    None,
                                )?,
                            );
                        }
                        text::draw(
                            dc,
                            &l,
                            r.left,
                            r.top,
                            &dc.CreateSolidColorBrush(&gfx::color(theme::text(), 0.5), None)?,
                        );
                        let b = rect(r.left - 6.0, r.top - 6.0, r.left + 12.0, r.top + 12.0);
                        fill_round(dc, b, 9.0, &g.solid(theme::blackout())?);
                        icons::draw(
                            dc,
                            icons::Icon::BellOff,
                            rect(b.left + 3.0, b.top + 3.0, b.right - 3.0, b.bottom - 3.0),
                            theme::text_muted(),
                            1.0,
                        );
                    } else {
                        text::draw(dc, &l, r.left, r.top, &g.brushes.text);
                    }
                    // Voice has its own top-right corner, leaving mute,
                    // unread and mention badges unchanged.
                    if let Some(connected) = self.space_voice(i) {
                        let b = rect(r.right - 12.0, r.top - 6.0, r.right + 6.0, r.top + 12.0);
                        fill_round(dc, b, 9.0, &g.brushes.blackout);
                        if connected {
                            fill_round(
                                dc,
                                rect(b.left + 2.0, b.top + 2.0, b.right - 2.0, b.bottom - 2.0),
                                7.0,
                                &g.solid(theme::voice_green())?,
                            );
                        }
                        icons::draw(
                            dc,
                            icons::Icon::Volume,
                            rect(b.left + 3.0, b.top + 3.0, b.right - 3.0, b.bottom - 3.0),
                            if connected { 0xFFFFFF } else { theme::text() },
                            1.0,
                        );
                    }
                    // egui's unread dot in the rail margin, and the
                    // terracotta mention count on the tile's corner.
                    let (unread, mentions) = space_badges.get(i).copied().unwrap_or_default();
                    let cy = r.top + h / 2.0;
                    // As web: a chalk pill at the rail's edge, centred on the
                    // tile: tall for the open space, short for a hovered one,
                    // in its unread dot's place. Its rounded left end sits
                    // off the rail, leaving 4 DIPs.
                    if i == c.space || lifted {
                        let half = if i == c.space { 10.0 } else { 5.0 };
                        fill_round(
                            dc,
                            rect(-2.0, cy - half, 4.0, cy + half),
                            2.0,
                            &g.brushes.text,
                        );
                    } else if unread {
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
                        fill_round(dc, b, 10.0, &g.solid(theme::blackout())?);
                        let inner = rect(b.left + 2.0, b.top + 2.0, b.right - 2.0, b.bottom - 2.0);
                        fill_round(dc, inner, 8.0, &g.brushes.terracotta);
                        n.SetMaxWidth(inner.right - inner.left)?;
                        n.SetMaxHeight(inner.bottom - inner.top)?;
                        text::draw(dc, &n, inner.left, inner.top, &g.solid(0xFFFFFF)?);
                    }
                }
                if let Some(names) = &rail_invites {
                    invites::draw_rail(g, dc, names, c.spaces.len(), invite_hover, create_space)?;
                }
                Ok(())
            })?;
        }
        if d.sidebar {
            let sw = self.sidebar_w();
            let (hover, layouts) = (self.hover_channel, &self.channel_layouts);
            let (drows, dtop, dhover) = (self.direct_rows(), self.directs_top(), self.hover_direct);
            let dmenu = self.dm_menu_row();
            // Notification menus: the space's button (and its bell while
            // muted), and per channel its button, bell, dimming and error.
            let open = self.dm_menu.as_ref().map(|m| m.scope.clone());
            let live = self.live.as_ref();
            let space_menu = self.space_scope().map(|scope| {
                let muted = live.and_then(|l| l.muted(&scope)).is_some();
                (
                    muted,
                    self.space_options_hover || open.as_ref() == Some(&scope),
                )
            });
            let channel_menus: Vec<_> = (0..c.channels.len())
                .map(|i| {
                    let scope = self.channel_scope(i);
                    let muted = scope.as_ref().and_then(|s| live?.muted(s)).is_some();
                    let error = scope.as_ref().and_then(|s| live?.notify_error(s));
                    let shown = scope.is_some() && open == scope;
                    (scope.is_some(), muted, error, shown)
                })
                .collect();
            let options_hover = self.hover_channel_options;
            // A hovered row's Join (Switch from a call elsewhere).
            let joins: Vec<Option<(bool, bool)>> = (0..c.channels.len())
                .map(|i| self.join_action(i).filter(|_| hover == Some(i)))
                .collect();
            let voice_hover = self.voice_hover;
            let side_ring = self
                .side_focus
                .filter(|_| self.side_ring && !self.is_narrow());
            self.sidebar.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(theme::surface_sidebar(), 1.0)));
                if self.panes.focus == Some(panes::Pane::Sidebar) {
                    dc.FillRectangle(&rect(sw - 2.0, 0.0, sw, self.h), &g.brushes.terracotta);
                }
                if let Some((muted, lit)) = space_menu {
                    let (x, y, w, h) = self.space_options_rect();
                    let b = rect(x - RAIL_W, y, x - RAIL_W + w, y + h);
                    if lit {
                        fill_round(dc, b, 6.0, &g.brushes.hover);
                    }
                    icons::draw(
                        dc,
                        icons::Icon::ChevronDown,
                        rect(b.left + 6.0, b.top + 6.0, b.right - 6.0, b.bottom - 6.0),
                        theme::text_muted(),
                        1.0,
                    );
                    if muted {
                        icons::draw(
                            dc,
                            icons::Icon::BellOff,
                            rect(b.left - 20.0, 20.0, b.left - 6.0, 34.0),
                            theme::text_muted(),
                            1.0,
                        );
                    }
                }
                // egui halves a muted row's colours.
                let dim_text = dc.CreateSolidColorBrush(&gfx::color(theme::text(), 0.5), None)?;
                let dim_muted =
                    dc.CreateSolidColorBrush(&gfx::color(theme::text_muted(), 0.5), None)?;
                let title = c.spaces.get(c.space).map_or("Caper", |s| s.name.as_str());
                let t = g.layout(&gfx::wide(title), &g.formats.space, 10_000.0)?;
                emoji::apply(&t, title, 17.0, 1.0)?;
                dc.PushAxisAlignedClip(
                    &rect(
                        20.0,
                        0.0,
                        (sw - if self.is_narrow() { 106.0 } else { 62.0 }).max(20.0),
                        HEADER_H,
                    ),
                    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                );
                text::draw(dc, &t, 20.0, 16.0, &g.brushes.text);
                dc.PopAxisAlignedClip();
                dc.FillRectangle(&rect(0.0, HEADER_H - 1.0, sw, HEADER_H), &g.brushes.border);
                let label = g.layout(
                    &gfx::wide(&format!("Channels  {}", c.channels.len())),
                    &g.formats.section,
                    10_000.0,
                )?;
                // On the rows' 18 DIP leading edge, as Direct messages.
                text::draw(dc, &label, 18.0, SECTION_TOP + 4.0, &g.brushes.muted);
                for (i, l) in layouts.iter().enumerate() {
                    let (_, y, w, h) = self.channel_rect(i);
                    let r = rect(12.0, y, 12.0 + w, y + h);
                    if i == c.channel {
                        fill_round(dc, r, RADIUS, &g.brushes.selected);
                    } else if hover == Some(i) {
                        fill_round(dc, r, RADIUS, &g.brushes.hover);
                    }
                    let (menu, muted, error, shown) = &channel_menus[i];
                    let cy = y + h / 2.0;
                    // From the right: the options button while revealed,
                    // the unread dot, then the bell-slash.
                    let mut right = r.right - 8.0;
                    if *menu && (hover == Some(i) || *shown) {
                        let b = rect(r.right - 30.0, cy - 12.0, r.right - 6.0, cy + 12.0);
                        if (hover == Some(i) && options_hover) || *shown {
                            fill_round(dc, b, 6.0, &g.brushes.hover);
                        }
                        icons::draw(
                            dc,
                            icons::Icon::More,
                            rect(b.left + 4.0, b.top + 4.0, b.right - 4.0, b.bottom - 4.0),
                            theme::text(),
                            1.0,
                        );
                        right = b.left - 4.0;
                    }
                    if let Some((_, enabled)) = joins[i] {
                        let b = rect(r.right - 58.0, cy - 12.0, r.right - 34.0, cy + 12.0);
                        let on = voice_hover == Some(call::VPart::Join(i));
                        if on && enabled {
                            fill_round(dc, b, 6.0, &g.brushes.hover);
                        }
                        icons::draw(
                            dc,
                            icons::Icon::Headphones,
                            rect(b.left + 4.0, b.top + 4.0, b.right - 4.0, b.bottom - 4.0),
                            if !enabled {
                                theme::border()
                            } else if on {
                                theme::voice_green()
                            } else {
                                theme::text_muted()
                            },
                            1.0,
                        );
                        right = r.right - 62.0;
                    }
                    // A channel with anything new: egui's small text-coloured dot.
                    if channel_badges.get(i).copied().unwrap_or(false) {
                        fill_round(
                            dc,
                            rect(right - 9.0, cy - 3.0, right - 3.0, cy + 3.0),
                            3.0,
                            &g.brushes.text,
                        );
                        right -= 16.0;
                    }
                    right = self.draw_voice_row(g, dc, i, right, cy)?;
                    if *muted {
                        icons::draw(
                            dc,
                            icons::Icon::BellOff,
                            rect(right - 14.0, cy - 7.0, right, cy + 7.0),
                            theme::text_muted(),
                            0.5,
                        );
                        right -= 20.0;
                    }
                    let (bright, quiet) = if *muted {
                        (&dim_text, &dim_muted)
                    } else {
                        (&g.brushes.text, &g.brushes.muted)
                    };
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
                    // 6 DIPs in from the highlight on every side.
                    text::draw(
                        dc,
                        &mark,
                        18.0,
                        y + 6.0,
                        if i == c.channel && !*muted {
                            &g.brushes.terracotta
                        } else {
                            quiet
                        },
                    );
                    dc.PushAxisAlignedClip(
                        &rect(40.0, y, right.max(40.0), y + h),
                        D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    );
                    text::draw(
                        dc,
                        l,
                        40.0,
                        y + 6.0,
                        if i == c.channel { bright } else { quiet },
                    );
                    dc.PopAxisAlignedClip();
                    // A failed notification change, after the name.
                    if let Some(e) = error {
                        let nx = 40.0 + gfx::metrics(l).widthIncludingTrailingWhitespace + 6.0;
                        if nx < right {
                            let el = g.layout(&gfx::wide(e), &g.formats.direct_note, right - nx)?;
                            el.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                            text::draw(dc, &el, nx, y + 7.0, &g.brushes.error);
                        }
                    }
                }
                invites::draw_channel_plus(g, dc, channel_plus, invite_hover, sw);
                invites::draw_sidebar(g, dc, &side_invites, invites_top, invite_hover, sw)?;
                directs::draw(g, dc, &drows, (dtop, sw), dhover, dmenu)?;
                let ring = match side_ring {
                    Some(Side::Channel(i)) if i < c.channels.len() => Some(self.channel_rect(i)),
                    Some(Side::Direct(k)) if k < drows.len() => Some(self.direct_rect(k)),
                    _ => None,
                };
                if let Some((x, y, w, h)) = ring {
                    let x = x - RAIL_W;
                    stroke_round(
                        dc,
                        rect(x + 1.0, y + 1.0, x + w - 1.0, y + h - 1.0),
                        RADIUS,
                        &g.brushes.terracotta,
                    );
                }
                if !self.inline_voice_error() {
                    self.draw_call(g, dc, RAIL_W)?;
                }
                if self.is_narrow() {
                    self.draw_update_dock(g, dc)?;
                } else {
                    self.draw_foot(g, dc, self.h)?;
                }
                Ok(())
            })?;
        }
        if d.header {
            let (name, hashed) = (c.title(), c.has_conversation() && c.direct.is_none());
            let lw = self.list_w();
            let status = self.status.as_ref().map(|(s, e)| (s.as_str(), *e));
            // The member-list toggle at the header's end, as in egui: filled
            // while the list shows or the pointer is over it.
            let lx = self.list_x();
            let toggle = self.members_toggle_rect().map(|(x, y, w, h)| {
                (
                    rect(x - lx, y, x - lx + w, y + h),
                    !self.members_hidden,
                    self.members_toggle_hover,
                )
            });
            // Pins left of the toggle, filled while open or under the pointer.
            let pins = self.pins_button_rect().map(|(x, y, w, h)| {
                (
                    rect(x - lx, y, x - lx + w, y + h),
                    self.pins.is_some() || self.pins_hover,
                )
            });
            let status_w = (lw
                - 200.0
                - if toggle.is_some() { 46.0 } else { 0.0 }
                - if pins.is_some() { 34.0 } else { 0.0 })
            .max(40.0);
            self.header.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(theme::surface_conversation(), 1.0)));
                if self.is_narrow() {
                    self.draw_narrow_header(dc)?;
                    dc.FillRectangle(&rect(0.0, HEADER_H - 1.0, lw, HEADER_H), &g.brushes.border);
                    return Ok(());
                }
                if hashed {
                    let hash = g.layout(&gfx::wide("#"), &g.formats.header, 40.0)?;
                    text::draw(dc, &hash, 24.0, 16.0, &g.brushes.muted);
                }
                let l = g.layout(&gfx::wide(&name), &g.formats.header, 10_000.0)?;
                emoji::apply(&l, &name, 16.0, 1.0)?;
                text::draw(
                    dc,
                    &l,
                    if hashed { 40.0 } else { 24.0 },
                    16.0,
                    &g.brushes.text,
                );
                if let Some((status, error)) = status {
                    let l = g.layout(&gfx::wide(status), &g.formats.time, status_w)?;
                    let brush = if error {
                        &g.brushes.terracotta
                    } else {
                        &g.brushes.muted
                    };
                    text::draw(dc, &l, 200.0, 21.0, brush);
                }
                if let Some((r, active, hover)) = toggle {
                    if active || hover {
                        fill_round(dc, r, 6.0, &g.brushes.hover);
                    }
                    let icon = rect(r.left + 5.0, r.top + 5.0, r.right - 5.0, r.bottom - 5.0);
                    let color = if active || hover {
                        theme::text()
                    } else {
                        theme::text_muted()
                    };
                    icons::draw(dc, icons::Icon::Users, icon, color, 1.0);
                }
                if let Some((r, lit)) = pins {
                    if lit {
                        fill_round(dc, r, 6.0, &g.brushes.hover);
                    }
                    let icon = rect(r.left + 5.0, r.top + 5.0, r.right - 5.0, r.bottom - 5.0);
                    let color = if lit {
                        theme::text()
                    } else {
                        theme::text_muted()
                    };
                    icons::draw(dc, icons::Icon::Pin, icon, color, 1.0);
                }
                dc.FillRectangle(&rect(0.0, HEADER_H - 1.0, lw, HEADER_H), &g.brushes.border);
                Ok(())
            })?;
        }
        if d.members {
            let mw = self.member_width();
            let (hover, layouts) = (self.hover_member, &self.member_layouts);
            let (shown, top0, searching) = (
                self.shown_members(),
                self.members_top(),
                self.member_search.is_some(),
            );
            let x0 = self.members_x();
            let this = &*self;
            self.members.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(0, 0.0)));
                fill_round(
                    dc,
                    rect(0.0, 0.0, mw, self.members_h()),
                    if self.is_narrow() { 16.0 } else { 0.0 },
                    &g.solid(theme::surface_sidebar())?,
                );
                dc.FillRectangle(&rect(0.0, 0.0, 1.0, 100_000.0), &g.brushes.border);
                if self.panes.focus == Some(panes::Pane::Members) {
                    dc.FillRectangle(&rect(0.0, 0.0, 2.0, self.h), &g.brushes.terracotta);
                }
                let t = g.layout(
                    &gfx::wide(&format!("Members  {}", c.listed_members)),
                    &g.formats.section,
                    10_000.0,
                )?;
                dc.PushAxisAlignedClip(
                    &rect(
                        20.0,
                        0.0,
                        mw - if self.members_tiled() { 52.0 } else { 88.0 },
                        HEADER_H,
                    ),
                    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                );
                text::draw(dc, &t, 20.0, 18.0, &g.brushes.text);
                dc.PopAxisAlignedClip();
                dc.FillRectangle(&rect(0.0, HEADER_H - 1.0, mw, HEADER_H), &g.brushes.border);
                member_search::draw(g, dc, this, x0)?;
                // As web: Online, then Offline under its own heading. Each
                // heading's text starts on the avatars' 16 DIP edge.
                let online = this.shown_online(&shown);
                let offline = shown.len() - online;
                let label = if !c.presence {
                    "Everyone".to_owned()
                } else if online > 0 || offline == 0 {
                    format!("Online  {online}")
                } else {
                    format!("Offline  {offline}")
                };
                let o = g.layout(&gfx::wide(&label), &g.formats.section, 10_000.0)?;
                text::draw(dc, &o, 16.0, SECTION_TOP + top0 + 4.0, &g.brushes.muted);
                if c.presence && online > 0 && offline > 0 {
                    let o = g.layout(
                        &gfx::wide(&format!("Offline  {offline}")),
                        &g.formats.section,
                        10_000.0,
                    )?;
                    let top = this.member_slot_top(online, online) - OFFLINE_H;
                    text::draw(dc, &o, 16.0, top + 8.0, &g.brushes.muted);
                }
                if searching && shown.is_empty() {
                    let l = g.layout(
                        &gfx::wide(member_search::NO_MATCH),
                        &g.formats.quote,
                        mw - 32.0,
                    )?;
                    text::draw(dc, &l, 16.0, MEMBER_TOP + top0 + 4.0, &g.brushes.muted);
                }
                for (slot, &i) in shown.iter().enumerate() {
                    let Some(l) = layouts.get(i) else { continue };
                    let top = this.member_slot_top(slot, online);
                    // 8 DIPs around the avatar on every side.
                    if hover == Some(i) {
                        fill_round(
                            dc,
                            rect(8.0, top, mw - 8.0, top + MEMBER_H),
                            RADIUS,
                            &g.brushes.hover,
                        );
                    }
                    let m = &c.members[i];
                    let av = rect(16.0, top + 8.0, 46.0, top + 38.0);
                    if !avatars::draw(dc, &m.avatar, av, 30.0 * 0.3) {
                        fill_round(dc, av, 15.0, &g.solid(m.color)?);
                    }
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
                        top + 14.0,
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
        if d.typing {
            let names = self.live.as_ref().map_or_else(Vec::new, |l| {
                l.typing.names().into_iter().map(str::to_owned).collect()
            });
            let (_, _, tw, _) = self.typing_rect();
            let lift = self.typing_animates().then(typing_phase);
            self.typing_p
                .draw(s, |dc| draw_typing(g, dc, &names, tw, lift))?;
        }
        if d.composer {
            let (_, _, bw, bh) = self.box_rect();
            let placeholder = format!("Message {}", c.place());
            let comp = &self.composer;
            let (drafts, tiles, top, notice) = self.box_files(bw);
            let open = self.gifs.as_ref().is_some_and(|p| p.root.is_none());
            let gif = self.gif_shown().then_some((bw, open));
            let emoji = self.emoji_open_for(None);
            let live = self.live.as_ref();
            let bar = self.dm_bar();
            let rec = self.rec().is_some();
            let mic = (bw, self.mic_blocked().is_none());
            let voice = self.playback.view();
            let card = self.preview_card().cloned();
            let (bar_hover, bar_pressed) = (self.bar_hover, self.bar_pressed);
            let box_hover = self.box_hover;
            if let Some((note, _)) = &self.note {
                self.note_p.draw(s, |dc| Self::draw_note(g, dc, note, bw))?;
            }
            self.composer_p.draw(s, |dc| {
                if let Some(bar) = &bar {
                    return bar.draw(g, dc, bw, bar_hover, bar_pressed);
                }
                if rec {
                    return self.draw_rec(g, dc, bw);
                }
                comp.draw(g, dc, bw, bh, &placeholder)?;
                if let Some(live) = live {
                    uploads::draw(
                        g,
                        dc,
                        &drafts,
                        &tiles,
                        top,
                        notice.as_deref().map(|n| (n, n == uploads::UPLOAD_WAIT)),
                        live.can_attach(),
                        gif,
                        emoji,
                        mic,
                        voice.as_ref(),
                        box_hover,
                    )?;
                }
                if let Some(embed) = &card {
                    preview::draw(
                        g,
                        dc,
                        embed,
                        preview_thumb.as_ref(),
                        preview::card_rect(bw, bh),
                        false,
                    )?;
                }
                Ok(())
            })?;
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
        self.draw_zoom()?;
        if self.signin.is_some() {
            self.close_drop();
            // The sign-in screen covers everything; the rest waits until it closes.
            self.draw_signin()?;
            unsafe { self.g.dcomp.Commit()? };
            self.bench.log_frame(started.elapsed());
            return Ok(());
        }
        self.dirty.layout |= self.history_y_drawn != self.history_y()
            || self.voice_error_drawn != self.voice_error_h();
        if self.dirty.layout {
            self.dirty.layout = false;
            self.apply_layout()?;
        }
        if let Some(live) = &mut self.live {
            // Inline images decode no larger than their largest frame (a
            // link preview's 400 DIPs) at this DPI and zoom.
            let dips = caper_client::embeds::MAX_WIDTH.max(caper_client::attachments::MAX_WIDTH);
            live.media
                .set_inline_edge((dips * self.scale).ceil() as u32);
        }
        self.update_history()?;
        self.inline_video_follow();
        self.tint_drafts()?;
        // Before the panels, which take the rest of the damage flags.
        self.draw_actions()?;
        self.draw_gifs()?;
        self.draw_empty()?;
        self.draw_overlays()?;
        self.draw_pins()?;
        self.draw_settings()?;
        self.draw_dm_menu()?;
        self.sync_thread_jump();
        self.draw_thread()?;
        self.track_card();
        self.draw_card()?;
        self.draw_profile()?;
        self.draw_suggest()?;
        self.draw_tip()?;
        self.dirty.narrow |= self.dirty.sidebar;
        self.draw_narrow()?;
        self.draw_panels()?;
        self.draw_drop()?;
        self.draw_autoscroll()?;
        self.draw_jump()?;
        unsafe { self.g.dcomp.Commit()? };
        self.place_web_player();
        let showing =
            self.chat_shown() && !self.narrow.menu && !(self.is_narrow() && self.members_w() > 0.0);
        if let Some(live) = &mut self.live {
            // Renews the worker's lease on the open conversation while focused.
            live.showing(
                showing
                    && unsafe {
                        GetForegroundWindow() == self.hwnd && !IsIconic(self.hwnd).as_bool()
                    },
            );
        }
        self.bench.log_frame(started.elapsed());
        Ok(())
    }
}

/// Each account's unsent drafts (`caper_client::saved_drafts`), one file per
/// account in `%APPDATA%\Caper\windows\drafts`. Tests keep them in memory.
fn drafts_dir() -> Option<std::path::PathBuf> {
    let appdata = std::env::var_os("APPDATA").filter(|_| !cfg!(test))?;
    Some(std::path::PathBuf::from(appdata).join("Caper/windows/drafts"))
}

/// How long a caret blinks after input before it rests: Windows' caret
/// timeout (5 s unless changed). `None` blinks for as long as it has focus.
fn caret_timeout() -> Option<std::time::Duration> {
    let mut ms = 0u32;
    unsafe {
        SystemParametersInfoW(
            SPI_GETCARETTIMEOUT,
            0,
            Some((&raw mut ms).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .ok()?;
    (ms > 0).then(|| std::time::Duration::from_millis(ms.into()))
}

/// The composer's lip, `LIP_H` tall from `y` and `w` wide: black at 14%
/// along its bottom edge (the composer's top), fading to transparent upward.
pub fn draw_lip(dc: &ID2D1DeviceContext, x: f32, y: f32, w: f32) {
    const STEPS: usize = 24;
    let step = LIP_H / STEPS as f32;
    unsafe {
        let Ok(brush) = dc.CreateSolidColorBrush(&gfx::color(0x000000, 1.0), None) else {
            return;
        };
        let max_opacity = if theme::is_light() { 0.06 } else { 0.14 };
        for i in 0..STEPS {
            // Strip i runs from the top; its opacity is taken at its middle.
            let opacity = max_opacity * (i as f32 + 0.5) / STEPS as f32;
            brush.SetOpacity(opacity);
            let top = y + i as f32 * step;
            dc.FillRectangle(&rect(x, top, x + w, top + step), &brush);
        }
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
            // Before the window hides, while its placement is still current.
            WM_CLOSE => {
                with_app(App::save_placement);
                // Caper keeps running in the tray unless that is turned off.
                if with_app(App::close_to_tray) == Some(true) {
                    return LRESULT(0);
                }
            }
            tray::WM_TRAY => {
                // The menu's modal loop runs outside the app borrow.
                if let Some(choice) = tray::event(hwnd, lp) {
                    with_app(|a| a.tray_choice(choice));
                }
                return LRESULT(0);
            }
            WM_ENDSESSION if wp.0 != 0 => {
                with_app(App::save_placement);
                return LRESULT(0);
            }
            WM_DESTROY => {
                with_app(|a| a.tray.hide());
                let _ = RevokeDragDrop(hwnd);
                // The player's child window goes with ours.
                with_app(|a| a.web_player = None);
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
            // Explorer restarted, taking the tray icon with it.
            m if with_app(|a| a.tray.taskbar_created == m) == Some(true) => {
                with_app(|a| a.tray.taskbar_restarted());
                return LRESULT(0);
            }
            _ => {}
        }
        let handled = with_app(|a| {
            let r = a.handle(msg, wp, lp);
            a.tip_input(
                msg,
                (lp.0 & 0xffff) as i16 as f32 / a.scale,
                ((lp.0 >> 16) & 0xffff) as i16 as f32 / a.scale,
            );
            // Anything that can edit a composer, move its caret or its
            // focus: the `:`/`@` suggestions follow.
            if matches!(
                msg,
                WM_KEYDOWN
                    | WM_CHAR
                    | WM_LBUTTONDOWN
                    | WM_LBUTTONUP
                    | WM_IME_ENDCOMPOSITION
                    | WM_SETFOCUS
                    | WM_KILLFOCUS
            ) {
                a.refresh_suggest();
            }
            // While a drag moves or resizes the window, Windows' modal loop
            // runs instead of ours: draw here, so the content follows the
            // new size and GIFs and the caret keep going.
            if a.sizing && a.need && !a.quit {
                a.need = false;
                if let Err(e) = a.frame() {
                    eprintln!("frame failed: {e:?}");
                }
            }
            r
        });
        if let Some(Some(r)) = handled {
            return r;
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

/// Opens a message link in the default browser. `links::segments` only
/// yields web addresses.
pub(crate) fn open_link(href: &str) {
    // A Caper message link goes through `caper://`, whose handler hands it
    // to this running window (`app_link`), instead of the browser.
    let app = caper_client::message_links::parse(href)
        .map(|link| format!("caper://m/{}/{}", link.channel, link.message));
    let href = app.as_deref().unwrap_or(href);
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

/// The three dots' bounce, as web's `.chat-typing-dots`: each rests, rises
/// 3 px at 30% of a 1.2 s cycle and settles by 60%, 0.15 s after the last.
fn typing_phase() -> [f32; 3] {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis()) as f64;
    std::array::from_fn(|i| {
        let phase = ((ms / 1000.0 - i as f64 * 0.15).rem_euclid(1.2) / 1.2) as f32;
        if phase < 0.6 {
            let t = if phase < 0.3 {
                phase / 0.3
            } else {
                (0.6 - phase) / 0.3
            };
            t * t * (3.0 - 2.0 * t)
        } else {
            0.0
        }
    })
}

/// Who is typing, `w` wide: web's dots, then the names in muted 11 px text
/// cut with an ellipsis. Still dots (`lift` None) rest at full strength.
fn draw_typing(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    names: &[String],
    w: f32,
    lift: Option<[f32; 3]>,
) -> Result<()> {
    unsafe {
        dc.Clear(Some(&gfx::color(0x000000, 0.0)));
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        if names.is_empty() {
            return Ok(());
        }
        let brush = g.solid(theme::text_muted())?;
        for i in 0..3 {
            let (rise, opacity) = lift.map_or((0.0, 1.0), |l| (3.0 * l[i], 0.45 + 0.55 * l[i]));
            brush.SetOpacity(opacity);
            dc.FillEllipse(
                &D2D1_ELLIPSE {
                    point: windows_numerics::Vector2 {
                        X: 2.0 + i as f32 * 7.0,
                        Y: 8.0 - rise,
                    },
                    radiusX: 2.0,
                    radiusY: 2.0,
                },
                &brush,
            );
        }
        brush.SetOpacity(1.0);
        let text = typing::label(&names);
        let x = 18.0 + 7.0;
        let l = g.layout(&gfx::wide(&text), &g.formats.file_detail, (w - x).max(1.0))?;
        crate::emoji::apply(&l, &text, 11.0, 1.0)?;
        crate::text::draw(
            dc,
            &l,
            x,
            (16.0 - gfx::metrics(&l).height) / 2.0,
            &g.brushes.muted,
        );
    }
    Ok(())
}

/// Shows the window at a remembered placement; false when no monitor
/// shows any of it now.
fn restore_placement(hwnd: HWND, p: window_state::Placement) -> bool {
    let (left, top, right, bottom) = p.rect;
    let r = RECT {
        left,
        top,
        right,
        bottom,
    };
    unsafe {
        if MonitorFromRect(&r, MONITOR_DEFAULTTONULL).0.is_null() {
            return false;
        }
        let wp = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            showCmd: if p.maximized {
                SW_SHOWMAXIMIZED
            } else {
                SW_SHOWNORMAL
            }
            .0 as u32,
            rcNormalPosition: r,
            ..Default::default()
        };
        SetWindowPlacement(hwnd, &wp).is_ok()
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
    // A caper:// link clicked while Caper is open goes to that window, as
    // in egui; this launch only carried it.
    let launch_invite = app_link::launch_code();
    let connected = live::endpoint().is_some();
    if connected && launch_invite.as_deref().is_some_and(app_link::hand_off) {
        return Ok(());
    }
    // Opening Caper again brings back the running one, which may be hidden
    // in the tray. Development and test runs open their own window.
    if launch_invite.is_none()
        && !cfg!(debug_assertions)
        && live::production()
        && app_link::hand_off(caper_client::app_link::OPEN)
    {
        return Ok(());
    }
    if connected {
        app_link::register();
    }
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // OLE's STA initialization is required by RegisterDragDrop.
        OleInitialize(None)?;
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            // build.rs embeds Caper's icon as resource 1, as egui's does.
            hIcon: LoadIconW(Some(instance.into()), PCWSTR(1 as _)).unwrap_or_default(),
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
        // Files dropped from Explorer attach to the composer.
        windows::Win32::UI::Shell::DragAcceptFiles(hwnd, true);
        let messages = std::env::var("MESSAGES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3000);
        let bench = Bench::from_env(process_start);
        let active = bench.active();
        let app = App::new(hwnd, Gfx::new()?, Conversation::demo(messages), bench)?;
        APP.with(|a| *a.borrow_mut() = Some(app));
        if let Err(e) = drop_overlay::register(hwnd) {
            // WM_DROPFILES remains available if OLE registration fails.
            eprintln!("file drag feedback unavailable: {e:?}");
        }
        with_app(|a| {
            a.live = live::start(hwnd);
            let light = a.live.as_ref().is_some_and(|live| {
                a.panes.restore(&live.preferences);
                theme::is_light_preference(live.preferences.theme_preference())
            });
            if light != theme::is_light() {
                a.apply_theme(light);
            }
            if a.live.is_some() {
                // Restoring the stored session; the sign-in form follows if there is none.
                a.signin = Some(signin::SignIn::new());
                a.signin_screen_changed();
                // Invite links and update checks wake the window as the worker does.
                let window = hwnd.0 as isize;
                let wake = move || {
                    let _ =
                        PostMessageW(Some(HWND(window as _)), live::WM_LIVE, WPARAM(0), LPARAM(0));
                };
                a.app_links = app_link::listen(wake);
                a.pending_invite = launch_invite;
                a.updates = caper_client::updates::Updates::start(wake, true);
            }
            a.sync_tray();
        });
        if active {
            SetTimer(Some(hwnd), TIMER_BENCH, 50, None);
        }
        if with_app(|a| a.live.is_some()) == Some(true) {
            SetTimer(Some(hwnd), TIMER_UNREAD, 15_000, None);
        }
        // Opens maximized for people, as egui does; development and bench
        // runs keep the 1440 by 900 window.
        let people = !cfg!(debug_assertions) && !active && live::production();
        // Where the window was last, unless that monitor is gone.
        let placement = with_app(|a| {
            a.live
                .as_ref()
                .and_then(|l| l.preferences.get(window_state::KEY))
                .and_then(window_state::Placement::parse)
        })
        .flatten();
        if !placement.is_some_and(|p| restore_placement(hwnd, p)) {
            let _ = ShowWindow(hwnd, if people { SW_SHOWMAXIMIZED } else { SW_SHOW });
        }
        let mut clock = true;
        loop {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    APP.with(|a| drop(a.borrow_mut().take()));
                    OleUninitialize();
                    return Ok(());
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let state = with_app(|a| {
                // A minimized window draws nothing: `need` waits for the
                // restore, and GIF and media timers, which re-arm only after
                // a drawn frame, stop meanwhile.
                // Hidden in the tray likewise.
                let render = !a.quit
                    && (a.need || a.bench.wants_frames())
                    && !IsIconic(a.hwnd).as_bool()
                    && IsWindowVisible(a.hwnd).as_bool();
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
                    a.web_player_to_start(),
                    a.reaction_alert.take(),
                )
            });
            let Some((rendered, quit, resize, events, player, alert)) = state else {
                continue;
            };
            if let Some(i) = alert {
                uia::raise_reaction_error(i);
            }
            if let Some(page) = player {
                // WebView2 pumps window messages while it starts, so the app
                // is not borrowed meanwhile.
                let view = web_player::create(hwnd, &page);
                with_app(|a| a.web_player_started(&page, view));
            }
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
            if events & EV_TYPING != 0 {
                uia::raise_typing_changed();
            }
            // After the frame laid out "Copied", with the app not borrowed.
            if events & EV_VERSION_COPIED != 0 {
                uia::raise_settings_live(settings::SPart::CopyVersion);
            }
            if quit {
                with_app(App::save_placement);
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
