//! Middle-click autoscroll over the conversation history, as in Windows apps
//! and browsers: press the middle button to drop an anchor, then move the
//! pointer away from it to scroll at a speed that grows with the distance.
//! A plain middle click keeps scrolling until the next click, Escape or wheel;
//! holding the button and moving stops on release.
//!
//! It works on raw input, before egui sees it: while active, the pointer stays
//! at the anchor (so the history under it keeps the scroll) and each frame
//! adds a wheel delta, which takes the history's existing scroll path.

use eframe::egui::{self, Color32, Pos2, Rect, Stroke};

/// Pointer travel from the anchor that does not scroll yet.
const DEAD_ZONE: f32 = 12.0;
/// Points per second of scrolling for each point beyond the dead zone.
const SPEED: f32 = 8.0;

#[derive(Default)]
pub struct Autoscroll {
    /// Where autoscroll started, while it is active.
    anchor: Option<Pos2>,
    /// The real pointer, which egui sees pinned at the anchor while active.
    pointer: Option<Pos2>,
    middle_held: bool,
    /// The middle button left the dead zone since it was pressed, so its
    /// release ends autoscroll.
    dragged: bool,
    /// A press that ended autoscroll, whose release must not click either.
    swallow_release: Option<egui::PointerButton>,
    last_time: Option<f64>,
    /// History viewports drawn this pass, and those of the previous pass,
    /// where a middle press starts autoscroll.
    next_areas: Vec<Rect>,
    areas: Vec<Rect>,
}

impl Autoscroll {
    /// Lets a middle press inside this history viewport start autoscroll.
    pub fn area(&mut self, rect: Rect) {
        self.next_areas.push(rect);
    }

    #[cfg(test)]
    pub fn active(&self) -> bool {
        self.anchor.is_some()
    }

    /// Rewrites this frame's raw input: see the module documentation.
    pub fn input(&mut self, raw_input: &mut egui::RawInput) {
        self.areas = std::mem::take(&mut self.next_areas);
        let events = std::mem::take(&mut raw_input.events);
        for mut event in events {
            match &mut event {
                egui::Event::PointerMoved(pos) => {
                    self.pointer = Some(*pos);
                    if let Some(anchor) = self.anchor {
                        if self.middle_held && (*pos - anchor).length() > DEAD_ZONE {
                            self.dragged = true;
                        }
                        *pos = anchor;
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } => {
                    let (pos, button, pressed) = (*pos, *button, *pressed);
                    if !pressed && self.swallow_release == Some(button) {
                        self.swallow_release = None;
                        continue;
                    }
                    if button == egui::PointerButton::Middle {
                        if pressed {
                            self.pointer = Some(pos);
                            if self.anchor.is_some() {
                                self.stop();
                                self.swallow_release = Some(button);
                            } else if self.areas.iter().any(|area| area.contains(pos)) {
                                self.anchor = Some(pos);
                                self.middle_held = true;
                                self.dragged = false;
                                self.last_time = None;
                            } else {
                                raw_input.events.push(event);
                            }
                        } else if self.anchor.is_some() {
                            self.middle_held = false;
                            if self.dragged {
                                self.stop();
                            }
                        } else {
                            raw_input.events.push(event);
                        }
                        continue;
                    }
                    if self.anchor.is_some() {
                        // Any other click only ends autoscroll.
                        if pressed {
                            self.stop();
                            self.swallow_release = Some(button);
                        }
                        continue;
                    }
                }
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed,
                    ..
                } if self.anchor.is_some() => {
                    if *pressed {
                        self.stop();
                    }
                    continue;
                }
                egui::Event::MouseWheel { .. } => self.stop(),
                egui::Event::PointerGone | egui::Event::WindowFocused(false) => self.stop(),
                _ => {}
            }
            raw_input.events.push(event);
        }
        let (Some(anchor), Some(pointer)) = (self.anchor, self.pointer) else {
            return;
        };
        let dt = match (self.last_time, raw_input.time) {
            (Some(last), Some(now)) => (now - last) as f32,
            _ => raw_input.predicted_dt,
        }
        .clamp(0.0, 0.1);
        self.last_time = raw_input.time;
        let speed = scroll_speed(pointer.y - anchor.y);
        if speed != 0.0 {
            raw_input.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                // Positive wheel deltas move the content down, which scrolls up.
                delta: egui::vec2(0.0, -speed * dt),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }

    fn stop(&mut self) {
        self.anchor = None;
        self.middle_held = false;
        self.dragged = false;
        self.last_time = None;
    }

    /// The anchor marker and the scroll cursor, after the page is drawn.
    pub fn show(&self, context: &egui::Context, fill: Color32, stroke: Color32, arrow: Color32) {
        let (Some(anchor), Some(pointer)) = (self.anchor, self.pointer) else {
            return;
        };
        context.request_repaint();
        let offset = pointer.y - anchor.y;
        context.set_cursor_icon(if offset.abs() <= DEAD_ZONE {
            egui::CursorIcon::ResizeVertical
        } else if offset < 0.0 {
            egui::CursorIcon::ResizeNorth
        } else {
            egui::CursorIcon::ResizeSouth
        });
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::unique("autoscroll-anchor"),
        ));
        painter.circle(anchor, 13.0, fill, Stroke::new(1.0, stroke));
        painter.circle_filled(anchor, 2.0, arrow);
        for direction in [-1.0, 1.0] {
            let tip = anchor + egui::vec2(0.0, 9.0 * direction);
            let base = anchor.y + 5.0 * direction;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    tip,
                    egui::pos2(anchor.x + 4.0, base),
                    egui::pos2(anchor.x - 4.0, base),
                ],
                arrow,
                Stroke::NONE,
            ));
        }
    }
}

/// Points per second for a pointer this far below (positive) or above the anchor.
fn scroll_speed(offset: f32) -> f32 {
    (offset.abs() - DEAD_ZONE).max(0.0) * SPEED * offset.signum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(pos: Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn frame(autoscroll: &mut Autoscroll, events: Vec<egui::Event>) -> Vec<egui::Event> {
        autoscroll.area(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0)));
        let mut raw = egui::RawInput {
            events,
            predicted_dt: 0.1,
            ..Default::default()
        };
        autoscroll.input(&mut raw);
        raw.events
    }

    fn wheel(events: &[egui::Event]) -> f32 {
        events
            .iter()
            .map(|event| match event {
                egui::Event::MouseWheel { delta, .. } => delta.y,
                _ => 0.0,
            })
            .sum()
    }

    #[test]
    fn middle_click_scrolls_toward_the_pointer_until_the_next_click() {
        let mut autoscroll = Autoscroll::default();
        let anchor = egui::pos2(400.0, 300.0);
        let middle = egui::PointerButton::Middle;
        frame(&mut autoscroll, vec![]);
        let events = frame(
            &mut autoscroll,
            vec![button(anchor, middle, true), button(anchor, middle, false)],
        );
        assert!(autoscroll.active());
        assert!(
            events.is_empty(),
            "the middle click reaches no widget: {events:?}"
        );
        // Inside the dead zone nothing scrolls.
        let events = frame(
            &mut autoscroll,
            vec![egui::Event::PointerMoved(anchor + egui::vec2(0.0, 10.0))],
        );
        assert_eq!(wheel(&events), 0.0);
        // 50 points below: (50 - 12) * 8 points per second for 0.1 s, downward.
        let events = frame(
            &mut autoscroll,
            vec![egui::Event::PointerMoved(anchor + egui::vec2(0.0, 50.0))],
        );
        assert!((wheel(&events) + 30.4).abs() < 0.01, "{events:?}");
        // egui keeps seeing the pointer at the anchor.
        assert!(events.contains(&egui::Event::PointerMoved(anchor)));
        // Above the anchor scrolls up, and keeps going without new movement.
        frame(
            &mut autoscroll,
            vec![egui::Event::PointerMoved(anchor - egui::vec2(0.0, 112.0))],
        );
        assert!((wheel(&frame(&mut autoscroll, vec![])) - 80.0).abs() < 0.01);
        // A primary click only ends it; neither its press nor release clicks.
        let primary = egui::PointerButton::Primary;
        let at = anchor - egui::vec2(0.0, 112.0);
        let events = frame(
            &mut autoscroll,
            vec![button(at, primary, true), button(at, primary, false)],
        );
        assert!(!autoscroll.active());
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, egui::Event::PointerButton { .. }))
        );
        assert_eq!(wheel(&frame(&mut autoscroll, vec![])), 0.0);
        // The next primary click goes through again.
        let events = frame(&mut autoscroll, vec![button(at, primary, true)]);
        assert_eq!(events, vec![button(at, primary, true)]);
    }

    #[test]
    fn holding_the_middle_button_scrolls_until_release() {
        let mut autoscroll = Autoscroll::default();
        let anchor = egui::pos2(400.0, 300.0);
        let middle = egui::PointerButton::Middle;
        frame(&mut autoscroll, vec![]);
        frame(&mut autoscroll, vec![button(anchor, middle, true)]);
        frame(
            &mut autoscroll,
            vec![egui::Event::PointerMoved(anchor + egui::vec2(0.0, 80.0))],
        );
        assert!(autoscroll.active());
        frame(
            &mut autoscroll,
            vec![button(anchor + egui::vec2(0.0, 80.0), middle, false)],
        );
        assert!(!autoscroll.active());
    }

    #[test]
    fn middle_press_outside_the_history_passes_through() {
        let mut autoscroll = Autoscroll::default();
        let outside = egui::pos2(900.0, 300.0);
        let middle = egui::PointerButton::Middle;
        frame(&mut autoscroll, vec![]);
        let events = frame(&mut autoscroll, vec![button(outside, middle, true)]);
        assert!(!autoscroll.active());
        assert_eq!(events, vec![button(outside, middle, true)]);
    }
}
