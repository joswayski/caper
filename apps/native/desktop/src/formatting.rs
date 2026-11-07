//! Deliberately small Markdown subset, aligned with the web formatter.
use crate::egui;
#[cfg(test)]
use caper_client::formatting::parser;
pub use caper_client::formatting::{
    adjacent_grapheme, edit, projection, runs, source_runs, toggle,
};

fn source_selection(
    indices: &[usize],
    len: usize,
    primary: usize,
    secondary: usize,
) -> egui::text::CCursorRange {
    let (primary, secondary) =
        caper_client::formatting::source_selection(indices, len, primary, secondary);
    egui::text::CCursorRange {
        primary: egui::text::CCursor::new(primary),
        secondary: egui::text::CCursor::new(secondary),
        ..Default::default()
    }
}

pub fn normalized_selection(
    text: &str,
    range: egui::text::CCursorRange,
) -> egui::text::CCursorRange {
    let (primary, secondary) = caper_client::formatting::normalized_selection(
        text,
        range.primary.index.0,
        range.secondary.index.0,
    );
    egui::text::CCursorRange {
        primary: egui::text::CCursor::new(primary),
        secondary: egui::text::CCursor::new(secondary),
        ..Default::default()
    }
}

pub fn copied_text(text: &str, range: egui::text::CCursorRange) -> String {
    let (visible, indices) = projection(text);
    let [start, end] = range
        .sorted_cursors()
        .map(|cursor| indices.partition_point(|index| *index < cursor.index.0));
    visible.chars().skip(start).take(end - start).collect()
}

/// Run before TextEdit: movement and deletion target visible graphemes. Actual text
/// changes are still performed by egui, preserving its normal undo transaction.
pub fn prepare(ui: &mut egui::Ui, text: &str, id: egui::Id, enabled: bool) {
    if !enabled || !ui.is_enabled() || !ui.memory(|memory| memory.has_focus(id)) {
        return;
    }
    let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) else {
        return;
    };
    let Some(mut range) = state.cursor.char_range() else {
        return;
    };
    let ime_id = id.with("format-ime");
    let mut composing = ui.data_mut(|data| data.get_temp::<bool>(ime_id).unwrap_or(false));
    let mut ime_event = false;
    ui.input(|input| {
        for event in &input.events {
            if let egui::Event::Ime(event) = event {
                ime_event = true;
                composing =
                    matches!(event, egui::ImeEvent::Preedit { text, .. } if !text.is_empty());
            }
        }
    });
    ui.data_mut(|data| data.insert_temp(ime_id, composing));
    if composing || ime_event {
        return;
    }
    range = normalized_selection(text, range);
    let (visible, indices) = projection(text);
    let mut copied = None;
    ui.input_mut(|input| {
        input.events.retain_mut(|event| {
            let primary = indices.partition_point(|index| *index < range.primary.index.0);
            let secondary = indices.partition_point(|index| *index < range.secondary.index.0);
            match event {
                egui::Event::Copy | egui::Event::Cut => {
                    if primary == secondary {
                        return false;
                    }
                    copied = Some(copied_text(text, range));
                    if matches!(event, egui::Event::Copy) {
                        return false;
                    }
                    *event = egui::Event::Key {
                        key: egui::Key::Backspace,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    };
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if !modifiers.command && !modifiers.ctrl && !modifiers.alt => {
                    let backward = matches!(key, egui::Key::ArrowLeft | egui::Key::Backspace);
                    if matches!(key, egui::Key::ArrowLeft | egui::Key::ArrowRight) {
                        let next = if primary != secondary && !modifiers.shift {
                            if backward {
                                primary.min(secondary)
                            } else {
                                primary.max(secondary)
                            }
                        } else {
                            adjacent_grapheme(&visible, primary, backward)
                        };
                        range = source_selection(
                            &indices,
                            text.chars().count(),
                            next,
                            if modifiers.shift { secondary } else { next },
                        );
                        return false;
                    }
                    if matches!(key, egui::Key::Backspace | egui::Key::Delete)
                        && primary == secondary
                    {
                        let next = adjacent_grapheme(&visible, primary, backward);
                        if next == primary {
                            return false;
                        }
                        range = source_selection(&indices, text.chars().count(), next, primary);
                    }
                }
                _ => {}
            }
            true
        });
    });
    if let Some(copied) = copied {
        ui.ctx().copy_text(copied);
    }
    state.cursor.set_char_range(Some(range));
    state.store(ui.ctx(), id);
}

pub fn composer_controls(
    ui: &mut egui::Ui,
    text: &mut String,
    output: &mut egui::text_edit::TextEditOutput,
    enabled: bool,
) {
    controls(ui, text, &output.response, enabled);
    if let Some(state) = egui::TextEdit::load_state(ui.ctx(), output.response.id) {
        output.cursor_range = state.cursor.char_range();
        output.state = state;
    }
}

/// egui collapses an unfocused editor's selection to its caret every pass, and
/// the emoji picker and toolbar buttons take focus before they read it. Keep
/// the editor's last focused selection while the draft is unchanged.
pub fn keep_selection(ui: &egui::Ui, text: &str, output: &mut egui::text_edit::TextEditOutput) {
    let id = output.response.id;
    let kept_id = id.with("kept-selection");
    let digest = egui::util::hash(text);
    if output.response.has_focus() {
        let range = output.state.cursor.char_range();
        ui.data_mut(|data| data.insert_temp(kept_id, (digest, range)));
    } else if let Some((kept, Some(range))) =
        ui.data(|data| data.get_temp::<(u64, Option<egui::text::CCursorRange>)>(kept_id))
        && kept == digest
        && output.state.cursor.char_range() != Some(range)
    {
        output.state.cursor.set_char_range(Some(range));
        output.cursor_range = Some(range);
        output.state.clone().store(ui.ctx(), id);
    }
}

/// Small selection toolbar; the draft retains wire markers, hidden by its layouter.
/// Ends the style `controls` carries between keys when the field `id` shows
/// another conversation, even one with an identical draft.
pub fn typing_in(
    ui: &egui::Ui,
    id: egui::Id,
    conversation: impl std::hash::Hash + std::fmt::Debug,
) {
    let (key, conversation) = (id.with("format-conversation"), id.with(conversation));
    ui.data_mut(|data| {
        if data.get_temp::<egui::Id>(key) != Some(conversation) {
            data.remove::<(i32, usize, String)>(id.with("format-pending"));
            data.insert_temp(key, conversation);
        }
    });
}

pub fn controls(ui: &mut egui::Ui, text: &mut String, response: &egui::Response, enabled: bool) {
    if !enabled || !response.enabled() {
        return;
    }
    let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) else {
        return;
    };
    let Some(mut range) = state.cursor.char_range() else {
        return;
    };
    let history_id = response.id.with("format-source");
    let previous = ui.data_mut(|data| data.get_temp::<String>(history_id));
    // An IME owns its marked text; undo/redo must restore the exact saved source.
    let native_transaction = ui.data_mut(|data| data.get_temp::<bool>(response.id.with("format-ime")).unwrap_or(false)) || ui.input(|input| input.raw.events.iter().any(|event| {
        matches!(event, egui::Event::Ime(_)) || matches!(event, egui::Event::Key { key: egui::Key::Z | egui::Key::Y, modifiers, .. } if modifiers.command)
    }));
    // The style web carries to a key typed at the caret the last key left, in
    // the text it left (an edit made elsewhere, such as an IME's, ends it).
    let pending_id = response.id.with("format-pending");
    let (mut pending, left, left_text) = ui
        .data(|data| data.get_temp::<(i32, usize, String)>(pending_id))
        .unwrap_or((-1, usize::MAX, String::new()));
    if response.changed()
        && !native_transaction
        && let Some(previous) = previous
    {
        let typed = text.chars().count().checked_sub(previous.chars().count());
        let continuing = if range.primary == range.secondary
            && typed.and_then(|count| range.primary.index.0.checked_sub(count)) == Some(left)
            && previous == left_text
        {
            pending
        } else {
            -1
        };
        let edited = edit(
            &previous,
            text,
            range.primary.index.0,
            range.secondary.index.0,
            continuing,
        );
        pending = edited.pending;
        if edited.text != *text
            || edited.start != range.primary.index.0
            || edited.end != range.secondary.index.0
        {
            *text = edited.text;
            range.primary.index = edited.start.into();
            range.secondary.index = edited.end.into();
            state.cursor.set_char_range(Some(range));
            state.clone().store(ui.ctx(), response.id);
            ui.ctx().request_repaint();
        }
    }
    if !native_transaction {
        range = normalized_selection(text, range);
        state.cursor.set_char_range(Some(range));
        state.clone().store(ui.ctx(), response.id);
    }
    let caret = if range.primary == range.secondary {
        range.primary.index.0
    } else {
        usize::MAX
    };
    // A moved caret, undo, redo, an IME, another draft or leaving the field ends it.
    if !response.has_focus()
        || native_transaction
        || !response.changed() && (caret != left || *text != left_text)
    {
        pending = -1;
    }
    ui.data_mut(|data| data.insert_temp(pending_id, (pending, caret, text.clone())));
    let [start, end] = range.sorted_cursors().map(|c| c.index.0);
    let mut chosen = None;
    if response.has_focus() && !native_transaction {
        ui.input_mut(|i| {
            if i.consume_key(egui::Modifiers::COMMAND, egui::Key::B) {
                chosen = Some("**");
            }
            if i.consume_key(egui::Modifiers::COMMAND, egui::Key::I) {
                chosen = Some("_");
            }
        });
    }
    let toolbar_id = response.id.with("format-toolbar");
    let over_toolbar = ui
        .ctx()
        .memory(|memory| memory.area_rect(toolbar_id))
        .is_some_and(|rect| {
            ui.ctx()
                .pointer_hover_pos()
                .is_some_and(|pos| rect.contains(pos))
        });
    let toolbar_focused = ui.memory(|memory| {
        ["**", "_"]
            .iter()
            .any(|marker| memory.has_focus(toolbar_id.with(marker)))
    });
    if !native_transaction
        && start != end
        && (response.has_focus() || over_toolbar || toolbar_focused)
    {
        if ui.layer_id().order == egui::Order::Foreground {
            ui.ctx().set_sublayer(
                ui.layer_id(),
                egui::LayerId::new(egui::Order::Foreground, toolbar_id),
            );
        }
        egui::Area::new(toolbar_id)
            .order(egui::Order::Foreground)
            .default_size(egui::vec2(72.0, 36.0))
            .movable(false)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(response.rect.left_top() - egui::vec2(0.0, 8.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(crate::SURFACE)
                    .corner_radius(8)
                    .shadow(ui.visuals().popup_shadow)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                        ui.horizontal(|ui| {
                            for (marker, glyph, label) in
                                [("**", "B", "Bold"), ("_", "I", "Italic")]
                            {
                                let (_, rect) = ui.allocate_space(egui::vec2(36.0, 36.0));
                                let button = ui.interact(
                                    rect,
                                    toolbar_id.with(marker),
                                    egui::Sense::click(),
                                );
                                if button.hovered()
                                    || button.has_focus()
                                    || button.is_pointer_button_down_on()
                                {
                                    let corners = if marker == "**" {
                                        egui::CornerRadius {
                                            nw: 8,
                                            sw: 8,
                                            ne: 0,
                                            se: 0,
                                        }
                                    } else {
                                        egui::CornerRadius {
                                            nw: 0,
                                            sw: 0,
                                            ne: 8,
                                            se: 8,
                                        }
                                    };
                                    ui.painter().rect_filled(
                                        rect,
                                        corners,
                                        if button.is_pointer_button_down_on() {
                                            crate::BORDER
                                        } else {
                                            crate::SURFACE_HOVER
                                        },
                                    );
                                }
                                let font = egui::FontId::new(
                                    14.0,
                                    if marker == "**" {
                                        egui::FontFamily::Name("Satoshi Bold".into())
                                    } else {
                                        egui::FontFamily::Proportional
                                    },
                                );
                                let mut job = egui::text::LayoutJob::simple(
                                    glyph.into(),
                                    font,
                                    crate::TEXT,
                                    f32::INFINITY,
                                );
                                job.sections[0].format.italics = marker == "_";
                                let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                                ui.painter().galley(
                                    rect.center() - galley.size() / 2.0,
                                    galley,
                                    crate::TEXT,
                                );
                                button.widget_info(|| {
                                    egui::WidgetInfo::labeled(egui::Role::Button, true, label)
                                });
                                if button
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text(format!(
                                        "{label} · Ctrl/Cmd {}",
                                        if marker == "**" { "B" } else { "I" }
                                    ))
                                    .clicked()
                                {
                                    chosen = Some(marker);
                                }
                            }
                        });
                    });
            });
    }
    if let Some(marker) = chosen {
        let (next, start, end) = toggle(text, start, end, marker);
        if next != *text && next.chars().count() <= 4000 {
            let mut undoer = state.undoer();
            undoer.add_undo(&(range, text.clone()));
            state.set_undoer(undoer);
            *text = next;
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(start),
                    egui::text::CCursor::new(end),
                )));
            state.store(ui.ctx(), response.id);
            response.request_focus();
            ui.ctx().request_repaint();
        }
    }
    ui.data_mut(|data| data.insert_temp(history_id, text.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestPass as _;
    use caper_client::formatting::repair_edit;
    #[test]
    fn visible_navigation_clipboard_and_link_edges() {
        let text = "**a👩‍💻b**";
        let caret = |index: usize| egui::text::CCursorRange::one(egui::text::CCursor::new(index));
        assert_eq!(normalized_selection(text, caret(1)).primary.index.0, 2);
        // Past the last visible character the caret stays after the closing markers,
        // so typing there keeps the style instead of breaking the pair.
        assert_eq!(
            normalized_selection("**bold**", caret(8)).primary.index.0,
            8
        );
        assert_eq!(
            normalized_selection("**bold**", caret(6)).primary.index.0,
            8
        );
        assert_eq!(
            repair_edit("**bold**", "**bold** ", 9, 9).0,
            "**bold** ",
            "typing after a closed run keeps its markers"
        );
        assert_eq!(adjacent_grapheme("a👩‍💻b", 1, false), 4);
        assert_eq!(adjacent_grapheme("a👩‍💻b", 4, true), 1);
        assert_eq!(
            copied_text(
                text,
                egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(9)
                )
            ),
            "a👩‍💻b"
        );
        assert_eq!(repair_edit("**hello**", "**new**", 5, 5).0, "**new**");
        let url = "https://example.com/a_b";
        for edited in [format!("** {url}**"), format!("**{url} **")] {
            let result = repair_edit(&format!("**{url}**"), &edited, 3, 3).0;
            assert_eq!(
                crate::links::segments(&result)
                    .into_iter()
                    .filter_map(|segment| segment.href)
                    .collect::<Vec<_>>(),
                [url]
            );
            assert_eq!(projection(&result).0, edited.replace("**", ""));
        }
    }

    #[test]
    fn native_editor_deletes_visible_graphemes_and_undo_restores_source() {
        for original in ["**a👩‍💻b**", "**https://example.com/a👩‍💻b**"] {
            let context = egui::Context::default();
            crate::configure(&context);
            let id = egui::Id::unique("visible-edit-test");
            let mut text = original.to_owned();
            let frame = |text: &mut String, events| {
                context.test_pass(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        ctx.root_ui(|root| {
                            egui::CentralPanel::default().show(root, |ui| {
                                prepare(ui, text, id, true);
                                let mut layouter =
                                    |ui: &egui::Ui, text: &dyn egui::TextBuffer, width| {
                                        crate::draft_layout(
                                            ui,
                                            text.as_str(),
                                            width,
                                            &[],
                                            crate::mentions::Specials::default(),
                                            egui::FontId::proportional(13.6),
                                        )
                                    };
                                let background = ui.painter().add(egui::Shape::Noop);
                                let mut output = egui::TextEdit::multiline(text)
                                    .id(id)
                                    .desired_width(140.0)
                                    .frame(egui::Frame::NONE)
                                    .layouter(&mut layouter)
                                    .show(ui);
                                crate::paint_draft_links(ui, &output, background);
                                composer_controls(ui, text, &mut output, true);
                                output.response.request_focus();
                            })
                        });
                    },
                )
            };
            let key = |key, modifiers| {
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }]
            };
            frame(&mut text, vec![]);
            let mut state = egui::TextEdit::load_state(&context, id).unwrap();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(original.chars().count() - 3),
                )));
            state.store(&context, id);
            frame(&mut text, key(egui::Key::Backspace, egui::Modifiers::NONE));
            assert_eq!(text, original.replace("👩‍💻", ""));
            frame(&mut text, key(egui::Key::Z, egui::Modifiers::COMMAND));
            assert_eq!(text, original);
            frame(&mut text, key(egui::Key::A, egui::Modifiers::COMMAND));
            let output = frame(&mut text, vec![egui::Event::Copy]);
            assert!(output.platform_output.commands.iter().any(
            |command| matches!(command, egui::OutputCommand::CopyText(value) if value == &projection(original).0)
        ));
            frame(&mut text, vec![egui::Event::Text("new".into())]);
            assert_eq!(text, "**new**");
            frame(&mut text, key(egui::Key::A, egui::Modifiers::COMMAND));
            frame(&mut text, key(egui::Key::Backspace, egui::Modifiers::NONE));
            assert_eq!(text, "");
        }
    }

    /// Whole drafts typed one key per real egui frame, closing markers and
    /// the spaces after them included, leave web's source after every key
    /// (shared/messages/composer-cases.json, "a whole draft typed").
    #[test]
    fn whole_drafts_type_as_web_through_egui_frames() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../shared/messages/composer-cases.json"
        ))
        .unwrap();
        let drafts: Vec<_> = cases["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| {
                case["note"]
                    .as_str()
                    .unwrap()
                    .starts_with("a whole draft typed")
            })
            .collect();
        assert!(drafts.len() >= 10);
        let context = egui::Context::default();
        crate::configure(&context);
        let id = egui::Id::unique("whole-draft-test");
        let frame = |text: &mut String, conversation: usize, events| {
            context.test_pass(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    ctx.root_ui(|root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            typing_in(ui, id, conversation);
                            prepare(ui, text, id, true);
                            let mut output = egui::TextEdit::multiline(text).id(id).show(ui);
                            composer_controls(ui, text, &mut output, true);
                            output.response.request_focus();
                        })
                    });
                },
            );
        };
        for (conversation, case) in drafts.into_iter().enumerate() {
            let note = case["note"].as_str().unwrap();
            let mut text = String::new();
            frame(&mut text, conversation, vec![]);
            frame(&mut text, conversation, vec![]);
            for step in case["steps"].as_array().unwrap() {
                assert!(!step["placed"].as_bool().unwrap(), "{note}");
                let key = step["key"].as_str().unwrap();
                frame(&mut text, conversation, vec![egui::Event::Text(key.into())]);
                assert_eq!(text, step["text"].as_str().unwrap(), "{note}, key {key:?}");
            }
            frame(
                &mut text,
                conversation,
                vec![egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }],
            );
            frame(&mut text, conversation, vec![egui::Event::Text("*".into())]);
            assert_eq!(
                text, "*",
                "{note}, replace the visible draft with a literal"
            );
        }
    }

    /// The style web carries to the next key continues only while the field
    /// shows what typing left: a moved caret, undo and redo, or another
    /// conversation with an identical draft type plain again.
    #[test]
    fn composer_typing_ends_when_the_field_changes_under_it() {
        let context = egui::Context::default();
        crate::configure(&context);
        let id = egui::Id::unique("typing-lifecycle-test");
        let frame = |text: &mut String, conversation: u8, events| {
            context.test_pass(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    ctx.root_ui(|root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            typing_in(ui, id, conversation);
                            prepare(ui, text, id, true);
                            let mut output = egui::TextEdit::multiline(text).id(id).show(ui);
                            composer_controls(ui, text, &mut output, true);
                            output.response.request_focus();
                        })
                    });
                },
            );
        };
        let caret = |at: usize| {
            let mut state = egui::TextEdit::load_state(&context, id).unwrap();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(at),
                )));
            state.store(&context, id);
        };
        let key = |key| {
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }]
        };
        let typed = |key: &str| vec![egui::Event::Text(key.into())];
        let start = |text: &mut String, conversation| {
            *text = "**bold**".into();
            frame(text, conversation, vec![]);
            caret(8);
            frame(text, conversation, vec![]);
            frame(text, conversation, typed(" "));
            frame(text, conversation, vec![]);
            assert_eq!(text, "**bold** ");
        };
        let mut text = String::new();
        start(&mut text, 0);
        frame(&mut text, 0, typed("a"));
        assert_eq!(text, "**bold** **a**", "continuous typing");
        start(&mut text, 1);
        caret(0);
        frame(&mut text, 1, vec![]);
        caret(9);
        frame(&mut text, 1, vec![]);
        frame(&mut text, 1, typed("a"));
        assert_eq!(text, "**bold** a", "caret moved away and back");
        start(&mut text, 2);
        frame(&mut text, 2, key(egui::Key::Z));
        frame(&mut text, 2, key(egui::Key::Y));
        caret(9);
        frame(&mut text, 2, vec![]);
        frame(&mut text, 2, typed("a"));
        assert_eq!(text, "**bold** a", "undo then redo");
        start(&mut text, 3);
        frame(&mut text, 4, vec![]);
        frame(&mut text, 4, typed("a"));
        assert_eq!(
            text, "**bold** a",
            "identical draft in another conversation"
        );
    }

    #[test]
    fn edits_discard_only_orphaned_delimiters() {
        for (before, after, caret, expected, expected_caret) in [
            ("**hello**", "****", 2, "", 0),
            ("**_🙂_**", "**__**", 3, "", 0),
            ("a **bold** z", "a ** z", 4, "a  z", 2),
            ("**hello** tail", "**llo** tail", 2, "**llo** tail", 2),
            ("a **", "a *", 3, "a *", 3),
        ] {
            assert_eq!(
                repair_edit(before, after, caret, caret),
                (expected.into(), expected_caret, expected_caret)
            );
        }
        assert_eq!(
            toggle("**hello** rest", 2, 9, "**"),
            ("hello rest".into(), 0, 5)
        );
        assert_eq!(toggle("identifier", 2, 5, "_"), ("identifier".into(), 2, 5));
    }

    #[test]
    fn formatted_completion_keeps_wire_ranges() {
        let emoji = crate::emoji::token("**:roc**", 6).unwrap();
        assert_eq!((emoji.start, emoji.end), (2, 6));
        assert_eq!(
            crate::emoji::insert("**:roc**", &emoji, "🚀").unwrap(),
            ("**🚀**".into(), 3)
        );
        let mention = crate::mentions::token("**@al**", 5).unwrap();
        assert_eq!(
            (mention.start, mention.end, mention.query.as_str()),
            (2, 5, "al")
        );
        assert_eq!(
            crate::mentions::insert("**@al**", &mention, "alice").unwrap(),
            ("**@alice** ".into(), 11)
        );
        let crossing = crate::emoji::Token {
            start: 0,
            end: 7,
            query: String::new(),
        };
        assert_eq!(
            crate::emoji::insert("**hello** tail", &crossing, "🚀").unwrap(),
            ("🚀 tail".into(), 1)
        );
    }

    #[test]
    fn source_projection_matches_shared_fixtures_without_losing_source_bytes() {
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../shared/messages/formatting-cases.json"
        ))
        .unwrap();
        for fixture in fixtures["cases"].as_array().unwrap() {
            let text = fixture["text"].as_str().unwrap();
            let source = source_runs(text);
            assert_eq!(
                source
                    .iter()
                    .map(|run| &text[run.range.clone()])
                    .collect::<String>(),
                text
            );
            let mut projected: Vec<parser::Run> = Vec::new();
            for run in source.into_iter().filter(|run| !run.hidden) {
                let value = &text[run.range];
                if let Some(last) = projected
                    .last_mut()
                    .filter(|last| last.bold == run.bold && last.italic == run.italic)
                {
                    last.text.push_str(value);
                } else {
                    projected.push(parser::Run {
                        text: value.into(),
                        bold: run.bold,
                        italic: run.italic,
                    });
                }
            }
            assert_eq!(projected, runs(text), "{text}");
        }
    }

    #[test]
    fn formatting_cannot_split_urls_but_can_wrap_them() {
        let text = "🙂 https://example.com/a_b?q=one&x=two tail";
        let start = text[..text.find("https:").unwrap()].chars().count();
        let end = text[..text.find(" tail").unwrap()].chars().count();
        for marker in ["**", "_"] {
            for (a, b) in [
                (start + 2, start + 2),
                (start + 2, end),
                (0, end - 2),
                (start, end - 1),
            ] {
                assert_eq!(toggle(text, a, b, marker), (text.into(), a, b));
            }
            let (wrapped, _, _) = toggle(text, start, end, marker);
            assert_eq!(
                runs(&wrapped)
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>(),
                text
            );
            assert_eq!(
                crate::links::segments(&wrapped)
                    .into_iter()
                    .filter_map(|part| part.href)
                    .collect::<Vec<_>>(),
                crate::links::segments(text)
                    .into_iter()
                    .filter_map(|part| part.href)
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn toggles_preserve_unicode_and_selection() {
        for (text, start, end, marker, expected, s, e) in [
            ("é🙂", 0, 2, "**", "**é🙂**", 2, 4),
            ("**é🙂**", 2, 4, "**", "é🙂", 0, 2),
            ("_hello_", 0, 7, "_", "hello", 0, 5),
            ("hi", 2, 2, "_", "hi", 2, 2),
            (" word \n", 0, 7, "_", " _word_ \n", 2, 6),
            (" \t", 0, 2, "**", " \t", 2, 2),
            (
                "**bold** plus normal",
                0,
                20,
                "**",
                "**bold plus normal**",
                2,
                18,
            ),
            ("**one** and **two**", 0, 19, "**", "**one and two**", 2, 13),
            ("**bold _both_**", 0, 15, "_", "_**bold both**_", 1, 14),
        ] {
            assert_eq!(toggle(text, start, end, marker), (expected.into(), s, e));
        }
    }

    #[test]
    fn composer_shortcuts_retain_selection_and_respect_limits_and_disabled_state() {
        let context = egui::Context::default();
        crate::configure(&context);
        let id = egui::Id::unique("format-test");
        let mut text = "é🙂".to_owned();
        let frame = |text: &mut String, events, enabled| {
            let _ = context.test_pass(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    ctx.root_ui(|root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            let mut output = egui::TextEdit::multiline(text).id(id).show(ui);
                            composer_controls(ui, text, &mut output, enabled);
                            output.response.request_focus();
                        })
                    });
                },
            );
        };
        frame(&mut text, vec![], true);
        let mut state = egui::TextEdit::load_state(&context, id).unwrap();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(2),
            )));
        state.store(&context, id);
        let key = || {
            vec![egui::Event::Key {
                key: egui::Key::B,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }]
        };
        frame(&mut text, key(), true);
        assert_eq!(text, "**é🙂**");
        assert_eq!(
            egui::TextEdit::load_state(&context, id)
                .unwrap()
                .cursor
                .char_range()
                .unwrap()
                .as_sorted_char_range(),
            2.into()..4.into()
        );
        frame(&mut text, key(), false);
        assert_eq!(text, "**é🙂**");
        frame(&mut text, key(), true);
        assert_eq!(text, "é🙂");
        text = "x".repeat(4000);
        frame(&mut text, key(), true);
        assert_eq!(text.chars().count(), 4000);
    }
}
