//! In-app playback (`caper_client::player`), with the current video frame
//! as an egui texture.

pub use caper_client::player::{State, time_label};
use eframe::egui;

pub struct Player {
    core: caper_client::player::Player,
    texture: Option<egui::TextureHandle>,
}

impl std::ops::Deref for Player {
    type Target = caper_client::player::Player;
    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl std::ops::DerefMut for Player {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}

impl Player {
    /// `width`/`height` are the video's display size (rotation applied);
    /// `fit` bounds the decoded frames, which are never enlarged.
    pub fn new(
        binary: &std::path::Path,
        input: String,
        width: u32,
        height: u32,
        duration_ms: Option<u64>,
        fps: Option<f32>,
        fit: egui::Vec2,
    ) -> Self {
        Self {
            core: caper_client::player::Player::new(
                binary,
                input,
                width,
                height,
                duration_ms,
                fps,
                fit.into(),
            ),
            texture: None,
        }
    }

    /// Sound alone (a voice message).
    pub fn audio(binary: &std::path::Path, input: String, duration_ms: Option<u64>) -> Self {
        Self {
            core: caper_client::player::Player::audio(binary, input, duration_ms),
            texture: None,
        }
    }

    /// The decoded frame size, for laying out before the first frame.
    pub fn frame_size(&self) -> egui::Vec2 {
        let [width, height] = self.core.frame_size();
        egui::vec2(width as f32, height as f32)
    }

    pub fn texture(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref()
    }

    /// Advance to the frame for the current time; returns when to repaint.
    pub fn update(&mut self, context: &egui::Context) -> Option<Duration> {
        let texture = &mut self.texture;
        self.core.update(|size, pixels| {
            let image = egui::ColorImage::from_rgba_unmultiplied(size, pixels);
            match texture {
                Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                None => {
                    *texture = Some(context.load_texture(
                        "video-frame",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
        })
    }
}

use std::time::Duration;
