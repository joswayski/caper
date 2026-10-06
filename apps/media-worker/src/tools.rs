//! Running the external encoders (ffmpeg, libvips, cwebp, avifenc, jpegtran)
//! with deadlines, and parsing their metadata output.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::process::Command;

/// Upper bound for any single tool run when no Lambda deadline applies.
const LOCAL_LIMIT: Duration = Duration::from_secs(3600);

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("{tool} could not start: {source}")]
    Spawn {
        tool: String,
        source: std::io::Error,
    },
    #[error("{tool} timed out")]
    Timeout { tool: String },
    #[error("{tool} failed ({status}): {stderr}")]
    Failed {
        tool: String,
        status: String,
        stderr: String,
    },
}

impl ToolError {
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout { .. })
    }
}

/// Time left before `deadline`, or the local limit.
pub fn remaining(deadline: Option<Instant>) -> Duration {
    deadline.map_or(LOCAL_LIMIT, |d| d.saturating_duration_since(Instant::now()))
}

/// Last ~400 characters of stderr: enough to diagnose a failure without
/// echoing large tool output into logs.
fn tail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    let start = text
        .char_indices()
        .rev()
        .nth(399)
        .map_or(0, |(index, _)| index);
    text[start..].replace('\n', " | ")
}

pub fn command<I, S>(tool: &str, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(tool);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

/// Runs a tool to completion within `deadline`, returning its stdout.
pub async fn run<I, S>(tool: &str, args: I, deadline: Option<Instant>) -> Result<Vec<u8>, ToolError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let child = command(tool, args)
        .spawn()
        .map_err(|source| ToolError::Spawn {
            tool: tool.into(),
            source,
        })?;
    let output = tokio::time::timeout(remaining(deadline), child.wait_with_output())
        .await
        .map_err(|_| ToolError::Timeout { tool: tool.into() })?
        .map_err(|source| ToolError::Spawn {
            tool: tool.into(),
            source,
        })?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(ToolError::Failed {
            tool: tool.into(),
            status: output.status.to_string(),
            stderr: tail(&output.stderr),
        })
    }
}

// ---------------------------------------------------------------- libvips

/// `vipsheader -a` fields.
#[derive(Debug, Default)]
pub struct VipsHeader {
    fields: HashMap<String, String>,
}

impl VipsHeader {
    pub fn parse(text: &str) -> Self {
        let fields = text
            .lines()
            .filter_map(|line| line.split_once(": "))
            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            .collect();
        Self { fields }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    fn int(&self, key: &str) -> Option<u32> {
        // Some fields print a type suffix, e.g. `1 (gint)` on older builds.
        self.get(key)?.split_whitespace().next()?.parse().ok()
    }

    pub fn width(&self) -> Option<u32> {
        self.int("width")
    }

    pub fn height(&self) -> Option<u32> {
        self.int("height")
    }

    pub fn pages(&self) -> u32 {
        self.int("n-pages").unwrap_or(1).max(1)
    }

    pub fn page_height(&self) -> Option<u32> {
        self.int("page-height")
    }

    /// EXIF orientation (1–8); 1 when absent or invalid.
    pub fn orientation(&self) -> u8 {
        self.int("orientation")
            .and_then(|o| u8::try_from(o).ok())
            .filter(|o| (1..=8).contains(o))
            .unwrap_or(1)
    }

    pub fn has(&self, key: &str) -> bool {
        self.fields.contains_key(key)
    }

    /// Per-frame delays in milliseconds (animated GIF/WebP).
    pub fn delays(&self) -> Vec<u32> {
        self.get("delay")
            .map(|d| {
                d.split_whitespace()
                    .filter_map(|v| v.parse().ok())
                    .collect()
            })
            .unwrap_or_default()
    }
}

pub async fn vipsheader(
    path: &std::path::Path,
    deadline: Option<Instant>,
) -> Result<VipsHeader, ToolError> {
    let out = run("vipsheader", [OsStr::new("-a"), path.as_os_str()], deadline).await?;
    Ok(VipsHeader::parse(&String::from_utf8_lossy(&out)))
}

// ---------------------------------------------------------------- ffprobe

#[derive(Debug, Default, Deserialize)]
pub struct Probe {
    #[serde(default)]
    pub streams: Vec<Stream>,
    #[serde(default)]
    pub format: Format,
}

#[derive(Debug, Default, Deserialize)]
pub struct Format {
    #[serde(default)]
    pub format_name: String,
    pub duration: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Stream {
    #[serde(default)]
    pub codec_type: String,
    #[serde(default)]
    pub codec_name: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub pix_fmt: Option<String>,
    pub sample_aspect_ratio: Option<String>,
    pub duration: Option<String>,
    pub channels: Option<u32>,
    #[serde(default)]
    pub disposition: HashMap<String, i64>,
    #[serde(default)]
    pub tags: HashMap<String, String>,
    #[serde(default)]
    pub side_data_list: Vec<serde_json::Value>,
}

fn seconds(value: Option<&String>) -> Option<f64> {
    value?
        .parse::<f64>()
        .ok()
        .filter(|s| s.is_finite() && *s >= 0.0)
}

impl Stream {
    pub fn is_video(&self) -> bool {
        self.codec_type == "video"
            && self.disposition.get("attached_pic").copied().unwrap_or(0) == 0
            && self.width.unwrap_or(0) > 0
            && self.height.unwrap_or(0) > 0
    }

    /// Display rotation in degrees, normalised to 0, 90, 180 or 270.
    pub fn rotation(&self) -> u32 {
        let from_side_data = self
            .side_data_list
            .iter()
            .find_map(|s| s.get("rotation").and_then(serde_json::Value::as_f64));
        let from_tag = self.tags.get("rotate").and_then(|r| r.parse::<f64>().ok());
        let degrees = from_side_data.or(from_tag).unwrap_or(0.0);
        #[allow(clippy::cast_possible_truncation)]
        let quarter = (degrees / 90.0).round() as i64;
        #[allow(clippy::cast_sign_loss)]
        let rotation = (quarter.rem_euclid(4) * 90) as u32;
        rotation
    }

    fn sar(&self) -> f64 {
        let Some((n, d)) = self
            .sample_aspect_ratio
            .as_deref()
            .and_then(|s| s.split_once(':'))
        else {
            return 1.0;
        };
        match (n.parse::<f64>(), d.parse::<f64>()) {
            (Ok(n), Ok(d)) if n > 0.0 && d > 0.0 => n / d,
            _ => 1.0,
        }
    }

    /// Width and height as displayed: pixel aspect applied, then rotation.
    pub fn display_size(&self) -> Option<(u32, u32)> {
        let (w, h) = (self.width?, self.height?);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let w = ((f64::from(w) * self.sar()).round() as u32).max(1);
        Some(if self.rotation() % 180 == 90 {
            (h, w)
        } else {
            (w, h)
        })
    }
}

impl Probe {
    pub fn video(&self) -> Option<&Stream> {
        self.streams.iter().find(|s| s.is_video())
    }

    pub fn audio(&self) -> impl Iterator<Item = &Stream> {
        self.streams.iter().filter(|s| s.codec_type == "audio")
    }

    pub fn duration_seconds(&self) -> Option<f64> {
        seconds(self.format.duration.as_ref()).or_else(|| {
            self.streams
                .iter()
                .find_map(|s| seconds(s.duration.as_ref()))
        })
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn duration_ms(&self) -> Option<u64> {
        self.duration_seconds().map(|s| (s * 1000.0).round() as u64)
    }

    /// QuickTime/ISO media (`mov,mp4,m4a,3gp,3g2,mj2`).
    pub fn is_iso_media(&self) -> bool {
        self.format
            .format_name
            .split(',')
            .any(|f| f == "mov" || f == "mp4")
    }
}

pub async fn ffprobe(
    path: &std::path::Path,
    deadline: Option<Instant>,
) -> Result<Probe, ToolError> {
    let out = run(
        "ffprobe",
        [
            OsStr::new("-v"),
            OsStr::new("error"),
            OsStr::new("-print_format"),
            OsStr::new("json"),
            OsStr::new("-show_format"),
            OsStr::new("-show_streams"),
            path.as_os_str(),
        ],
        deadline,
    )
    .await?;
    serde_json::from_slice(&out).map_err(|e| ToolError::Failed {
        tool: "ffprobe".into(),
        status: "invalid json".into(),
        stderr: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vips_header_fields() {
        let h = VipsHeader::parse(
            "width: 4032\nheight: 3024\nn-pages: 3\norientation: 6\ndelay: 100 50 0 \nexif-data: 1234 bytes of binary data\n",
        );
        assert_eq!((h.width(), h.height()), (Some(4032), Some(3024)));
        assert_eq!(h.pages(), 3);
        assert_eq!(h.orientation(), 6);
        assert_eq!(h.delays(), vec![100, 50, 0]);
        assert!(h.has("exif-data"));
        let empty = VipsHeader::parse("");
        assert_eq!(empty.pages(), 1);
        assert_eq!(empty.orientation(), 1);
        assert_eq!(VipsHeader::parse("orientation: 9").orientation(), 1);
    }

    #[test]
    fn probe_rotation_and_display_size() {
        let probe: Probe = serde_json::from_str(
            r#"{"streams":[
                {"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"sample_aspect_ratio":"1:1",
                 "side_data_list":[{"side_data_type":"Display Matrix","rotation":-90}]},
                {"codec_type":"audio","codec_name":"aac","channels":2},
                {"codec_type":"video","codec_name":"mjpeg","width":300,"height":300,"disposition":{"attached_pic":1}}
            ],"format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"12.345"}}"#,
        )
        .unwrap();
        let video = probe.video().unwrap();
        assert_eq!(video.rotation(), 270);
        assert_eq!(video.display_size(), Some((1080, 1920)));
        assert_eq!(probe.duration_ms(), Some(12_345));
        assert!(probe.is_iso_media());
        assert_eq!(probe.audio().count(), 1);

        let anamorphic: Stream = serde_json::from_str(
            r#"{"codec_type":"video","width":720,"height":576,"sample_aspect_ratio":"16:11","tags":{"rotate":"180"}}"#,
        )
        .unwrap();
        assert_eq!(anamorphic.rotation(), 180);
        assert_eq!(anamorphic.display_size(), Some((1047, 576)));
    }

    #[test]
    fn stderr_tail_is_bounded() {
        let long = "x".repeat(5000);
        assert_eq!(tail(long.as_bytes()).len(), 400);
        assert_eq!(tail(b"a\nb\n"), "a | b");
    }
}
