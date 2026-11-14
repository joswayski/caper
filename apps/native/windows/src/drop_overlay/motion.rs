//! Win32's implementation of the seeded drop motion shared with GTK, egui,
//! web, Apple and Android. Keep the random consumption order identical to
//! `shared/messages/drop-motion-cases.json`.
const POSES: [(f64, f64, f64); 3] = [(-46.0, 10.0, -16.0), (0.0, -26.0, 0.0), (46.0, 10.0, 16.0)];
const GRAVITY: f64 = 420.0;
const CROUCH: f64 = 0.12;

fn progress(t: f64, delay: f64, duration: f64) -> f64 {
    ((t - delay) / duration).clamp(0.0, 1.0)
}

fn cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

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
    jitter: [f64; 3],
    delay: f64,
    duration: f64,
    waves: [f64; 12],
}

pub struct Plan {
    cards: [CardPlan; 3],
    tilt: (f64, f64),
    hops: Vec<(f64, f64)>,
}

fn air(height: f64) -> f64 {
    2.0 * (2.0 * height / GRAVITY).sqrt()
}

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

#[derive(Debug, PartialEq)]
pub struct CardPose {
    pub x: f64,
    pub y: f64,
    pub turn: f64,
    pub opacity: f64,
}

#[derive(Debug, PartialEq)]
pub struct CharacterPose {
    pub y: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub turn: f64,
    pub opacity: f64,
}

#[derive(Debug, PartialEq)]
pub struct Pose {
    pub fade: f64,
    pub card_opacity: f64,
    pub card_rise: f64,
    pub cards: [CardPose; 3],
    pub character: CharacterPose,
}

impl Pose {
    pub fn at(plan: &Plan, elapsed: f64, reduced: bool) -> Self {
        let t = if reduced { 0.0 } else { elapsed };
        let tau = std::f64::consts::TAU;
        let wave = |waves: &[f64; 12], at: usize| {
            0.6 * (tau * t / waves[at] + waves[at + 1]).sin()
                + 0.4 * (tau * t / waves[at + 2] + waves[at + 3]).sin()
        };
        let cards = std::array::from_fn(|index| {
            let card = &plan.cards[index];
            let (x, y, turn) = POSES[index];
            let out = cubic(if reduced {
                1.0
            } else {
                progress(t, card.delay, card.duration)
            });
            let drift = if reduced { 0.0 } else { out };
            CardPose {
                x: (x + card.jitter[0]) * out + 2.5 * wave(&card.waves, 0) * drift,
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
            cubic(progress(t, 0.08, 0.5))
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
                let size = height / 9.0;
                squash += spring(-0.05 * size, since)
                    + spring(0.09 * size, flying)
                    + spring(-0.1 * size, flying - up)
                    + spring(-0.035 * size, flying - up - rebound);
            }
        }
        let fixed = |value: f64| if reduced { 1.0 } else { value };
        let entrance = progress(t, 0.0, 0.26);
        Self {
            fade: fixed(progress(t, 0.0, 0.14)),
            card_opacity: fixed(progress(t, 0.0, 0.12)),
            card_rise: fixed(if entrance >= 1.0 {
                1.0
            } else {
                1.0 - 2f64.powf(-10.0 * entrance)
            }),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_drop_motion_and_reduced_motion() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../shared/messages/drop-motion-cases.json"
        ))
        .unwrap();
        let mut random = Random(cases["random"]["seed"].as_u64().unwrap() as u32);
        for expected in cases["random"]["values"].as_array().unwrap() {
            assert!((random.next() - expected.as_f64().unwrap()).abs() < 1e-6);
        }
        let close = |actual: f64, expected: &serde_json::Value| {
            let expected = expected.as_f64().unwrap();
            assert!((actual - expected).abs() < 1e-4, "{actual} != {expected}");
        };
        let plan = Plan::new(cases["seed"].as_u64().unwrap() as u32);
        for case in cases["cases"].as_array().unwrap() {
            let pose = Pose::at(
                &plan,
                case["elapsed"].as_f64().unwrap(),
                case["still"].as_bool().unwrap(),
            );
            for (card, expected) in pose.cards.iter().zip(case["cards"].as_array().unwrap()) {
                close(card.x, &expected["x"]);
                close(card.y, &expected["y"]);
                close(card.turn, &expected["turn"]);
                close(card.opacity, &expected["opacity"]);
            }
            let c = &case["character"];
            close(pose.character.y, &c["y"]);
            close(pose.character.scale_x, &c["scaleX"]);
            close(pose.character.scale_y, &c["scaleY"]);
            close(pose.character.turn, &c["turn"]);
            close(pose.character.opacity, &c["opacity"]);
        }
        let rest = Pose::at(&plan, 0.0, true);
        assert_eq!(rest, Pose::at(&plan, 100.0, true));
        assert_eq!(
            (rest.fade, rest.card_opacity, rest.card_rise),
            (1.0, 1.0, 1.0)
        );
        assert_eq!(rest.character.opacity, 1.0);
    }
}
