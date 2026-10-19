//! Keep the normal restore rectangle, not Windows' minimized/maximized bounds.
use eframe::egui::{self, Pos2, Vec2};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

// This is eframe 0.33's persisted `window` format. Writing the same record lets
// eframe retain its monitor/DPI-aware placement and off-screen clamping.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct WindowState {
    inner_position_pixels: Option<Pos2>,
    outer_position_pixels: Option<Pos2>,
    fullscreen: bool,
    maximized: bool,
    inner_size_points: Option<Vec2>,
    #[serde(skip)]
    restore_pending: bool,
    #[serde(skip)]
    restore_deadline: Option<Instant>,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            inner_position_pixels: None,
            outer_position_pixels: None,
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
            .and_then(|storage| eframe::get_value::<Self>(storage, "window"))
            .filter(|state| state.inner_size_points.is_some_and(valid_size))
            .unwrap_or_default()
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "window", self);
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
            let scale = scale * zoom;
            self.inner_position_pixels = Some(rect.min * scale);
            self.outer_position_pixels = viewport.outer_rect.map(|rect| rect.min * scale);
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
    use std::collections::HashMap;

    #[derive(Default)]
    struct Storage(HashMap<String, String>);
    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
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
        context.run(input, |context| state.update(context))
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
