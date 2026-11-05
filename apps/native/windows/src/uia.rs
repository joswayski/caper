#![allow(non_upper_case_globals)]
//! UI Automation provider: a fragment tree of the space rail, channels, the
//! realized message rows, members and the composer. Providers are
//! lightweight handles (a node id); every call reads the live app state on
//! the UI thread, so nothing is mirrored per frame.
//!
//! The composer implements Value and Text/Text2 (value, caret, selection,
//! bounding rectangles). Each message row implements Text too, over its
//! plain text: screen readers can read it by character, word and line, find
//! its bounds and select in it (the selection is the history's selection).
use crate::{App, gfx};
use std::cell::Cell;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::*;
use windows::Win32::System::Ole::*;
use windows::Win32::System::Variant::*;
use windows::Win32::UI::Accessibility::*;
use windows::core::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Id {
    Root,
    Rail,
    Space(usize),
    Channels,
    Channel(usize),
    History,
    Row(usize),
    Members,
    Member(usize),
    Composer,
}

fn na() -> Error {
    HRESULT(UIA_E_ELEMENTNOTAVAILABLE as _).into()
}

fn with<R>(f: impl FnOnce(&mut App) -> Result<R>) -> Result<R> {
    crate::with_app(f).unwrap_or_else(|| Err(na()))
}

fn children(a: &App, id: Id) -> Vec<Id> {
    match id {
        Id::Root => vec![
            Id::Rail,
            Id::Channels,
            Id::History,
            Id::Composer,
            Id::Members,
        ],
        Id::Rail => (0..a.c.spaces.len()).map(Id::Space).collect(),
        Id::Channels => (0..a.c.channels.len()).map(Id::Channel).collect(),
        Id::History => a.visible_rows().map(Id::Row).collect(),
        Id::Members => (0..a.c.members.len()).map(Id::Member).collect(),
        _ => Vec::new(),
    }
}

fn parent(id: Id) -> Option<Id> {
    Some(match id {
        Id::Root => return None,
        Id::Space(_) => Id::Rail,
        Id::Channel(_) => Id::Channels,
        Id::Row(_) => Id::History,
        Id::Member(_) => Id::Members,
        _ => Id::Root,
    })
}

fn exists(a: &App, id: Id) -> bool {
    match id {
        Id::Row(i) => a.visible_rows().contains(&i),
        Id::Channel(i) => i < a.c.channels.len(),
        Id::Member(i) => i < a.c.members.len(),
        Id::Space(i) => i < a.c.spaces.len(),
        _ => true,
    }
}

fn name(a: &App, id: Id) -> String {
    let c = &a.c;
    match id {
        Id::Root => "Caper".into(),
        Id::Rail => "Spaces".into(),
        Id::Space(i) => c.spaces[i].name.clone(),
        Id::Channels => format!("Channels in {}", c.spaces[c.space].name),
        Id::Channel(i) => format!(
            "{}{}",
            c.channels[i].name,
            if c.channels[i].private {
                ", private"
            } else {
                ""
            }
        ),
        Id::History => format!("Messages in #{}", c.channels[c.channel].name),
        Id::Row(i) => {
            let m = &c.messages[i];
            let text = a.rows.row(i).map(|r| r.plain.clone()).unwrap_or_default();
            format!("{}, {}: {}", c.members[m.author].name, m.time, text)
        }
        Id::Members => format!("Members, {}", c.members.len()),
        Id::Member(i) => format!(
            "{}, {}",
            c.members[i].name,
            if c.members[i].online {
                "online"
            } else {
                "offline"
            }
        ),
        Id::Composer => format!("Message #{}", c.channels[c.channel].name),
    }
}

fn control_type(id: Id) -> UIA_CONTROLTYPE_ID {
    match id {
        Id::Root => UIA_PaneControlTypeId,
        Id::Rail => UIA_ListControlTypeId,
        Id::Space(_) => UIA_ListItemControlTypeId,
        Id::Channels | Id::History | Id::Members => UIA_ListControlTypeId,
        Id::Channel(_) | Id::Row(_) | Id::Member(_) => UIA_ListItemControlTypeId,
        Id::Composer => UIA_EditControlTypeId,
    }
}

fn runtime_id(id: Id) -> [i32; 3] {
    let (k, i) = match id {
        Id::Root => (0, 0),
        Id::Rail => (1, 0),
        Id::Space(i) => (2, i),
        Id::Channels => (3, 0),
        Id::Channel(i) => (4, i),
        Id::History => (5, 0),
        Id::Row(i) => (6, i),
        Id::Members => (7, 0),
        Id::Member(i) => (8, i),
        Id::Composer => (9, 0),
    };
    [UiaAppendRuntimeId as i32, k, i as i32]
}

fn node_rect(a: &App, id: Id) -> (f32, f32, f32, f32) {
    match id {
        Id::Root => (0.0, 0.0, a.w, a.h),
        Id::Rail => (0.0, 0.0, crate::theme::RAIL_W, a.h),
        Id::Space(i) => App::space_rect(i),
        Id::Channels => (crate::theme::RAIL_W, 0.0, crate::theme::SIDEBAR_W, a.h),
        Id::Channel(i) => App::channel_rect(i),
        Id::History => (a.list_x(), crate::theme::HEADER_H, a.list_w(), a.vh()),
        Id::Row(i) => a.row_rect(i),
        Id::Members => (
            a.w - crate::theme::MEMBERS_W,
            0.0,
            crate::theme::MEMBERS_W,
            a.h,
        ),
        Id::Member(i) => a.member_rect(i),
        Id::Composer => a.box_rect(),
    }
}

fn screen_origin(a: &App) -> (f64, f64) {
    let mut p = POINT::default();
    unsafe {
        let _ = ClientToScreen(a.hwnd, &mut p);
    }
    (p.x as f64, p.y as f64)
}

fn bounds(a: &App, id: Id) -> UiaRect {
    let (x, y, w, h) = node_rect(a, id);
    let (ox, oy) = screen_origin(a);
    let s = a.scale as f64;
    UiaRect {
        left: ox + x as f64 * s,
        top: oy + y as f64 * s,
        width: w as f64 * s,
        height: h as f64 * s,
    }
}

fn safe_array_i32(v: &[i32]) -> *mut SAFEARRAY {
    unsafe {
        let sa = SafeArrayCreateVector(VT_I4, 0, v.len() as u32);
        for (i, x) in v.iter().enumerate() {
            let _ = SafeArrayPutElement(sa, &(i as i32), std::ptr::from_ref(x).cast());
        }
        sa
    }
}

fn safe_array_f64(v: &[f64]) -> *mut SAFEARRAY {
    unsafe {
        let sa = SafeArrayCreateVector(VT_R8, 0, v.len() as u32);
        for (i, x) in v.iter().enumerate() {
            let _ = SafeArrayPutElement(sa, &(i as i32), std::ptr::from_ref(x).cast());
        }
        sa
    }
}

fn safe_array_unknown(v: &[IUnknown]) -> *mut SAFEARRAY {
    unsafe {
        let sa = SafeArrayCreateVector(VT_UNKNOWN, 0, v.len() as u32);
        for (i, x) in v.iter().enumerate() {
            let _ = SafeArrayPutElement(sa, &(i as i32), x.as_raw());
        }
        sa
    }
}

#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot,
    IInvokeProvider,
    ISelectionItemProvider,
    IValueProvider,
    ITextProvider,
    ITextProvider2
)]
pub struct Node {
    pub id: Id,
}

pub fn node(id: Id) -> IRawElementProviderSimple {
    Node { id }.into()
}

fn fragment(id: Id) -> IRawElementProviderFragment {
    Node { id }.into()
}

impl IRawElementProviderSimple_Impl for Node_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        let ok = match self.id {
            Id::Space(_) | Id::Channel(_) => {
                pattern == UIA_InvokePatternId || pattern == UIA_SelectionItemPatternId
            }
            Id::Composer => {
                pattern == UIA_ValuePatternId
                    || pattern == UIA_TextPatternId
                    || pattern == UIA_TextPattern2Id
            }
            Id::Row(_) => pattern == UIA_TextPatternId,
            _ => false,
        };
        if ok {
            Ok(self.to_interface::<IUnknown>())
        } else {
            Err(Error::empty())
        }
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let id = self.id;
        with(|a| {
            if !exists(a, id) {
                return Err(na());
            }
            Ok(match property {
                UIA_NamePropertyId => VARIANT::from(BSTR::from(name(a, id))),
                UIA_ControlTypePropertyId => VARIANT::from(control_type(id).0),
                UIA_AutomationIdPropertyId => VARIANT::from(BSTR::from(match id {
                    Id::Row(i) => format!("message-{}", a.c.messages[i].id),
                    _ => format!("{id:?}"),
                })),
                UIA_FrameworkIdPropertyId => VARIANT::from(BSTR::from("Win32")),
                UIA_IsKeyboardFocusablePropertyId => {
                    VARIANT::from(matches!(id, Id::Composer | Id::Space(_) | Id::Channel(_)))
                }
                UIA_HasKeyboardFocusPropertyId => {
                    VARIANT::from(id == Id::Composer && a.composer.focused)
                }
                UIA_IsEnabledPropertyId => VARIANT::from(true),
                UIA_IsOffscreenPropertyId => {
                    let (_, y, _, h) = node_rect(a, id);
                    VARIANT::from(y + h < 0.0 || y > a.h)
                }
                UIA_IsPasswordPropertyId => VARIANT::from(false),
                _ => VARIANT::default(),
            })
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        if self.id == Id::Root {
            let hwnd = with(|a| Ok(a.hwnd))?;
            unsafe { UiaHostProviderFromHwnd(hwnd) }
        } else {
            Err(Error::empty())
        }
    }
}

impl IRawElementProviderFragment_Impl for Node_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let id = self.id;
        let target = with(|a| {
            Ok(match direction {
                NavigateDirection_Parent => parent(id),
                NavigateDirection_FirstChild => children(a, id).first().copied(),
                NavigateDirection_LastChild => children(a, id).last().copied(),
                NavigateDirection_NextSibling | NavigateDirection_PreviousSibling => parent(id)
                    .and_then(|p| {
                        let sibs = children(a, p);
                        let i = sibs.iter().position(|s| *s == id)?;
                        if direction == NavigateDirection_NextSibling {
                            sibs.get(i + 1).copied()
                        } else {
                            i.checked_sub(1).map(|j| sibs[j])
                        }
                    }),
                _ => None,
            })
        })?;
        target.map(fragment).ok_or_else(Error::empty)
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        if self.id == Id::Root {
            return Ok(std::ptr::null_mut());
        }
        Ok(safe_array_i32(&runtime_id(self.id)))
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        let id = self.id;
        if id == Id::Root {
            return Ok(UiaRect::default());
        }
        with(|a| Ok(bounds(a, id)))
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        let id = self.id;
        with(|a| {
            if id == Id::Composer {
                a.set_composer_focus(true);
            }
            Ok(())
        })
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(Node { id: Id::Root }.into())
    }
}

impl IRawElementProviderFragmentRoot_Impl for Node_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let hit = with(|a| {
            let (ox, oy) = screen_origin(a);
            let (px, py) = (
                ((x - ox) / a.scale as f64) as f32,
                ((y - oy) / a.scale as f64) as f32,
            );
            let inside = |id: Id| {
                let (l, t, w, h) = node_rect(a, id);
                px >= l && px < l + w && py >= t && py < t + h
            };
            let mut best = Id::Root;
            if let Some(top) = children(a, Id::Root).into_iter().find(|t| inside(*t)) {
                best = children(a, top)
                    .into_iter()
                    .find(|c| inside(*c))
                    .unwrap_or(top);
            }
            Ok(best)
        })?;
        Ok(fragment(hit))
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        let focused = with(|a| Ok(a.composer.focused))?;
        if focused {
            Ok(fragment(Id::Composer))
        } else {
            Err(Error::empty())
        }
    }
}

impl IInvokeProvider_Impl for Node_Impl {
    fn Invoke(&self) -> Result<()> {
        let id = self.id;
        with(|a| {
            match id {
                Id::Space(i) => a.select_space(i),
                Id::Channel(i) => a.select_channel(i),
                _ => {}
            }
            Ok(())
        })
    }
}

impl ISelectionItemProvider_Impl for Node_Impl {
    fn Select(&self) -> Result<()> {
        IInvokeProvider_Impl::Invoke(self)
    }

    fn AddToSelection(&self) -> Result<()> {
        IInvokeProvider_Impl::Invoke(self)
    }

    fn RemoveFromSelection(&self) -> Result<()> {
        Err(HRESULT(UIA_E_INVALIDOPERATION as _).into())
    }

    fn IsSelected(&self) -> Result<BOOL> {
        let id = self.id;
        with(|a| {
            Ok(match id {
                Id::Space(i) => i == a.c.space,
                Id::Channel(i) => i == a.c.channel,
                _ => false,
            }
            .into())
        })
    }

    fn SelectionContainer(&self) -> Result<IRawElementProviderSimple> {
        Ok(node(parent(self.id).unwrap_or(Id::Root)))
    }
}

impl IValueProvider_Impl for Node_Impl {
    fn SetValue(&self, val: &PCWSTR) -> Result<()> {
        let text = unsafe { val.as_wide().to_vec() };
        with(|a| {
            a.composer.text = text;
            a.composer.caret = a.composer.text.len();
            a.composer.anchor = a.composer.caret;
            a.composer_changed();
            Ok(())
        })
    }

    fn Value(&self) -> Result<BSTR> {
        with(|a| Ok(BSTR::from_wide(&a.composer.text)))
    }

    fn IsReadOnly(&self) -> Result<BOOL> {
        Ok(false.into())
    }
}

// ---------------------------------------------------------------------------
// Text: one implementation over two sources, the composer and a message row.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Doc {
    Composer,
    Row(usize),
}

impl Doc {
    fn of(id: Id) -> Self {
        match id {
            Id::Row(i) => Self::Row(i),
            _ => Self::Composer,
        }
    }

    fn id(self) -> Id {
        match self {
            Self::Composer => Id::Composer,
            Self::Row(i) => Id::Row(i),
        }
    }

    fn text(self, a: &App) -> Result<Vec<u16>> {
        match self {
            Self::Composer => Ok(a.composer.text.clone()),
            Self::Row(i) if exists(a, Id::Row(i)) => {
                Ok(a.rows.row(i).map(|r| r.plain16.clone()).unwrap_or_default())
            }
            Self::Row(_) => Err(na()),
        }
    }

    fn layout(self, a: &App) -> Option<IDWriteTextLayout> {
        match self {
            Self::Composer => a.composer.layout.clone(),
            Self::Row(i) => a.rows.row(i).map(|r| r.text.clone()),
        }
    }

    /// Client DIP position of the layout origin.
    fn origin(self, a: &App) -> (f32, f32) {
        match self {
            Self::Composer => a.composer_text_origin(),
            Self::Row(i) => a.row_text_origin(i),
        }
    }

    fn selection(self, a: &App) -> (usize, usize) {
        match self {
            Self::Composer => a.composer.selection(),
            Self::Row(i) => App::sel_for(a.sel, i, a.row_len(i))
                .map_or((0, 0), |(s, e)| (s as usize, e as usize)),
        }
    }

    fn select(self, a: &mut App, s: usize, e: usize) {
        match self {
            Self::Composer => {
                a.composer.anchor = s;
                a.composer.caret = e;
                a.composer_changed();
            }
            Self::Row(i) => a.set_sel(Some(((i, s as u32), (i, e as u32)))),
        }
    }
}

fn range(doc: Doc, start: usize, end: usize) -> ITextRangeProvider {
    TextRange {
        doc,
        start: Cell::new(start),
        end: Cell::new(end),
    }
    .into()
}

impl ITextProvider_Impl for Node_Impl {
    fn GetSelection(&self) -> Result<*mut SAFEARRAY> {
        let doc = Doc::of(self.id);
        let (s, e) = with(|a| Ok(doc.selection(a)))?;
        let r: IUnknown = range(doc, s, e).cast()?;
        Ok(safe_array_unknown(&[r]))
    }

    fn GetVisibleRanges(&self) -> Result<*mut SAFEARRAY> {
        let doc = Doc::of(self.id);
        let n = with(|a| Ok(doc.text(a)?.len()))?;
        let r: IUnknown = range(doc, 0, n).cast()?;
        Ok(safe_array_unknown(&[r]))
    }

    fn RangeFromChild(&self, _child: Ref<IRawElementProviderSimple>) -> Result<ITextRangeProvider> {
        Err(E_INVALIDARG.into())
    }

    fn RangeFromPoint(&self, point: &UiaPoint) -> Result<ITextRangeProvider> {
        let doc = Doc::of(self.id);
        let pos = with(|a| {
            let (ox, oy) = screen_origin(a);
            let (x, y) = doc.origin(a);
            let s = a.scale as f64;
            let n = doc.text(a)?.len();
            let (px, py) = (
                ((point.x - ox) / s) as f32 - x,
                ((point.y - oy) / s) as f32 - y,
            );
            Ok(doc
                .layout(a)
                .map_or(0, |l| gfx::hit_point(&l, px, py) as usize)
                .min(n))
        })?;
        Ok(range(doc, pos, pos))
    }

    fn DocumentRange(&self) -> Result<ITextRangeProvider> {
        let doc = Doc::of(self.id);
        let n = with(|a| Ok(doc.text(a)?.len()))?;
        Ok(range(doc, 0, n))
    }

    fn SupportedTextSelection(&self) -> Result<SupportedTextSelection> {
        Ok(SupportedTextSelection_Single)
    }
}

impl ITextProvider2_Impl for Node_Impl {
    fn RangeFromAnnotation(
        &self,
        _a: Ref<IRawElementProviderSimple>,
    ) -> Result<ITextRangeProvider> {
        Err(E_INVALIDARG.into())
    }

    fn GetCaretRange(&self, isactive: *mut BOOL) -> Result<ITextRangeProvider> {
        let (caret, focused) = with(|a| Ok((a.composer.caret, a.composer.focused)))?;
        if !isactive.is_null() {
            unsafe { *isactive = focused.into() };
        }
        Ok(range(Doc::Composer, caret, caret))
    }
}

#[implement(ITextRangeProvider)]
struct TextRange {
    doc: Doc,
    start: Cell<usize>,
    end: Cell<usize>,
}

fn is_word(c: u16) -> bool {
    char::from_u32(c as u32).is_some_and(|c| c.is_alphanumeric() || c == '_')
        || (0xD800..0xE000).contains(&c)
}

/// Unit boundaries of a document, sorted, always including 0 and its length.
fn boundaries(t: &[u16], layout: Option<&IDWriteTextLayout>, unit: TextUnit) -> Vec<usize> {
    let n = t.len();
    let mut b = vec![0];
    match unit {
        TextUnit_Character | TextUnit_Format => {
            let mut i = 0;
            while i < n {
                i += if (0xD800..0xDC00).contains(&t[i]) && i + 1 < n {
                    2
                } else {
                    1
                };
                b.push(i);
            }
        }
        TextUnit_Word => {
            let mut i = 0;
            while i < n {
                while i < n && is_word(t[i]) {
                    i += 1;
                }
                while i < n && !is_word(t[i]) {
                    i += 1;
                }
                b.push(i);
            }
        }
        TextUnit_Line => {
            if let Some(l) = layout {
                let mut count = 0u32;
                unsafe {
                    let _ = l.GetLineMetrics(None, &mut count);
                    let mut lines = vec![DWRITE_LINE_METRICS::default(); count as usize];
                    if l.GetLineMetrics(Some(&mut lines), &mut count).is_ok() {
                        let mut p = 0usize;
                        for m in &lines {
                            p += m.length as usize;
                            b.push(p.min(n));
                        }
                    }
                }
            }
        }
        TextUnit_Paragraph => b.extend(
            t.iter()
                .enumerate()
                .filter(|(_, c)| **c == '\n' as u16)
                .map(|(i, _)| i + 1),
        ),
        _ => {}
    }
    b.push(n);
    b.sort_unstable();
    b.dedup();
    b
}

/// Moves `pos` by `count` boundaries; returns the new position and how far it moved.
fn step(b: &[usize], pos: usize, count: i32) -> (usize, i32) {
    let (mut p, mut moved) = (pos, 0);
    while moved < count {
        let Some(x) = b.iter().find(|x| **x > p) else {
            break;
        };
        p = *x;
        moved += 1;
    }
    while moved > count {
        let Some(x) = b.iter().rev().find(|x| **x < p) else {
            break;
        };
        p = *x;
        moved -= 1;
    }
    (p, moved)
}

fn other(r: Ref<ITextRangeProvider>) -> Result<(Doc, usize, usize)> {
    let r = r.ok().map_err(|_| Error::from(E_INVALIDARG))?;
    let r: &TextRange = unsafe { r.as_impl() };
    Ok((r.doc, r.start.get(), r.end.get()))
}

impl TextRange {
    /// The document's text, layout and this range clamped to it.
    fn read<R>(
        &self,
        f: impl FnOnce(&mut App, &[u16], Option<IDWriteTextLayout>, usize, usize) -> Result<R>,
    ) -> Result<R> {
        with(|a| {
            let t = self.doc.text(a)?;
            let s = self.start.get().min(t.len());
            let e = self.end.get().min(t.len()).max(s);
            let l = self.doc.layout(a);
            f(a, &t, l, s, e)
        })
    }
}

impl ITextRangeProvider_Impl for TextRange_Impl {
    fn Clone(&self) -> Result<ITextRangeProvider> {
        Ok(range(self.doc, self.start.get(), self.end.get()))
    }

    fn Compare(&self, r: Ref<ITextRangeProvider>) -> Result<BOOL> {
        Ok((other(r)? == (self.doc, self.start.get(), self.end.get())).into())
    }

    fn CompareEndpoints(
        &self,
        ep: TextPatternRangeEndpoint,
        r: Ref<ITextRangeProvider>,
        tep: TextPatternRangeEndpoint,
    ) -> Result<i32> {
        let (doc, os, oe) = other(r)?;
        if doc != self.doc {
            return Err(E_INVALIDARG.into());
        }
        let a = if ep == TextPatternRangeEndpoint_Start {
            self.start.get()
        } else {
            self.end.get()
        };
        let b = if tep == TextPatternRangeEndpoint_Start {
            os
        } else {
            oe
        };
        Ok(a as i32 - b as i32)
    }

    fn ExpandToEnclosingUnit(&self, unit: TextUnit) -> Result<()> {
        self.read(|_, t, l, s, _| {
            let b = boundaries(t, l.as_ref(), unit);
            let lo = *b.iter().rev().find(|x| **x <= s).unwrap_or(&0);
            let hi = *b.iter().find(|x| **x > lo).unwrap_or(&t.len());
            self.start.set(lo);
            self.end.set(hi);
            Ok(())
        })
    }

    fn FindAttribute(
        &self,
        _id: UIA_TEXTATTRIBUTE_ID,
        _v: &VARIANT,
        _back: BOOL,
    ) -> Result<ITextRangeProvider> {
        Err(Error::empty())
    }

    fn FindText(
        &self,
        text: &BSTR,
        backward: BOOL,
        ignorecase: BOOL,
    ) -> Result<ITextRangeProvider> {
        let found = self.read(|_, t, _, s, e| {
            let hay = String::from_utf16_lossy(&t[s..e]);
            let needle = text.to_string();
            let (hay, needle) = if ignorecase.as_bool() {
                (hay.to_lowercase(), needle.to_lowercase())
            } else {
                (hay, needle)
            };
            let at = if backward.as_bool() {
                hay.rfind(&needle)
            } else {
                hay.find(&needle)
            };
            Ok(at.map(|i| {
                let off = hay[..i].encode_utf16().count();
                (s + off, s + off + needle.encode_utf16().count())
            }))
        })?;
        found
            .map(|(s, e)| range(self.doc, s, e))
            .ok_or_else(Error::empty)
    }

    fn GetAttributeValue(&self, id: UIA_TEXTATTRIBUTE_ID) -> Result<VARIANT> {
        Ok(match id {
            UIA_IsReadOnlyAttributeId => VARIANT::from(self.doc != Doc::Composer),
            UIA_FontNameAttributeId => VARIANT::from(BSTR::from("Satoshi")),
            UIA_FontSizeAttributeId => VARIANT::from(11.25f64),
            _ => VARIANT::from(unsafe { UiaGetReservedNotSupportedValue()? }),
        })
    }

    fn GetBoundingRectangles(&self) -> Result<*mut SAFEARRAY> {
        let doc = self.doc;
        self.read(|a, _, l, s, e| {
            let mut out = Vec::new();
            if let Some(l) = l {
                let (ox, oy) = screen_origin(a);
                let (tx, ty) = doc.origin(a);
                let sc = a.scale as f64;
                for m in gfx::range_rects(&l, s as u32, (e - s) as u32) {
                    out.extend_from_slice(&[
                        ox + (tx + m.left) as f64 * sc,
                        oy + (ty + m.top) as f64 * sc,
                        m.width.max(1.0) as f64 * sc,
                        m.height as f64 * sc,
                    ]);
                }
            }
            Ok(safe_array_f64(&out))
        })
    }

    fn GetEnclosingElement(&self) -> Result<IRawElementProviderSimple> {
        Ok(node(self.doc.id()))
    }

    fn GetText(&self, max: i32) -> Result<BSTR> {
        self.read(|_, t, _, s, e| {
            let e = if max >= 0 { e.min(s + max as usize) } else { e };
            Ok(BSTR::from_wide(&t[s..e]))
        })
    }

    fn Move(&self, unit: TextUnit, count: i32) -> Result<i32> {
        self.read(|_, t, l, s, e| {
            let b = boundaries(t, l.as_ref(), unit);
            let base = *b.iter().rev().find(|x| **x <= s).unwrap_or(&0);
            let (p, moved) = step(&b, base, count);
            self.start.set(p);
            self.end.set(if s == e {
                p
            } else {
                *b.iter().find(|x| **x > p).unwrap_or(&t.len())
            });
            Ok(moved)
        })
    }

    fn MoveEndpointByUnit(
        &self,
        ep: TextPatternRangeEndpoint,
        unit: TextUnit,
        count: i32,
    ) -> Result<i32> {
        self.read(|_, t, l, s, e| {
            let b = boundaries(t, l.as_ref(), unit);
            let (p, moved) = step(
                &b,
                if ep == TextPatternRangeEndpoint_Start {
                    s
                } else {
                    e
                },
                count,
            );
            if ep == TextPatternRangeEndpoint_Start {
                self.start.set(p);
                self.end.set(e.max(p));
            } else {
                self.end.set(p);
                self.start.set(s.min(p));
            }
            Ok(moved)
        })
    }

    fn MoveEndpointByRange(
        &self,
        ep: TextPatternRangeEndpoint,
        r: Ref<ITextRangeProvider>,
        tep: TextPatternRangeEndpoint,
    ) -> Result<()> {
        let (_, os, oe) = other(r)?;
        let p = if tep == TextPatternRangeEndpoint_Start {
            os
        } else {
            oe
        };
        if ep == TextPatternRangeEndpoint_Start {
            self.start.set(p);
            self.end.set(self.end.get().max(p));
        } else {
            self.end.set(p);
            self.start.set(self.start.get().min(p));
        }
        Ok(())
    }

    fn Select(&self) -> Result<()> {
        let doc = self.doc;
        self.read(|a, _, _, s, e| {
            doc.select(a, s, e);
            Ok(())
        })
    }

    fn AddToSelection(&self) -> Result<()> {
        Err(HRESULT(UIA_E_INVALIDOPERATION as _).into())
    }

    fn RemoveFromSelection(&self) -> Result<()> {
        Err(HRESULT(UIA_E_INVALIDOPERATION as _).into())
    }

    fn ScrollIntoView(&self, _top: BOOL) -> Result<()> {
        Ok(())
    }

    fn GetChildren(&self) -> Result<*mut SAFEARRAY> {
        Ok(safe_array_unknown(&[]))
    }
}

/// Raised after composer edits while a client listens.
pub fn raise_composer_events() {
    unsafe {
        if UiaClientsAreListening().as_bool() {
            let n = node(Id::Composer);
            let _ = UiaRaiseAutomationEvent(&n, UIA_Text_TextChangedEventId);
            let _ = UiaRaiseAutomationEvent(&n, UIA_Text_TextSelectionChangedEventId);
        }
    }
}

pub fn raise_focus() {
    unsafe {
        if UiaClientsAreListening().as_bool() {
            let _ = UiaRaiseAutomationEvent(&node(Id::Composer), UIA_AutomationFocusChangedEventId);
        }
    }
}

/// The realized message rows changed (scroll, new messages, resize).
pub fn raise_rows_changed() {
    unsafe {
        if UiaClientsAreListening().as_bool() {
            let mut id = runtime_id(Id::History);
            let _ = UiaRaiseStructureChangedEvent(
                &node(Id::History),
                StructureChangeType_ChildrenInvalidated,
                id.as_mut_ptr(),
                3,
            );
        }
    }
}
