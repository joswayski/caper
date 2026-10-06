//! Compression settings sent by the API with every job (`/start`).
//!
//! The API owns the values; the worker validates them defensively so a bad
//! configuration can never make a tool misbehave: out-of-range numbers are
//! clamped, and wrong types or unknown presets fall back to the defaults
//! documented in `docs/media.md`.

use serde::Serialize;
use serde_json::Value;

/// x264 presets the worker accepts (`placebo` is never worth the time).
pub const PRESETS: [&str; 9] = [
    "ultrafast",
    "superfast",
    "veryfast",
    "faster",
    "fast",
    "medium",
    "slow",
    "slower",
    "veryslow",
];

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub image_avif_quality: u8,
    pub image_avif_speed: u8,
    pub image_lossless_ratio: f64,
    pub image_min_savings_percent: f64,
    pub image_max_edge: u32,
    pub preview_edge: u32,
    pub video_crf: u8,
    pub video_preset: String,
    pub video_long_seconds: u32,
    pub video_long_preset: String,
    pub video_max_height: u32,
    pub audio_kbps: u32,
    pub file_min_savings_percent: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            image_avif_quality: 90,
            image_avif_speed: 6,
            image_lossless_ratio: 1.3,
            image_min_savings_percent: 15.0,
            image_max_edge: 0,
            preview_edge: 640,
            video_crf: 20,
            video_preset: "slow".into(),
            video_long_seconds: 300,
            video_long_preset: "veryfast".into(),
            video_max_height: 1080,
            audio_kbps: 128,
            file_min_savings_percent: 10.0,
        }
    }
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key)?.as_f64().filter(|v| v.is_finite())
}

/// Integer setting clamped to `min..=max`; fractional values are truncated.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn int(value: &Value, key: &str, default: u32, min: u32, max: u32) -> u32 {
    number(value, key).map_or(default, |v| v.clamp(f64::from(min), f64::from(max)) as u32)
}

fn float(value: &Value, key: &str, default: f64, min: f64, max: f64) -> f64 {
    number(value, key).map_or(default, |v| v.clamp(min, max))
}

fn preset(value: &Value, key: &str, default: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .filter(|p| PRESETS.contains(&p.as_str()))
        .unwrap_or_else(|| default.to_owned())
}

/// Edge limits: `0` disables the limit, anything else is kept within a
/// sensible range.
fn edge(value: &Value, key: &str, default: u32, min: u32, max: u32) -> u32 {
    match number(value, key) {
        Some(v) if v <= 0.0 => 0,
        Some(_) => int(value, key, default, min, max),
        None => default,
    }
}

impl Settings {
    /// Reads the API's `settings` object leniently (see the module docs).
    #[allow(clippy::cast_possible_truncation)]
    pub fn from_value(value: &Value) -> Self {
        let d = Self::default();
        Self {
            image_avif_quality: int(
                value,
                "imageAvifQuality",
                d.image_avif_quality.into(),
                1,
                100,
            ) as u8,
            image_avif_speed: int(value, "imageAvifSpeed", d.image_avif_speed.into(), 0, 10) as u8,
            image_lossless_ratio: float(
                value,
                "imageLosslessRatio",
                d.image_lossless_ratio,
                0.0,
                10.0,
            ),
            image_min_savings_percent: float(
                value,
                "imageMinSavingsPercent",
                d.image_min_savings_percent,
                0.0,
                100.0,
            ),
            image_max_edge: edge(value, "imageMaxEdge", d.image_max_edge, 16, 65_535),
            preview_edge: int(value, "previewEdge", d.preview_edge, 16, 4096),
            video_crf: int(value, "videoCrf", d.video_crf.into(), 0, 51) as u8,
            video_preset: preset(value, "videoPreset", &d.video_preset),
            video_long_seconds: int(value, "videoLongSeconds", d.video_long_seconds, 0, 86_400),
            video_long_preset: preset(value, "videoLongPreset", &d.video_long_preset),
            video_max_height: edge(value, "videoMaxHeight", d.video_max_height, 144, 8192),
            audio_kbps: int(value, "audioKbps", d.audio_kbps, 32, 512),
            file_min_savings_percent: float(
                value,
                "fileMinSavingsPercent",
                d.file_min_savings_percent,
                0.0,
                100.0,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn documented_defaults_round_trip() {
        let value = json!({"imageAvifQuality":90,"imageAvifSpeed":6,"imageLosslessRatio":1.3,"imageMinSavingsPercent":15,"imageMaxEdge":0,"previewEdge":640,"videoCrf":20,"videoPreset":"slow","videoLongSeconds":300,"videoLongPreset":"veryfast","videoMaxHeight":1080,"audioKbps":128,"fileMinSavingsPercent":10});
        assert_eq!(Settings::from_value(&value), Settings::default());
        let serialized = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(Settings::from_value(&serialized), Settings::default());
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let s = Settings::from_value(&json!({
            "imageAvifQuality": 400, "imageAvifSpeed": -3, "imageLosslessRatio": 99.0,
            "imageMinSavingsPercent": -5, "imageMaxEdge": 3, "previewEdge": 1_000_000,
            "videoCrf": 80, "videoLongSeconds": -1, "videoMaxHeight": 100,
            "audioKbps": 4, "fileMinSavingsPercent": 250
        }));
        assert_eq!(s.image_avif_quality, 100);
        assert_eq!(s.image_avif_speed, 0);
        assert!((s.image_lossless_ratio - 10.0).abs() < f64::EPSILON);
        assert!(s.image_min_savings_percent.abs() < f64::EPSILON);
        assert_eq!(s.image_max_edge, 16);
        assert_eq!(s.preview_edge, 4096);
        assert_eq!(s.video_crf, 51);
        assert_eq!(s.video_long_seconds, 0);
        assert_eq!(s.video_max_height, 144);
        assert_eq!(s.audio_kbps, 32);
        assert!((s.file_min_savings_percent - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn wrong_types_and_unknown_presets_fall_back_to_defaults() {
        let s = Settings::from_value(&json!({
            "imageAvifQuality": "high", "videoPreset": "placebo", "videoLongPreset": "FAST",
            "imageLosslessRatio": null, "videoMaxHeight": -1, "imageMaxEdge": 0
        }));
        assert_eq!(s.image_avif_quality, 90);
        assert_eq!(s.video_preset, "slow");
        assert_eq!(s.video_long_preset, "fast");
        assert!((s.image_lossless_ratio - 1.3).abs() < f64::EPSILON);
        assert_eq!(s.video_max_height, 0);
        assert_eq!(s.image_max_edge, 0);
        assert_eq!(Settings::from_value(&json!(null)), Settings::default());
    }
}
