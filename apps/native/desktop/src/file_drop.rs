//! The overlay over a conversation while files are dragged onto the window:
//! a random Caper character hops in front of three file cards that drift
//! apart behind it. Web (`apps/web/src/chat/drop.ts`), Apple and Android draw
//! the same motion from a seed (`shared/messages/drop-motion-cases.json`);
//! every client's text matches `shared/messages/drop-cases.json`. Change them
//! together.

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
const POSES: [(f64, f64, f64); 3] = [(-46.0, 10.0, -16.0), (0.0, -26.0, 0.0), (46.0, 10.0, 16.0)];
const ART: egui::Vec2 = egui::vec2(176.0, 128.0);
const CARD: egui::Vec2 = egui::vec2(54.0, 66.0);
const CHARACTER: f32 = 76.0;
const GRAVITY: f64 = 420.0;
const CROUCH: f64 = 0.12;

/// Progress through an animation of `duration` seconds that starts after `delay`.
fn progress(elapsed: f64, delay: f64, duration: f64) -> f64 {
    ((elapsed - delay) / duration).clamp(0.0, 1.0)
}

fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

/// mulberry32: small, and identical on every client's integer arithmetic.
struct Random(u32);

impl Random {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6D2B_79F5);
        let a = self.0;
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

#[derive(Debug, PartialEq)]
struct CardPlan {
    /// Offset from the resting pose, so no two opens rest the same way.
    jitter: [f64; 3],
    delay: f64,
    duration: f64,
    /// Two (period, phase) waves each for x, y and turn.
    waves: [f64; 12],
}

/// One open's motion. Each open picks a random seed, so the cards drift and
/// the character hops differently every time; every client derives the same
/// motion from a seed (`shared/messages/drop-motion-cases.json`). Times are
/// seconds since the drag arrived; lengths are points.
#[derive(Debug, PartialEq)]
pub struct Plan {
    cards: [CardPlan; 3],
    tilt: (f64, f64),
    hops: Vec<(f64, f64)>,
}

/// One drag over the window: when it arrived, and its own character and
/// motion. The fixture always shows the first character and seed.
pub struct Open {
    pub since: f64,
    pub character: usize,
    pub plan: Plan,
}

impl Open {
    pub fn new(since: f64, characters: usize, fixture: bool) -> Self {
        let random = uuid::Uuid::new_v4().as_u128();
        Self {
            since,
            character: if fixture {
                0
            } else {
                (random % characters as u128) as usize
            },
            plan: Plan::new(if fixture { 0 } else { (random >> 64) as u32 }),
        }
    }
}

/// Seconds in the air for a hop `height` high.
fn air(height: f64) -> f64 {
    2.0 * (2.0 * height / GRAVITY).sqrt()
}

/// A damped spring's response `elapsed` after a kick, peaking near `size`.
fn spring(size: f64, elapsed: f64) -> f64 {
    if elapsed <= 0.0 {
        0.0
    } else {
        size * (-6.0 * elapsed).exp() * (16.0 * elapsed).sin() / 0.6
    }
}

impl Plan {
    pub fn new(seed: u32) -> Self {
        let mut random = Random(seed);
        let tau = std::f64::consts::TAU;
        let cards = std::array::from_fn(|_| {
            let jitter = std::array::from_fn(|_| (random.next() * 2.0 - 1.0) * 3.0);
            let delay = 0.04 + random.next() * 0.14;
            let duration = 1.3 + random.next() * 0.6;
            let mut waves = [0.0; 12];
            for wave in waves.chunks_mut(2) {
                wave[0] = 4.0 + random.next() * 5.0;
                wave[1] = random.next() * tau;
            }
            CardPlan {
                jitter,
                delay,
                duration,
                waves,
            }
        });
        let period = 5.0 + random.next() * 3.0;
        let tilt = (period, random.next() * tau);
        let mut start = 0.75 + random.next() * 0.5;
        let hops = (0..32)
            .map(|_| {
                let height = 5.0 + random.next() * 7.0;
                let quick = random.next() < 0.25;
                let pause = random.next();
                let hop = (start, height);
                start += CROUCH
                    + air(height)
                    + air(height * 0.2)
                    + if quick {
                        0.06 + pause * 0.1
                    } else {
                        0.7 + pause * 1.8
                    };
                hop
            })
            .collect();
        Self { cards, tilt, hops }
    }
}

/// Where a file card is: offset from the stage's bottom centre, turn in
/// degrees, and opacity.
#[derive(Debug, PartialEq)]
struct CardPose {
    x: f64,
    y: f64,
    turn: f64,
    opacity: f64,
}

/// Where the character is: how far below its resting place, squash, turn and
/// opacity.
#[derive(Debug, PartialEq)]
struct CharacterPose {
    y: f64,
    scale_x: f64,
    scale_y: f64,
    turn: f64,
    opacity: f64,
}

/// Where the moving parts are, `elapsed` seconds after the drag arrived. The
/// card arrives like other entrances (`--ease-out`, opacity and translation
/// only); the file cards drift apart and the character hops.
#[derive(Debug, PartialEq)]
struct Pose {
    /// The dimmed backdrop's opacity.
    fade: f32,
    /// The card's opacity, and how far it has risen into place.
    card_opacity: f32,
    card_rise: f32,
    cards: [CardPose; 3],
    character: CharacterPose,
}

impl Pose {
    /// With reduced motion everything shows its resting pose at once.
    fn at(plan: &Plan, elapsed: f64, reduced: bool) -> Self {
        let t = if reduced { 0.0 } else { elapsed };
        let tau = std::f64::consts::TAU;
        let wave = |waves: &[f64; 12], at: usize| {
            0.6 * (tau * t / waves[at] + waves[at + 1]).sin()
                + 0.4 * (tau * t / waves[at + 2] + waves[at + 3]).sin()
        };
        let cards = std::array::from_fn(|index| {
            let card = &plan.cards[index];
            let (x, y, turn) = POSES[index];
            let out = ease_out_cubic(if reduced {
                1.0
            } else {
                progress(t, card.delay, card.duration)
            });
            let drift = if reduced { 0.0 } else { out };
            CardPose {
                x: (x + card.jitter[0]) * out + 2.5 * wave(&card.waves, 0) * drift,
                // Each card slides out and up from just below the middle.
                y: 24.0 + (y + card.jitter[1] - 24.0) * out + 3.5 * wave(&card.waves, 4) * drift,
                turn: turn + card.jitter[2] + 2.5 * wave(&card.waves, 8) * drift,
                opacity: if reduced {
                    1.0
                } else {
                    progress(t, card.delay, 0.3)
                },
            }
        });
        let rise = if reduced {
            1.0
        } else {
            ease_out_cubic(progress(t, 0.08, 0.5))
        };
        let (mut lift, mut squash) = (0.0, 0.0);
        if !reduced {
            for &(start, height) in &plan.hops {
                let since = t - start;
                if since < 0.0 {
                    break;
                }
                let (up, rebound) = (air(height), air(height * 0.2));
                let flying = since - CROUCH;
                if (0.0..up).contains(&flying) {
                    lift = GRAVITY / 2.0 * flying * (up - flying);
                } else if (up..up + rebound).contains(&flying) {
                    lift = GRAVITY / 2.0 * (flying - up) * (up + rebound - flying);
                }
                // Crouch, spring up, land, and settle after the rebound;
                // higher hops squash more.
                let size = height / 9.0;
                squash += spring(-0.05 * size, since)
                    + spring(0.09 * size, flying)
                    + spring(-0.1 * size, flying - up)
                    + spring(-0.035 * size, flying - up - rebound);
            }
        }
        let fixed = |value: f64| if reduced { 1.0 } else { value as f32 };
        Self {
            fade: fixed(progress(t, 0.0, f64::from(crate::motion::FAST))),
            card_opacity: fixed(progress(t, 0.0, 0.12)),
            card_rise: fixed(f64::from(crate::motion::ease_out(progress(
                t,
                0.0,
                f64::from(crate::motion::SLOW),
            ) as f32))),
            cards,
            character: CharacterPose {
                y: 28.0 * (1.0 - rise) - lift,
                scale_x: 1.0 - 0.8 * squash,
                scale_y: 1.0 + squash,
                turn: if reduced {
                    0.0
                } else {
                    2.0 * (tau * t / plan.tilt.0 + plan.tilt.1).sin() * rise
                },
                opacity: if reduced { 1.0 } else { progress(t, 0.08, 0.2) },
            },
        }
    }
}

/// Paints the overlay over `area`, `elapsed` seconds after the drag arrived,
/// and returns whether it is still moving (it never stops unless the system
/// reduces motion). It never takes input: dropping stays with the window.
pub fn paint(
    context: &egui::Context,
    area: egui::Rect,
    plan: &Plan,
    elapsed: f64,
    character: (String, &'static [u8]),
    title: &str,
    detail: &str,
) -> bool {
    let reduced = crate::motion::reduced();
    let pose = Pose::at(plan, elapsed, reduced);
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
                Stroke::new(1.0_f32, crate::BORDER),
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
                Stroke::new(1.5_f32, crate::TERRACOTTA),
                5.0,
                4.0,
            ));

            let art = egui::Rect::from_min_size(
                egui::pos2(card.center().x - ART.x / 2.0, card.top() + 20.0),
                ART,
            );
            // The middle card first, behind the other two.
            for index in [1, 0, 2] {
                let card = &pose.cards[index];
                // The card's bottom edge sits 98 points down the stage.
                let pivot = art.min + egui::vec2(ART.x / 2.0 + card.x as f32, 98.0 + card.y as f32);
                let rect = egui::Rect::from_min_max(
                    pivot - egui::vec2(CARD.x / 2.0, CARD.y),
                    pivot + egui::vec2(CARD.x / 2.0, 0.0),
                );
                let (uri, svg) = CARDS[index];
                egui::Image::from_bytes(uri, svg.as_bytes())
                    .rotate((card.turn as f32).to_radians(), egui::vec2(0.5, 1.0))
                    .tint(Color32::WHITE.gamma_multiply(card.opacity as f32))
                    .paint_at(ui, rect);
            }

            let caper = &pose.character;
            let (squash_x, squash_y) = (caper.scale_x as f32, caper.scale_y as f32);
            let bottom = art.center_bottom() + egui::vec2(0.0, 4.0 + caper.y as f32);
            let rect = egui::Rect::from_min_max(
                bottom - egui::vec2(CHARACTER * squash_x / 2.0, CHARACTER * squash_y),
                bottom + egui::vec2(CHARACTER * squash_x / 2.0, 0.0),
            );
            egui::Image::from_bytes(character.0, character.1)
                .rotate((caper.turn as f32).to_radians(), egui::vec2(0.5, 1.0))
                .tint(Color32::WHITE.gamma_multiply(caper.opacity as f32))
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

    #[derive(serde::Deserialize)]
    struct Motion {
        random: Sample,
        seed: u32,
        cases: Vec<MotionCase>,
    }

    #[derive(serde::Deserialize)]
    struct Sample {
        seed: u32,
        values: Vec<f64>,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct MotionCase {
        elapsed: f64,
        still: bool,
        cards: Vec<std::collections::BTreeMap<String, f64>>,
        character: std::collections::BTreeMap<String, f64>,
    }

    #[test]
    fn shared_cases_produce_identical_motion() {
        let motion: Motion = serde_json::from_str(include_str!(
            "../../../../shared/messages/drop-motion-cases.json"
        ))
        .expect("valid shared drop motion");
        let mut random = Random(motion.random.seed);
        for value in motion.random.values {
            assert!((random.next() - value).abs() < 1e-6);
        }
        let plan = Plan::new(motion.seed);
        assert!(motion.cases.len() >= 5, "the shared cases loaded");
        let close = |actual: f64, expected: f64, what: &str| {
            assert!(
                (actual - expected).abs() < 1e-4,
                "{what}: {actual} vs {expected}"
            );
        };
        for case in motion.cases {
            let pose = Pose::at(&plan, case.elapsed, case.still);
            for (card, expected) in pose.cards.iter().zip(&case.cards) {
                close(card.x, expected["x"], "card x");
                close(card.y, expected["y"], "card y");
                close(card.turn, expected["turn"], "card turn");
                close(card.opacity, expected["opacity"], "card opacity");
            }
            let caper = &pose.character;
            close(caper.y, case.character["y"], "character y");
            close(caper.scale_x, case.character["scaleX"], "character scaleX");
            close(caper.scale_y, case.character["scaleY"], "character scaleY");
            close(caper.turn, case.character["turn"], "character turn");
            close(
                caper.opacity,
                case.character["opacity"],
                "character opacity",
            );
        }
    }

    #[test]
    fn reduced_motion_rests_at_once() {
        let plan = Plan::new(7);
        let rest = Pose::at(&plan, 0.0, true);
        assert_eq!(Pose::at(&plan, 5.0, true), rest);
        assert_eq!(
            (rest.fade, rest.card_opacity, rest.card_rise),
            (1.0, 1.0, 1.0)
        );
        let start = Pose::at(&plan, 0.0, false);
        assert_eq!((start.fade, start.character.opacity), (0.0, 0.0));
    }
}
