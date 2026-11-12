//! The overlay over a conversation or thread while files are dragged onto
//! it: a random Caper character hops in front of three file cards that
//! drift apart behind it, as desktop's `file_drop.rs`. Web
//! (`apps/web/src/chat/drop.ts`), Apple, Android and desktop draw the same
//! motion from a seed (`shared/messages/drop-motion-cases.json`); change
//! them together. It never takes input: dropping stays with the pane.

use caper_client::avatar_images::BRANDING;
use gtk::{cairo, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

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
const CARDS: [&str; 3] = [
    card!(
        r#"<rect width="18" height="18" x="3" y="3" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#
    ),
    card!(
        r#"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M10 9H8"/><path d="M16 13H8"/><path d="M16 17H8"/>"#
    ),
    card!(
        r#"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M7 3v18M3 7.5h4M3 12h18M3 16.5h4M17 3v18M17 7.5h4M17 16.5h4"/>"#
    ),
];

/// Resting pose per card: horizontal and vertical offset, and turn in degrees.
const POSES: [(f64, f64, f64); 3] = [(-46.0, 10.0, -16.0), (0.0, -26.0, 0.0), (46.0, 10.0, 16.0)];
/// The stage the cards and character move on, and their sizes.
const ART: (f64, f64) = (176.0, 128.0);
const CARD: (f64, f64) = (54.0, 66.0);
const CHARACTER: f64 = 76.0;
const GRAVITY: f64 = 420.0;
const CROUCH: f64 = 0.12;
/// `--motion-fast` and `--motion-slow`, in seconds.
const FAST: f64 = 0.14;
const SLOW: f64 = 0.26;
/// Pixels per point the art is rendered at, for sharp edges when scaled.
const DENSITY: f64 = 2.0;

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

struct CardPlan {
    /// Offset from the resting pose, so no two opens rest the same way.
    jitter: [f64; 3],
    delay: f64,
    duration: f64,
    /// Two (period, phase) waves each for x, y and turn.
    waves: [f64; 12],
}

/// One open's motion: times are seconds since the drag arrived, lengths
/// are points.
struct Plan {
    cards: [CardPlan; 3],
    tilt: (f64, f64),
    hops: Vec<(f64, f64)>,
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
    fn new(seed: u32) -> Self {
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

/// A file card: offset from the stage's bottom centre, turn in degrees, and
/// opacity.
#[derive(Debug, PartialEq)]
struct CardPose {
    x: f64,
    y: f64,
    turn: f64,
    opacity: f64,
}

/// The character: how far below its resting place, squash, turn, opacity.
#[derive(Debug, PartialEq)]
struct CharacterPose {
    y: f64,
    scale_x: f64,
    scale_y: f64,
    turn: f64,
    opacity: f64,
}

/// Where the moving parts are, `elapsed` seconds after the drag arrived.
#[derive(Debug, PartialEq)]
struct Pose {
    /// The dimmed backdrop's opacity.
    fade: f64,
    /// The card's opacity, and how far it has risen into place.
    card_opacity: f64,
    card_rise: f64,
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
        let fixed = |value: f64| if reduced { 1.0 } else { value };
        Self {
            fade: fixed(progress(t, 0.0, FAST)),
            card_opacity: fixed(progress(t, 0.0, 0.12)),
            card_rise: fixed(crate::ui::ease_out(progress(t, 0.0, SLOW))),
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

/// One drag over the pane: when it arrived, its motion and character.
struct Open {
    since: Instant,
    plan: Plan,
    character: cairo::ImageSurface,
}

/// The overlay's widgets: a dimmed backdrop and the card with the moving
/// art, the title and the detail.
#[derive(Clone)]
pub struct Overlay {
    pub root: gtk::Overlay,
    backdrop: gtk::Box,
    card: gtk::Box,
    title: gtk::Label,
    detail: gtk::Label,
    stage: gtk::DrawingArea,
    open: Rc<RefCell<Option<Open>>>,
    ticking: Rc<Cell<Option<gtk::TickCallbackId>>>,
}

impl Overlay {
    pub fn new() -> Self {
        let root = gtk::Overlay::new();
        root.set_can_target(false);
        root.set_visible(false);
        let backdrop = gtk::Box::new(gtk::Orientation::Vertical, 0);
        backdrop.add_css_class("drop");
        root.set_child(Some(&backdrop));
        let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        card.add_css_class("drop-card");
        card.set_halign(gtk::Align::Center);
        card.set_valign(gtk::Align::Center);
        // The character's feet reach 4 points below the stage.
        let stage = gtk::DrawingArea::new();
        stage.set_content_width(ART.0 as i32);
        stage.set_content_height(ART.1 as i32 + 4);
        stage.set_halign(gtk::Align::Center);
        card.append(&stage);
        let title = crate::ui::label("", &["drop-title"]);
        let detail = crate::ui::label("", &["muted", "drop-detail"]);
        for label in [&title, &detail] {
            label.set_justify(gtk::Justification::Center);
            label.set_xalign(0.5);
            label.set_wrap(true);
        }
        card.append(&title);
        card.append(&detail);
        root.add_overlay(&card);
        let open: Rc<RefCell<Option<Open>>> = Rc::default();
        let shown = open.clone();
        stage.set_draw_func(move |_, cr, _, _| {
            if let Some(open) = shown.borrow().as_ref() {
                let pose = Pose::at(&open.plan, elapsed(open), !crate::images::motion());
                paint(cr, &pose, &open.character);
            }
        });
        Self {
            root,
            backdrop,
            card,
            title,
            detail,
            stage,
            open,
            ticking: Rc::default(),
        }
    }

    /// Shows the overlay with `title` and `detail`: a new character and
    /// motion when it was closed. It moves until `close`, unless animations
    /// are off (reduced motion), when it shows its resting pose at once.
    pub fn show(&self, title: &str, detail: &str) {
        self.title.set_label(title);
        self.detail.set_label(detail);
        // As desktop: the card is the pane less 32 points, 120 to 360 wide.
        let width = (self.root.parent().map_or(0, |pane| pane.width()) - 32).clamp(120, 360);
        self.card.set_width_request(width);
        if self.open.borrow().is_none() {
            let random = uuid::Uuid::new_v4().as_u128();
            let index = (random % BRANDING.len() as u128) as usize;
            let Some(character) = surface(BRANDING[index], CHARACTER, CHARACTER) else {
                return;
            };
            *self.open.borrow_mut() = Some(Open {
                since: Instant::now(),
                plan: Plan::new((random >> 64) as u32),
                character,
            });
        }
        self.pose();
        self.root.set_visible(true);
        if crate::images::motion() {
            let id = self.ticking.take().unwrap_or_else(|| {
                let overlay = self.clone();
                self.root.add_tick_callback(move |_, _| {
                    overlay.pose();
                    glib::ControlFlow::Continue
                })
            });
            self.ticking.set(Some(id));
        }
    }

    /// Hides it at once (closing stays instant).
    pub fn close(&self) {
        if let Some(id) = self.ticking.take() {
            id.remove();
        }
        self.root.set_visible(false);
        *self.open.borrow_mut() = None;
    }

    /// The backdrop's fade, the card's fade and rise, and the art's pose.
    fn pose(&self) {
        let Some((fade, opacity, rise)) = self.open.borrow().as_ref().map(|open| {
            let pose = Pose::at(&open.plan, elapsed(open), !crate::images::motion());
            (pose.fade, pose.card_opacity, pose.card_rise)
        }) else {
            return;
        };
        self.backdrop.set_opacity(fade);
        self.card.set_opacity(opacity);
        // 10 points below its place, rising: a top margin of twice the
        // offset moves a centred card down by it.
        self.card
            .set_margin_top((20.0 * (1.0 - rise)).round() as i32);
        self.stage.queue_draw();
    }
}

fn elapsed(open: &Open) -> f64 {
    open.since.elapsed().as_secs_f64()
}

/// The cards, the middle one behind, and the character over them.
fn paint(cr: &cairo::Context, pose: &Pose, character: &cairo::ImageSurface) {
    thread_local! {
        static CARD_ART: Vec<cairo::ImageSurface> = CARDS
            .iter()
            .filter_map(|svg| surface(svg.as_bytes(), CARD.0, CARD.1))
            .collect();
    }
    // Each image turns about its bottom centre at `pivot`.
    let place = |art: &cairo::ImageSurface,
                 pivot: (f64, f64),
                 size: (f64, f64),
                 turn: f64,
                 opacity: f64| {
        cr.save().ok();
        cr.translate(pivot.0, pivot.1);
        cr.rotate(turn.to_radians());
        cr.scale(
            size.0 / f64::from(art.width()),
            size.1 / f64::from(art.height()),
        );
        let _ = cr.set_source_surface(art, -f64::from(art.width()) / 2.0, -f64::from(art.height()));
        let _ = cr.paint_with_alpha(opacity);
        cr.restore().ok();
    };
    CARD_ART.with(|cards| {
        for index in [1, 0, 2] {
            let (Some(art), card) = (cards.get(index), &pose.cards[index]) else {
                continue;
            };
            // The card's bottom edge sits 98 points down the stage.
            let pivot = (ART.0 / 2.0 + card.x, 98.0 + card.y);
            place(art, pivot, CARD, card.turn, card.opacity);
        }
    });
    let caper = &pose.character;
    let pivot = (ART.0 / 2.0, ART.1 + 4.0 + caper.y);
    let size = (CHARACTER * caper.scale_x, CHARACTER * caper.scale_y);
    place(character, pivot, size, caper.turn, caper.opacity);
}

/// `svg` rendered `width` × `height` points, at `DENSITY`.
fn surface(svg: &[u8], width: f64, height: f64) -> Option<cairo::ImageSurface> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).ok()?;
    let (pixels_x, pixels_y) = ((width * DENSITY) as u32, (height * DENSITY) as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(pixels_x, pixels_y)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(
            pixels_x as f32 / tree.size().width(),
            pixels_y as f32 / tree.size().height(),
        ),
        &mut pixmap.as_mut(),
    );
    let mut surface =
        cairo::ImageSurface::create(cairo::Format::ARgb32, pixels_x as i32, pixels_y as i32)
            .ok()?;
    let stride = surface.stride() as usize;
    {
        let mut data = surface.data().ok()?;
        // Both are premultiplied; cairo keeps each pixel as a native-endian
        // 0xAARRGGBB word.
        for (row, source) in pixmap.data().chunks(pixels_x as usize * 4).enumerate() {
            for (column, pixel) in source.chunks(4).enumerate() {
                let word = u32::from_be_bytes([pixel[3], pixel[0], pixel[1], pixel[2]]);
                let at = row * stride + column * 4;
                data[at..at + 4].copy_from_slice(&word.to_ne_bytes());
            }
        }
    }
    Some(surface)
}

#[cfg(test)]
mod tests {
    use super::*;

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
