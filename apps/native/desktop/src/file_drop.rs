//! The overlay over a conversation while files are dragged onto the window:
//! today's Caper character hops in front of three file cards that fan out
//! behind it. Web (`apps/web/src/chat/DropOverlay.tsx`), Apple and Android
//! draw the same pose and timing; every client's text matches
//! `shared/messages/drop-cases.json`. Change them together.

use eframe::egui::{self, Color32, Stroke};

use crate::uploads::MAX_ATTACHMENTS;

/// The overlay's title and detail line.
pub fn text(channel: &str, direct: bool, attached: usize) -> (String, String) {
    let room = MAX_ATTACHMENTS.saturating_sub(attached);
    let title = format!("Attach to {}{channel}", if direct { "" } else { "#" });
    let detail = if room == 0 {
        format!("{MAX_ATTACHMENTS} files attached. Remove one to add another.")
    } else if room == MAX_ATTACHMENTS {
        format!("Up to {MAX_ATTACHMENTS} files. Add a message before sending.")
    } else {
        format!(
            "{room} more file{}. Add a message before sending.",
            if room == 1 { "" } else { "s" }
        )
    };
    (title, detail)
}

/// A raised card with a folded corner around a 20-point Lucide glyph.
macro_rules! card {
    ($glyph:literal) => {
        concat!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="54" height="66" viewBox="0 0 54 66">"##,
            r##"<path d="M8 .5h31.5l14 14V58a7.5 7.5 0 0 1-7.5 7.5H8A7.5 7.5 0 0 1 .5 58V8A7.5 7.5 0 0 1 8 .5Z" fill="#1c1f21" stroke="#34383b"/>"##,
            r##"<path d="M39.5 .5v8a6 6 0 0 0 6 6h8" fill="#151719" stroke="#34383b"/>"##,
            r##"<g transform="translate(17 23) scale(0.8333)" fill="none" stroke="#b9bcbe" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">"##,
            $glyph,
            "</g></svg>"
        )
    };
}

/// Left to right: image, document, video. The middle card sits behind.
const CARDS: [(&str, &str); 3] = [
    (
        "bytes://file-drop/image.svg",
        card!(
            r#"<rect width="18" height="18" x="3" y="3" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#
        ),
    ),
    (
        "bytes://file-drop/document.svg",
        card!(
            r#"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M10 9H8"/><path d="M16 13H8"/><path d="M16 17H8"/>"#
        ),
    ),
    (
        "bytes://file-drop/video.svg",
        card!(
            r#"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M7 3v18M3 7.5h4M3 12h18M3 16.5h4M17 3v18M17 7.5h4M17 16.5h4"/>"#
        ),
    ),
];

/// Resting pose per card: horizontal and vertical offset, and turn in degrees.
const POSES: [(f32, f32, f32); 3] = [(-46.0, 10.0, -16.0), (0.0, -26.0, 0.0), (46.0, 10.0, 16.0)];
const ART: egui::Vec2 = egui::vec2(176.0, 128.0);
const CARD: egui::Vec2 = egui::vec2(54.0, 66.0);
const CHARACTER: f32 = 76.0;

fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Progress through an animation of `duration` seconds that starts after `delay`.
fn progress(elapsed: f32, delay: f32, duration: f32) -> f32 {
    ((elapsed - delay) / duration).clamp(0.0, 1.0)
}

/// 0 to 1 and back over 2.4 s each way, after the cards have fanned out.
fn sway(elapsed: f32, delay: f32) -> f32 {
    if elapsed < delay {
        return 0.0;
    }
    let phase = ((elapsed - delay) / 2.4) % 2.0;
    ease_in_out(if phase < 1.0 { phase } else { 2.0 - phase })
}

/// The character's hop: lift, and horizontal/vertical squash, over 1.6 s.
pub(crate) fn hop(elapsed: f32) -> (f32, f32, f32) {
    if elapsed < 0.5 {
        return (0.0, 1.0, 1.0);
    }
    let t = ((elapsed - 0.5) / 1.6) % 1.0;
    // Keyframes: rest, up and stretched at 25%, landed and squashed at 52%, rest from 60%.
    let frames = [
        (0.0, 0.0, 1.0, 1.0),
        (0.25, -9.0, 0.97, 1.03),
        (0.52, 0.0, 1.05, 0.95),
        (0.6, 0.0, 1.0, 1.0),
        (1.0, 0.0, 1.0, 1.0),
    ];
    let next = frames
        .iter()
        .position(|frame| frame.0 > t)
        .unwrap_or(frames.len() - 1);
    let (from, to) = (frames[next - 1], frames[next]);
    let k = ease_in_out((t - from.0) / (to.0 - from.0));
    let lerp = |a: f32, b: f32| a + (b - a) * k;
    (lerp(from.1, to.1), lerp(from.2, to.2), lerp(from.3, to.3))
}

/// Where the moving parts are, `elapsed` seconds after the drag arrived. The
/// card, fan and character arrive like other entrances (`--ease-out`, opacity
/// and translation only); then the cards sway and the character hops.
#[derive(Debug, PartialEq)]
struct Pose {
    /// The dimmed backdrop's opacity.
    fade: f32,
    /// The card's opacity, and how far it has risen into place.
    card_opacity: f32,
    card_rise: f32,
    /// The file cards' opacity, how far they have spread into place, and
    /// each one's sway.
    fan_opacity: f32,
    fan: f32,
    sway: [f32; 3],
    /// The character's opacity, how far it has risen, and its hop.
    character_opacity: f32,
    character_rise: f32,
    hop: (f32, f32, f32),
}

impl Pose {
    /// With reduced motion everything shows its resting pose at once.
    fn at(elapsed: f32, reduced: bool) -> Self {
        if reduced {
            return Self {
                fade: 1.0,
                card_opacity: 1.0,
                card_rise: 1.0,
                fan_opacity: 1.0,
                fan: 1.0,
                sway: [0.0; 3],
                character_opacity: 1.0,
                character_rise: 1.0,
                hop: (0.0, 1.0, 1.0),
            };
        }
        let ease = crate::motion::ease_out;
        let fan = progress(elapsed, 0.06, crate::motion::SLOW);
        let rise = progress(elapsed, 0.12, crate::motion::SLOW);
        Self {
            fade: progress(elapsed, 0.0, crate::motion::FAST),
            card_opacity: progress(elapsed, 0.0, 0.12),
            card_rise: ease(progress(elapsed, 0.0, crate::motion::SLOW)),
            fan_opacity: fan,
            fan: ease(fan),
            sway: [
                sway(elapsed, 0.48),
                sway(elapsed, 0.78),
                1.0 - sway(elapsed, 0.48),
            ],
            character_opacity: rise,
            character_rise: ease(rise),
            hop: hop(elapsed),
        }
    }
}

/// Paints the overlay over `area`, `elapsed` seconds after the drag arrived,
/// and returns whether it is still moving (it never stops unless the system
/// reduces motion). It never takes input: dropping stays with the window.
pub fn paint(
    context: &egui::Context,
    area: egui::Rect,
    elapsed: f32,
    character: (String, &'static [u8]),
    title: &str,
    detail: &str,
) -> bool {
    let reduced = crate::motion::reduced();
    let pose = Pose::at(elapsed, reduced);
    egui::Area::new(egui::Id::new("file-drop"))
        .order(egui::Order::Foreground)
        .fixed_pos(area.min)
        .interactable(false)
        .show(context, |ui| {
            ui.set_clip_rect(area);
            ui.painter().rect_filled(
                area,
                0.0,
                Color32::from_rgba_unmultiplied(12, 13, 15, (209.0 * pose.fade) as u8),
            );

            let width = (area.width() - 32.0).clamp(120.0, 360.0);
            let wrap = width - 48.0;
            let centered = |text: &str, font: egui::FontId, color: Color32| {
                let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, wrap);
                job.halign = egui::Align::Center;
                ui.fonts_mut(|fonts| fonts.layout_job(job))
            };
            let title = centered(
                title,
                egui::FontId::new(18.0, egui::FontFamily::Name("Satoshi Bold".into())),
                crate::TEXT,
            );
            let detail = centered(detail, egui::FontId::proportional(13.0), crate::MUTED);
            let height = 20.0 + ART.y + 8.0 + 6.0 + title.size().y + 6.0 + detail.size().y + 26.0;
            let card = egui::Rect::from_center_size(
                area.center() + egui::vec2(0.0, 10.0 * (1.0 - pose.card_rise)),
                egui::vec2(width, height),
            );
            ui.multiply_opacity(pose.card_opacity);
            let painter = ui.painter().clone();
            painter.add(
                egui::epaint::Shadow {
                    offset: [0, 18],
                    blur: 48,
                    spread: 0,
                    color: Color32::from_black_alpha(115),
                }
                .as_shape(card, 8.0),
            );
            painter.rect_filled(card, 8.0, crate::SURFACE);
            painter.rect_stroke(
                card,
                8.0,
                Stroke::new(1.0, crate::BORDER),
                egui::StrokeKind::Inside,
            );
            let dashed = card.shrink(9.75);
            painter.extend(egui::Shape::dashed_line(
                &[
                    dashed.left_top(),
                    dashed.right_top(),
                    dashed.right_bottom(),
                    dashed.left_bottom(),
                    dashed.left_top(),
                ],
                Stroke::new(1.5, crate::TERRACOTTA),
                5.0,
                4.0,
            ));

            let art = egui::Rect::from_min_size(
                egui::pos2(card.center().x - ART.x / 2.0, card.top() + 20.0),
                ART,
            );
            // The middle card first, behind the other two. Each slides out
            // and up from the middle at its resting turn.
            for index in [1, 0, 2] {
                let (x, y, turn) = POSES[index];
                let swayed = pose.sway[index];
                let pivot = art.min
                    + egui::vec2(
                        ART.x / 2.0 + x * pose.fan,
                        98.0 + 24.0 + (y - 24.0) * pose.fan - 3.0 * swayed,
                    );
                let rect = egui::Rect::from_min_max(
                    pivot - egui::vec2(CARD.x / 2.0, CARD.y),
                    pivot + egui::vec2(CARD.x / 2.0, 0.0),
                );
                let (uri, svg) = CARDS[index];
                egui::Image::from_bytes(uri, svg.as_bytes())
                    .rotate((turn + 4.0 * swayed).to_radians(), egui::vec2(0.5, 1.0))
                    .tint(Color32::WHITE.gamma_multiply(pose.fan_opacity))
                    .paint_at(ui, rect);
            }

            let (lift, squash_x, squash_y) = pose.hop;
            let bottom = art.center_bottom()
                + egui::vec2(0.0, 4.0 + 28.0 * (1.0 - pose.character_rise) + lift);
            let rect = egui::Rect::from_min_max(
                bottom - egui::vec2(CHARACTER * squash_x / 2.0, CHARACTER * squash_y),
                bottom + egui::vec2(CHARACTER * squash_x / 2.0, 0.0),
            );
            egui::Image::from_bytes(character.0, character.1)
                .tint(Color32::WHITE.gamma_multiply(pose.character_opacity))
                .paint_at(ui, rect);

            let mut top = art.bottom() + 8.0 + 6.0;
            painter.galley(egui::pos2(card.center().x, top), title.clone(), crate::TEXT);
            top += title.size().y + 6.0;
            painter.galley(egui::pos2(card.center().x, top), detail, crate::MUTED);
        });
    !reduced
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Cases {
        cases: Vec<Case>,
    }

    #[derive(serde::Deserialize)]
    struct Case {
        note: String,
        channel: String,
        direct: bool,
        attached: usize,
        title: String,
        detail: String,
    }

    #[test]
    fn shared_cases_produce_identical_text() {
        let cases: Cases =
            serde_json::from_str(include_str!("../../../../shared/messages/drop-cases.json"))
                .expect("valid shared drop cases");
        assert!(cases.cases.len() >= 5, "the shared cases loaded");
        for case in cases.cases {
            assert_eq!(
                text(&case.channel, case.direct, case.attached),
                (case.title, case.detail),
                "{}",
                case.note
            );
        }
    }

    #[test]
    fn animations_settle_at_the_resting_pose() {
        let rest = Pose::at(0.0, true);
        // Reduced motion shows the resting pose from the first frame.
        assert_eq!(Pose::at(5.0, true), rest);
        assert_eq!(
            (rest.fan, rest.character_rise, rest.sway),
            (1.0, 1.0, [0.0; 3])
        );
        let start = Pose::at(0.0, false);
        assert_eq!(
            (start.fade, start.fan_opacity, start.character_rise),
            (0.0, 0.0, 0.0)
        );
        // Entrances decelerate into place without overshooting.
        let mut previous = 0.0;
        for step in 0..=40 {
            let rise = Pose::at(0.12 + step as f32 * 0.01, false).character_rise;
            assert!((previous..=1.0).contains(&rise), "{rise} after {previous}");
            previous = rise;
        }
        let landed = Pose::at(0.4, false);
        assert_eq!(
            (landed.fan, landed.card_rise, landed.character_rise),
            (1.0, 1.0, 1.0)
        );
        assert_eq!(hop(0.0), (0.0, 1.0, 1.0));
        let (lift, _, _) = hop(0.5 + 1.6 * 0.25);
        assert!((lift + 9.0).abs() < 1e-3, "the top of the hop");
        assert_eq!(sway(0.2, 0.48), 0.0);
        assert!((sway(0.48 + 2.4, 0.48) - 1.0).abs() < 1e-6);
    }
}
