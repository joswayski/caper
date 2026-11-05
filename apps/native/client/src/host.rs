//! What the background worker asks of the UI that runs it.

use std::time::Duration;

/// The UI the worker reports to. Calls come from background threads.
pub trait Host: Clone + Send + Sync + 'static {
    /// Process new events as soon as possible.
    fn wake(&self);
    /// Process events again after `delay`.
    fn wake_after(&self, delay: Duration);
    /// Restore the window from the taskbar and focus it.
    fn bring_to_front(&self);
    /// The window's title (the unread count fallback for launcher badges).
    fn set_title(&self, title: String);
}

/// A decoded image: `size` is `[width, height]` and `pixels` are
/// unmultiplied RGBA8, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub size: [usize; 2],
    pub pixels: Vec<u8>,
}

impl Rgba {
    pub fn from_image(image: image::RgbaImage) -> Self {
        Self {
            size: [image.width() as usize, image.height() as usize],
            pixels: image.into_raw(),
        }
    }
}

/// A host for tests: nothing to wake, focus or retitle.
#[cfg(test)]
#[derive(Clone)]
pub struct Quiet;

#[cfg(test)]
impl Host for Quiet {
    fn wake(&self) {}
    fn wake_after(&self, _: Duration) {}
    fn bring_to_front(&self) {}
    fn set_title(&self, _: String) {}
}
