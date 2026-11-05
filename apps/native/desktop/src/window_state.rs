//! Keep the normal restore rectangle, not Windows' minimized/maximized bounds.
use eframe::egui::{self, Pos2, Vec2};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct WindowState {
    outer_position_pixels: Option<Pos2>,
    scale_factor: f32,
    fullscreen: bool,
    maximized: bool,
    inner_size_points: Option<Vec2>,
    restore_pending: bool,
    restore_deadline: Option<Instant>,
}

// eframe 0.37's persisted `window` record (`egui_winit::WindowSettings`), so
// eframe restores the normal geometry with its monitor/DPI placement and
// off-screen clamping. Its own `fullscreen`/`maximized` stay false: eframe
// would apply them to the still-hidden window, so Caper restores the mode after
// the first paint from `mode`, which eframe ignores.
#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Record {
    normal_geometry: Geometry,
    fullscreen: bool,
    maximized: bool,
    mode: Option<Mode>,
    // eframe 0.33's flat record, from older builds: geometry and Caper's mode.
    #[serde(skip_serializing)]
    outer_position_pixels: Option<Pos2>,
    #[serde(skip_serializing)]
    inner_size_points: Option<Vec2>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Geometry {
    outer_position_pixels: Option<Pos2>,
    scale_factor: f32,
    inner_size_points: Vec2,
}

#[derive(Deserialize, Serialize)]
struct Mode {
    fullscreen: bool,
    maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            outer_position_pixels: None,
            scale_factor: 1.0,
            fullscreen: false,
            maximized: true,
            inner_size_points: Some(egui::vec2(1440.0, 900.0)),
            restore_pending: true,
            restore_deadline: None,
        }
    }
}

impl WindowState {
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|storage| eframe::get_value::<Record>(storage, "window"))
            .map(|record| {
                let mode = record.mode.unwrap_or(Mode {
                    fullscreen: record.fullscreen,
                    maximized: record.maximized,
                });
                let geometry = record.normal_geometry;
                Self {
                    outer_position_pixels: geometry
                        .outer_position_pixels
                        .or(record.outer_position_pixels),
                    scale_factor: if geometry.scale_factor > 0.0 {
                        geometry.scale_factor
                    } else {
                        1.0
                    },
                    fullscreen: mode.fullscreen,
                    maximized: mode.maximized,
                    inner_size_points: record
                        .inner_size_points
                        .or(Some(geometry.inner_size_points)),
                    ..Default::default()
                }
            })
            .filter(|state| state.inner_size_points.is_some_and(valid_size))
            .unwrap_or_default()
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        let record = Record {
            normal_geometry: Geometry {
                outer_position_pixels: self.outer_position_pixels,
                scale_factor: self.scale_factor,
                inner_size_points: self.inner_size_points.unwrap_or_default(),
            },
            mode: Some(Mode {
                fullscreen: self.fullscreen,
                maximized: self.maximized,
            }),
            ..Default::default()
        };
        eframe::set_value(storage, "window", &record);
    }

    pub fn update(&mut self, context: &egui::Context) {
        if std::mem::take(&mut self.restore_pending) {
            // WGPU applies these after showing the first painted frame. Never
            // maximize a hidden Windows window (it flashes before first paint).
            context.send_viewport_cmd(egui::ViewportCommand::Maximized(self.maximized));
            context.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            let settle = Duration::from_secs(2);
            self.restore_deadline = Some(Instant::now() + settle);
            context.request_repaint_after(settle);
            context.request_repaint();
            return;
        }
        let zoom = context.zoom_factor();
        context.input(|input| self.observe(input.viewport(), zoom));
    }

    fn observe(&mut self, viewport: &egui::ViewportInfo, zoom: f32) {
        if viewport.minimized != Some(false) {
            return;
        }
        if let Some(deadline) = self.restore_deadline {
            // Window-manager mode changes are asynchronous. Do not replace the
            // saved mode with the temporary windowed first frame (or a second
            // egui layout pass) while waiting for the restore command to land.
            // Stop waiting if a window manager rejects it or an invite/user
            // action supersedes it, so later geometry still gets saved.
            if Instant::now() < deadline
                && (viewport.fullscreen != Some(self.fullscreen)
                    || (!self.fullscreen && viewport.maximized != Some(self.maximized)))
            {
                return;
            }
            self.restore_deadline = None;
        }
        if let Some(fullscreen) = viewport.fullscreen {
            self.fullscreen = fullscreen;
        }
        // Fullscreen can temporarily clear the OS maximized flag.
        if !self.fullscreen
            && let Some(maximized) = viewport.maximized
        {
            self.maximized = maximized;
        }
        if viewport.maximized != Some(false) || viewport.fullscreen != Some(false) {
            return;
        }
        let Some(rect) = viewport.inner_rect.filter(|rect| valid_size(rect.size())) else {
            return;
        };
        self.inner_size_points = Some(rect.size());
        if let Some(scale) = viewport.native_pixels_per_point {
            self.scale_factor = scale;
            self.outer_position_pixels = viewport.outer_rect.map(|rect| rect.min * scale * zoom);
        }
    }
}

fn valid_size(size: Vec2) -> bool {
    size.is_finite() && size.x >= 320.0 && size.y >= 560.0
}

pub fn window_builder(mut builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
    // Runs after eframe reads legacy geometry, even with persist_window=false.
    // Discard corrupt/minimized saves from older builds before creating a window.
    if !builder.inner_size.is_some_and(valid_size) {
        builder.inner_size = Some(egui::vec2(1440.0, 900.0));
        builder.position = None;
    }
    builder.maximized = None;
    builder.fullscreen = Some(false);
    builder
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestPass as _;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Storage(HashMap<String, String>);
    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn flush(&mut self) {}
    }

    fn frame(state: &mut WindowState, viewport: &egui::ViewportInfo) -> egui::FullOutput {
        let context = egui::Context::default();
        let mut input = egui::RawInput::default();
        input
            .viewports
            .insert(egui::ViewportId::ROOT, viewport.clone());
        context.test_pass(input, |context| state.update(context))
    }

    #[test]
    fn restart_preserves_normal_bounds_and_mode_without_saving_minimized_bounds() {
        let mut state = WindowState {
            maximized: false,
            ..Default::default()
        };
        let mut viewport = egui::ViewportInfo {
            minimized: Some(false),
            maximized: Some(false),
            fullscreen: Some(false),
            native_pixels_per_point: Some(1.5),
            inner_rect: Some(egui::Rect::from_min_size(
                egui::pos2(120.0, 80.0),
                egui::vec2(830.0, 670.0),
            )),
            outer_rect: Some(egui::Rect::from_min_size(
                egui::pos2(112.0, 50.0),
                egui::vec2(846.0, 708.0),
            )),
            ..Default::default()
        };
        frame(&mut state, &viewport); // Initial mode restoration, before observing bounds.
        frame(&mut state, &viewport);
        for maximized in [false, true] {
            viewport.minimized = Some(false);
            viewport.maximized = Some(maximized);
            if maximized {
                viewport.inner_rect = Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(1920.0, 1080.0),
                ));
            }
            frame(&mut state, &viewport);
            viewport.minimized = Some(true);
            viewport.inner_rect = Some(egui::Rect::ZERO);
            viewport.maximized = Some(false);
            frame(&mut state, &viewport);
            let mut storage = Storage::default();
            state.save(&mut storage);
            state = WindowState::load(Some(&storage));
            assert_eq!(state.inner_size_points, Some(egui::vec2(830.0, 670.0)));
            assert_eq!(state.outer_position_pixels, Some(egui::pos2(168.0, 75.0)));
            let output = frame(&mut state, &viewport);
            assert!(
                output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .contains(&egui::ViewportCommand::Maximized(maximized))
            );
            viewport.minimized = Some(false);
            // A redraw can precede the OS applying the requested mode.
            frame(&mut state, &viewport);
            state.save(&mut storage);
            assert_eq!(WindowState::load(Some(&storage)).maximized, maximized);
            viewport.maximized = Some(maximized);
            frame(&mut state, &viewport);
        }
        viewport.minimized = Some(false);
        viewport.fullscreen = Some(true);
        viewport.maximized = Some(false);
        frame(&mut state, &viewport);
        let mut storage = Storage::default();
        state.save(&mut storage);
        let mut restored = WindowState::load(Some(&storage));
        let output = frame(&mut restored, &viewport);
        let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(commands.contains(&egui::ViewportCommand::Fullscreen(true)));
        assert!(commands.contains(&egui::ViewportCommand::Maximized(true)));
        assert_eq!(restored.inner_size_points, Some(egui::vec2(830.0, 670.0)));
    }

    #[test]
    fn eframe_restores_geometry_but_never_the_mode() {
        let state = WindowState {
            outer_position_pixels: Some(egui::pos2(168.0, 75.0)),
            scale_factor: 1.5,
            fullscreen: true,
            maximized: true,
            inner_size_points: Some(egui::vec2(830.0, 670.0)),
            ..Default::default()
        };
        let mut storage = Storage::default();
        state.save(&mut storage);
        let eframe = eframe::get_value::<egui_winit::WindowSettings>(&storage, "window")
            .expect("eframe parses the record");
        assert_eq!(eframe.inner_size_points(), egui::vec2(830.0, 670.0));
        let record = eframe::get_value::<Record>(&storage, "window").unwrap();
        assert!(!record.fullscreen && !record.maximized);
        let restored = WindowState::load(Some(&storage));
        assert!(restored.fullscreen && restored.maximized);
        assert_eq!(restored.scale_factor, 1.5);
        assert_eq!(
            restored.outer_position_pixels,
            Some(egui::pos2(168.0, 75.0))
        );
    }

    #[test]
    fn migrates_eframe_0_33_records() {
        let mut storage = Storage::default();
        for maximized in [false, true] {
            eframe::Storage::set_string(
                &mut storage,
                "window",
                format!(
                    "(inner_position_pixels:Some((x:180.0,y:120.0)),\
                     outer_position_pixels:Some((x:168.0,y:75.0)),fullscreen:false,\
                     maximized:{maximized},inner_size_points:Some((x:830.0,y:670.0)))"
                ),
            );
            let state = WindowState::load(Some(&storage));
            assert_eq!(state.maximized, maximized);
            assert_eq!(state.inner_size_points, Some(egui::vec2(830.0, 670.0)));
            assert_eq!(state.outer_position_pixels, Some(egui::pos2(168.0, 75.0)));
        }
    }

    #[test]
    fn legacy_minimized_geometry_falls_back_to_usable_defaults() {
        let state = WindowState {
            inner_size_points: Some(egui::vec2(160.0, 28.0)),
            ..Default::default()
        };
        let mut storage = Storage::default();
        state.save(&mut storage);
        assert_eq!(
            WindowState::load(Some(&storage)).inner_size_points,
            Some(egui::vec2(1440.0, 900.0))
        );
        let builder = window_builder(
            egui::ViewportBuilder::default()
                .with_inner_size([160.0, 28.0])
                .with_position([-32000.0, -32000.0])
                .with_maximized(true),
        );
        assert_eq!(builder.inner_size, Some(egui::vec2(1440.0, 900.0)));
        assert_eq!(builder.position, None);
        assert_eq!(builder.maximized, None);
    }
}
