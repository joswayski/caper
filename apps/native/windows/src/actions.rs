//! The hover toolbar over a message (quick reactions, Reply, Add reaction
//! and More), the reaction picker it opens, and the reply shown above the
//! composer. Each looks and behaves like the egui client's: the same
//! controls in the same order, the same picker categories, order and search
//! (`caper_client::emoji`), and the same "Replying to" panel.
use crate::composer::{Composer, LINE};
use crate::emoji::{self, CATEGORIES, Entry};
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::theme::*;
use crate::{App, EV_FOCUS, EV_SCREEN, EV_TEXT};
use caper_client::model as core;
use std::sync::Arc;
use windows::Win32::Foundation::{LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

/// (x, y, width, height) in client DIPs.
pub type Rect = (f32, f32, f32, f32);

fn contains((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

// ---------------------------------------------------------------------------
// The hover toolbar.

/// Quick reactions at the start of the toolbar, as in the egui client.
pub const QUICK: [&str; 3] = ["\u{1f44d}", "\u{2764}\u{fe0f}", "\u{1f602}"];
const SLOT: f32 = 24.0;
pub const BAR_H: f32 = 30.0;
/// The widest toolbar: three quick reactions and four buttons.
pub const BAR_MAX_W: f32 = 7.0 * SLOT + 6.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Tool {
    Quick(usize),
    Reply,
    Emoji,
    More,
}

impl Tool {
    pub fn label(self) -> String {
        match self {
            Self::Quick(k) => format!("React with {}", QUICK[k]),
            Self::Reply => "Reply".into(),
            Self::Emoji => "Add reaction".into(),
            Self::More => "Message actions".into(),
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Quick(_) | Self::Emoji => Icon::SmilePlus,
            Self::Reply => Icon::Reply,
            Self::More => Icon::More,
        }
    }
}

/// One toolbar control: what it does, whether it can be used, and (for a
/// quick reaction) whether it is yours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ToolState {
    pub tool: Tool,
    pub enabled: bool,
    pub on: bool,
}

/// What the toolbar surface shows; it is redrawn only when this changes.
#[derive(Clone, PartialEq, Debug)]
pub struct BarKey {
    tools: Vec<ToolState>,
    hover: Option<usize>,
    open: Option<Tool>,
}

pub fn bar_width(n: usize) -> f32 {
    n as f32 * SLOT + 6.0
}

fn slot_rect(k: usize) -> windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
    let x = 3.0 + SLOT * k as f32;
    rect(x, 3.0, x + SLOT, 3.0 + SLOT)
}

fn draw_bar(g: &Gfx, dc: &ID2D1DeviceContext, key: &BarKey) {
    unsafe { dc.Clear(Some(&D2D1_COLOR_F::default())) };
    let body = rect(0.0, 0.0, bar_width(key.tools.len()), BAR_H);
    fill_round(dc, body, RADIUS, &g.brushes.surface);
    stroke_round(dc, body, RADIUS, &g.brushes.border);
    for (k, t) in key.tools.iter().enumerate() {
        let r = slot_rect(k);
        let hovered = key.hover == Some(k);
        let open = key.open == Some(t.tool);
        match t.tool {
            Tool::Quick(q) => {
                if t.on || (t.enabled && hovered) {
                    let fill = if t.on {
                        &g.brushes.pressed
                    } else {
                        &g.brushes.raised
                    };
                    fill_round(dc, r, 5.0, fill);
                    if t.on {
                        stroke_round(dc, r, 5.0, &g.brushes.terracotta);
                    }
                }
                if let Some(entry) = emoji::find(QUICK[q]) {
                    let alpha = if t.enabled { 1.0 } else { 0.5 };
                    let e = rect(r.left + 3.0, r.top + 3.0, r.right - 3.0, r.bottom - 3.0);
                    emoji::draw_with(dc, entry, e, alpha);
                }
            }
            tool => {
                if open || (t.enabled && hovered) {
                    fill_round(dc, r, 5.0, &g.brushes.raised);
                    if open {
                        stroke_round(dc, r, 5.0, &g.brushes.terracotta);
                    }
                }
                let (color, alpha) = if !t.enabled {
                    (TEXT_MUTED, 0.5)
                } else if open || hovered {
                    (TEXT, 1.0)
                } else {
                    (TEXT_MUTED, 1.0)
                };
                let i = rect(r.left + 5.0, r.top + 5.0, r.right - 5.0, r.bottom - 5.0);
                icons::draw(dc, tool.icon(), i, color, alpha);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The reaction picker.

const PICKER_W: f32 = 364.0;
const PICKER_H: f32 = 442.0;
const M: f32 = 6.0;
const INNER_W: f32 = PICKER_W - 2.0 * M;
const CAT_Y: f32 = M + 30.0 + 6.0;
const CAT_H: f32 = 40.0;
const SEARCH_Y: f32 = CAT_Y + CAT_H + 10.0;
const SEARCH_H: f32 = 26.0;
const LABEL_Y: f32 = SEARCH_Y + SEARCH_H + 6.0;
const GRID_Y: f32 = LABEL_Y + 17.0 + 6.0;
const CELL_H: f32 = 40.0;
const COLUMNS: usize = 8;
const CELL_W: f32 = INNER_W / COLUMNS as f32;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum PPart {
    Close,
    Category(usize),
    Search,
    /// An emoji of the current choices, by its index.
    Emoji(usize),
}

/// "Add a reaction" for message `id` (row `row`), anchored under its
/// toolbar's Add reaction button.
pub struct Picker {
    pub row: usize,
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub category: usize,
    pub search: Composer,
    pub scroll: f32,
    pub hover: Option<PPart>,
    pub choices: Arc<Vec<&'static Entry>>,
}

impl Picker {
    fn new(row: usize, id: String, anchor: (f32, f32), window: (f32, f32)) -> Self {
        let mut search = Composer {
            focused: true,
            caret_on: true,
            ..Default::default()
        };
        search.width = INNER_W - 12.0;
        let mut p = Self {
            row,
            id,
            // egui keeps windows inside the viewport.
            x: anchor.0.min(window.0 - PICKER_W).max(0.0),
            y: anchor.1.min(window.1 - PICKER_H).max(0.0),
            category: 0,
            search,
            scroll: 0.0,
            hover: None,
            choices: Arc::new(Vec::new()),
        };
        p.refresh();
        p
    }

    pub fn query(&self) -> String {
        String::from_utf16_lossy(&self.search.text)
            .trim()
            .to_lowercase()
    }

    /// The choices for the current category or search, in web's order.
    fn refresh(&mut self) {
        self.choices = emoji::picker_choices(CATEGORIES[self.category].0, &self.query());
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    pub fn rect(&self) -> Rect {
        (self.x, self.y, PICKER_W, PICKER_H)
    }

    fn grid_h(&self) -> f32 {
        PICKER_H - M - GRID_Y
    }

    fn max_scroll(&self) -> f32 {
        let rows = self.choices.len().div_ceil(COLUMNS);
        (rows as f32 * CELL_H - self.grid_h()).max(0.0)
    }

    /// The choices whose cells are at least partly visible.
    pub fn visible(&self) -> std::ops::Range<usize> {
        let first = (self.scroll / CELL_H).floor() as usize * COLUMNS;
        let last = (((self.scroll + self.grid_h()) / CELL_H).ceil() as usize * COLUMNS)
            .min(self.choices.len());
        first.min(last)..last
    }

    /// A part's rectangle relative to the picker's top-left.
    fn local(&self, part: PPart) -> Rect {
        match part {
            PPart::Close => (M + INNER_W - 28.0, M + 1.0, 28.0, 28.0),
            PPart::Category(k) => (M + INNER_W / 8.0 * k as f32, CAT_Y, INNER_W / 8.0, CAT_H),
            PPart::Search => (M, SEARCH_Y, INNER_W, SEARCH_H),
            PPart::Emoji(i) => (
                M + CELL_W * (i % COLUMNS) as f32,
                GRID_Y + CELL_H * (i / COLUMNS) as f32 - self.scroll,
                CELL_W,
                CELL_H,
            ),
        }
    }

    pub fn part_rect(&self, part: PPart) -> Rect {
        let (x, y, w, h) = self.local(part);
        (self.x + x, self.y + y, w, h)
    }

    pub fn parts(&self) -> Vec<PPart> {
        let mut out = vec![PPart::Close];
        out.extend((0..CATEGORIES.len()).map(PPart::Category));
        out.push(PPart::Search);
        out.extend(self.visible().map(PPart::Emoji));
        out
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<PPart> {
        let (lx, ly) = (x - self.x, y - self.y);
        if (GRID_Y..PICKER_H - M).contains(&ly) {
            return self
                .visible()
                .map(PPart::Emoji)
                .find(|p| contains(self.local(*p), lx, ly));
        }
        self.parts()
            .into_iter()
            .filter(|p| !matches!(p, PPart::Emoji(_)))
            .find(|p| contains(self.local(*p), lx, ly))
    }

    /// Client DIP origin of the search text.
    pub fn text_origin(&self) -> (f32, f32) {
        (
            self.x + M + 6.0,
            self.y + SEARCH_Y + (SEARCH_H - LINE) / 2.0,
        )
    }

    pub fn name(&self, part: PPart) -> String {
        match part {
            PPart::Close => "Close emoji picker".into(),
            PPart::Category(k) => CATEGORIES[k].1.into(),
            PPart::Search => "Search emoji".into(),
            PPart::Emoji(i) => self
                .choices
                .get(i)
                .map_or_else(String::new, |e| e.name.clone()),
        }
    }

    pub fn selected_category(&self) -> Option<usize> {
        self.search.text.is_empty().then_some(self.category)
    }

    fn draw(&self, g: &Gfx, dc: &ID2D1DeviceContext) -> Result<()> {
        unsafe {
            dc.Clear(Some(&D2D1_COLOR_F::default()));
            let body = rect(0.0, 0.0, PICKER_W, PICKER_H);
            fill_round(dc, body, RADIUS, &g.brushes.surface);
            stroke_round(dc, body, RADIUS, &g.brushes.border);
            let title = g.layout(&gfx::wide("Add a reaction"), &g.formats.section, 1e6)?;
            let th = gfx::metrics(&title).height;
            crate::text::draw(dc, &title, M, M + (30.0 - th) / 2.0, &g.brushes.text);
            let (cx, cy, cw, ch) = self.local(PPart::Close);
            let close_hover = self.hover == Some(PPart::Close);
            if close_hover {
                fill_round(dc, rect(cx, cy, cx + cw, cy + ch), 6.0, &g.brushes.hover);
            }
            icons::draw(
                dc,
                Icon::Close,
                rect(cx + 5.0, cy + 5.0, cx + cw - 5.0, cy + ch - 5.0),
                if close_hover { TEXT } else { TEXT_MUTED },
                1.0,
            );
            for (k, (_, _, icon)) in CATEGORIES.iter().enumerate() {
                let (x, y, w, h) = self.local(PPart::Category(k));
                if self.hover == Some(PPart::Category(k)) {
                    fill_round(
                        dc,
                        rect(x + 2.0, y + 2.0, x + w - 2.0, y + h - 2.0),
                        8.0,
                        &g.brushes.hover,
                    );
                }
                if let Some(entry) = emoji::find(icon) {
                    let (mx, my) = (x + w / 2.0, y + h / 2.0);
                    emoji::draw(dc, entry, rect(mx - 11.0, my - 11.0, mx + 11.0, my + 11.0));
                }
                if self.selected_category() == Some(k) {
                    dc.FillRectangle(
                        &rect(x + 2.0, y + h - 1.0, x + w - 2.0, y + h + 1.0),
                        &g.brushes.terracotta,
                    );
                }
            }
            let (sx, sy, sw, sh) = self.local(PPart::Search);
            let field = rect(sx, sy, sx + sw, sy + sh);
            fill_round(dc, field, 7.0, &g.brushes.composer);
            let stroke = if self.search.focused {
                g.brushes.terracotta.clone()
            } else if self.hover == Some(PPart::Search) {
                g.brushes.terracotta_bright.clone()
            } else {
                g.brushes.border.clone()
            };
            stroke_round(dc, field, 7.0, &stroke);
            dc.PushAxisAlignedClip(
                &rect(sx + 2.0, sy + 1.0, sx + sw - 2.0, sy + sh - 1.0),
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            let hint = g.layout(&gfx::wide("Search emoji"), &g.formats.composer, 1e6)?;
            let (ox, oy) = (sx + 6.0, sy + (sh - LINE) / 2.0);
            self.search
                .draw_text(g, dc, ox, oy, Some((&hint, &g.brushes.hint)));
            dc.PopAxisAlignedClip();
            let label = if self.search.text.is_empty() {
                CATEGORIES[self.category].1
            } else {
                "Search results"
            };
            let l = g.layout(&gfx::wide(label), &g.formats.picker, 1e6)?;
            crate::text::draw(dc, &l, M, LABEL_Y, &g.brushes.muted);
            if self.choices.is_empty() {
                let l = g.layout(
                    &gfx::wide("No results. Try another search."),
                    &g.formats.picker,
                    1e6,
                )?;
                crate::text::draw(dc, &l, M, GRID_Y, &g.brushes.muted);
            }
            dc.PushAxisAlignedClip(
                &rect(M, GRID_Y, PICKER_W - M, PICKER_H - M),
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            emoji::batch(|| {
                for i in self.visible() {
                    let (x, y, w, h) = self.local(PPart::Emoji(i));
                    if self.hover == Some(PPart::Emoji(i)) {
                        fill_round(
                            dc,
                            rect(x + 2.0, y + 2.0, x + w - 2.0, y + h - 2.0),
                            8.0,
                            &g.brushes.hover,
                        );
                    }
                    let (mx, my) = (x + w / 2.0, y + h / 2.0);
                    emoji::draw(
                        dc,
                        self.choices[i],
                        rect(mx - 15.0, my - 15.0, mx + 15.0, my + 15.0),
                    );
                }
            });
            dc.PopAxisAlignedClip();
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The reply above the composer.

const REPLY_GAP: f32 = 6.0;

/// The "Replying to" panel's layouts at its current width.
pub struct ReplyBar {
    pub target: core::MessageReply,
    name: IDWriteTextLayout,
    text: Option<IDWriteTextLayout>,
    pub height: f32,
    pub cancel_hover: bool,
}

impl ReplyBar {
    fn new(g: &Gfx, target: core::MessageReply, width: f32) -> Result<Self> {
        let inner = (width - 18.0 - 44.0 - 8.0).max(40.0);
        let name = g.layout(
            &gfx::wide(&format!("Replying to {}", target.author.name)),
            &g.formats.quote_name,
            inner,
        )?;
        let shown = target.full_text.as_deref().unwrap_or(&target.text);
        let shown = if shown.is_empty() {
            if target.has_attachments {
                core::attachments_label(&target.attachments).to_owned()
            } else {
                "Message".to_owned()
            }
        } else {
            shown.to_owned()
        };
        let text = g.layout(&gfx::wide(&shown), &g.formats.quote, inner)?;
        crate::emoji::apply(&text, &shown, 12.0, 1.0)?;
        // At most two lines, then an ellipsis.
        let mut lines = [DWRITE_LINE_METRICS::default(); 3];
        let mut count = 0;
        let text_h = unsafe {
            let _ = text.GetLineMetrics(Some(&mut lines), &mut count);
            let h: f32 = lines[..(count as usize).min(2)]
                .iter()
                .map(|l| l.height)
                .sum();
            if count > 2 {
                text.SetMaxHeight(h)?;
            }
            h
        };
        let column = gfx::metrics(&name).height + 3.0 + text_h;
        Ok(Self {
            target,
            name,
            text: Some(text),
            height: column.max(44.0) + 16.0,
            cancel_hover: false,
        })
    }

    fn draw(&self, g: &Gfx, dc: &ID2D1DeviceContext, w: f32) {
        unsafe {
            dc.Clear(Some(&D2D1_COLOR_F::default()));
            dc.FillRectangle(&rect(0.0, 0.0, 2.0, self.height), &g.brushes.terracotta);
        }
        crate::text::draw(dc, &self.name, 10.0, 8.0, &g.brushes.muted);
        if let Some(t) = &self.text {
            let y = 8.0 + gfx::metrics(&self.name).height + 3.0;
            crate::text::draw(dc, t, 10.0, y, &g.brushes.muted);
        }
        let (cx, cy) = (w - 8.0 - 44.0, 8.0);
        if self.cancel_hover {
            fill_round(
                dc,
                rect(cx, cy, cx + 44.0, cy + 44.0),
                6.0,
                &g.brushes.hover,
            );
        }
        icons::draw(
            dc,
            Icon::Close,
            rect(cx + 14.0, cy + 14.0, cx + 30.0, cy + 30.0),
            if self.cancel_hover { TEXT } else { TEXT_MUTED },
            1.0,
        );
    }
}

// ---------------------------------------------------------------------------
// The window's side.

impl App {
    /// The row the toolbar shows on: the one whose menu or picker is open,
    /// else the hovered one.
    pub fn bar_row(&self) -> Option<usize> {
        self.menu
            .as_ref()
            .map(|m| m.row)
            .or_else(|| self.picker.as_ref().map(|p| p.row))
            .or(self.hover_row)
    }

    /// Row `i`'s toolbar controls, as the egui client offers them: quick
    /// reactions, Reply and Add reaction unless the message is deleted
    /// (usable once you can react), then More.
    pub fn tools(&self, i: usize) -> Vec<ToolState> {
        let Some(m) = self.c.messages.get(i) else {
            return Vec::new();
        };
        let deleted = m.deleted;
        // The development mock offers everything, like the recreations.
        let can_react = !deleted && self.live.as_ref().is_none_or(|l| l.can_react());
        let mut out = Vec::new();
        if !deleted {
            for (k, q) in QUICK.iter().enumerate() {
                let on = m
                    .reactions
                    .iter()
                    .any(|r| r.mine && same_emoji(&r.emoji, q));
                out.push(ToolState {
                    tool: Tool::Quick(k),
                    enabled: can_react,
                    on,
                });
            }
        }
        if can_react {
            out.push(ToolState {
                tool: Tool::Reply,
                enabled: true,
                on: false,
            });
        }
        if !deleted {
            out.push(ToolState {
                tool: Tool::Emoji,
                enabled: can_react,
                on: false,
            });
        }
        out.push(ToolState {
            tool: Tool::More,
            enabled: self.live.is_some(),
            on: false,
        });
        out
    }

    /// Row `i`'s toolbar rectangle in client DIPs, where it shows on hover.
    pub fn bar_rect_for(&self, i: usize) -> Rect {
        let w = bar_width(self.tools(i).len());
        let y = ((self.rows.top[i] - self.scroll_top) as f32 + self.row_lead(i) - 12.0).max(0.0);
        (
            self.list_x() + self.list_w() - w - 16.0,
            HEADER_H + y,
            w,
            BAR_H,
        )
    }

    /// The shown toolbar's rectangle and its row.
    pub fn toolbar_rect(&self) -> Option<(usize, Rect)> {
        let i = self.bar_row()?;
        Some((i, self.bar_rect_for(i)))
    }

    /// Row `i`'s control `k` in client DIPs.
    pub fn tool_rect(&self, i: usize, k: usize) -> Rect {
        let (x, y, _, _) = self.bar_rect_for(i);
        (x + 3.0 + SLOT * k as f32, y + 3.0, SLOT, SLOT)
    }

    /// The shown toolbar's control under (x, y): its row and index.
    pub fn tool_at(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        let (i, _) = self.toolbar_rect()?;
        (0..self.tools(i).len())
            .find(|&k| contains(self.tool_rect(i, k), x, y))
            .map(|k| (i, k))
    }

    /// Tracks the hovered toolbar control; true when it changed.
    pub fn track_tool_hover(&mut self) {
        let hover = self
            .pointer
            .and_then(|(x, y)| self.tool_at(x, y))
            .map(|(_, k)| k);
        if hover != self.tool_hover {
            self.tool_hover = hover;
            self.need = true;
        }
    }

    /// Clicks and UIA Invoke on row `i`'s toolbar control.
    pub fn tool_activate(&mut self, i: usize, tool: Tool) {
        let Some(t) = self.tools(i).into_iter().find(|t| t.tool == tool) else {
            return;
        };
        if !t.enabled {
            return;
        }
        match tool {
            Tool::Quick(k) => {
                let id = self.c.messages[i].id.clone();
                if let Some(live) = &mut self.live {
                    live.react(&id, QUICK[k], !t.on);
                }
                if let Err(e) = self.drain_live() {
                    eprintln!("reaction update failed: {e:?}");
                }
            }
            Tool::Reply => self.select_reply(i),
            Tool::Emoji => self.open_picker(i),
            Tool::More => self.open_menu(i),
        }
    }

    // ---- picker ----

    /// Opens "Add a reaction" for row `i` under its toolbar's button; a
    /// second click on the same button closes it.
    pub fn open_picker(&mut self, i: usize) {
        let k = self
            .tools(i)
            .iter()
            .position(|t| t.tool == Tool::Emoji)
            .unwrap_or(0);
        let (x, y, _, h) = self.tool_rect(i, k);
        self.open_picker_at(i, (x, y + h + 4.0));
    }

    /// Opens "Add a reaction" for row `i` under the Add reaction button
    /// after its chips.
    pub fn open_add_picker(&mut self, i: usize) {
        let (_, ry, _, _) = self.row_rect(i);
        let Some((_, r)) = self
            .row_controls(i)
            .into_iter()
            .find(|(a, _)| *a == crate::list::Action::Add)
        else {
            return;
        };
        self.open_picker_at(i, (self.list_x() + r.left, ry + r.bottom + 4.0));
    }

    fn open_picker_at(&mut self, i: usize, anchor: (f32, f32)) {
        let Some(id) = self.c.messages.get(i).map(|m| m.id.clone()) else {
            return;
        };
        self.open_picker_for(i, id, anchor);
    }

    /// Opens "Add a reaction" for message `id` at `anchor`; `row` is its
    /// channel row (the thread pane passes one past the rows).
    pub fn open_picker_for(&mut self, row: usize, id: String, anchor: (f32, f32)) {
        let i = row;
        if self.picker.as_ref().is_some_and(|p| p.id == id) {
            return self.close_picker();
        }
        self.close_menu();
        self.close_gifs(false);
        self.set_composer_focus(false);
        self.picker = Some(Picker::new(i, id, anchor, (self.w, self.h)));
        self.picker_changed(true);
        self.events |= EV_SCREEN | EV_FOCUS;
    }

    pub fn close_picker(&mut self) {
        if self.picker.take().is_some() {
            self.picker_changed(false);
            self.events |= EV_SCREEN | EV_FOCUS;
        }
    }

    /// After the picker's search, category, scroll or hover changed.
    pub fn picker_changed(&mut self, text: bool) {
        let g = &self.g;
        if let Some(p) = &mut self.picker
            && text
        {
            p.search.caret_on = true;
            let w = p.search.width;
            if let Err(e) = p.search.relayout_with(g, &g.formats.composer, 15.0, w) {
                eprintln!("picker layout failed: {e:?}");
            }
            p.refresh();
            self.events |= EV_TEXT | EV_SCREEN;
        }
        self.dirty.picker = true;
        self.need = true;
        self.ime_position();
    }

    pub fn picker_activate(&mut self, part: PPart) {
        let Some(p) = &mut self.picker else { return };
        match part {
            PPart::Close => return self.close_picker(),
            PPart::Category(k) => {
                p.category = k;
                p.search.text.clear();
                p.search.caret = 0;
                p.search.anchor = 0;
                p.scroll = 0.0;
            }
            PPart::Search => {
                p.search.focused = true;
                self.events |= EV_FOCUS;
            }
            PPart::Emoji(i) => {
                let Some(entry) = p.choices.get(i) else {
                    return;
                };
                let (id, emoji) = (p.id.clone(), entry.emoji.clone());
                if let Some(live) = &mut self.live {
                    live.react(&id, &emoji, true);
                }
                self.close_picker();
                if let Err(e) = self.drain_live() {
                    eprintln!("reaction update failed: {e:?}");
                }
                return;
            }
        }
        self.picker_changed(true);
    }

    /// Input while the picker is open. None leaves it to the normal handler.
    pub fn picker_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) -> Option<Option<LRESULT>> {
        let done = Some(Some(LRESULT(0)));
        let hwnd = self.hwnd;
        // Wheel messages carry screen coordinates; the pointer is in client DIPs.
        let in_picker = self.pointer_in_picker();
        let p = self.picker.as_mut()?;
        match msg {
            WM_MOUSEMOVE => {
                let hover = p.hit(x, y);
                if hover != p.hover {
                    p.hover = hover;
                    self.dirty.picker = true;
                    self.need = true;
                }
                // The window under the picker still tracks the pointer.
                None
            }
            WM_LBUTTONDOWN => {
                if !contains(p.rect(), x, y) {
                    // An outside click closes only the picker and still
                    // reaches what it landed on, as in the egui client.
                    self.close_picker();
                    return None;
                }
                match p.hit(x, y) {
                    Some(PPart::Search) => {
                        let (ox, oy) = p.text_origin();
                        let pos = p.search.hit(x - ox, y - oy);
                        p.search.move_to(pos, shift);
                        p.search.focused = true;
                        self.picker_changed(false);
                    }
                    Some(part) => self.picker_activate(part),
                    None => {}
                }
                done
            }
            WM_MOUSEWHEEL if in_picker => {
                let delta = ((wp.0 >> 16) & 0xffff) as i16 as f32;
                p.scroll = (p.scroll - delta / 120.0 * 48.0).clamp(0.0, p.max_scroll());
                p.hover = self.pointer.and_then(|(x, y)| p.hit(x, y));
                self.picker_changed(false);
                self.events |= EV_SCREEN;
                done
            }
            WM_LBUTTONDBLCLK if contains(p.rect(), x, y) => done,
            WM_SETCURSOR => {
                let (px, py) = self.pointer.filter(|(x, y)| contains(p.rect(), *x, *y))?;
                let cursor = match p.hit(px, py) {
                    Some(PPart::Search) => IDC_IBEAM,
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
                match vk {
                    VK_ESCAPE => self.close_picker(),
                    _ if ctrl && vk == VIRTUAL_KEY('C' as u16) => {
                        let (a, b) = p.search.selection();
                        if b > a {
                            crate::set_clipboard(hwnd, &p.search.text[a..b]);
                        }
                    }
                    _ if p.search.focused && p.search.edit_key(hwnd, vk, ctrl, shift, false) => {
                        self.picker_changed(true);
                    }
                    _ => return Some(None),
                }
                done
            }
            WM_CHAR => {
                let c = wp.0 as u16;
                if p.search.focused && !ctrl && c >= 0x20 && c != 0x7f {
                    p.search.insert(&[c]);
                    self.picker_changed(true);
                }
                done
            }
            _ => None,
        }
    }

    fn pointer_in_picker(&self) -> bool {
        let (Some(p), Some((x, y))) = (&self.picker, self.pointer) else {
            return false;
        };
        contains(p.rect(), x, y)
    }

    // ---- reply ----

    /// Replies to row `i` from the composer, as the egui client's Reply does.
    pub fn select_reply(&mut self, i: usize) {
        let Some(m) = self
            .c
            .messages
            .get(i)
            .and_then(|r| self.live.as_ref()?.message(&r.id))
        else {
            return;
        };
        let (_, _, bw, _) = self.box_rect();
        match ReplyBar::new(&self.g, core::MessageReply::from_message(&m), bw) {
            Ok(bar) => self.reply = Some(bar),
            Err(e) => eprintln!("reply layout failed: {e:?}"),
        }
        self.close_menu();
        self.set_composer_focus(true);
        self.reply_changed();
    }

    pub fn cancel_reply(&mut self) {
        if self.reply.take().is_some() {
            self.reply_changed();
        }
    }

    fn reply_changed(&mut self) {
        self.dirty.layout = true;
        self.dirty.reply = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    /// The reply panel's height and the gap under it; 0 without a reply.
    pub fn reply_h(&self) -> f32 {
        self.reply.as_ref().map_or(0.0, |r| r.height + REPLY_GAP)
    }

    /// The reply panel's rectangle (client DIPs).
    pub fn reply_rect(&self) -> Option<Rect> {
        let r = self.reply.as_ref()?;
        let (bx, by, bw, _) = self.box_rect();
        Some((bx, by - REPLY_GAP - r.height, bw, r.height))
    }

    pub fn reply_cancel_rect(&self) -> Option<Rect> {
        let (x, y, w, _) = self.reply_rect()?;
        Some((x + w - 8.0 - 44.0, y + 8.0, 44.0, 44.0))
    }

    /// Clicks on the reply panel: its Cancel. True when it took the click.
    pub fn reply_click(&mut self, x: f32, y: f32) -> bool {
        if self.reply_cancel_rect().is_some_and(|r| contains(r, x, y)) {
            self.cancel_reply();
            return true;
        }
        self.reply_rect().is_some_and(|r| contains(r, x, y))
    }

    pub fn track_reply_hover(&mut self) {
        let hover = self
            .pointer
            .zip(self.reply_cancel_rect())
            .is_some_and(|((x, y), r)| contains(r, x, y));
        if let Some(r) = &mut self.reply
            && r.cancel_hover != hover
        {
            r.cancel_hover = hover;
            self.dirty.reply = true;
            self.need = true;
        }
    }

    /// After the composer's width changed: lays the reply out again.
    pub fn relayout_reply(&mut self) {
        let (_, _, bw, _) = self.box_rect();
        if let Some(r) = self.reply.take() {
            match ReplyBar::new(&self.g, r.target, bw) {
                Ok(bar) => self.reply = Some(bar),
                Err(e) => eprintln!("reply layout failed: {e:?}"),
            }
        }
    }

    // ---- drawing ----

    /// Redraws the toolbar, picker and reply surfaces when they changed.
    pub fn draw_actions(&mut self) -> Result<()> {
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let g = &self.g;
        let key = self.bar_row().map(|i| BarKey {
            tools: self.tools(i),
            hover: self.tool_hover,
            open: if self.menu.as_ref().is_some_and(|m| m.row == i) {
                Some(Tool::More)
            } else if self.picker.as_ref().is_some_and(|p| p.row == i) {
                Some(Tool::Emoji)
            } else {
                None
            },
        });
        let fresh = self.toolbar.ensure(g, px(BAR_MAX_W), px(BAR_H))?;
        if let Some(key) = &key
            && (fresh
                || std::mem::take(&mut self.dirty.toolbar)
                || self.bar_drawn.as_ref() != Some(key))
        {
            self.toolbar.draw(s, |dc| {
                draw_bar(g, dc, key);
                Ok(())
            })?;
            self.bar_drawn = Some(key.clone());
        }
        if std::mem::take(&mut self.dirty.picker) {
            match &self.picker {
                Some(p) => {
                    self.picker_p.ensure(g, px(PICKER_W), px(PICKER_H))?;
                    self.picker_p.offset((p.x * s).round(), (p.y * s).round())?;
                    self.picker_p.draw(s, |dc| p.draw(g, dc))?;
                }
                None => self.picker_p.offset(-10_000.0, 0.0)?,
            }
        }
        if std::mem::take(&mut self.dirty.reply) {
            match (&self.reply, self.reply_rect()) {
                (Some(r), Some((x, y, w, h))) => {
                    self.reply_p.ensure(g, px(w), px(h))?;
                    self.reply_p.offset((x * s).round(), (y * s).round())?;
                    self.reply_p.draw(s, |dc| {
                        r.draw(g, dc, w);
                        Ok(())
                    })?;
                }
                _ => self.reply_p.offset(-10_000.0, 0.0)?,
            }
        }
        Ok(())
    }
}

/// Whether two reaction emoji are the same, ignoring U+FE0F.
pub fn same_emoji(a: &str, b: &str) -> bool {
    a.replace('\u{fe0f}', "") == b.replace('\u{fe0f}', "")
}
