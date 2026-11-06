use eframe::egui;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use unicode_segmentation::UnicodeSegmentation;

pub use caper_client::emoji::*;

/// Lay out names with a square for each emoji, independent of installed fonts.
/// Keep the original Unicode text for accessibility. Each emoji is one
/// section whose leading space pads its shaped width out to one em.
pub fn layout_job(ui: &egui::Ui, mut job: egui::text::LayoutJob) -> Arc<egui::Galley> {
    let emojis = inline(&job.text);
    if !emojis.is_empty() {
        let sections = std::mem::take(&mut job.sections);
        for section in sections {
            let mut start = section.byte_range.start.0;
            let mut gap = section.leading_space;
            for (range, _) in emojis.iter().filter(|(range, _)| {
                range.start >= section.byte_range.start.0 && range.end <= section.byte_range.end.0
            }) {
                if start < range.start {
                    job.sections.push(egui::text::LayoutSection {
                        leading_space: gap,
                        byte_range: start.into()..range.start.into(),
                        format: section.format.clone(),
                    });
                    gap = 0.0;
                }
                // Shaping lays a sequence out as one cluster, so measure it whole.
                let format = section.format.clone();
                let width = ui.fonts_mut(|fonts| {
                    fonts
                        .layout_no_wrap(
                            job.text[range.clone()].to_owned(),
                            format.font_id.clone(),
                            format.color,
                        )
                        .size()
                        .x
                });
                job.sections.push(egui::text::LayoutSection {
                    leading_space: gap + format.font_id.size - width,
                    byte_range: range.start.into()..range.end.into(),
                    format,
                });
                gap = 0.0;
                start = range.end;
            }
            if start < section.byte_range.end.0 {
                job.sections.push(egui::text::LayoutSection {
                    leading_space: gap,
                    byte_range: start.into()..section.byte_range.end,
                    format: section.format,
                });
            }
        }
    }
    let mut galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    if !emojis.is_empty() {
        // Hide only the actual emoji glyph vertices after layout. Keeping
        // normal font/color during layout makes an overflow ellipsis visible
        // even when truncation lands inside a multi-code-point emoji.
        let text = galley.job.text.clone();
        let row = Arc::make_mut(&mut Arc::make_mut(&mut galley).rows[0].row);
        for (range, _) in emojis {
            let start = text[..range.start].chars().count();
            // One glyph per character; continuations carry the cluster's
            // first character. Keep an overflow ellipsis visible.
            for index in start..start + text[range].chars().count() {
                if let Some(glyph) = row.glyphs.get(index)
                    && glyph.chr != '…'
                {
                    let first = glyph.first_vertex as usize;
                    let end = row
                        .glyphs
                        .get(index + 1)
                        .map_or(row.visuals.glyph_vertex_range.end, |next| {
                            next.first_vertex as usize
                        });
                    for vertex in &mut row.visuals.mesh.vertices[first..end] {
                        vertex.color = egui::Color32::TRANSPARENT;
                    }
                }
            }
        }
    }
    galley
}

/// Paint a name laid out above, including only emoji that survived truncation.
pub fn paint_galley(
    painter: &egui::Painter,
    position: egui::Pos2,
    galley: Arc<egui::Galley>,
    color: egui::Color32,
) {
    painter.galley(position, galley.clone(), color);
    paint_inline(painter, position, &galley);
}

/// Add artwork when the text itself is painted by an egui widget (a hint).
pub fn paint_inline(painter: &egui::Painter, position: egui::Pos2, galley: &egui::Galley) {
    let textures = Textures::shared(painter.ctx());
    for (range, entry) in inline(&galley.job.text) {
        let index = galley.job.text[..range.start].chars().count();
        let character = galley.job.text[range.clone()].chars().next().unwrap();
        // These are single-line labels. An overflow ellipsis replaces the
        // first omitted glyph; never paint the omitted emoji over that mark.
        if let Some(row) = galley.rows.first()
            && let Some(glyph) = row.glyphs.get(index)
            && glyph.chr == character
        {
            let size = galley
                .job
                .sections
                .iter()
                .find(|section| section.byte_range.start.0 == range.start)
                .unwrap()
                .format
                .font_id
                .size;
            let center = position
                + row.pos.to_vec2()
                + egui::vec2(glyph.max_x() - size / 2.0, glyph.logical_rect().center().y);
            textures.paint(
                painter,
                entry,
                egui::Rect::from_center_size(center, egui::vec2(size, size)),
            );
        }
    }
}

/// A non-selectable, single-line name label, with the same bundled artwork as
/// the sidebar. The full original name remains available to screen readers.
pub fn name_label(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    sense: egui::Sense,
) -> egui::Response {
    let mut job = Arc::unwrap_or_clone(text.into().into_layout_job(
        ui.style(),
        egui::FontSelection::Default,
        egui::Align::Center,
    ));
    job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width().max(1.0));
    let galley = layout_job(ui, job);
    let (rect, response) = ui.allocate_exact_size(galley.size(), sense);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), galley.text())
    });
    paint_galley(ui.painter(), rect.min, galley, ui.visuals().text_color());
    response
}

/// Observe the editor's rendered glyphs without adding interactive widgets or changing selection.
pub fn editor_tooltip(output: &egui::text_edit::TextEditOutput, text: &str) {
    let response = &output.response.response;
    let hold_id = response.id.with("emoji-name-hold");
    let held = response
        .ctx
        .data_mut(|data| data.remove_temp::<(egui::Pos2, String)>(hold_id));
    let Some(pointer) = response.ctx.input(|input| input.pointer.hover_pos()) else {
        return;
    };
    if !response.rect.contains(pointer) || !output.text_clip_rect.contains(pointer) {
        return;
    }
    let local = pointer - output.galley_pos;
    let mut scalar = 0;
    let mut hit = None;
    for row in &output.galley.rows {
        for glyph in &row.glyphs {
            if glyph
                .logical_rect()
                .translate(row.pos.to_vec2())
                .contains(local.to_pos2())
            {
                hit = Some(scalar);
                break;
            }
            scalar += 1;
        }
        if hit.is_some() {
            break;
        }
        scalar += usize::from(row.ends_with_newline);
    }
    let Some(hit) = hit else {
        return;
    };
    let mut start = 0;
    for grapheme in text.graphemes(true) {
        let end = start + grapheme.chars().count();
        if (start..end).contains(&hit) {
            if let Some(name) = name(grapheme) {
                let label = format!(":{name}:");
                let holding = response
                    .ctx
                    .input(|input| input.pointer.any_down() && !input.pointer.any_pressed())
                    && held.as_ref().is_some_and(|(origin, previous)| {
                        previous == &label && origin.distance(pointer) <= 10.0
                    });
                // long_touched is a one-frame event; keep displaying during the hold.
                if response.long_touched() || holding {
                    response.ctx.data_mut(|data| {
                        data.insert_temp(hold_id, held.unwrap_or((pointer, label.clone())));
                    });
                    response.show_tooltip_text(label);
                } else {
                    response.clone().on_hover_text_at_pointer(label);
                }
            }
            break;
        }
        start = end;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub query: String,
}

fn query_character(value: char) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, '_' | '+' | '-')
}

// egui cursors count Unicode scalars, not UTF-8 bytes or grapheme clusters.
pub fn token(text: &str, caret: usize) -> Option<Token> {
    if caret > text.chars().count() {
        return None;
    }
    let (visible, indices) = crate::formatting::projection(text);
    let caret = indices.partition_point(|index| *index < caret);
    let chars: Vec<_> = visible.chars().collect();
    if chars
        .get(caret)
        .is_some_and(|c| *c == ':' || query_character(*c))
    {
        return None;
    }
    let mut start = caret;
    while start > 0 && query_character(chars[start - 1]) {
        start -= 1;
    }
    let colon = start.checked_sub(1)?;
    if chars[colon] != ':'
        || (colon > 0 && !chars[colon - 1].is_whitespace() && !"([{".contains(chars[colon - 1]))
    {
        return None;
    }
    // One character is an emoticon (":D", ":P", ":3"), not a search; Enter sends it as typed.
    if caret - start == 1 {
        return None;
    }
    Some(Token {
        start: indices[colon],
        end: indices[caret - 1] + 1,
        query: chars[start..caret].iter().collect(),
    })
}

pub fn insert(text: &str, token: &Token, emoji: &str) -> Option<(String, usize)> {
    let value: String = text
        .chars()
        .take(token.start)
        .chain(emoji.chars())
        .chain(text.chars().skip(token.end))
        .collect();
    (value.chars().count() <= 4_000).then(|| {
        let caret = token.start + emoji.chars().count();
        let (value, caret, _) = crate::formatting::repair_edit(text, &value, caret, caret);
        (value, caret)
    })
}

/// The bundled sheets, uploaded once and shared: reactions, the picker and
/// message text (through `shared`) draw from the same textures.
#[derive(Default, Clone)]
pub struct Textures(Arc<Mutex<BTreeMap<usize, egui::TextureHandle>>>);

impl Textures {
    /// The context's shared sheets, so free functions drawing text reuse them.
    pub fn shared(context: &egui::Context) -> Self {
        let id = egui::Id::unique("emoji-textures");
        context.data_mut(|data| data.get_temp_mut_or_default::<Self>(id).clone())
    }

    fn texture(&self, context: &egui::Context, page: usize) -> egui::TextureHandle {
        let mut sheets = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sheets
            .entry(page)
            .or_insert_with(|| {
                let decoded = image::load_from_memory(sheet(page))
                    .expect("embedded emoji sheet is valid")
                    .to_rgba8();
                let dimensions = [decoded.width() as usize, decoded.height() as usize];
                context.load_texture(
                    format!("emoji-sheet-{page}"),
                    egui::ColorImage::from_rgba_unmultiplied(dimensions, decoded.as_raw()),
                    egui::TextureOptions::LINEAR,
                )
            })
            .clone()
    }

    /// Decode/upload only the opening window's bundled sheets. Repeated load,
    /// hover and focus intent reuses the same textures, without network work.
    pub fn preload(&self, context: &egui::Context, category: &str) {
        let mut entries: Vec<_> = catalog()
            .iter()
            .filter(|entry| entry.selectable && entry.category == category)
            .collect();
        entries.sort_by_key(|entry| picker_order(entry));
        for entry in entries.into_iter().take(128) {
            let _ = self.texture(context, entry.sheet);
        }
    }

    pub fn image(&self, ui: &egui::Ui, entry: &Entry, size: f32) -> egui::Image<'static> {
        let texture = self.texture(ui.ctx(), entry.sheet);
        let uv = Self::uv(entry);
        egui::Image::new((texture.id(), egui::vec2(size, size))).uv(uv)
    }

    /// Paints `entry` in color over `rect`, as message text does.
    pub fn paint(&self, painter: &egui::Painter, entry: &Entry, rect: egui::Rect) {
        let texture = self.texture(painter.ctx(), entry.sheet);
        painter.image(texture.id(), rect, Self::uv(entry), egui::Color32::WHITE);
    }

    fn uv(entry: &Entry) -> egui::Rect {
        egui::Rect::from_min_max(
            egui::pos2(entry.x as f32 / 1024.0, entry.y as f32 / 1024.0),
            egui::pos2(
                (entry.x + 64) as f32 / 1024.0,
                (entry.y + 64) as f32 / 1024.0,
            ),
        )
    }
}

fn sheet(page: usize) -> &'static [u8] {
    match page {
        0 => include_bytes!("../../../../shared/emoji/sheet-0.png"),
        1 => include_bytes!("../../../../shared/emoji/sheet-1.png"),
        2 => include_bytes!("../../../../shared/emoji/sheet-2.png"),
        3 => include_bytes!("../../../../shared/emoji/sheet-3.png"),
        4 => include_bytes!("../../../../shared/emoji/sheet-4.png"),
        5 => include_bytes!("../../../../shared/emoji/sheet-5.png"),
        6 => include_bytes!("../../../../shared/emoji/sheet-6.png"),
        7 => include_bytes!("../../../../shared/emoji/sheet-7.png"),
        8 => include_bytes!("../../../../shared/emoji/sheet-8.png"),
        9 => include_bytes!("../../../../shared/emoji/sheet-9.png"),
        10 => include_bytes!("../../../../shared/emoji/sheet-10.png"),
        11 => include_bytes!("../../../../shared/emoji/sheet-11.png"),
        12 => include_bytes!("../../../../shared/emoji/sheet-12.png"),
        13 => include_bytes!("../../../../shared/emoji/sheet-13.png"),
        14 => include_bytes!("../../../../shared/emoji/sheet-14.png"),
        _ => unreachable!("catalog references an unknown emoji sheet"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestPass as _;

    #[test]
    fn composer_names_follow_graphemes_and_persist_during_a_touch_hold() {
        let context = egui::Context::default();
        context.global_style_mut(|style| style.interaction.tooltip_delay = 0.0);
        let original = "prefix 🚀\n👩‍💻 🎎 suffix";
        let mut text = original.to_owned();
        let mut frame = |time, events| {
            let mut editor = None;
            let painted = context.test_pass(
                egui::RawInput {
                    time: Some(time),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |context| {
                    context.root_ui(|root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            let output = egui::TextEdit::multiline(&mut text).show(ui);
                            editor_tooltip(&output, &text);
                            editor = Some(output);
                        })
                    });
                },
            );
            (painted, editor.unwrap())
        };
        let (_, output) = frame(0.0, vec![]);
        let point = |prefix: &str| {
            output.galley_pos
                + output
                    .galley
                    .pos_from_cursor(egui::text::CCursor::new(prefix.chars().count()))
                    .center()
                    .to_vec2()
                + egui::vec2(3.0, 0.0)
        };
        let has_label = |painted: &egui::FullOutput, label| {
            painted.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == label)
            })
        };
        for (time, prefix, label) in [
            (1.0, "prefix ", ":rocket:"),
            (2.0, "prefix 🚀\n", ":woman-technologist:"),
            (3.0, "prefix 🚀\n👩‍💻 ", ":Japanese-dolls:"),
        ] {
            frame(time, vec![egui::Event::PointerMoved(point(prefix))]);
            frame(time + 0.1, vec![]);
            assert!(has_label(&frame(time + 0.2, vec![]).0, label), "{label}");
        }
        frame(4.0, vec![egui::Event::PointerMoved(point("pre"))]);
        assert!(!has_label(&frame(4.1, vec![]).0, ":Japanese-dolls:"));
        let pos = point("prefix 🚀\n👩‍💻 ");
        frame(
            5.0,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::Touch {
                    device_id: egui::TouchDeviceId(1),
                    id: egui::TouchId(1),
                    phase: egui::TouchPhase::Start,
                    pos,
                    force: None,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame(6.0, vec![]);
        assert!(has_label(&frame(6.1, vec![]).0, ":Japanese-dolls:"));
        assert!(has_label(&frame(6.4, vec![]).0, ":Japanese-dolls:"));
        assert_eq!(text, original, "inspection never changes the draft");
    }

    #[test]
    fn autocomplete_boundaries_search_and_unicode_insertion() {
        for value in ["word:tom", "12:30", "https://tom", ":tom:"] {
            assert!(token(value, value.chars().count()).is_none(), "{value}");
        }
        assert!(token(":tomato", 4).is_none());
        assert!(token(":tom:", 4).is_none());
        assert!(token("hi (:tom", 8).is_some());
        for text in [":D", "ok :P", "hi (:3"] {
            assert!(token(text, text.chars().count()).is_none(), "{text}");
        }
        assert!(token(":", 1).is_some_and(|token| token.query.is_empty()));
        let text = "👩‍💻 hi :roc suffix 🚀";
        let active = token(text, "👩‍💻 hi :roc".chars().count()).unwrap();
        assert_eq!(
            insert(text, &active, "🚀"),
            Some(("👩‍💻 hi 🚀 suffix 🚀".into(), "👩‍💻 hi 🚀".chars().count()))
        );
        let text = format!("{} :xy", "😀".repeat(3998));
        let active = token(&text, text.chars().count()).unwrap();
        assert!(insert(&text, &active, "🚀").is_some());
        assert!(insert(&text, &active, "👩‍💻").is_none());
    }
}
