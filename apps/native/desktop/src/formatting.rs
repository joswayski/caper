//! Deliberately small Markdown subset, aligned with the web formatter.
use crate::egui;
#[path = "../../../../shared/messages/formatting.rs"]
mod parser;
pub use parser::runs;

/// Selection offsets are Unicode scalar positions, as used by egui.
pub fn toggle(
    text: &str,
    mut start: usize,
    mut end: usize,
    marker: &str,
) -> (String, usize, usize) {
    let c: Vec<char> = text.chars().collect();
    if start != end {
        let whitespace = |c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}';
        while start < end && whitespace(c[start]) {
            start += 1;
        }
        while end > start && whitespace(c[end - 1]) {
            end -= 1;
        }
        if start == end {
            return (text.into(), start, end);
        }
    }
    let m: Vec<char> = marker.chars().collect();
    let w = m.len();
    let mut selected: String = c[start..end].iter().collect();
    let parsed = runs(&selected);
    let has_style = |run: &parser::Run| if marker == "**" { run.bold } else { run.italic };
    let (a, b, value, s, e) = if start >= w
        && c.get(start - w..start) == Some(&m[..])
        && c.get(end..end + w) == Some(&m[..])
    {
        (
            start - w,
            end + w,
            c[start..end].iter().collect::<String>(),
            start - w,
            end - w,
        )
    } else if end - start >= w * 2
        && c[start..end].starts_with(&m)
        && c[start..end].ends_with(&m)
        && parsed.iter().all(has_style)
    {
        (
            start,
            end,
            c[start + w..end - w].iter().collect(),
            start,
            end - w * 2,
        )
    } else {
        if parsed.iter().any(has_style) {
            let mut remaining: Vec<(String, bool)> = Vec::new();
            for run in parsed {
                let styled = if marker == "**" { run.italic } else { run.bold };
                if let Some(last) = remaining.last_mut().filter(|last| last.1 == styled) {
                    last.0.push_str(&run.text);
                } else {
                    remaining.push((run.text, styled));
                }
            }
            let other = if marker == "**" { "_" } else { "**" };
            selected = remaining
                .into_iter()
                .map(|(text, styled)| {
                    if styled {
                        format!("{other}{text}{other}")
                    } else {
                        text
                    }
                })
                .collect();
        }
        (
            start,
            end,
            format!("{marker}{selected}{marker}"),
            start + w,
            start + w + selected.chars().count(),
        )
    };
    (
        format!(
            "{}{}{}",
            c[..a].iter().collect::<String>(),
            value,
            c[b..].iter().collect::<String>()
        ),
        s,
        e,
    )
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

/// Small selection toolbar; drafts intentionally keep their Markdown markers.
pub fn controls(ui: &mut egui::Ui, text: &mut String, response: &egui::Response, enabled: bool) {
    if !enabled || !response.enabled() {
        return;
    }
    let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) else {
        return;
    };
    let Some(range) = state.cursor.char_range() else {
        return;
    };
    let [start, end] = range.sorted_cursors().map(|c| c.index);
    let mut chosen = None;
    if response.has_focus() {
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
    if start != end && (response.has_focus() || over_toolbar) {
        if ui.layer_id().order == egui::Order::Foreground {
            ui.ctx().set_sublayer(
                ui.layer_id(),
                egui::LayerId::new(egui::Order::Foreground, toolbar_id),
            );
        }
        egui::Area::new(toolbar_id)
            .order(egui::Order::Foreground)
            .default_size(egui::vec2(64.0, 28.0))
            .movable(false)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(response.rect.left_top() - egui::vec2(0.0, 8.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .small_button(egui::RichText::new("B").strong())
                            .on_hover_text("Bold · Ctrl/Cmd B")
                            .clicked()
                        {
                            chosen = Some("**");
                        }
                        if ui
                            .small_button(egui::RichText::new("I").italics())
                            .on_hover_text("Italic · Ctrl/Cmd I")
                            .clicked()
                        {
                            chosen = Some("_");
                        }
                    });
                });
            });
    }
    if let Some(marker) = chosen {
        let (next, start, end) = toggle(text, start, end, marker);
        if next.chars().count() <= 4000 {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toggles_preserve_unicode_and_selection() {
        for (text, start, end, marker, expected, s, e) in [
            ("é🙂", 0, 2, "**", "**é🙂**", 2, 4),
            ("**é🙂**", 2, 4, "**", "é🙂", 0, 2),
            ("_hello_", 0, 7, "_", "hello", 0, 5),
            ("hi", 2, 2, "_", "hi__", 3, 3),
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
        let id = egui::Id::new("format-test");
        let mut text = "é🙂".to_owned();
        let frame = |text: &mut String, events, enabled| {
            let _ = context.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut output = egui::TextEdit::multiline(text).id(id).show(ui);
                        composer_controls(ui, text, &mut output, enabled);
                        output.response.request_focus();
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
            2..4
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
