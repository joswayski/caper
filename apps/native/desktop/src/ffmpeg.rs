//! The FFmpeg executable shipped next to the app (see `build.sh`/`build.ps1`):
//! video compression before upload and decoding for the in-app player.
//!
//! It runs as a separate program over pipes, never linked in, so the app stays
//! free of `unsafe` and of FFmpeg's GPL. When it is missing (a development
//! build without `CAPER_FFMPEG`), videos upload unchanged and the player
//! offers the browser instead.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::compress::Compression;

/// Same rules as web (`apps/web/src/chat/prepare.ts`): bitrate headroom before
/// a re-encode, a floor for small videos, and the 1080p reference size.
const BITRATE_SLACK: f64 = 1.25;
const MIN_VIDEO_KBPS: u32 = 1500;
const FULL_HD_PIXELS: f64 = 1920.0 * 1080.0;
const MIN_SAVING: f64 = 0.1;

/// The bundled executable, or `CAPER_FFMPEG` for development and tests.
pub fn binary() -> Option<&'static Path> {
    static BINARY: OnceLock<Option<PathBuf>> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            let name = if cfg!(windows) {
                "ffmpeg.exe"
            } else {
                "ffmpeg"
            };
            let beside = std::env::current_exe()
                .ok()
                .and_then(|exe| exe.canonicalize().ok())
                .and_then(|exe| exe.parent().map(|dir| dir.join(name)));
            std::env::var_os("CAPER_FFMPEG")
                .map(PathBuf::from)
                .into_iter()
                .chain(beside)
                // The .deb keeps it beside its libraries, like the updater.
                .chain(
                    cfg!(target_os = "linux")
                        .then(|| PathBuf::from("/usr/lib/caper-desktop").join(name)),
                )
                .find(|path| path.is_file())
        })
        .as_deref()
}

/// The FFmpeg that makes test inputs: `CAPER_FFMPEG_FIXTURES` when the one
/// under test (`CAPER_FFMPEG`, such as the shipped trimmed build) has no lavfi
/// test sources or libx265, otherwise that same one.
#[cfg(test)]
pub fn fixture_binary() -> PathBuf {
    std::env::var_os("CAPER_FFMPEG_FIXTURES")
        .map(PathBuf::from)
        .or_else(|| binary().map(Path::to_path_buf))
        .expect("set CAPER_FFMPEG")
}

/// A quiet, windowless invocation with no stdin.
pub fn command(binary: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .args(["-nostdin", "-hide_banner"])
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes up for each call.
        command.creation_flags(0x0800_0000);
    }
    command
}

/// Decode an AVIF/HEIC still at the caller's display bound. The container
/// needs seeking, so use the same disposable scratch files as video uploads.
pub fn decode_image(
    bytes: &[u8],
    content_type: &str,
    max_edge: u32,
) -> Option<eframe::egui::ColorImage> {
    let binary = binary()?;
    let extension = match content_type {
        "image/avif" => "avif",
        "image/heic" | "image/heif" => "heic",
        _ => return None,
    };
    let scratch = crate::uploads::Scratch::new(extension);
    std::fs::write(scratch.path(), bytes).ok()?;
    let stream = probe(binary, scratch.path()).ok()?.video?;
    let [width, height] = crate::player::output_size(
        stream.width,
        stream.height,
        eframe::egui::vec2(max_edge as f32, max_edge as f32),
    );
    let output = command(binary)
        .args(["-loglevel", "error", "-i"])
        .arg(scratch.path())
        .args([
            "-frames:v",
            "1",
            "-vf",
            &format!("scale={width}:{height}:flags=lanczos,format=rgba"),
        ])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    (output.status.success() && output.stdout.len() == width * height * 4)
        .then(|| eframe::egui::ColorImage::from_rgba_unmultiplied([width, height], &output.stdout))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Probe {
    pub duration_ms: Option<u64>,
    /// Whole-file bitrate.
    pub total_kbps: Option<u32>,
    pub video: Option<VideoStream>,
    pub audio: Option<AudioStream>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VideoStream {
    pub codec: String,
    /// Display size, after rotation.
    pub width: u32,
    pub height: u32,
    pub kbps: Option<u32>,
    pub fps: Option<f32>,
    /// BT.2100 PQ or HLG.
    pub hdr: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioStream {
    pub codec: String,
    pub channels: u32,
    pub kbps: Option<u32>,
}

/// Reads FFmpeg's input summary (`ffmpeg -i <file>`). Only the first video
/// and audio streams matter.
pub fn parse_probe(log: &str) -> Probe {
    let mut probe = Probe::default();
    let mut rotation = 0_i32;
    for line in log.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Duration: ") {
            probe.duration_ms = rest.split(',').next().and_then(duration_ms);
            probe.total_kbps = rest
                .split(", ")
                .find_map(|part| part.strip_prefix("bitrate: "))
                .and_then(kbps);
        } else if let Some(rest) = rotation_of(line) {
            if probe.video.is_some() && rotation == 0 {
                rotation = rest
                    .split_whitespace()
                    .next()
                    .and_then(|degrees| degrees.parse::<f32>().ok())
                    .map_or(0, |degrees| degrees.round() as i32);
            }
        } else if line.starts_with("Stream #")
            && let Some((_, details)) = line.split_once(": Video: ")
            && probe.video.is_none()
        {
            probe.video = parse_video(details);
        } else if line.starts_with("Stream #")
            && let Some((_, details)) = line.split_once(": Audio: ")
            && probe.audio.is_none()
        {
            probe.audio = Some(parse_audio(details));
        }
    }
    if let Some(video) = &mut probe.video
        && rotation.rem_euclid(180) == 90
    {
        std::mem::swap(&mut video.width, &mut video.height);
    }
    probe
}

/// `displaymatrix: rotation of -90.00 degrees` (FFmpeg 6) or `Display
/// Matrix: rotation of 90.00 degrees` (FFmpeg 7 and later).
fn rotation_of(line: &str) -> Option<&str> {
    let (label, rest) = line.split_once(": rotation of ")?;
    matches!(
        label.to_ascii_lowercase().as_str(),
        "displaymatrix" | "display matrix"
    )
    .then_some(rest)
}

fn duration_ms(value: &str) -> Option<u64> {
    let mut parts = value.trim().split(':');
    let hours: f64 = parts.next()?.parse().ok()?;
    let minutes: f64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next()?.parse().ok()?;
    let total = (hours * 3600.0 + minutes * 60.0 + seconds) * 1000.0;
    (total.is_finite() && total > 0.0).then(|| total.round() as u64)
}

/// `"172 kb/s"`, also with a trailing note such as `(default)`.
fn kbps(value: &str) -> Option<u32> {
    value.trim().split_once(" kb/s")?.0.trim().parse().ok()
}

/// Top-level comma-separated fields: commas inside parentheses (colour
/// details such as `yuv420p10le(tv, bt2020nc/bt2020/arib-std-b67)`) stay put.
fn fields(details: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let (mut depth, mut start) = (0_i32, 0);
    for (index, character) in details.char_indices() {
        match character {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                fields.push(details[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    fields.push(details[start..].trim());
    fields
}

fn parse_video(details: &str) -> Option<VideoStream> {
    let fields = fields(details);
    let codec = fields.first()?.split_whitespace().next()?.to_owned();
    let (width, height) = fields.iter().find_map(|field| {
        let size = field.split_whitespace().next()?;
        let (width, height) = size.split_once('x')?;
        Some((width.parse().ok()?, height.parse().ok()?))
    })?;
    let hdr = fields
        .iter()
        .any(|field| field.contains("smpte2084") || field.contains("arib-std-b67"));
    Some(VideoStream {
        codec,
        width,
        height,
        kbps: fields.iter().find_map(|field| kbps(field)),
        fps: fields
            .iter()
            .find_map(|field| field.strip_suffix(" fps")?.parse().ok()),
        hdr,
    })
}

fn parse_audio(details: &str) -> AudioStream {
    let fields = fields(details);
    let channels = fields
        .iter()
        .find_map(|field| match *field {
            "mono" => Some(1),
            "stereo" => Some(2),
            layout => layout
                .split(['(', ' '])
                .next()
                .and_then(|layout| layout.split_once('.'))
                .and_then(|(main, lfe)| Some(main.parse::<u32>().ok()? + lfe.parse::<u32>().ok()?)),
        })
        .unwrap_or(2);
    AudioStream {
        codec: fields
            .first()
            .and_then(|field| field.split_whitespace().next())
            .unwrap_or_default()
            .to_owned(),
        channels,
        kbps: fields.iter().find_map(|field| kbps(field)),
    }
}

/// `ffmpeg -i`: prints the summary and exits non-zero (no output was named).
pub fn probe(binary: &Path, input: &Path) -> Result<Probe, String> {
    let output = command(binary)
        .args(["-loglevel", "info", "-i"])
        .arg(input)
        .output()
        .map_err(|error| format!("Could not start FFmpeg: {error}"))?;
    let probe = parse_probe(&String::from_utf8_lossy(&output.stderr));
    if probe.video.is_none() {
        return Err("FFmpeg found no video stream.".into());
    }
    Ok(probe)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Hdr,
    Resolution,
    Codec,
    Container,
    Bitrate,
    /// Cropped, turned or drawn on before sending: always re-encoded.
    Edited,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoPlan {
    pub reasons: Vec<Reason>,
    pub width: u32,
    pub height: u32,
    pub kbps: u32,
    /// The sender's edit, baked into the frames.
    pub edit: Option<VideoEdit>,
}

/// An edit as FFmpeg applies it, in the display orientation (after the
/// track's own rotation): clockwise quarter turns, then the crop in pixels of
/// the turned frame, then (after scaling) the marks as a still PNG overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoEdit {
    pub rotation: u16,
    pub crop: crate::attachment_edit::PixelRect,
    pub overlay: Option<PathBuf>,
}

/// `videoBitrateKbps` at 1080p30, scaled by pixel count and (above 30 fps,
/// up to `videoHighFpsBoostPercent`) frame rate, with a floor (web's
/// `videoTargetKbps`).
pub fn target_kbps(width: u32, height: u32, fps: Option<f32>, settings: &Compression) -> u32 {
    let boost = fps
        .filter(|fps| *fps > 30.0)
        .map_or(1.0, |fps| {
            (f64::from(fps) / 30.0).min(f64::from(settings.video_high_fps_boost_percent) / 100.0)
        })
        .max(1.0);
    let scaled =
        f64::from(settings.video_bitrate_kbps) * f64::from(width) * f64::from(height) * boost
            / FULL_HD_PIXELS;
    (scaled.round() as u32).max(MIN_VIDEO_KBPS)
}

/// The target, but never above `videoSourceBitratePercent` of the source's
/// own bitrate (100% for H.264, which more bits cannot improve), as web's
/// `outputKbps`. The floor still applies.
pub fn output_kbps(
    target: u32,
    source: Option<u32>,
    source_is_h264: bool,
    settings: &Compression,
) -> u32 {
    let percent = if source_is_h264 {
        100
    } else {
        settings.video_source_bitrate_percent
    };
    match source {
        Some(source) if source > 0 && percent > 0 => {
            let limit = (f64::from(source) * f64::from(percent) / 100.0).round() as u32;
            target.min(limit).max(MIN_VIDEO_KBPS)
        }
        _ => target,
    }
}

/// Transcode only when needed (web's `videoPlan`); `None` uploads the
/// original. The short edge is bounded, so portrait video keeps its detail.
pub fn video_plan(
    probe: &Probe,
    inline_container: bool,
    settings: &Compression,
) -> Option<VideoPlan> {
    let video = probe.video.as_ref()?;
    if settings.video_max_height == 0 || video.width == 0 || video.height == 0 {
        return None;
    }
    let mut reasons = Vec::new();
    if video.hdr {
        reasons.push(Reason::Hdr);
    }
    let short = video.width.min(video.height);
    let (mut width, mut height) = (video.width, video.height);
    if short > settings.video_max_height {
        reasons.push(Reason::Resolution);
        let even = (settings.video_max_height / 2 * 2).max(2);
        let scale = f64::from(even) / f64::from(short);
        let other = |size: u32| ((f64::from(size) * scale / 2.0).round() as u32 * 2).max(2);
        (width, height) = if video.width < video.height {
            (even, other(video.height))
        } else {
            (other(video.width), even)
        };
    }
    if video.codec != "h264" {
        reasons.push(Reason::Codec);
    }
    if !inline_container {
        reasons.push(Reason::Container);
    }
    let kbps = video_kbps(probe);
    let source_target = f64::from(target_kbps(video.width, video.height, video.fps, settings));
    if kbps.is_some_and(|kbps| f64::from(kbps) > source_target * BITRATE_SLACK) {
        reasons.push(Reason::Bitrate);
    }
    (!reasons.is_empty()).then(|| VideoPlan {
        kbps: output_kbps(
            target_kbps(width, height, video.fps, settings),
            kbps,
            video.codec == "h264",
            settings,
        ),
        reasons,
        width,
        height,
        edit: None,
    })
}

/// The video stream's own bitrate, else the whole file's less its audio.
fn video_kbps(probe: &Probe) -> Option<u32> {
    probe.video.as_ref()?.kbps.or_else(|| {
        let audio = probe
            .audio
            .as_ref()
            .and_then(|audio| audio.kbps)
            .unwrap_or(0);
        probe.total_kbps.map(|total| total.saturating_sub(audio))
    })
}

/// An edited video is always transcoded, whatever [`video_plan`] would say:
/// the crop of the turned display frame (even pixels), its short edge capped
/// at `videoMaxHeight` (0 = no cap), at the bitrate for that size. The
/// overlay is added once it has been painted at the plan's size.
pub fn edited_plan(
    probe: &Probe,
    edit: &crate::attachment_edit::Edit,
    settings: &Compression,
) -> Option<VideoPlan> {
    let video = probe.video.as_ref()?;
    if video.width < 2 || video.height < 2 {
        return None;
    }
    let crop = edit.crop_pixels(video.width, video.height, true);
    let (mut width, mut height) = (crop.width, crop.height);
    let short = width.min(height);
    if settings.video_max_height > 0 && short > settings.video_max_height {
        let even = (settings.video_max_height / 2 * 2).max(2);
        let scale = f64::from(even) / f64::from(short);
        let other = |size: u32| ((f64::from(size) * scale / 2.0).round() as u32 * 2).max(2);
        (width, height) = if width < height {
            (even, other(height))
        } else {
            (other(width), even)
        };
    }
    let mut reasons = vec![Reason::Edited];
    if video.hdr {
        reasons.push(Reason::Hdr);
    }
    Some(VideoPlan {
        kbps: output_kbps(
            target_kbps(width, height, video.fps, settings),
            video_kbps(probe),
            video.codec == "h264",
            settings,
        ),
        reasons,
        width,
        height,
        edit: Some(VideoEdit {
            rotation: edit.rotation,
            crop,
            overlay: None,
        }),
    })
}

/// A transcode for playability is always kept; one for HDR or resolution
/// only when smaller; a bitrate-only one only when at least 10% smaller.
pub fn keep_transcode(reasons: &[Reason], original: u64, output: u64) -> bool {
    if reasons.contains(&Reason::Codec)
        || reasons.contains(&Reason::Container)
        || reasons.contains(&Reason::Edited)
    {
        return true;
    }
    if reasons.contains(&Reason::Resolution) || reasons.contains(&Reason::Hdr) {
        return output < original;
    }
    (output as f64) <= original as f64 * (1.0 - MIN_SAVING)
}

/// The transcode's filter chain. FFmpeg rotates frames before user filters,
/// so sizes are expressions on the rotated frame: the short edge becomes the
/// plan's, the long edge keeps the aspect ratio (even), and nothing depends
/// on how the probe read the rotation.
pub fn transcode_filter(plan: &VideoPlan, hdr: bool) -> String {
    if let Some(edit) = &plan.edit {
        return edited_filter(plan, edit, hdr);
    }
    let (width, height) = if plan.reasons.contains(&Reason::Resolution) {
        let short = plan.width.min(plan.height);
        (
            format!("'if(lte(iw,ih),{short},-2)'"),
            format!("'if(lte(iw,ih),-2,{short})'"),
        )
    } else {
        ("'trunc(iw/2)*2'".to_owned(), "'trunc(ih/2)*2'".to_owned())
    };
    sized_filter(&width, &height, hdr)
}

/// A filter chain to an exact size (posters, whose raw frames must match).
/// HDR goes through linear light to SDR BT.709 with Hable's curve, as on web.
pub fn video_filter(plan: &VideoPlan, hdr: bool) -> String {
    sized_filter(&plan.width.to_string(), &plan.height.to_string(), hdr)
}

/// An edit in the display orientation: turned, then cropped, then scaled
/// (and tone mapped) to the plan's exact size, then the marks laid over
/// every frame from the second input (`[1:v]`). With marks this is a
/// `-filter_complex` graph whose output is `[v]`, otherwise a `-vf` chain.
fn edited_filter(plan: &VideoPlan, edit: &VideoEdit, hdr: bool) -> String {
    let mut steps = Vec::new();
    match edit.rotation {
        90 => steps.push("transpose=clock".to_owned()),
        180 => steps.push("hflip,vflip".to_owned()),
        270 => steps.push("transpose=cclock".to_owned()),
        _ => {}
    }
    let crop = edit.crop;
    steps.push(format!(
        "crop=w={}:h={}:x={}:y={}:exact=1",
        crop.width, crop.height, crop.left, crop.top
    ));
    steps.push(sized_filter(
        &plan.width.to_string(),
        &plan.height.to_string(),
        hdr,
    ));
    let chain = steps.join(",");
    if edit.overlay.is_some() {
        format!("[0:v:0]{chain}[base];[base][1:v]overlay=x=0:y=0:format=yuv420,format=yuv420p[v]")
    } else {
        chain
    }
}

fn sized_filter(width: &str, height: &str, hdr: bool) -> String {
    if hdr {
        format!(
            "zscale=w={width}:h={height}:t=linear:npl=203,format=gbrpf32le,zscale=p=bt709,\
             tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p"
        )
    } else {
        format!("scale=w={width}:h={height}:flags=bicubic,format=yuv420p")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoder {
    /// Windows' own H.264 encoder (hardware on most PCs).
    MediaFoundation,
    X264,
}

impl Encoder {
    /// Tried in order; the first that succeeds wins.
    pub fn candidates() -> &'static [Encoder] {
        if cfg!(windows) {
            &[Encoder::MediaFoundation, Encoder::X264]
        } else {
            &[Encoder::X264]
        }
    }
}

/// The full argument list for one transcode.
pub fn transcode_args(
    input: &Path,
    output: &Path,
    probe: &Probe,
    plan: &VideoPlan,
    settings: &Compression,
    encoder: Encoder,
) -> Vec<std::ffi::OsString> {
    let kbps = plan.kbps;
    let mut args: Vec<std::ffi::OsString> = ["-loglevel", "error", "-y", "-i"]
        .iter()
        .map(Into::into)
        .collect();
    args.push(input.into());
    let overlay = plan.edit.as_ref().and_then(|edit| edit.overlay.as_ref());
    if let Some(overlay) = overlay {
        // Read as one PNG, never as an image2 file-name pattern.
        args.extend(["-f", "png_pipe", "-i"].map(Into::into));
        args.push(overlay.into());
    }
    let hdr = probe.video.as_ref().is_some_and(|video| video.hdr);
    let mut push = |values: &[&str]| args.extend(values.iter().map(Into::into));
    if overlay.is_some() {
        push(&["-filter_complex", &transcode_filter(plan, hdr)]);
        push(&["-map", "[v]", "-map", "0:a:0?"]);
    } else {
        push(&["-map", "0:v:0", "-map", "0:a:0?", "-vf"]);
        push(&[&transcode_filter(plan, hdr)]);
    }
    match encoder {
        // H.264 High (100); Media Foundation otherwise picks Baseline, which
        // looks worse at the same bitrate.
        Encoder::MediaFoundation => push(&[
            "-c:v",
            "h264_mf",
            "-rate_control",
            "u_vbr",
            "-profile:v",
            "100",
        ]),
        Encoder::X264 => push(&[
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-profile:v",
            "high",
        ]),
    }
    push(&[
        "-b:v",
        &format!("{kbps}k"),
        "-maxrate",
        &format!("{}k", kbps * 3 / 2),
        "-bufsize",
        &format!("{}k", kbps * 2),
        "-color_primaries",
        "bt709",
        "-color_trc",
        "bt709",
        "-colorspace",
        "bt709",
    ]);
    match &probe.audio {
        Some(audio) if audio.codec == "aac" && audio.channels <= 2 => push(&["-c:a", "copy"]),
        Some(audio) => push(&[
            "-c:a",
            "aac",
            "-b:a",
            &format!("{}k", settings.audio_bitrate_kbps),
            "-ac",
            &audio.channels.clamp(1, 2).to_string(),
        ]),
        None => {}
    }
    // No location or device metadata; the moov box first, for streaming.
    push(&[
        "-map_metadata",
        "-1",
        "-map_chapters",
        "-1",
        "-movflags",
        "+faststart",
        "-progress",
        "pipe:1",
        "-nostats",
        "-f",
        "mp4",
    ]);
    args.push(output.into());
    args
}

/// Encode `input` to `output` per the plan, reporting 0–1 progress. Stops
/// (and removes the partial output) as soon as `cancel` is set.
#[allow(clippy::too_many_arguments)]
pub fn transcode(
    binary: &Path,
    input: &Path,
    output: &Path,
    probe: &Probe,
    plan: &VideoPlan,
    settings: &Compression,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut failure = "FFmpeg could not compress this video.".to_owned();
    for &encoder in Encoder::candidates() {
        match run_transcode(
            binary, input, output, probe, plan, settings, encoder, progress, cancel,
        ) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let _ = std::fs::remove_file(output);
                if cancel.load(Ordering::Relaxed) {
                    return Err("Cancelled.".into());
                }
                failure = error;
            }
        }
    }
    Err(failure)
}

#[allow(clippy::too_many_arguments)]
fn run_transcode(
    binary: &Path,
    input: &Path,
    output: &Path,
    probe: &Probe,
    plan: &VideoPlan,
    settings: &Compression,
    encoder: Encoder,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut child = command(binary)
        .args(transcode_args(
            input, output, probe, plan, settings, encoder,
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start FFmpeg: {error}"))?;
    let mut errors = child.stderr.take();
    let collected = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(errors) = &mut errors {
            let _ = errors.read_to_string(&mut text);
        }
        text
    });
    let duration = probe.duration_ms.unwrap_or(0) as f64;
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                break;
            }
            let Ok(line) = line else { break };
            if let Some(micros) = line
                .strip_prefix("out_time_us=")
                .and_then(|value| value.trim().parse::<f64>().ok())
                && duration > 0.0
            {
                progress((micros / 1000.0 / duration).clamp(0.0, 1.0) as f32);
            }
        }
    }
    let status = child.wait().map_err(|error| error.to_string())?;
    let log = collected.join().unwrap_or_default();
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled.".into());
    }
    if !status.success() {
        let detail = log.lines().last().unwrap_or("").trim().to_owned();
        return Err(if detail.is_empty() {
            "FFmpeg could not compress this video.".into()
        } else {
            format!("FFmpeg could not compress this video: {detail}")
        });
    }
    Ok(())
}

/// One RGB frame near the start, at most `edge` pixels on its long side, for
/// the preview and the sender's chip. HDR is tone mapped like the video.
pub fn poster(binary: &Path, input: &Path, probe: &Probe, edge: u32) -> Option<image::RgbImage> {
    let video = probe.video.as_ref()?;
    let (width, height) = crate::compress::fit_within(video.width, video.height, edge.max(2));
    let (width, height) = ((width / 2 * 2).max(2), (height / 2 * 2).max(2));
    let plan = VideoPlan {
        reasons: Vec::new(),
        width,
        height,
        kbps: 0,
        edit: None,
    };
    // A frame a little in avoids black fade-ins; short clips use their middle.
    let at = probe
        .duration_ms
        .map_or(0, |duration| (duration / 2).min(1000));
    let filter = video_filter(&plan, video.hdr).replace("format=yuv420p", "format=rgb24");
    let output = command(binary)
        .args(["-loglevel", "error", "-ss", &format!("{}ms", at), "-i"])
        .arg(input)
        .args(["-map", "0:v:0", "-frames:v", "1", "-vf", &filter])
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let expected = width as usize * height as usize * 3;
    (output.status.success() && output.stdout.len() == expected)
        .then(|| image::RgbImage::from_raw(width, height, output.stdout))
        .flatten()
}

/// Whether this FFmpeg can lay a still PNG over every frame: the `overlay`
/// filter, the PNG decoder and the `png_pipe` demuxer. Caper's trimmed build
/// has none of them, so drawings and text on videos need another FFmpeg
/// (`CAPER_FFMPEG`); crops and turns use only the bundled filters. Asked
/// once per run, off the UI thread.
pub fn overlays(binary: &Path) -> bool {
    static OVERLAYS: OnceLock<bool> = OnceLock::new();
    *OVERLAYS.get_or_init(|| {
        let list = |what: &str| {
            command(binary)
                .arg(what)
                .stderr(Stdio::null())
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                .unwrap_or_default()
        };
        lists(&list("-filters"), "overlay")
            && lists(&list("-decoders"), "png")
            && lists(&list("-demuxers"), "png_pipe")
    })
}

/// Whether an `ffmpeg -filters`/`-decoders`/`-demuxers` listing names
/// `name`: each entry is flags, then its name (or comma-separated names).
pub fn lists(listing: &str, name: &str) -> bool {
    listing.lines().any(|line| {
        let mut words = line.split_whitespace();
        words.next().is_some()
            && words
                .next()
                .is_some_and(|names| names.split(',').any(|listed| listed == name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `ffmpeg -i` on phone, camera, browser and screen videos.
    const IPHONE_HDR: &str = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'IMG_0001.MOV':
  Metadata:
    major_brand     : qt
    com.apple.quicktime.location.ISO6709: +40.7128-074.0060+010.000/
  Duration: 00:00:12.53, start: 0.000000, bitrate: 25471 kb/s
  Stream #0:0[0x1](und): Video: hevc (Main 10) (hvc1 / 0x31637668), yuv420p10le(tv, bt2020nc/bt2020/arib-std-b67), 3840x2160, 25032 kb/s, 29.98 fps, 30 tbr, 600 tbn (default)
      Metadata:
        handler_name    : Core Media Video
      Side data:
        DOVI configuration record: version: 1.0, profile: 8, level: 7
        displaymatrix: rotation of -90.00 degrees
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 44100 Hz, stereo, fltp, 172 kb/s (default)";

    const ANDROID: &str = "  Duration: 00:00:04.00, start: 0.000000, bitrate: 15976 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(progressive), 1920x1080 [SAR 1:1 DAR 16:9], 15894 kb/s, 30 fps, 30 tbr, 15360 tbn (default)
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, mono, fltp, 69 kb/s (default)";

    const WEBM: &str = "  Duration: 00:00:02.01, start: -0.007000, bitrate: 716 kb/s
  Stream #0:0: Video: vp9 (Profile 0), yuv420p(tv, progressive), 640x360, SAR 1:1 DAR 16:9, 30 fps, 30 tbr, 1k tbn
  Stream #0:1: Audio: opus, 48000 Hz, 5.1(side), fltp";

    const EFFICIENT: &str = "  Duration: 00:00:03.00, start: 0.000000, bitrate: 2076 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(progressive), 1280x720 [SAR 1:1 DAR 16:9], 1994 kb/s, 30 fps, 30 tbr, 15360 tbn (default)";

    #[test]
    fn probe_reads_size_rotation_hdr_bitrate_and_audio() {
        let probe = parse_probe(IPHONE_HDR);
        assert_eq!(probe.duration_ms, Some(12_530));
        assert_eq!(probe.total_kbps, Some(25_471));
        let video = probe.video.unwrap();
        assert_eq!(
            (video.codec.as_str(), video.width, video.height),
            ("hevc", 2160, 3840)
        );
        assert!(video.hdr);
        assert_eq!(video.kbps, Some(25_032));
        assert_eq!(video.fps, Some(29.98));
        let audio = probe.audio.unwrap();
        assert_eq!(
            (audio.codec.as_str(), audio.channels, audio.kbps),
            ("aac", 2, Some(172))
        );

        let webm = parse_probe(WEBM);
        let video = webm.video.unwrap();
        assert_eq!(
            (video.codec.as_str(), video.width, video.height, video.kbps),
            ("vp9", 640, 360, None)
        );
        assert!(!video.hdr);
        assert_eq!(webm.audio.unwrap().channels, 6);
        assert_eq!(parse_probe("Duration: N/A, bitrate: N/A").duration_ms, None);
        // FFmpeg 7+ spells the rotation differently; both swap to portrait.
        let ffmpeg9 = IPHONE_HDR.replace(
            "displaymatrix: rotation of -90.00 degrees",
            "Display Matrix: rotation of 90.00 degrees",
        );
        let video = parse_probe(&ffmpeg9).video.unwrap();
        assert_eq!((video.width, video.height), (2160, 3840));
    }

    #[test]
    fn output_bitrate_follows_the_source_cap_and_frame_rate() {
        let s = Compression::default();
        assert_eq!(output_kbps(8000, None, false, &s), 8000);
        assert_eq!(output_kbps(8000, Some(10_683), false, &s), 8000);
        assert_eq!(
            output_kbps(8000, Some(2500), false, &s),
            5000,
            "HEVC: twice its bitrate"
        );
        assert_eq!(
            output_kbps(8000, Some(2500), true, &s),
            2500,
            "H.264: its own bitrate"
        );
        assert_eq!(output_kbps(8000, Some(600), false, &s), MIN_VIDEO_KBPS);
        let uncapped = Compression {
            video_source_bitrate_percent: 0,
            ..Compression::default()
        };
        assert_eq!(output_kbps(8000, Some(2500), false, &uncapped), 8000);
        assert_eq!(target_kbps(1920, 1080, None, &s), 8000);
        assert_eq!(target_kbps(1920, 1080, Some(24.0), &s), 8000);
        assert_eq!(
            target_kbps(1920, 1080, Some(60.0), &s),
            12_000,
            "boost capped at 150%"
        );
        assert_eq!(target_kbps(1280, 720, Some(30.0), &s), 3556);
    }

    #[test]
    fn plans_follow_the_web_rules() {
        let settings = Compression::default();
        // Phone HDR 4K HEVC: portrait keeps a 1440 short edge. 25 Mbps is
        // within the 4K target (8 Mbps scaled by pixels), so not "bitrate".
        let plan = video_plan(&parse_probe(IPHONE_HDR), true, &settings).unwrap();
        assert_eq!(
            plan.reasons,
            [Reason::Hdr, Reason::Resolution, Reason::Codec]
        );
        assert_eq!((plan.width, plan.height), (1440, 2560));
        assert_eq!(plan.kbps, 14_222);
        // 1080p H.264 at 16 Mbps: bitrate only.
        let plan = video_plan(&parse_probe(ANDROID), true, &settings).unwrap();
        assert_eq!(plan.reasons, [Reason::Bitrate]);
        assert_eq!((plan.width, plan.height, plan.kbps), (1920, 1080, 8000));
        // Already efficient H.264 MP4 uploads unchanged; the same in MKV is remuxed.
        assert_eq!(video_plan(&parse_probe(EFFICIENT), true, &settings), None);
        assert_eq!(
            video_plan(&parse_probe(EFFICIENT), false, &settings)
                .unwrap()
                .reasons,
            [Reason::Container]
        );
        // VP9 has no per-stream rate; small video gets the bitrate floor.
        let plan = video_plan(&parse_probe(WEBM), true, &settings).unwrap();
        assert_eq!(plan.reasons, [Reason::Codec]);
        assert_eq!(plan.kbps, MIN_VIDEO_KBPS);
        let disabled = Compression {
            video_max_height: 0,
            ..Compression::default()
        };
        assert_eq!(video_plan(&parse_probe(ANDROID), true, &disabled), None);
    }

    #[test]
    fn transcodes_are_kept_only_when_worth_it() {
        assert!(keep_transcode(&[Reason::Codec], 100, 200));
        assert!(keep_transcode(&[Reason::Resolution], 100, 99));
        assert!(!keep_transcode(&[Reason::Hdr], 100, 100));
        assert!(keep_transcode(&[Reason::Bitrate], 100, 90));
        assert!(!keep_transcode(&[Reason::Bitrate], 100, 91));
    }

    #[test]
    fn arguments_strip_metadata_and_pick_the_encoder() {
        let probe = parse_probe(IPHONE_HDR);
        let plan = video_plan(&probe, true, &Compression::default()).unwrap();
        let args = transcode_args(
            Path::new("in.mov"),
            Path::new("out.mp4"),
            &probe,
            &plan,
            &Compression::default(),
            Encoder::X264,
        );
        let text: Vec<_> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let joined = text.join(" ");
        assert!(joined.contains("-c:v libx264"));
        assert!(joined.contains("tonemap=tonemap=hable"));
        assert!(joined.contains("zscale=w='if(lte(iw,ih),1440,-2)':h='if(lte(iw,ih),-2,1440)'"));
        assert!(joined.contains("-b:v 14222k -maxrate 21333k -bufsize 28444k"));
        assert!(joined.contains("-c:a copy"));
        assert!(joined.contains("-map_metadata -1"));
        assert!(joined.contains("-movflags +faststart"));
        assert_eq!(text.last().map(String::as_str), Some("out.mp4"));
        let webm = parse_probe(WEBM);
        let plan = video_plan(&webm, true, &Compression::default()).unwrap();
        let joined = transcode_args(
            Path::new("in.webm"),
            Path::new("out.mp4"),
            &webm,
            &plan,
            &Compression::default(),
            Encoder::MediaFoundation,
        )
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
        assert!(joined.contains("-c:v h264_mf -rate_control u_vbr -profile:v 100"));
        assert!(
            joined
                .contains("scale=w='trunc(iw/2)*2':h='trunc(ih/2)*2':flags=bicubic,format=yuv420p")
        );
        assert!(joined.contains("-c:a aac -b:a 128k -ac 2"));
    }

    fn joined(args: &[std::ffi::OsString]) -> String {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn edited_videos_are_always_transcoded_turned_and_cropped() {
        use crate::attachment_edit::{Crop, Edit, Mark, PixelRect};
        let settings = Compression::default();
        // Already efficient: unedited, it would upload unchanged.
        assert_eq!(video_plan(&parse_probe(EFFICIENT), true, &settings), None);
        let full = edited_plan(&parse_probe(EFFICIENT), &Edit::default(), &settings).unwrap();
        assert_eq!(full.reasons, [Reason::Edited]);
        assert_eq!((full.width, full.height), (1280, 720));
        assert!(keep_transcode(&full.reasons, 100, 200));
        // The shared case: a landscape clip turned to portrait, cropped to its
        // left half, at the bitrate for the cropped size.
        let android = parse_probe(ANDROID);
        let edit = Edit {
            rotation: 270,
            crop: Crop {
                x: 0.1,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
            marks: Vec::new(),
        };
        let plan = edited_plan(&android, &edit, &settings).unwrap();
        assert_eq!((plan.width, plan.height, plan.kbps), (540, 1920, 4000));
        assert_eq!(
            plan.edit,
            Some(VideoEdit {
                rotation: 270,
                crop: PixelRect {
                    left: 108,
                    top: 0,
                    width: 540,
                    height: 1920
                },
                overlay: None,
            })
        );
        let args = joined(&transcode_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &android,
            &plan,
            &settings,
            Encoder::X264,
        ));
        assert!(args.contains(
            "-map 0:v:0 -map 0:a:0? -vf transpose=cclock,crop=w=540:h=1920:x=108:y=0:exact=1,\
             scale=w=540:h=1920:flags=bicubic,format=yuv420p -c:v libx264"
        ));
        assert!(args.contains("-b:v 4000k") && args.contains("-c:a copy"));
        assert!(!args.contains("png_pipe") && !args.contains("overlay"));
        // HDR is still tone mapped, after the crop; the short edge is capped;
        // marks come in as a second, PNG input laid over every frame.
        let iphone = parse_probe(IPHONE_HDR);
        let marked = Edit {
            rotation: 180,
            marks: vec![Mark::Stroke {
                color: [255, 255, 255],
                width: 0.01,
                points: vec![0.5, 0.5],
            }],
            ..Edit::default()
        };
        let mut plan = edited_plan(&iphone, &marked, &settings).unwrap();
        assert_eq!(plan.reasons, [Reason::Edited, Reason::Hdr]);
        assert_eq!((plan.width, plan.height), (1440, 2560));
        plan.edit.as_mut().unwrap().overlay = Some(PathBuf::from("marks.png"));
        let args = joined(&transcode_args(
            Path::new("in.mov"),
            Path::new("out.mp4"),
            &iphone,
            &plan,
            &settings,
            Encoder::X264,
        ));
        assert!(
            args.starts_with(
                "-loglevel error -y -i in.mov -f png_pipe -i marks.png -filter_complex "
            )
        );
        assert!(args.contains(
            "[0:v:0]hflip,vflip,crop=w=2160:h=3840:x=0:y=0:exact=1,zscale=w=1440:h=2560:t=linear"
        ));
        assert!(args.contains(
            "format=yuv420p[base];[base][1:v]overlay=x=0:y=0:format=yuv420,format=yuv420p[v] \
             -map [v] -map 0:a:0? -c:v libx264"
        ));
        assert!(args.contains("-map_metadata -1") && args.ends_with("out.mp4"));
        // No cap when the server turns compression off: edits still apply.
        let uncapped = Compression {
            video_max_height: 0,
            ..Compression::default()
        };
        let plan = edited_plan(&iphone, &marked, &uncapped).unwrap();
        assert_eq!((plan.width, plan.height), (2160, 3840));
    }

    #[test]
    fn overlay_support_is_read_from_the_listings() {
        // Caper's trimmed build, then a distribution FFmpeg.
        let trimmed_filters = " .. crop              V->V       Crop the input video.
 .S transpose         V->V       Transpose input video.";
        let full_filters =
            " TSC overlay           VV->V      Overlay a video source on top of the input.";
        assert!(!lists(trimmed_filters, "overlay"));
        assert!(lists(trimmed_filters, "transpose"));
        assert!(lists(full_filters, "overlay"));
        assert!(lists(
            " VF...D png                  PNG (Portable Network Graphics) image",
            "png"
        ));
        assert!(!lists(
            " V....D mjpeg                MJPEG (Motion JPEG)",
            "png"
        ));
        assert!(lists(" D  png_pipe        piped png sequence", "png_pipe"));
        assert!(lists(" D   matroska,webm   Matroska / WebM", "webm"));
        assert!(!lists(
            " D   mov,mp4,m4a,3gp,3g2,mj2 QuickTime / MOV",
            "png_pipe"
        ));
        assert!(!lists("", "overlay"));
    }

    /// End to end with a real FFmpeg (`CAPER_FFMPEG=/usr/bin/ffmpeg`), when one
    /// is available: compress a generated 1080p clip and grab its poster.
    #[test]
    #[ignore = "needs CAPER_FFMPEG (and CAPER_FFMPEG_FIXTURES with lavfi if it lacks it)"]
    fn compresses_and_grabs_a_poster_with_a_real_ffmpeg() {
        let binary = binary().expect("set CAPER_FFMPEG");
        let dir = std::env::temp_dir().join(format!("caper-ffmpeg-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("in.mp4");
        let status = command(&fixture_binary())
            .args([
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1920x1080:rate=30",
            ])
            .args([
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "2",
            ])
            .args([
                "-c:v", "libx264", "-b:v", "20M", "-pix_fmt", "yuv420p", "-c:a", "aac",
            ])
            .arg(&input)
            .status()
            .unwrap();
        assert!(status.success());
        let probe = probe(binary, &input).unwrap();
        let plan = video_plan(&probe, true, &Compression::default()).unwrap();
        assert_eq!(plan.reasons, [Reason::Bitrate]);
        let output = dir.join("out.mp4");
        let reported = std::sync::Mutex::new(0.0_f32);
        transcode(
            binary,
            &input,
            &output,
            &probe,
            &plan,
            &Compression::default(),
            &|fraction| *reported.lock().unwrap() = fraction,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(*reported.lock().unwrap() > 0.5);
        let original = std::fs::metadata(&input).unwrap().len();
        let compressed = std::fs::metadata(&output).unwrap().len();
        assert!(keep_transcode(&plan.reasons, original, compressed));
        let result = self::probe(binary, &output).unwrap();
        assert_eq!(result.video.as_ref().unwrap().codec, "h264");
        let poster = poster(binary, &output, &result, 640).unwrap();
        assert_eq!(poster.dimensions(), (640, 360));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
