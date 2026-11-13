//! The formatted composer, as desktop's (`apps/native/desktop/src/formatting.rs`)
//! and web's: the buffer holds the draft's Markdown source with its
//! delimiters hidden and its runs drawn in their styles: bold, italic,
//! underline, strikethrough, spoilers on a #34383B wash and quoted lines
//! indented beside a bar. Each typed edit goes through the shared
//! `composer::edit` (`shared/messages/composer.rs`), which leaves the source
//! as web's composer would. Backspace and Delete remove visible characters,
//! Ctrl+B, Ctrl+I and Ctrl+U toggle a selection's style, and a selection
//! shows web's formatting toolbar above the field, whose toggles are the
//! shared `composer::toggle`.
//!
//! A repair joins the user's own undo step: a buffer emits `end-user-action`
//! before its history closes the step. Undo, redo and programmatic writes
//! (drafts, sending, inserted emoji) are not user actions, so they keep the
//! source they write, and an IME's committed text is kept as committed. Each
//! of those, a moved caret, and leaving the field end the style web carries
//! to the next key. Copy and Cut take the visible text, as desktop's, so a
//! paste is typed text wherever it lands.

use caper_client::formatting::{
    self, BOLD, ITALIC, QUOTE, SPOILER, STRIKETHROUGH, UNDERLINE, normalized_selection, projection,
    selection_styles, source_runs, toggle,
};
use gtk::{gdk, glib, pango, prelude::*};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::{Rc, Weak};

/// The longest draft a style toggle may leave, as desktop's.
const MAX_LENGTH: usize = 4000;

/// A quoted line's indent: web's 3 px bar and 12 px of space.
const QUOTE_INDENT: i32 = 15;

/// The toolbar, in web's order: each style bit, its name, its Lucide icon's
/// SVG elements and its shortcut key. A gap comes before Quote.
const TOOLS: [(i32, &str, &str, Option<char>); 6] = [
    (
        BOLD,
        "Bold",
        r#"<path d="M6 12h9a4 4 0 0 1 0 8H7a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h7a4 4 0 0 1 0 8"/>"#,
        Some('B'),
    ),
    (
        ITALIC,
        "Italic",
        r#"<line x1="19" x2="10" y1="4" y2="4"/><line x1="14" x2="5" y1="20" y2="20"/><line x1="15" x2="9" y1="4" y2="20"/>"#,
        Some('I'),
    ),
    (
        UNDERLINE,
        "Underline",
        r#"<path d="M6 4v6a6 6 0 0 0 12 0V4"/><line x1="4" x2="20" y1="20" y2="20"/>"#,
        Some('U'),
    ),
    (
        STRIKETHROUGH,
        "Strikethrough",
        r#"<path d="M16 4H9a3 3 0 0 0-2.83 4"/><path d="M14 12a4 4 0 0 1 0 8H6"/><line x1="4" x2="20" y1="12" y2="12"/>"#,
        None,
    ),
    (
        QUOTE,
        "Quote",
        r#"<path d="M17 6H3"/><path d="M21 12H8"/><path d="M21 18H8"/><path d="M3 12v6"/>"#,
        None,
    ),
    (
        SPOILER,
        "Spoiler",
        r#"<path d="M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49"/><path d="M14.084 14.158a3 3 0 0 1-4.242-4.242"/><path d="M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143"/><path d="m2 2 20 20"/>"#,
        None,
    ),
];

/// A toolbar button's tooltip, as web's title.
fn tool_tip(name: &str, key: Option<char>, allowed: bool) -> String {
    match key {
        _ if !allowed => {
            "This selection cannot be formatted without changing its text or link".into()
        }
        Some(key) => format!("{name} · Ctrl+{key}"),
        None => name.into(),
    }
}

/// What typing left in a field: its source, the caret the last key left, and
/// the style web carries to a key typed there next (`composer::edit`'s
/// `pending`).
pub struct Typing {
    text: String,
    caret: Option<usize>,
    pending: i32,
}

impl Typing {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.into(),
            caret: None,
            pending: -1,
        }
    }

    /// A user's edit that left `next` with selection `insert..bound`: the
    /// source and selection the field shows instead.
    pub fn edit(&mut self, next: &str, insert: usize, bound: usize) -> (String, usize, usize) {
        let typed = next.chars().count().checked_sub(self.text.chars().count());
        let continuing = match (
            typed.and_then(|count| insert.checked_sub(count)),
            self.caret,
        ) {
            (Some(at), Some(caret)) if insert == bound && at == caret => self.pending,
            _ => -1,
        };
        let edited = formatting::edit(&self.text, next, insert, bound, continuing);
        let (insert, bound) = normalized_selection(&edited.text, edited.start, edited.end);
        self.pending = edited.pending;
        self.caret = (insert == bound).then_some(insert);
        self.text.clone_from(&edited.text);
        (edited.text, insert, bound)
    }

    /// Anything but typing left `text` with selection `insert..bound`: the
    /// next key takes the style around the caret.
    pub fn ended(&mut self, text: &str, insert: usize, bound: usize) {
        self.text = text.into();
        self.caret = (insert == bound).then_some(insert);
        self.pending = -1;
    }
}

/// The source range Backspace (`backward`) or Delete removes at source caret
/// `caret`: the visible grapheme beside it, or `None` when there is none.
pub fn deletion(text: &str, caret: usize, backward: bool) -> Option<(usize, usize)> {
    let (visible, indices) = projection(text);
    let at = indices.partition_point(|index| *index < caret);
    let next = formatting::adjacent_grapheme(&visible, at, backward);
    (next != at).then(|| {
        let (a, b) = formatting::source_selection(&indices, text.chars().count(), next, at);
        (a.min(b), a.max(b))
    })
}

/// The visible text of source range `start..end`, as Copy takes it.
pub fn copied(text: &str, start: usize, end: usize) -> String {
    let (visible, indices) = projection(text);
    let [start, end] = [start, end].map(|at| indices.partition_point(|index| *index < at));
    visible.chars().skip(start).take(end - start).collect()
}

/// A composer field's formatting (`attach`).
pub struct Rich {
    me: Weak<Rich>,
    view: glib::WeakRef<gtk::TextView>,
    typing: RefCell<Typing>,
    /// Hidden delimiters, then bold, italic, underline, strikethrough,
    /// spoiler and quote, in style bit order.
    tags: [gtk::TextTag; 7],
    /// The lines a quote covers, from the last restyle.
    quoted: RefCell<BTreeSet<i32>>,
    /// The quote blocks' bars, overlaid on the text.
    bars: RefCell<Vec<gtk::Widget>>,
    /// A bar update is waiting for the layout.
    bars_queued: Cell<bool>,
    /// The formatting toolbar over a selection.
    bar: gtk::Popover,
    /// Its buttons, in `TOOLS` order.
    tools: RefCell<Vec<gtk::ToggleButton>>,
    /// Inside a user action (typing, deleting, pasting, an IME commit).
    in_action: Cell<bool>,
    /// While this module writes the buffer or its selection itself.
    applying: Cell<bool>,
    /// An IME is composing (its preedit is not in the buffer yet).
    preedit: Cell<bool>,
    /// An IME composed since the last edit, so that edit is its commit.
    ime: Cell<bool>,
}

fn text(buffer: &gtk::TextBuffer) -> String {
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string()
}

fn at(buffer: &gtk::TextBuffer, offset: usize) -> gtk::TextIter {
    buffer.iter_at_offset(i32::try_from(offset).unwrap_or(i32::MAX))
}

fn selection(buffer: &gtk::TextBuffer) -> (usize, usize) {
    let offset = |mark: &gtk::TextMark| {
        usize::try_from(buffer.iter_at_mark(mark).offset()).unwrap_or_default()
    };
    (
        offset(&buffer.get_insert()),
        offset(&buffer.selection_bound()),
    )
}

/// Rewrites only what differs between `from`, the buffer's text, and `to`.
fn replace(buffer: &gtk::TextBuffer, from: &str, to: &str) {
    let (from, to): (Vec<char>, Vec<char>) = (from.chars().collect(), to.chars().collect());
    let prefix = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let suffix = from[prefix..]
        .iter()
        .rev()
        .zip(to[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (mut start, mut end) = (at(buffer, prefix), at(buffer, from.len() - suffix));
    buffer.delete(&mut start, &mut end);
    let inserted: String = to[prefix..to.len() - suffix].iter().collect();
    buffer.insert(&mut start, &inserted);
}

/// Makes `view` a formatted composer. Its selection's formatting toolbar
/// belongs to `anchor`, a box holding the field.
pub fn attach(view: &gtk::TextView, anchor: &gtk::Box) -> Rc<Rich> {
    let buffer = view.buffer();
    let tags = [
        gtk::TextTag::builder().invisible(true).build(),
        gtk::TextTag::builder().weight(700).build(),
        gtk::TextTag::builder().style(pango::Style::Italic).build(),
        gtk::TextTag::builder()
            .underline(pango::Underline::Single)
            .build(),
        gtk::TextTag::builder().strikethrough(true).build(),
        gtk::TextTag::builder().background("#34383b").build(),
        gtk::TextTag::builder().left_margin(QUOTE_INDENT).build(),
    ];
    for tag in &tags {
        buffer.tag_table().add(tag);
    }
    let bar = gtk::Popover::new();
    bar.set_autohide(false);
    bar.set_has_arrow(false);
    bar.set_can_focus(false);
    bar.set_position(gtk::PositionType::Top);
    bar.add_css_class("format-bar");
    bar.set_parent(anchor);
    let rich = Rc::new_cyclic(|me| Rich {
        me: me.clone(),
        view: view.downgrade(),
        typing: RefCell::new(Typing::new("")),
        tags,
        quoted: RefCell::default(),
        bars: RefCell::default(),
        bars_queued: Cell::new(false),
        bar,
        tools: RefCell::default(),
        in_action: Cell::new(false),
        applying: Cell::new(false),
        preedit: Cell::new(false),
        ime: Cell::new(false),
    });
    let row: gtk::Box = crate::app::named("GtkBox", "Message formatting");
    for (style, name, icon, key) in TOOLS {
        if style == QUOTE {
            let divider = gtk::Separator::new(gtk::Orientation::Vertical);
            divider.add_css_class("format-divider");
            row.append(&divider);
        }
        let button: gtk::ToggleButton = crate::app::named("GtkToggleButton", name);
        // At its own size: a filled 36px button would scale it up.
        let image = crate::images::lucide_image(icon, "#b9bcbe", 17);
        image.set_halign(gtk::Align::Center);
        image.set_valign(gtk::Align::Center);
        button.set_child(Some(&image));
        button.set_tooltip_text(Some(&tool_tip(name, key, true)));
        button.add_css_class("flat");
        // The field keeps the focus and its selection.
        button.set_can_focus(false);
        button.set_focus_on_click(false);
        let weak: Weak<Rich> = Rc::downgrade(&rich);
        button.connect_clicked(move |_| {
            if let Some(rich) = weak.upgrade() {
                rich.toggle(style);
            }
        });
        row.append(&button);
        rich.tools.borrow_mut().push(button);
    }
    rich.bar.set_child(Some(&row));
    // Rewrapping moves the quote bars; it changes the text's height.
    let this = Rc::downgrade(&rich);
    let watch = move |adjustment: Option<gtk::Adjustment>| {
        if let Some(adjustment) = adjustment {
            let this = this.clone();
            adjustment.connect_changed(move |_| {
                if let Some(rich) = this.upgrade() {
                    rich.queue_bars();
                }
            });
        }
    };
    watch(view.vadjustment());
    view.connect_vadjustment_notify(move |view| watch(view.vadjustment()));

    let this = rich.clone();
    buffer.connect_begin_user_action(move |_| this.in_action.set(true));
    let this = rich.clone();
    buffer.connect_end_user_action(move |buffer| {
        this.in_action.set(false);
        if !this.applying.get() {
            this.edited(buffer);
        }
    });
    let this = rich.clone();
    buffer.connect_changed(move |buffer| {
        if this.in_action.get() || this.applying.get() {
            return;
        }
        let (insert, bound) = selection(buffer);
        this.typing.borrow_mut().ended(&text(buffer), insert, bound);
        this.restyle(buffer);
        this.show_bar();
    });
    let this = rich.clone();
    buffer.connect_mark_set(move |buffer, _, mark| {
        if (*mark != buffer.get_insert() && *mark != buffer.selection_bound())
            || this.in_action.get()
            || this.applying.get()
            || this.preedit.get()
        {
            return;
        }
        // A placed caret, even where it was, ends the carried style.
        let text = text(buffer);
        let (insert, bound) = selection(buffer);
        let (insert, bound) = this.place(buffer, &text, insert, bound);
        this.typing.borrow_mut().ended(&text, insert, bound);
        this.show_bar();
    });
    let this = rich.clone();
    view.connect_preedit_changed(move |_, preedit| {
        this.preedit.set(!preedit.is_empty());
        if !preedit.is_empty() {
            this.ime.set(true);
            this.bar.popdown();
        }
    });
    // GTK copies the selected source, hidden delimiters and all.
    for cut in [false, true] {
        let signal = if cut {
            "cut-clipboard"
        } else {
            "copy-clipboard"
        };
        view.connect_local(signal, false, move |values| {
            let view = values[0].get::<gtk::TextView>().ok()?;
            view.stop_signal_emission_by_name(signal);
            let buffer = view.buffer();
            let (insert, bound) = selection(&buffer);
            if insert != bound {
                let copied = copied(&text(&buffer), insert.min(bound), insert.max(bound));
                view.clipboard().set_text(&copied);
                if cut {
                    buffer.delete_selection(true, view.is_editable());
                }
            }
            None
        });
    }
    let focus = gtk::EventControllerFocus::new();
    let this = rich.clone();
    focus.connect_enter(move |_| this.show_bar());
    let this = rich.clone();
    focus.connect_leave(move |_| {
        this.end();
        this.bar.popdown();
    });
    view.add_controller(focus);
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let this = rich.clone();
    keys.connect_key_pressed(move |_, key, _, modifiers| this.key(key, modifiers));
    view.add_controller(keys);
    rich
}

impl Rich {
    /// An IME is composing: Enter and Backspace are its own.
    pub fn composing(&self) -> bool {
        self.preedit.get()
    }

    /// Ends the style carried to the next key, as when the field shows
    /// another conversation.
    pub fn end(&self) {
        if let Some(view) = self.view.upgrade() {
            let buffer = view.buffer();
            let (insert, bound) = selection(&buffer);
            self.typing
                .borrow_mut()
                .ended(&text(&buffer), insert, bound);
        }
    }

    /// Moves a selection's ends to their visible places.
    fn place(
        &self,
        buffer: &gtk::TextBuffer,
        text: &str,
        insert: usize,
        bound: usize,
    ) -> (usize, usize) {
        let placed = normalized_selection(text, insert, bound);
        if placed != (insert, bound) {
            self.applying.set(true);
            buffer.select_range(&at(buffer, placed.0), &at(buffer, placed.1));
            self.applying.set(false);
        }
        placed
    }

    /// After a user action: repairs it as web's composer would.
    fn edited(&self, buffer: &gtk::TextBuffer) {
        let next = text(buffer);
        let (insert, bound) = selection(buffer);
        if self.ime.get() {
            // An IME owns what it committed.
            self.typing.borrow_mut().ended(&next, insert, bound);
            self.ime.set(self.preedit.get());
        } else {
            let (shown, start, end) = self.typing.borrow_mut().edit(&next, insert, bound);
            if shown != next || (start, end) != (insert, bound) {
                self.applying.set(true);
                if shown != next {
                    replace(buffer, &next, &shown);
                }
                buffer.select_range(&at(buffer, start), &at(buffer, end));
                self.applying.set(false);
            }
        }
        self.restyle(buffer);
        self.show_bar();
    }

    /// Hides delimiters and styles the runs between them. A quote's
    /// indent covers its whole lines, `> ` included, since a paragraph takes
    /// its margin from its first character.
    fn restyle(&self, buffer: &gtk::TextBuffer) {
        let (start, end) = buffer.bounds();
        for tag in &self.tags {
            buffer.remove_tag(tag, &start, &end);
        }
        let text = text(buffer);
        let mut offset = 0;
        let mut quoted = BTreeSet::new();
        for run in source_runs(&text) {
            let count = text[run.range].chars().count();
            let (start, end) = (at(buffer, offset), at(buffer, offset + count));
            if run.hidden {
                buffer.apply_tag(&self.tags[0], &start, &end);
            }
            for (bit, tag) in self.tags[1..6].iter().enumerate() {
                if run.style & (1 << bit) != 0 {
                    buffer.apply_tag(tag, &start, &end);
                }
            }
            if run.style & QUOTE != 0 {
                quoted.extend(start.line()..=end.line());
            }
            offset += count;
        }
        for &line in &quoted {
            if let Some(start) = buffer.iter_at_line(line) {
                let mut end = start;
                if !end.ends_line() {
                    end.forward_to_line_end();
                }
                buffer.apply_tag(&self.tags[6], &start, &end);
            }
        }
        self.quoted.replace(quoted);
        self.queue_bars();
    }

    /// Places the quote bars once the layout has the restyled lines.
    fn queue_bars(&self) {
        if self.bars_queued.replace(true) {
            return;
        }
        let me = self.me.clone();
        glib::idle_add_local_once(move || {
            if let Some(rich) = me.upgrade() {
                rich.bars_queued.set(false);
                rich.place_bars();
            }
        });
    }

    /// One 3 px bar beside each run of quoted lines.
    fn place_bars(&self) {
        let Some(view) = self.view.upgrade() else {
            return;
        };
        let buffer = view.buffer();
        let mut blocks: Vec<(i32, i32)> = Vec::new();
        for &line in self.quoted.borrow().iter() {
            match blocks.last_mut() {
                Some((_, last)) if *last + 1 == line => *last = line,
                _ => blocks.push((line, line)),
            }
        }
        let mut bars = self.bars.borrow_mut();
        while bars.len() > blocks.len() {
            if let Some(bar) = bars.pop() {
                view.remove(&bar);
            }
        }
        for (index, (first, last)) in blocks.into_iter().enumerate() {
            let (Some(top), Some(bottom)) = (buffer.iter_at_line(first), buffer.iter_at_line(last))
            else {
                continue;
            };
            let (y, _) = view.line_yrange(&top);
            let (bottom_y, bottom_height) = view.line_yrange(&bottom);
            let height = (bottom_y + bottom_height - y).max(1);
            if let Some(bar) = bars.get(index) {
                bar.set_size_request(3, height);
                view.move_overlay(bar, 0, y);
            } else {
                let bar = gtk::Box::new(gtk::Orientation::Vertical, 0);
                bar.add_css_class("composer-quote-bar");
                bar.set_can_target(false);
                bar.set_size_request(3, height);
                view.add_overlay(&bar, 0, y);
                bars.push(bar.upcast());
            }
        }
    }

    fn key(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> glib::Propagation {
        let Some(view) = self.view.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if self.preedit.get() || modifiers.intersects(gdk::ModifierType::ALT_MASK) {
            return glib::Propagation::Proceed;
        }
        let buffer = view.buffer();
        let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        match key {
            gdk::Key::b | gdk::Key::B if control => self.toggle(BOLD),
            gdk::Key::i | gdk::Key::I if control => self.toggle(ITALIC),
            gdk::Key::u | gdk::Key::U if control => self.toggle(UNDERLINE),
            // GTK moves arrows through the layout, which leaves out
            // invisible text and can land among hidden delimiters.
            gdk::Key::Left | gdk::Key::Right | gdk::Key::KP_Left | gdk::Key::KP_Right
                if !control =>
            {
                let text = text(&buffer);
                let (visible, indices) = projection(&text);
                let place = |source: usize| indices.partition_point(|index| *index < source);
                let (insert, bound) = selection(&buffer);
                let (primary, secondary) = (place(insert), place(bound));
                let backward = matches!(key, gdk::Key::Left | gdk::Key::KP_Left);
                let extend = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
                let next = if primary != secondary && !extend {
                    if backward {
                        primary.min(secondary)
                    } else {
                        primary.max(secondary)
                    }
                } else {
                    formatting::adjacent_grapheme(&visible, primary, backward)
                };
                let (insert, bound) = formatting::source_selection(
                    &indices,
                    text.chars().count(),
                    next,
                    if extend { secondary } else { next },
                );
                buffer.select_range(&at(&buffer, insert), &at(&buffer, bound));
                view.scroll_mark_onscreen(&buffer.get_insert());
            }
            gdk::Key::BackSpace | gdk::Key::Delete | gdk::Key::KP_Delete
                if !control && !buffer.has_selection() =>
            {
                let (caret, _) = selection(&buffer);
                // Select the visible character, which GTK then deletes.
                let Some((start, end)) =
                    deletion(&text(&buffer), caret, key == gdk::Key::BackSpace)
                else {
                    return glib::Propagation::Stop;
                };
                self.applying.set(true);
                buffer.select_range(&at(&buffer, start), &at(&buffer, end));
                self.applying.set(false);
                return glib::Propagation::Proceed;
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    /// Style bit `style` on, or off, for the selection, as web's toolbar
    /// (`composer::toggle`).
    fn toggle(&self, style: i32) {
        let Some(view) = self.view.upgrade() else {
            return;
        };
        let buffer = view.buffer();
        let current = text(&buffer);
        let (insert, bound) = selection(&buffer);
        let (next, start, end) = toggle(&current, insert.min(bound), insert.max(bound), style);
        if next == current || next.chars().count() > MAX_LENGTH {
            // A refused toggle leaves its button as the selection is.
            self.show_bar();
            return;
        }
        self.applying.set(true);
        // One undo step.
        buffer.begin_user_action();
        replace(&buffer, &current, &next);
        buffer.end_user_action();
        let (start, end) = normalized_selection(&next, start, end);
        buffer.select_range(&at(&buffer, start), &at(&buffer, end));
        self.applying.set(false);
        self.typing.borrow_mut().ended(&next, start, end);
        self.restyle(&buffer);
        view.grab_focus();
        self.show_bar();
    }

    /// The formatting toolbar over the field while text in it is selected:
    /// a style's button is pressed when a selected character has it, and
    /// off when the selection cannot take its toggle.
    fn show_bar(&self) {
        let Some(view) = self.view.upgrade() else {
            return;
        };
        // From the marks: `mark-set` handlers run before the buffer's
        // `has-selection` follows them.
        let buffer = view.buffer();
        let (insert, bound) = selection(&buffer);
        if !(view.has_focus() && view.is_mapped() && insert != bound && !self.preedit.get()) {
            self.bar.popdown();
            return;
        }
        let source = text(&buffer);
        let (start, end) = (insert.min(bound), insert.max(bound));
        let styles = selection_styles(&source, start, end);
        for (button, (style, name, _, key)) in self.tools.borrow().iter().zip(TOOLS) {
            let allowed = formatting::composer::toggle(&source, start, end, style).is_some();
            button.set_active(styles & style != 0);
            button.set_sensitive(allowed);
            button.set_tooltip_text(Some(&tool_tip(name, key, allowed)));
        }
        // Its left edge 8 px above the field's, as desktop's.
        let field = view.parent().unwrap_or_else(|| view.clone().upcast());
        if let Some(anchor) = self.bar.parent()
            && let Some(row) = self.bar.child()
            && let Some(bounds) = field.compute_bounds(&anchor)
        {
            // A hidden popover measures nothing; its buttons do.
            let (_, width, _, _) = row.measure(gtk::Orientation::Horizontal, -1);
            self.bar.set_pointing_to(Some(&gdk::Rectangle::new(
                bounds.x() as i32 + width / 2,
                bounds.y() as i32 - 8,
                1,
                1,
            )));
        }
        if !self.bar.is_visible() {
            crate::ui::hands(&self.bar);
            crate::ui::arrive(&self.bar);
            self.bar.popup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible(text: &str, caret: usize) -> usize {
        projection(text).1.partition_point(|index| *index < caret)
    }

    /// Every client types the shared steps generated from web with its own
    /// caret mapping. GTK's caret always sits at its visible place, and a
    /// placed caret (a click, even where it already is) ends the style the
    /// last key carried.
    #[test]
    fn typing_follows_the_shared_cases() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../shared/messages/composer-cases.json"
        ))
        .unwrap();
        let cases = cases["cases"].as_array().unwrap();
        assert!(cases.len() > 20);
        // Visible UTF-16 offsets, as web counts them.
        let utf16 = |text: &str, offset: usize| -> usize {
            projection(text)
                .0
                .chars()
                .take(offset)
                .map(char::len_utf16)
                .sum()
        };
        for case in cases {
            let note = case["note"].as_str().unwrap();
            let mut text = case["text"].as_str().unwrap().to_owned();
            let (shown, indices) = projection(&text);
            let wanted = case["caret"].as_u64().unwrap() as usize;
            let offset = (0..=shown.chars().count())
                .find(|offset| utf16(&text, *offset) == wanted)
                .unwrap();
            let (mut caret, _) =
                formatting::source_selection(&indices, text.chars().count(), offset, offset);
            let mut typing = Typing::new("");
            typing.ended(&text, caret, caret);
            for step in case["steps"].as_array().unwrap() {
                if step["placed"].as_bool().unwrap() {
                    typing.ended(&text, caret, caret);
                }
                let key = step["key"].as_str().unwrap();
                let mut next: Vec<char> = text.chars().collect();
                next.splice(caret..caret, key.chars());
                let typed = caret + key.chars().count();
                let (shown, start, end) =
                    typing.edit(&next.into_iter().collect::<String>(), typed, typed);
                assert_eq!(start, end, "{note}");
                (text, caret) = (shown, start);
                assert_eq!(
                    (
                        text.as_str(),
                        utf16(&text, visible(&text, caret)),
                        typing.pending
                    ),
                    (
                        step["text"].as_str().unwrap(),
                        step["caret"].as_u64().unwrap() as usize,
                        step["pending"].as_i64().unwrap() as i32
                    ),
                    "{note}"
                );
            }
        }
    }

    /// Markers typed by hand stay as typed: a pair that closes becomes its
    /// style, and markers that close nothing stay plain text.
    #[test]
    fn hand_typed_markers_keep_their_source() {
        for (source, shown) in [
            ("**bold**", "bold"),
            ("_it_", "it"),
            ("snake_case_name", "snake_case_name"),
            ("2*3*4", "2*3*4"),
            ("a ** b", "a ** b"),
        ] {
            let mut typing = Typing::new("");
            let mut text = String::new();
            for key in source.chars() {
                let caret = text.chars().count();
                text.push(key);
                (text, _, _) = typing.edit(&text.clone(), caret + 1, caret + 1);
            }
            assert_eq!(
                (text.as_str(), projection(&text).0.as_str()),
                (source, shown)
            );
        }
    }

    #[test]
    fn backspace_and_delete_remove_visible_graphemes() {
        // a, the three-scalar 👩‍💻 and b, inside bold's markers.
        let text = "**a👩‍💻b**";
        assert_eq!(deletion(text, 6, true), Some((3, 6)));
        assert_eq!(deletion(text, 9, true), Some((6, 7)));
        assert_eq!(deletion(text, 3, false), Some((3, 6)));
        assert_eq!(deletion(text, 2, true), None);
        assert_eq!(deletion(text, 9, false), None);
        assert_eq!(copied(text, 0, 9), "a👩‍💻b");
        assert_eq!(copied("keep **this**", 0, 11), "keep this");
        // What remains keeps its style.
        let mut typing = Typing::new("");
        typing.ended(text, 6, 6);
        let (after, caret, _) = typing.edit("**ab**", 3, 3);
        assert_eq!((after.as_str(), caret), ("**ab**", 3));
    }
}
