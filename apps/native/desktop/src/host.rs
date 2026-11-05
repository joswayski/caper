//! The egui window as the background worker's host.

use caper_client::{Host, Rgba};
use eframe::egui;
use std::time::Duration;

/// Repaints and commands the root viewport for the worker.
#[derive(Clone)]
pub struct Repaint(pub egui::Context);

impl Host for Repaint {
    fn wake(&self) {
        self.0.request_repaint();
    }

    fn wake_after(&self, delay: Duration) {
        self.0.request_repaint_after(delay);
    }

    fn bring_to_front(&self) {
        self.0
            .send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        self.0.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn set_title(&self, title: String) {
        self.0
            .send_viewport_cmd(egui::ViewportCommand::Title(title));
    }
}

/// A core image as an egui image, exactly: both are unmultiplied RGBA8.
pub fn color_image(image: Rgba) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied(image.size, &image.pixels)
}
