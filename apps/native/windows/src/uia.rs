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
use crate::actions::{PPart, Tool};
use crate::list::Action;
use crate::overlay::{Kind, Part as DPart};
use crate::signin::Part;
use crate::thread::Part as TPart;
use crate::{App, composer::Composer, gfx};
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
    /// A row's reaction chip or its error's Retry/Dismiss button.
    Control(usize, Action),
    Members,
    Member(usize),
    Composer,
    /// A part of the sign-in form, which replaces the tree while shown.
    Sign(Part),
    /// The open message menu and its items.
    Menu,
    MenuItem(usize),
    /// A part of the open dialog, which replaces the tree while shown.
    Dialog(DPart),
    /// A row's hover-toolbar control, reachable without hovering.
    Tool(usize, Tool),
    MenuClose,
    /// The reaction picker and its parts.
    Picker,
    Pick(PPart),
    /// "Replying to …" above the composer, and its Cancel reply.
    Reply,
    ReplyCancel,
    /// The history's head: "Load older messages" and its other states.
    Older,
    /// The history's foot after View original: Load newer messages (0)
    /// and Back to latest messages (1).
    Newer(usize),
    /// View original on row `i`'s quote.
    Original(usize),
    /// Row `i`'s "(edited)" marker, which opens its message history.
    Edited(usize),
    /// Row `i`'s attachment `k`.
    File(usize, usize),
    /// Row `i`'s link preview part `k` (a link, image or player).
    Embed(usize, usize),
    /// The composer box's Attach files and its staged files' controls.
    BoxPart(crate::uploads::Part),
    /// The thread beside the conversation, and its parts.
    Thread,
    ThreadPart(TPart),
}

fn na() -> Error {
    HRESULT(UIA_E_ELEMENTNOTAVAILABLE as _).into()
}

fn with<R>(f: impl FnOnce(&mut App) -> Result<R>) -> Result<R> {
    crate::with_app(f).unwrap_or_else(|| Err(na()))
}

/// The editable field behind a node: the sign-in field or the composer.
fn field(a: &App, id: Id) -> &Composer {
    if let (Id::Pick(_), Some(p)) = (id, &a.picker) {
        return &p.search;
    }
    if let (Id::ThreadPart(TPart::Field), Some(t)) = (id, &a.thread) {
        return &t.composer;
    }
    match (id, &a.signin, &a.dialog) {
        (Id::Dialog(_), _, Some(d)) => &d.field,
        (Id::Sign(_), Some(s), _) => s.field(),
        _ => &a.composer,
    }
}

fn field_mut(a: &mut App, id: Id) -> &mut Composer {
    if let (Id::Dialog(_), Some(d)) = (id, &mut a.dialog) {
        return &mut d.field;
    }
    if let (Id::Pick(_), Some(p)) = (id, &mut a.picker) {
        return &mut p.search;
    }
    if let (Id::ThreadPart(TPart::Field), Some(t)) = (id, &mut a.thread) {
        return &mut t.composer;
    }
    match (id, &mut a.signin) {
        (Id::Sign(_), Some(s)) => s.field_mut(),
        _ => &mut a.composer,
    }
}

fn field_changed(a: &mut App, id: Id, text: bool) {
    if matches!(id, Id::Dialog(_)) {
        a.dialog_changed();
    } else if matches!(id, Id::Pick(_)) {
        a.picker_changed(text);
    } else if matches!(id, Id::Sign(_)) {
        a.signin_changed(text);
    } else if matches!(id, Id::ThreadPart(_)) {
        a.thread_composer_changed();
    } else {
        a.composer_changed();
    }
}

/// The node of the focused text field, while one is focused.
fn focused(a: &App) -> Option<Id> {
    if let Some(d) = &a.dialog {
        return d.field.focused.then_some(Id::Dialog(DPart::Field));
    }
    if let Some(p) = &a.picker {
        return p.search.focused.then_some(Id::Pick(PPart::Search));
    }
    if a.thread.as_ref().is_some_and(|t| t.composer.focused) {
        return Some(Id::ThreadPart(TPart::Field));
    }
    if a.signin.is_none()
        && let Some((i, action)) = a.kb_control()
    {
        return Some(Id::Control(i, action));
    }
    match &a.signin {
        Some(s) => s.field().focused.then_some(Id::Sign(Part::Field)),
        None => a.composer.focused.then_some(Id::Composer),
    }
}

fn children(a: &App, id: Id) -> Vec<Id> {
    if let (Id::Root, Some(s)) = (id, &a.signin) {
        return s.parts().map(Id::Sign).collect();
    }
    if let (Id::Root, Some(d)) = (id, &a.dialog) {
        return d.parts().map(Id::Dialog).collect();
    }
    match id {
        Id::Root => {
            let mut out = vec![Id::Rail, Id::Channels, Id::History];
            out.extend(a.thread.as_ref().map(|_| Id::Thread));
            if a.reply.is_some() {
                out.extend([Id::Reply, Id::ReplyCancel]);
            }
            out.extend(a.box_parts().into_iter().map(Id::BoxPart));
            out.extend([Id::Composer, Id::Members]);
            out.extend(a.menu.as_ref().map(|_| Id::Menu));
            out.extend(a.picker.as_ref().map(|_| Id::Picker));
            out
        }
        Id::Menu => a.menu.as_ref().map_or_else(Vec::new, |m| {
            std::iter::once(Id::MenuClose)
                .chain((0..m.items.len()).map(Id::MenuItem))
                .collect()
        }),
        Id::Picker => a
            .picker
            .as_ref()
            .map_or_else(Vec::new, |p| p.parts().into_iter().map(Id::Pick).collect()),
        Id::Rail => (0..a.c.spaces.len()).map(Id::Space).collect(),
        Id::Channels => (0..a.c.channels.len()).map(Id::Channel).collect(),
        Id::History => a
            .head_rect()
            .filter(|(_, y, _, h)| *y + *h > crate::theme::HEADER_H)
            .map(|_| Id::Older)
            .into_iter()
            .chain(a.visible_rows().map(Id::Row))
            .chain((0..2).filter(|k| a.foot_rect(*k).is_some()).map(Id::Newer))
            .collect(),
        Id::Members => (0..a.c.members.len()).map(Id::Member).collect(),
        Id::Row(i) => a
            .row_controls(i)
            .into_iter()
            .map(|(action, _)| Id::Control(i, action))
            .chain(a.tools(i).into_iter().map(|t| Id::Tool(i, t.tool)))
            .chain(a.quote_link_rect(i).map(|_| Id::Original(i)))
            .chain(a.edited_rect(i).map(|_| Id::Edited(i)))
            .chain(a.file_ids(i).into_iter().map(move |k| Id::File(i, k)))
            .chain((0..a.embed_parts(i).len()).map(move |k| Id::Embed(i, k)))
            .collect(),
        Id::Thread => a
            .thread_parts()
            .into_iter()
            .filter(|p| !matches!(p, TPart::Chip(..) | TPart::Add(_)))
            .map(Id::ThreadPart)
            .collect(),
        Id::ThreadPart(TPart::Row(i)) => a
            .thread_parts()
            .into_iter()
            .filter(|p| matches!(p, TPart::Chip(r, _) | TPart::Add(r) if *r == i))
            .map(Id::ThreadPart)
            .collect(),
        _ => Vec::new(),
    }
}

fn parent(id: Id) -> Option<Id> {
    Some(match id {
        Id::Root => return None,
        Id::Space(_) => Id::Rail,
        Id::Channel(_) => Id::Channels,
        Id::Row(_) => Id::History,
        Id::Control(i, _) | Id::Tool(i, _) => Id::Row(i),
        Id::Older | Id::Newer(_) => Id::History,
        Id::Original(i) | Id::Edited(i) | Id::File(i, _) | Id::Embed(i, _) => Id::Row(i),
        Id::MenuItem(_) | Id::MenuClose => Id::Menu,
        Id::Pick(_) => Id::Picker,
        Id::Member(_) => Id::Members,
        Id::ThreadPart(TPart::Chip(i, _) | TPart::Add(i)) => Id::ThreadPart(TPart::Row(i)),
        Id::ThreadPart(_) => Id::Thread,
        _ => Id::Root,
    })
}

fn exists(a: &App, id: Id) -> bool {
    match id {
        Id::Row(i) => a.visible_rows().contains(&i),
        Id::Control(i, action) => {
            a.visible_rows().contains(&i) && a.row_controls(i).iter().any(|(c, _)| *c == action)
        }
        Id::Channel(i) => i < a.c.channels.len(),
        Id::Member(i) => i < a.c.members.len(),
        Id::Space(i) => i < a.c.spaces.len(),
        Id::Sign(p) => a.signin.as_ref().is_some_and(|s| s.part_rect(p).is_some()),
        Id::Dialog(p) => a.dialog.as_ref().is_some_and(|d| d.part_rect(p).is_some()),
        Id::Menu => a.menu.is_some() && a.dialog.is_none(),
        Id::MenuItem(k) => a.dialog.is_none() && a.menu.as_ref().is_some_and(|m| k < m.items.len()),
        Id::MenuClose => a.menu.is_some() && a.dialog.is_none(),
        Id::Tool(i, t) => {
            a.dialog.is_none()
                && a.visible_rows().contains(&i)
                && a.tools(i).iter().any(|s| s.tool == t)
        }
        Id::Picker => a.picker.is_some() && a.dialog.is_none(),
        Id::Pick(part) => {
            a.dialog.is_none() && a.picker.as_ref().is_some_and(|p| p.parts().contains(&part))
        }
        Id::Reply | Id::ReplyCancel => a.reply.is_some() && a.dialog.is_none(),
        Id::Older => a.dialog.is_none() && a.head_rect().is_some(),
        Id::Newer(k) => a.dialog.is_none() && a.foot_rect(k).is_some(),
        Id::Original(i) => a.dialog.is_none() && a.quote_link_rect(i).is_some(),
        Id::Edited(i) => a.dialog.is_none() && a.edited_rect(i).is_some(),
        Id::File(i, k) => a.dialog.is_none() && a.file_rect(i, k).is_some(),
        Id::BoxPart(p) => a.dialog.is_none() && a.signin.is_none() && a.box_parts().contains(&p),
        Id::Embed(i, k) => a.dialog.is_none() && a.embed_rect(i, k).is_some(),
        Id::Thread => a.thread.is_some() && a.dialog.is_none() && a.signin.is_none(),
        Id::ThreadPart(p) => {
            a.dialog.is_none() && a.signin.is_none() && a.thread_parts().contains(&p)
        }
        _ => (a.signin.is_none() && a.dialog.is_none()) || id == Id::Root,
    }
}

fn name(a: &App, id: Id) -> String {
    let c = &a.c;
    match id {
        Id::Root => "Caper".into(),
        Id::Rail => "Spaces".into(),
        Id::Space(i) => {
            let (unread, mentions) = a.badges().0.get(i).copied().unwrap_or_default();
            let mut n = c.spaces[i].name.clone();
            if unread {
                n.push_str(", unread messages");
            }
            if mentions > 0 {
                let unit = if mentions == 1 { "mention" } else { "mentions" };
                n.push_str(&format!(", {mentions} {unit}"));
            }
            n
        }
        Id::Channels => format!("Channels in {}", c.spaces[c.space].name),
        Id::Channel(i) => format!(
            "{}{}{}",
            c.channels[i].name,
            if c.channels[i].private {
                ", private"
            } else {
                ""
            },
            if a.badges().1.get(i).copied().unwrap_or(false) {
                ", unread activity"
            } else {
                ""
            }
        ),
        Id::History => format!("Messages in #{}", c.channels[c.channel].name),
        Id::Row(i) => row_name(c, i, a.rows.row(i).map(|r| r.plain.clone())),
        // Each chip as the egui client labels it.
        Id::Control(i, Action::Chip(k)) => chip_name(c, i, k),
        Id::Thread => a.thread_name(),
        Id::ThreadPart(TPart::Row(i)) => a.thread.as_ref().map_or_else(String::new, |t| {
            row_name(&t.c, i, t.rows.row(i).map(|r| r.plain.clone()))
        }),
        Id::ThreadPart(TPart::Chip(i, k)) => a
            .thread
            .as_ref()
            .map_or_else(String::new, |t| chip_name(&t.c, i, k)),
        Id::ThreadPart(p) => a.thread_part_name(p),
        Id::Control(_, Action::Add) => "Add reaction".into(),
        Id::Control(_, Action::Retry) => "Retry".into(),
        Id::Control(_, Action::Dismiss) => "Dismiss".into(),
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
        Id::Menu => "Message actions".into(),
        Id::MenuClose => "Close message actions".into(),
        Id::Tool(_, t) => t.label(),
        Id::Picker => "Add a reaction".into(),
        Id::Pick(part) => a.picker.as_ref().map_or_else(String::new, |p| p.name(part)),
        Id::Reply => a.reply.as_ref().map_or_else(String::new, |r| {
            format!("Replying to {}: {}", r.target.author.name, r.target.text)
        }),
        Id::ReplyCancel => "Cancel reply".into(),
        Id::Newer(k) => a.foot_head(k).label.into(),
        Id::Original(_) => "View original".into(),
        Id::Edited(_) => "Message history".into(),
        Id::File(i, k) => a.file_label(i, k),
        Id::BoxPart(p) => a.box_part_label(p),
        Id::Embed(i, k) => a.embed_label(i, k),
        Id::Older => match a.live.as_ref().map(crate::live::Live::older) {
            Some(crate::live::Older::Failed) => "Couldn\u{2019}t load older messages. Retry".into(),
            Some(state) => crate::list::head_label(state).0.into(),
            None => String::new(),
        },
        Id::MenuItem(k) => a
            .menu
            .as_ref()
            .and_then(|m| m.items.get(k))
            .map_or_else(String::new, |item| item.label().into()),
        Id::Dialog(p) => {
            let Some(d) = &a.dialog else {
                return String::new();
            };
            match p {
                DPart::Title => d.title().into(),
                DPart::Note => d.note(),
                DPart::Field => "Message".into(),
                DPart::Error => d.error.clone().unwrap_or_default(),
                DPart::Reload => "Discard draft and load latest".into(),
                DPart::Cancel => "Cancel".into(),
                DPart::Primary => d.primary_label().into(),
                DPart::Version(k) if d.version_selected(k) => {
                    format!("{}, selected", d.history_text(p))
                }
                _ => d.history_text(p),
            }
        }
        Id::Sign(p) => {
            let Some(s) = &a.signin else {
                return String::new();
            };
            match p {
                Part::Title => s.title().into(),
                Part::Field => s.field_name().into(),
                Part::Message => s.message().unwrap_or_default().into(),
                Part::Primary => s.primary_label().into(),
                Part::OtherEmail => "Use a different email".into(),
                Part::Resend => s.resend_label(),
            }
        }
    }
}

/// A message row's name: its reply, pin and thread context, author, time
/// and text, as the egui client's accessibility labels read.
fn row_name(c: &crate::model::Conversation, i: usize, text: Option<String>) -> String {
    let m = &c.messages[i];
    let text = text.unwrap_or_default();
    let mut name = String::new();
    // egui's divider is its own label; here it leads the first unread row.
    if m.unread {
        name.push_str("New messages. ");
    }
    if let Some(r) = &m.reply {
        if r.deleted {
            name.push_str("Replying to a deleted message. ");
        } else {
            name.push_str(&format!("Replying to {}: {}. ", r.author.name, r.text));
        }
    }
    if let Some(who) = &m.pinned_by {
        name.push_str(&format!("Pinned by {who}. "));
    }
    if m.thread_context {
        name.push_str("Replied to a thread. ");
    }
    name.push_str(&format!(
        "{}, {}: {}",
        c.members[m.author].name, m.time, text
    ));
    if m.edited_inline {
        name.push_str(" (edited)");
    }
    if let Some(n) = m.thread_replies {
        let s = if n == 1 { "reply" } else { "replies" };
        name.push_str(&format!(". {n} {s}, View thread"));
    }
    name
}

/// A reaction chip's name, as the egui client labels it.
fn chip_name(c: &crate::model::Conversation, i: usize, k: usize) -> String {
    c.messages[i]
        .reactions
        .get(k)
        .map_or_else(String::new, |r| {
            let n = r.count;
            let s = if n == 1 { "reaction" } else { "reactions" };
            let you = if r.mine { ", including you" } else { "" };
            let name = crate::emoji::find(&r.emoji).map_or(r.emoji.as_str(), |e| &e.name);
            format!("{name}, {n} {s}{you}")
        })
}

fn control_type(id: Id) -> UIA_CONTROLTYPE_ID {
    match id {
        Id::Root => UIA_PaneControlTypeId,
        Id::Rail => UIA_ListControlTypeId,
        Id::Space(_) => UIA_ListItemControlTypeId,
        Id::Channels | Id::History | Id::Members => UIA_ListControlTypeId,
        Id::Channel(_) | Id::Row(_) | Id::Member(_) => UIA_ListItemControlTypeId,
        Id::Composer | Id::Sign(Part::Field) => UIA_EditControlTypeId,
        Id::Sign(Part::Title | Part::Message) => UIA_TextControlTypeId,
        Id::Menu => UIA_MenuControlTypeId,
        Id::Picker => UIA_PaneControlTypeId,
        Id::Pick(PPart::Category(_)) => UIA_TabItemControlTypeId,
        Id::Pick(PPart::Search) => UIA_EditControlTypeId,
        Id::Reply => UIA_TextControlTypeId,
        Id::Older | Id::Newer(_) => UIA_ButtonControlTypeId,
        Id::Original(_) | Id::Embed(..) => UIA_HyperlinkControlTypeId,
        Id::Edited(_) | Id::File(..) | Id::BoxPart(_) => UIA_ButtonControlTypeId,
        Id::Tool(..) | Id::MenuClose | Id::Pick(_) | Id::ReplyCancel => UIA_ButtonControlTypeId,
        Id::MenuItem(_) => UIA_MenuItemControlTypeId,
        Id::Thread => UIA_PaneControlTypeId,
        Id::ThreadPart(TPart::Row(_)) => UIA_ListItemControlTypeId,
        Id::ThreadPart(TPart::Broadcast) => UIA_CheckBoxControlTypeId,
        Id::ThreadPart(TPart::Field) => UIA_EditControlTypeId,
        Id::ThreadPart(_) => UIA_ButtonControlTypeId,
        Id::Dialog(DPart::Field) => UIA_EditControlTypeId,
        Id::Dialog(
            DPart::Title
            | DPart::Note
            | DPart::Error
            | DPart::Column(_)
            | DPart::Choose
            | DPart::Hint
            | DPart::Loading,
        ) => UIA_TextControlTypeId,
        Id::Dialog(_) => UIA_ButtonControlTypeId,
        Id::Sign(_) | Id::Control(..) => UIA_ButtonControlTypeId,
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
        Id::Sign(p) => (10, p as usize),
        Id::Menu => (12, 0),
        Id::MenuItem(k) => (13, k),
        Id::Dialog(p) => (14, p.index()),
        Id::Tool(i, t) => (
            15,
            i * 8
                + match t {
                    Tool::Quick(k) => k,
                    Tool::Reply => 3,
                    Tool::Emoji => 4,
                    Tool::More => 5,
                },
        ),
        Id::MenuClose => (16, 0),
        Id::Picker => (17, 0),
        Id::Pick(part) => (
            18,
            match part {
                PPart::Close => 0,
                PPart::Search => 1,
                PPart::Category(k) => 2 + k,
                PPart::Emoji(i) => 16 + i,
            },
        ),
        Id::Reply => (19, 0),
        Id::Older => (20, 0),
        Id::Newer(k) => (26, k),
        Id::Original(i) => (27, i),
        Id::Edited(i) => (28, i),
        Id::File(i, k) => (29, i * 16 + k.min(15)),
        Id::Embed(i, k) => (31, i * 64 + k.min(63)),
        Id::BoxPart(p) => (
            30,
            match p {
                crate::uploads::Part::Attach => 0,
                crate::uploads::Part::Open(k) => 1 + 4 * k,
                crate::uploads::Part::Spoiler(k) => 2 + 4 * k,
                crate::uploads::Part::Remove(k) => 3 + 4 * k,
                crate::uploads::Part::Retry(k) => 4 + 4 * k,
            },
        ),
        Id::ReplyCancel => (19, 1),
        Id::Thread => (21, 0),
        Id::ThreadPart(p) => match p {
            TPart::Close => (22, 0),
            TPart::Older => (22, 1),
            TPart::Broadcast => (22, 2),
            TPart::Field => (22, 3),
            TPart::Send => (22, 4),
            TPart::Row(i) => (23, i),
            TPart::Chip(i, k) => (24, i * 64 + k.min(62)),
            TPart::Add(i) => (24, i * 64 + 63),
        },
        Id::Control(i, action) => (
            11,
            i * 64
                + match action {
                    Action::Retry => 0,
                    Action::Dismiss => 1,
                    Action::Add => 2,
                    Action::Chip(k) => 3 + k.min(60),
                },
        ),
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
        Id::Control(i, action) => {
            let (_, y, _, _) = a.row_rect(i);
            a.row_controls(i)
                .into_iter()
                .find(|(c, _)| *c == action)
                .map_or((0.0, 0.0, 0.0, 0.0), |(_, r)| {
                    (
                        a.list_x() + r.left,
                        y + r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                    )
                })
        }
        Id::Members => (
            a.w - crate::theme::MEMBERS_W,
            0.0,
            crate::theme::MEMBERS_W,
            a.h,
        ),
        Id::Member(i) => a.member_rect(i),
        Id::Composer => a.box_rect(),
        Id::Menu => a
            .menu
            .as_ref()
            .map_or((0.0, 0.0, 0.0, 0.0), crate::overlay::Menu::rect),
        Id::MenuItem(k) => a
            .menu
            .as_ref()
            .map_or((0.0, 0.0, 0.0, 0.0), |m| m.item_rect(k)),
        Id::Dialog(p) => a
            .dialog
            .as_ref()
            .and_then(|d| d.part_rect(p))
            .unwrap_or((0.0, 0.0, 0.0, 0.0)),
        Id::Tool(i, t) => a
            .tools(i)
            .iter()
            .position(|s| s.tool == t)
            .map_or((0.0, 0.0, 0.0, 0.0), |k| a.tool_rect(i, k)),
        Id::MenuClose => a
            .menu
            .as_ref()
            .map_or((0.0, 0.0, 0.0, 0.0), crate::overlay::Menu::close_rect),
        Id::Picker => a
            .picker
            .as_ref()
            .map_or((0.0, 0.0, 0.0, 0.0), crate::actions::Picker::rect),
        Id::Pick(part) => a
            .picker
            .as_ref()
            .map_or((0.0, 0.0, 0.0, 0.0), |p| p.part_rect(part)),
        Id::Reply => a.reply_rect().unwrap_or((0.0, 0.0, 0.0, 0.0)),
        Id::ReplyCancel => a.reply_cancel_rect().unwrap_or((0.0, 0.0, 0.0, 0.0)),
        Id::Older => a.head_rect().unwrap_or((0.0, 0.0, 0.0, 0.0)),
        Id::Newer(k) => a.foot_rect(k).unwrap_or((0.0, 0.0, 0.0, 0.0)),
        Id::Original(i) => a.quote_link_rect(i).unwrap_or((0.0, 0.0, 0.0, 0.0)),
        Id::Edited(i) => a.edited_rect(i).unwrap_or((0.0, 0.0, 0.0, 0.0)),
        Id::File(i, k) => a.file_rect(i, k).unwrap_or((0.0, 0.0, 0.0, 0.0)),
        Id::BoxPart(p) => a.box_part_rect(p).unwrap_or((0.0, 0.0, 0.0, 0.0)),
        Id::Embed(i, k) => a.embed_rect(i, k).unwrap_or((0.0, 0.0, 0.0, 0.0)),
        Id::Thread => (a.thread_x(), 0.0, crate::thread::THREAD_W, a.h),
        Id::ThreadPart(p) => a.thread_part_rect(p).unwrap_or((0.0, 0.0, 0.0, 0.0)),
        Id::Sign(p) => a
            .signin
            .as_ref()
            .and_then(|s| s.part_rect(p))
            .map_or((0.0, 0.0, 0.0, 0.0), |(r, _)| r),
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
    IToggleProvider,
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
            Id::Sign(Part::Title | Part::Message) => false,
            Id::MenuItem(_)
            | Id::MenuClose
            | Id::Tool(..)
            | Id::ReplyCancel
            | Id::Older
            | Id::Newer(_)
            | Id::Original(_)
            | Id::Edited(_)
            | Id::File(..)
            | Id::Embed(..)
            | Id::BoxPart(_) => pattern == UIA_InvokePatternId,
            Id::Pick(PPart::Category(_)) => {
                pattern == UIA_InvokePatternId || pattern == UIA_SelectionItemPatternId
            }
            Id::Pick(PPart::Search) => {
                pattern == UIA_ValuePatternId
                    || pattern == UIA_TextPatternId
                    || pattern == UIA_TextPattern2Id
            }
            Id::Pick(_) => pattern == UIA_InvokePatternId,
            Id::Dialog(DPart::Field) => {
                pattern == UIA_ValuePatternId
                    || pattern == UIA_TextPatternId
                    || pattern == UIA_TextPattern2Id
            }
            Id::Dialog(
                DPart::Reload
                | DPart::Cancel
                | DPart::Primary
                | DPart::Close
                | DPart::Version(_)
                | DPart::Retry
                | DPart::Older,
            ) => pattern == UIA_InvokePatternId,
            Id::Sign(Part::Primary | Part::OtherEmail | Part::Resend) => {
                pattern == UIA_InvokePatternId
            }
            Id::Composer | Id::Sign(Part::Field) => {
                pattern == UIA_ValuePatternId
                    || pattern == UIA_TextPatternId
                    || pattern == UIA_TextPattern2Id
            }
            Id::Row(_) | Id::ThreadPart(TPart::Row(_)) => pattern == UIA_TextPatternId,
            Id::Control(..) => pattern == UIA_InvokePatternId,
            Id::ThreadPart(TPart::Field) => {
                pattern == UIA_ValuePatternId
                    || pattern == UIA_TextPatternId
                    || pattern == UIA_TextPattern2Id
            }
            Id::ThreadPart(TPart::Broadcast) => pattern == UIA_TogglePatternId,
            Id::ThreadPart(_) => pattern == UIA_InvokePatternId,
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
                    Id::ThreadPart(TPart::Row(i)) => {
                        a.thread.as_ref().map_or_else(String::new, |t| {
                            format!("thread-message-{}", t.c.messages[i].id)
                        })
                    }
                    _ => format!("{id:?}"),
                })),
                UIA_FrameworkIdPropertyId => VARIANT::from(BSTR::from("Win32")),
                // A chip's who-reacted summary, as its card shows it.
                UIA_HelpTextPropertyId => match id {
                    Id::Control(i, Action::Chip(k)) => {
                        let m = &a.c.messages[i];
                        a.live
                            .as_ref()
                            .and_then(|l| l.reactor_summary(&m.id, &m.reactions.get(k)?.emoji))
                            .map_or_else(VARIANT::default, |s| VARIANT::from(BSTR::from(s)))
                    }
                    _ => VARIANT::default(),
                },
                UIA_IsKeyboardFocusablePropertyId => VARIANT::from(matches!(
                    id,
                    Id::Composer
                        | Id::Control(..)
                        | Id::ThreadPart(TPart::Field)
                        | Id::Space(_)
                        | Id::Channel(_)
                        | Id::MenuItem(_)
                        | Id::Pick(PPart::Search)
                        | Id::Dialog(DPart::Field | DPart::Reload | DPart::Cancel | DPart::Primary)
                        | Id::Sign(Part::Field | Part::Primary | Part::OtherEmail | Part::Resend)
                )),
                UIA_HasKeyboardFocusPropertyId => VARIANT::from(focused(a) == Some(id)),
                UIA_IsEnabledPropertyId => VARIANT::from(match (id, &a.signin) {
                    (Id::Sign(Part::Field), Some(s)) => s.editable(),
                    (Id::Sign(Part::Title | Part::Message), _) => true,
                    (Id::Dialog(p), _) => a.dialog.as_ref().is_some_and(|d| d.enabled(p)),
                    (Id::MenuItem(k), _) => a
                        .menu
                        .as_ref()
                        .and_then(|m| m.items.get(k))
                        .is_some_and(|item| item.enabled()),
                    (Id::Sign(p), Some(s)) => s.part_rect(p).is_some_and(|(_, on)| on),
                    (Id::Tool(i, t), _) => a.tools(i).iter().any(|s| s.tool == t && s.enabled),
                    (Id::ThreadPart(p), _) => a.thread_part_enabled(p),
                    (Id::BoxPart(crate::uploads::Part::Attach), _) => {
                        a.live.as_ref().is_some_and(|l| l.can_attach())
                    }
                    (Id::Newer(k), _) => a.foot_head(k).enabled,
                    (Id::Older, _) => a.live.as_ref().is_some_and(|l| {
                        matches!(
                            l.older(),
                            crate::live::Older::More | crate::live::Older::Failed
                        )
                    }),
                    _ => true,
                }),
                // Errors and \"We sent a new code\" are announced when they appear.
                UIA_LiveSettingPropertyId if id == Id::Sign(Part::Message) => {
                    VARIANT::from(Assertive.0)
                }
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
            match id {
                Id::Composer => a.set_composer_focus(true),
                Id::Sign(Part::Field) => a.set_signin_focus(true),
                Id::Dialog(DPart::Field) => a.dialog_activate(DPart::Field),
                Id::Pick(PPart::Search) => a.picker_activate(PPart::Search),
                Id::ThreadPart(TPart::Field) => a.set_thread_focus(true),
                Id::Control(i, action) => a.set_kb(Some((i, action))),
                _ => {}
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
        match with(|a| Ok(focused(a)))? {
            Some(id) => Ok(fragment(id)),
            None => Err(Error::empty()),
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
                Id::Sign(p) => a.signin_activate(Some(p)),
                Id::MenuItem(k) => a.menu_activate(k),
                Id::Dialog(p) => a.dialog_activate(p),
                Id::Control(i, action) => a.row_action(i, action),
                Id::Tool(i, t) => a.tool_activate(i, t),
                Id::MenuClose => a.close_menu(),
                Id::Pick(part) => a.picker_activate(part),
                Id::ReplyCancel => a.cancel_reply(),
                Id::Older => a.load_older(),
                Id::Newer(k) => a.foot_activate(k),
                Id::Original(i) => a.view_original_of(i),
                Id::Edited(i) => a.open_history_of(i),
                Id::File(i, k) => a.file_activate(i, k, false),
                Id::BoxPart(p) => a.box_activate(p),
                Id::Embed(i, k) => a.embed_activate(i, k),
                Id::ThreadPart(p) => a.thread_activate(p),
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
                Id::Pick(PPart::Category(k)) => {
                    a.picker.as_ref().and_then(|p| p.selected_category()) == Some(k)
                }
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
        let id = self.id;
        with(|a| {
            if a.signin.as_ref().is_some_and(|s| !s.editable()) {
                return Err(HRESULT(UIA_E_ELEMENTNOTENABLED as _).into());
            }
            let f = field_mut(a, id);
            f.text = text;
            f.caret = f.text.len();
            f.anchor = f.caret;
            field_changed(a, id, true);
            Ok(())
        })
    }

    fn Value(&self) -> Result<BSTR> {
        let id = self.id;
        with(|a| Ok(BSTR::from_wide(&field(a, id).text)))
    }

    fn IsReadOnly(&self) -> Result<BOOL> {
        with(|a| Ok(a.signin.as_ref().is_some_and(|s| !s.editable()).into()))
    }
}

impl IToggleProvider_Impl for Node_Impl {
    fn Toggle(&self) -> Result<()> {
        let id = self.id;
        with(|a| {
            if let Id::ThreadPart(p) = id {
                a.thread_activate(p);
            }
            Ok(())
        })
    }

    fn ToggleState(&self) -> Result<ToggleState> {
        with(|a| {
            Ok(if a.thread.as_ref().is_some_and(|t| t.broadcast) {
                ToggleState_On
            } else {
                ToggleState_Off
            })
        })
    }
}

// ---------------------------------------------------------------------------
// Text: one implementation over two sources, a text field (the composer or
// the sign-in field) and a message row.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Doc {
    /// The composer or the sign-in field, by node.
    Field(Id),
    Row(usize),
    /// A row in the thread pane.
    ThreadRow(usize),
}

impl Doc {
    fn of(id: Id) -> Self {
        match id {
            Id::Row(i) => Self::Row(i),
            Id::ThreadPart(TPart::Row(i)) => Self::ThreadRow(i),
            _ => Self::Field(id),
        }
    }

    fn id(self) -> Id {
        match self {
            Self::Field(id) => id,
            Self::Row(i) => Id::Row(i),
            Self::ThreadRow(i) => Id::ThreadPart(TPart::Row(i)),
        }
    }

    fn text(self, a: &App) -> Result<Vec<u16>> {
        match self {
            Self::Field(id) if exists(a, id) => Ok(field(a, id).text.clone()),
            Self::Row(i) if exists(a, Id::Row(i)) => {
                Ok(a.rows.row(i).map(|r| r.plain16.clone()).unwrap_or_default())
            }
            Self::ThreadRow(i) if exists(a, self.id()) => Ok(a
                .thread
                .as_ref()
                .and_then(|t| t.rows.row(i))
                .map(|r| r.plain16.clone())
                .unwrap_or_default()),
            _ => Err(na()),
        }
    }

    fn layout(self, a: &App) -> Option<IDWriteTextLayout> {
        match self {
            Self::Field(id) => field(a, id).layout.clone(),
            Self::Row(i) => a.rows.row(i).map(|r| r.text.clone()),
            Self::ThreadRow(i) => a.thread.as_ref()?.rows.row(i).map(|r| r.text.clone()),
        }
    }

    /// Client DIP position of the layout origin.
    fn origin(self, a: &App) -> (f32, f32) {
        match (self, &a.signin) {
            (Self::Field(Id::Dialog(_)), _) => {
                a.dialog.as_ref().map_or((0.0, 0.0), |d| d.text_origin())
            }
            (Self::Field(Id::Sign(_)), Some(s)) => s.field_origin,
            (Self::Field(Id::Pick(_)), _) => {
                a.picker.as_ref().map_or((0.0, 0.0), |p| p.text_origin())
            }
            (Self::Field(Id::ThreadPart(_)), _) => a.thread_text_origin(),
            (Self::Field(_), _) => a.composer_text_origin(),
            (Self::Row(i), _) => a.row_text_origin(i),
            (Self::ThreadRow(i), _) => a.thread_row_text_origin(i),
        }
    }

    fn selection(self, a: &App) -> (usize, usize) {
        match self {
            Self::Field(id) => field(a, id).selection(),
            Self::Row(i) => App::sel_for(a.sel, i, a.row_len(i))
                .map_or((0, 0), |(s, e)| (s as usize, e as usize)),
            // The thread's rows are read, not selected.
            Self::ThreadRow(_) => (0, 0),
        }
    }

    fn select(self, a: &mut App, s: usize, e: usize) {
        match self {
            Self::Field(id) => {
                let f = field_mut(a, id);
                f.anchor = s;
                f.caret = e;
                field_changed(a, id, false);
            }
            Self::Row(i) => a.set_sel(Some(((i, s as u32), (i, e as u32)))),
            Self::ThreadRow(_) => {}
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
        let id = self.id;
        let (caret, focused) = with(|a| {
            let f = field(a, id);
            Ok((f.caret, f.focused))
        })?;
        if !isactive.is_null() {
            unsafe { *isactive = focused.into() };
        }
        Ok(range(Doc::Field(id), caret, caret))
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
            UIA_IsReadOnlyAttributeId => VARIANT::from(matches!(self.doc, Doc::Row(_))),
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

/// Raised after edits in the focused field while a client listens.
pub fn raise_composer_events() {
    unsafe {
        if UiaClientsAreListening().as_bool() {
            let n = node(editable());
            let _ = UiaRaiseAutomationEvent(&n, UIA_Text_TextChangedEventId);
            let _ = UiaRaiseAutomationEvent(&n, UIA_Text_TextSelectionChangedEventId);
        }
    }
}

pub fn raise_focus() {
    unsafe {
        if UiaClientsAreListening().as_bool() {
            let id = crate::with_app(|a| focused(a))
                .flatten()
                .unwrap_or_else(editable);
            let _ = UiaRaiseAutomationEvent(&node(id), UIA_AutomationFocusChangedEventId);
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

/// The text field on screen: the sign-in field while signed out.
fn editable() -> Id {
    match crate::with_app(|a| {
        (
            a.dialog.as_ref().map(|d| d.kind),
            a.signin.is_some(),
            a.picker.is_some(),
            a.thread.as_ref().is_some_and(|t| t.composer.focused),
        )
    }) {
        Some((Some(Kind::Edit), _, _, _)) => Id::Dialog(DPart::Field),
        Some((_, true, _, _)) => Id::Sign(Part::Field),
        Some((_, _, true, _)) => Id::Pick(PPart::Search),
        Some((_, _, _, true)) => Id::ThreadPart(TPart::Field),
        _ => Id::Composer,
    }
}

/// The sign-in screen appeared, changed step or closed: the tree under the
/// root changed, and its message (an assertive live region) is announced.
pub fn raise_screen_changed() {
    unsafe {
        if UiaClientsAreListening().as_bool() {
            let mut id = runtime_id(Id::Root);
            let _ = UiaRaiseStructureChangedEvent(
                &node(Id::Root),
                StructureChangeType_ChildrenInvalidated,
                id.as_mut_ptr(),
                3,
            );
            if crate::with_app(|a| a.signin.as_ref().and_then(|s| s.message()).is_some())
                == Some(true)
            {
                let _ = UiaRaiseAutomationEvent(
                    &node(Id::Sign(Part::Message)),
                    UIA_LiveRegionChangedEventId,
                );
            }
        }
    }
}
