#![allow(non_upper_case_globals)]
//! UI Automation provider: a fragment tree of the rail, channels, the realized message rows,
//! members and the composer. The composer implements Value and Text/Text2 (value, caret,
//! selection, bounding rectangles). Providers are lightweight handles (a node id); every call
//! reads the live app state on the UI thread, so nothing is mirrored per frame.
use crate::App;
use std::cell::Cell;
use std::mem::ManuallyDrop;
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
    RailItem(usize),
    Channels,
    Channel(usize),
    History,
    Row(usize),
    Members,
    Member(usize),
    Composer,
}

pub const RAIL_NAMES: [&str; 8] = [
    "Caper Crew",
    "Friends",
    "Game night",
    "Music",
    "Work",
    "Design",
    "Reading",
    "Photos",
];

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
        Id::Rail => (0..8).map(Id::RailItem).collect(),
        Id::Channels => (0..a.fx.channels.len()).map(Id::Channel).collect(),
        Id::History => a.visible_rows().map(Id::Row).collect(),
        Id::Members => (0..a.fx.authors.len()).map(Id::Member).collect(),
        _ => Vec::new(),
    }
}

fn parent(id: Id) -> Option<Id> {
    Some(match id {
        Id::Root => return None,
        Id::RailItem(_) => Id::Rail,
        Id::Channel(_) => Id::Channels,
        Id::Row(_) => Id::History,
        Id::Member(_) => Id::Members,
        _ => Id::Root,
    })
}

fn exists(a: &App, id: Id) -> bool {
    match id {
        Id::Row(i) => a.visible_rows().contains(&i),
        Id::Channel(i) => i < a.fx.channels.len(),
        Id::Member(i) => i < a.fx.authors.len(),
        Id::RailItem(i) => i < 8,
        _ => true,
    }
}

fn name(a: &App, id: Id) -> String {
    match id {
        Id::Root => "Caper (native)".into(),
        Id::Rail => "Servers".into(),
        Id::RailItem(i) => RAIL_NAMES[i].into(),
        Id::Channels => "Channels".into(),
        Id::Channel(i) => a.fx.channels[i].clone(),
        Id::History => format!("Messages in #{}", a.fx.channels[a.selected_channel]),
        Id::Row(i) => {
            let m = &a.fx.messages[i];
            let text = a.rows.rows[i]
                .as_ref()
                .map(|r| String::from_utf16_lossy(&r.plain))
                .unwrap_or_default();
            format!("{}, {}: {}", a.fx.authors[m.author].name, m.time, text)
        }
        Id::Members => format!("Members, {}", a.fx.authors.len()),
        Id::Member(i) => format!(
            "{}, {}",
            a.fx.authors[i].name,
            if i % 3 != 0 { "online" } else { "offline" }
        ),
        Id::Composer => format!("Message #{}", a.fx.channels[a.selected_channel]),
    }
}

fn control_type(id: Id) -> UIA_CONTROLTYPE_ID {
    match id {
        Id::Root => UIA_PaneControlTypeId,
        Id::Rail => UIA_ToolBarControlTypeId,
        Id::RailItem(_) => UIA_ButtonControlTypeId,
        Id::Channels | Id::History | Id::Members => UIA_ListControlTypeId,
        Id::Channel(_) | Id::Row(_) | Id::Member(_) => UIA_ListItemControlTypeId,
        Id::Composer => UIA_EditControlTypeId,
    }
}

fn runtime_id(id: Id) -> [i32; 3] {
    let (k, i) = match id {
        Id::Root => (0, 0),
        Id::Rail => (1, 0),
        Id::RailItem(i) => (2, i),
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

fn screen_origin(a: &App) -> (f64, f64) {
    let mut p = POINT::default();
    unsafe {
        let _ = ClientToScreen(a.hwnd, &mut p);
    }
    (p.x as f64, p.y as f64)
}

fn bounds(a: &App, id: Id) -> UiaRect {
    let (x, y, w, h) = a.node_rect(id);
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
            let _ = SafeArrayPutElement(sa, &(i as i32), x as *const i32 as _);
        }
        sa
    }
}

fn safe_array_f64(v: &[f64]) -> *mut SAFEARRAY {
    unsafe {
        let sa = SafeArrayCreateVector(VT_R8, 0, v.len() as u32);
        for (i, x) in v.iter().enumerate() {
            let _ = SafeArrayPutElement(sa, &(i as i32), x as *const f64 as _);
        }
        sa
    }
}

fn safe_array_unknown(v: &[IUnknown]) -> *mut SAFEARRAY {
    unsafe {
        let sa = SafeArrayCreateVector(VT_UNKNOWN, 0, v.len() as u32);
        for (i, x) in v.iter().enumerate() {
            let _ = SafeArrayPutElement(sa, &(i as i32), std::mem::transmute_copy(x));
        }
        sa
    }
}

#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot,
    IInvokeProvider,
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
            Id::RailItem(_) | Id::Channel(_) => pattern == UIA_InvokePatternId,
            Id::Composer => {
                pattern == UIA_ValuePatternId
                    || pattern == UIA_TextPatternId
                    || pattern == UIA_TextPattern2Id
            }
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
                UIA_AutomationIdPropertyId => VARIANT::from(BSTR::from(format!("{id:?}"))),
                UIA_FrameworkIdPropertyId => VARIANT::from(BSTR::from("Win32")),
                UIA_IsKeyboardFocusablePropertyId => VARIANT::from(matches!(
                    id,
                    Id::Composer | Id::RailItem(_) | Id::Channel(_)
                )),
                UIA_HasKeyboardFocusPropertyId => {
                    VARIANT::from(id == Id::Composer && a.composer.focused)
                }
                UIA_IsEnabledPropertyId => VARIANT::from(true),
                UIA_IsOffscreenPropertyId => {
                    let (_, y, _, h) = a.node_rect(id);
                    VARIANT::from(y + h < 0.0 || y > a.h)
                }
                UIA_IsPasswordPropertyId => VARIANT::from(false),
                UIA_SelectionItemIsSelectedPropertyId if matches!(id, Id::Channel(_)) => {
                    VARIANT::from(id == Id::Channel(a.selected_channel))
                }
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
                let (l, t, w, h) = a.node_rect(id);
                px >= l && px < l + w && py >= t && py < t + h
            };
            let mut best = Id::Root;
            for top in children(a, Id::Root) {
                if inside(top) {
                    best = top;
                    for c in children(a, top) {
                        if inside(c) {
                            best = c;
                            break;
                        }
                    }
                    break;
                }
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
            a.invoke(id);
            Ok(())
        })
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

fn range(start: usize, end: usize) -> ITextRangeProvider {
    Range {
        start: Cell::new(start),
        end: Cell::new(end),
    }
    .into()
}

impl ITextProvider_Impl for Node_Impl {
    fn GetSelection(&self) -> Result<*mut SAFEARRAY> {
        let (a, b) = with(|a| Ok(a.composer.selection()))?;
        let r: IUnknown = range(a, b).cast()?;
        Ok(safe_array_unknown(&[r]))
    }

    fn GetVisibleRanges(&self) -> Result<*mut SAFEARRAY> {
        let n = with(|a| Ok(a.composer.text.len()))?;
        let r: IUnknown = range(0, n).cast()?;
        Ok(safe_array_unknown(&[r]))
    }

    fn RangeFromChild(&self, _child: Ref<IRawElementProviderSimple>) -> Result<ITextRangeProvider> {
        Err(E_INVALIDARG.into())
    }

    fn RangeFromPoint(&self, point: &UiaPoint) -> Result<ITextRangeProvider> {
        let pos = with(|a| {
            let (ox, oy) = screen_origin(a);
            let (x, y, _, _) = a.composer_text_origin();
            let s = a.scale as f64;
            Ok(a.composer.hit(
                ((point.x - ox) / s) as f32 - x,
                ((point.y - oy) / s) as f32 - y - a.composer.scroll(),
            ))
        })?;
        Ok(range(pos, pos))
    }

    fn DocumentRange(&self) -> Result<ITextRangeProvider> {
        let n = with(|a| Ok(a.composer.text.len()))?;
        Ok(range(0, n))
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
        Ok(range(caret, caret))
    }
}

// ---------------------------------------------------------------------------
// Text ranges over the composer's UTF-16 text.

#[implement(ITextRangeProvider)]
pub struct Range {
    start: Cell<usize>,
    end: Cell<usize>,
}

/// Unit boundaries of the composer text, sorted, always including 0 and len.
fn boundaries(a: &App, unit: TextUnit) -> Vec<usize> {
    let t = &a.composer.text;
    let n = t.len();
    let mut b = vec![0];
    match unit {
        TextUnit_Character | TextUnit_Format => {
            let mut i = 0;
            while i < n {
                i = a.composer.next(i);
                b.push(i);
            }
        }
        TextUnit_Word => {
            let mut i = 0;
            while i < n {
                i = a.composer.word_right(i);
                b.push(i);
            }
        }
        TextUnit_Line => {
            if let Some(l) = &a.composer.layout {
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
        TextUnit_Paragraph => {
            for (i, c) in t.iter().enumerate() {
                if *c == '\n' as u16 {
                    b.push(i + 1);
                }
            }
        }
        _ => {}
    }
    b.push(n);
    b.sort_unstable();
    b.dedup();
    b
}

fn start_end(r: &Range, n: usize) -> (usize, usize) {
    let s = r.start.get().min(n);
    let e = r.end.get().min(n).max(s);
    (s, e)
}

/// Moves `pos` by `count` boundaries; returns the new position and how many it moved.
fn step(b: &[usize], pos: usize, count: i32) -> (usize, i32) {
    let mut p = pos;
    let mut moved = 0;
    if count > 0 {
        while moved < count {
            match b.iter().find(|x| **x > p) {
                Some(x) => {
                    p = *x;
                    moved += 1;
                }
                None => break,
            }
        }
    } else {
        while moved > count {
            match b.iter().rev().find(|x| **x < p) {
                Some(x) => {
                    p = *x;
                    moved -= 1;
                }
                None => break,
            }
        }
    }
    (p, moved)
}

fn other(r: Ref<ITextRangeProvider>) -> Result<(usize, usize)> {
    let r = r.ok().map_err(|_| Error::from(E_INVALIDARG))?;
    let r: &Range = unsafe { r.as_impl() };
    Ok((r.start.get(), r.end.get()))
}

impl ITextRangeProvider_Impl for Range_Impl {
    fn Clone(&self) -> Result<ITextRangeProvider> {
        Ok(range(self.start.get(), self.end.get()))
    }

    fn Compare(&self, r: Ref<ITextRangeProvider>) -> Result<BOOL> {
        Ok((other(r)? == (self.start.get(), self.end.get())).into())
    }

    fn CompareEndpoints(
        &self,
        ep: TextPatternRangeEndpoint,
        r: Ref<ITextRangeProvider>,
        tep: TextPatternRangeEndpoint,
    ) -> Result<i32> {
        let (os, oe) = other(r)?;
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
        with(|a| {
            let b = boundaries(a, unit);
            let (s, _) = start_end(self, a.composer.text.len());
            let lo = *b.iter().rev().find(|x| **x <= s).unwrap_or(&0);
            let hi = *b
                .iter()
                .find(|x| **x > lo)
                .unwrap_or(&a.composer.text.len());
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
        let found = with(|a| {
            let (s, e) = start_end(self, a.composer.text.len());
            let hay = String::from_utf16_lossy(&a.composer.text[s..e]);
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
        found.map(|(s, e)| range(s, e)).ok_or_else(Error::empty)
    }

    fn GetAttributeValue(&self, id: UIA_TEXTATTRIBUTE_ID) -> Result<VARIANT> {
        Ok(match id {
            UIA_IsReadOnlyAttributeId => VARIANT::from(false),
            UIA_FontNameAttributeId => VARIANT::from(BSTR::from("Satoshi")),
            UIA_FontSizeAttributeId => VARIANT::from(10.5f64),
            _ => VARIANT::from(unsafe { UiaGetReservedNotSupportedValue()? }),
        })
    }

    fn GetBoundingRectangles(&self) -> Result<*mut SAFEARRAY> {
        with(|a| {
            let (s, e) = start_end(self, a.composer.text.len());
            let mut out = Vec::new();
            if let Some(l) = &a.composer.layout {
                let (ox, oy) = screen_origin(a);
                let (tx, ty, _, _) = a.composer_text_origin();
                let sc = a.scale as f64;
                let mut buf = [DWRITE_HIT_TEST_METRICS::default(); 32];
                let mut n = 0u32;
                unsafe {
                    if l.HitTestTextRange(
                        s as u32,
                        (e - s).max(if e == s { 0 } else { 1 }) as u32,
                        0.0,
                        0.0,
                        Some(&mut buf),
                        &mut n,
                    )
                    .is_ok()
                    {
                        for m in &buf[..(n as usize).min(32)] {
                            out.extend_from_slice(&[
                                ox + (tx + m.left) as f64 * sc,
                                oy + (ty + m.top) as f64 * sc,
                                (m.width.max(1.0)) as f64 * sc,
                                m.height as f64 * sc,
                            ]);
                        }
                    }
                }
            }
            Ok(safe_array_f64(&out))
        })
    }

    fn GetEnclosingElement(&self) -> Result<IRawElementProviderSimple> {
        Ok(node(Id::Composer))
    }

    fn GetText(&self, max: i32) -> Result<BSTR> {
        with(|a| {
            let (s, e) = start_end(self, a.composer.text.len());
            let e = if max >= 0 { e.min(s + max as usize) } else { e };
            Ok(BSTR::from_wide(&a.composer.text[s..e]))
        })
    }

    fn Move(&self, unit: TextUnit, count: i32) -> Result<i32> {
        with(|a| {
            let b = boundaries(a, unit);
            let (s, e) = start_end(self, a.composer.text.len());
            let degenerate = s == e;
            let base = *b.iter().rev().find(|x| **x <= s).unwrap_or(&0);
            let (p, moved) = step(&b, base, count);
            self.start.set(p);
            self.end.set(if degenerate {
                p
            } else {
                *b.iter().find(|x| **x > p).unwrap_or(&a.composer.text.len())
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
        with(|a| {
            let b = boundaries(a, unit);
            let (s, e) = start_end(self, a.composer.text.len());
            let from = if ep == TextPatternRangeEndpoint_Start {
                s
            } else {
                e
            };
            let (p, moved) = step(&b, from, count);
            if ep == TextPatternRangeEndpoint_Start {
                self.start.set(p);
                if p > e {
                    self.end.set(p);
                }
            } else {
                self.end.set(p);
                if p < s {
                    self.start.set(p);
                }
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
        let (os, oe) = other(r)?;
        let p = if tep == TextPatternRangeEndpoint_Start {
            os
        } else {
            oe
        };
        if ep == TextPatternRangeEndpoint_Start {
            self.start.set(p);
            if p > self.end.get() {
                self.end.set(p);
            }
        } else {
            self.end.set(p);
            if p < self.start.get() {
                self.start.set(p);
            }
        }
        Ok(())
    }

    fn Select(&self) -> Result<()> {
        with(|a| {
            let (s, e) = start_end(self, a.composer.text.len());
            a.composer.anchor = s;
            a.composer.caret = e;
            a.composer_changed();
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

/// Raised after composer edits when a client listens (text and caret/selection changed).
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

/// The visible message rows changed (scroll, burst, resize).
pub fn raise_rows_changed() {
    unsafe {
        if UiaClientsAreListening().as_bool() {
            let n = node(Id::History);
            let _ = UiaRaiseStructureChangedEvent(
                &n,
                StructureChangeType_ChildrenInvalidated,
                runtime_id(Id::History).as_mut_ptr(),
                3,
            );
        }
    }
}

#[allow(dead_code)]
fn _unused(_: ManuallyDrop<()>) {}
