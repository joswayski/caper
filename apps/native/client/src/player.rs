//! Frame sizing shared by the in-app video player and FFmpeg still decoding.

/// The largest even size within `fit` (and the video's own size) that keeps
/// the aspect ratio.
pub fn output_size(width: u32, height: u32, fit: [f32; 2]) -> [usize; 2] {
    let (width, height) = (width.max(2) as f32, height.max(2) as f32);
    let scale = (fit[0] / width).min(fit[1] / height).clamp(0.01, 1.0);
    let even = |value: f32| ((value * scale / 2.0).round() as usize * 2).max(2);
    [even(width), even(height)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_fit_the_viewer_without_enlarging_and_stay_even() {
        assert_eq!(output_size(3840, 2160, [1280.0, 800.0]), [1280, 720]);
        assert_eq!(output_size(1080, 1920, [1280.0, 800.0]), [450, 800]);
        assert_eq!(output_size(640, 360, [1920.0, 1080.0]), [640, 360]);
        assert_eq!(output_size(641, 361, [1920.0, 1080.0]), [642, 362]);
    }
}
