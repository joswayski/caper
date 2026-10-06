//! H.264/AAC MP4 for video, and silent looping MP4 for animated images.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::detect::ImageFormat;
use crate::image;
use crate::process::{Ctx, EarlyPreview, Error, Kind, Outcome};
use crate::tools::{self, Probe, ToolError, VipsHeader};

/// Minimum spacing of progress callbacks.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(3);
/// Encode time before the deadline projection is trusted.
const PROJECTION_WARMUP: Duration = Duration::from_secs(15);
/// Animated WebP frame limit (frames are extracted one by one).
const MAX_WEBP_FRAMES: u32 = 3000;

/// Throttles `attachment.progress`: integer percent, only increasing, at
/// most every [`PROGRESS_INTERVAL`].
#[derive(Debug, Default)]
pub struct Throttle {
    last: Option<(Instant, u8)>,
}

impl Throttle {
    pub fn update(&mut self, now: Instant, percent: u8) -> Option<u8> {
        match self.last {
            Some((at, last)) if percent <= last || now.duration_since(at) < PROGRESS_INTERVAL => {
                None
            }
            _ => {
                self.last = Some((now, percent));
                Some(percent)
            }
        }
    }
}

/// Parses one `ffmpeg -progress` line into encoded seconds.
pub fn progress_seconds(line: &str) -> Option<f64> {
    let (key, value) = line.split_once('=')?;
    // Both keys are microseconds (`out_time_ms` is misnamed upstream).
    if key != "out_time_us" && key != "out_time_ms" {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let us = value.trim().parse::<i64>().ok()? as f64;
    (us >= 0.0).then_some(us / 1_000_000.0)
}

/// Percent for the progress callback, capped below 100 until finished.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn percent(done: f64, total: f64) -> u8 {
    if total <= 0.0 {
        return 0;
    }
    (done / total * 100.0).clamp(0.0, 99.0) as u8
}

fn even(value: f64) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let value = (value / 2.0).round() as u32 * 2;
    value.max(2)
}

/// Output size: short edge capped at `max_short` (0 keeps), even dimensions.
pub fn target_size(width: u32, height: u32, max_short: u32) -> (u32, u32) {
    let short = width.min(height);
    if max_short > 0 && short > max_short {
        let scale = f64::from(max_short) / f64::from(short);
        (
            even(f64::from(width) * scale),
            even(f64::from(height) * scale),
        )
    } else {
        (even(f64::from(width)), even(f64::from(height)))
    }
}

/// x264 preset for a video of `duration` seconds.
pub fn preset(settings: &crate::settings::Settings, duration: Option<f64>) -> &str {
    match duration {
        Some(d) if d > f64::from(settings.video_long_seconds) => &settings.video_long_preset,
        _ => &settings.video_preset,
    }
}

/// Faster preset to retry with when `preset` cannot finish in time:
/// `superfast` (still High profile with CABAC and B-frames), then
/// `ultrafast` (Constrained Baseline, much larger files).
pub fn fallback_preset(preset: &str) -> Option<&'static str> {
    match preset {
        "ultrafast" => None,
        "superfast" => Some("ultrafast"),
        _ => Some("superfast"),
    }
}

/// Whether an encode that has done `fraction` of the work after `elapsed`
/// would run past `budget` (measured from its start).
pub fn too_slow(elapsed: Duration, fraction: f64, budget: Duration) -> bool {
    if elapsed < PROJECTION_WARMUP || fraction <= 0.01 {
        return false;
    }
    elapsed.as_secs_f64() / fraction * 1.05 > budget.as_secs_f64()
}

fn os<S: Into<OsString>>(values: impl IntoIterator<Item = S>) -> Vec<OsString> {
    values.into_iter().map(Into::into).collect()
}

#[derive(Debug, thiserror::Error)]
enum EncodeError {
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error("encode would not finish before the deadline")]
    TooSlow,
}

/// Runs ffmpeg with `-progress pipe:1`, reporting throttled progress when
/// `duration` is known. With `adaptive`, gives up early (`TooSlow`) when
/// the projected finish passes the deadline.
async fn ffmpeg_with_progress(
    args: Vec<OsString>,
    duration: Option<f64>,
    adaptive: bool,
    throttle: &mut Throttle,
    ctx: &Ctx<'_>,
) -> Result<(), EncodeError> {
    let mut full = os([
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-nostats",
        "-progress",
        "pipe:1",
        "-y",
    ]);
    full.extend(args);
    let mut child = tools::command("ffmpeg", full)
        .spawn()
        .map_err(|source| ToolError::Spawn {
            tool: "ffmpeg".into(),
            source,
        })?;
    let stdout = child.stdout.take().ok_or(ToolError::Timeout {
        tool: "ffmpeg".into(),
    })?;
    let mut stderr = child.stderr.take();
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(stderr) = stderr.as_mut() {
            let _ = stderr.take(64 * 1024).read_to_end(&mut buf).await;
            // Keep draining so ffmpeg never blocks on a full pipe.
            let _ = tokio::io::copy(stderr, &mut tokio::io::sink()).await;
        }
        buf
    });
    let started = Instant::now();
    let budget = tools::remaining(ctx.deadline);
    let mut lines = BufReader::new(stdout).lines();
    let reading = async {
        while let Ok(Some(line)) = lines.next_line().await {
            let (Some(done), Some(total)) = (progress_seconds(&line), duration) else {
                continue;
            };
            let fraction = (done / total).clamp(0.0, 1.0);
            if adaptive && too_slow(started.elapsed(), fraction, budget) {
                return Err(EncodeError::TooSlow);
            }
            if let Some(p) = throttle.update(Instant::now(), percent(done, total)) {
                ctx.hooks.progress(p).await;
            }
        }
        Ok(())
    };
    match tokio::time::timeout(budget, reading).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let _ = child.kill().await;
            return Err(error);
        }
        Err(_) => {
            let _ = child.kill().await;
            return Err(ToolError::Timeout {
                tool: "ffmpeg".into(),
            }
            .into());
        }
    }
    let status = tokio::time::timeout(tools::remaining(ctx.deadline), child.wait())
        .await
        .map_err(|_| ToolError::Timeout {
            tool: "ffmpeg".into(),
        })?
        .map_err(|source| ToolError::Spawn {
            tool: "ffmpeg".into(),
            source,
        })?;
    let stderr = stderr_task.await.unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        let text = String::from_utf8_lossy(&stderr);
        let text = text.trim();
        let start = text.len().saturating_sub(400);
        let start = (start..=text.len())
            .find(|i| text.is_char_boundary(*i))
            .unwrap_or(0);
        Err(ToolError::Failed {
            tool: "ffmpeg".into(),
            status: status.to_string(),
            stderr: text[start..].replace('\n', " | "),
        }
        .into())
    }
}

fn h264_args(preset: &str, crf: u8) -> Vec<OsString> {
    os([
        "-c:v".to_owned(),
        "libx264".to_owned(),
        "-profile:v".to_owned(),
        "high".to_owned(),
        "-preset".to_owned(),
        preset.to_owned(),
        "-crf".to_owned(),
        crf.to_string(),
        "-pix_fmt".to_owned(),
        "yuv420p".to_owned(),
        "-map_metadata".to_owned(),
        "-1".to_owned(),
        "-map_chapters".to_owned(),
        "-1".to_owned(),
        "-movflags".to_owned(),
        "+faststart".to_owned(),
    ])
}

fn size_of(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

/// Displayed size and duration of a produced MP4.
async fn measure(path: &Path, ctx: &Ctx<'_>) -> (Option<u32>, Option<u32>, Option<u64>) {
    match tools::ffprobe(path, ctx.deadline).await {
        Ok(probe) => {
            let size = probe.video().and_then(tools::Stream::display_size);
            (size.map(|s| s.0), size.map(|s| s.1), probe.duration_ms())
        }
        Err(_) => (None, None, None),
    }
}

// ---------------------------------------------------------------- animated

/// Writes every frame of an animated WebP as PNG plus an ffconcat list with
/// the frame delays (ffmpeg cannot decode animated WebP itself).
async fn webp_frames(input: &Path, header: &VipsHeader, ctx: &Ctx<'_>) -> Result<PathBuf, Error> {
    let pages = header.pages();
    if pages > MAX_WEBP_FRAMES {
        return Err("too many frames".into());
    }
    let strip = ctx.path("strip.v");
    let mut source = input.as_os_str().to_owned();
    source.push("[n=-1]");
    tools::run(
        "vips",
        [OsString::from("copy"), source, strip.as_os_str().to_owned()],
        ctx.deadline,
    )
    .await?;
    let strip_header = tools::vipsheader(&strip, ctx.deadline).await?;
    let width = strip_header.width().ok_or("no width")?;
    let page_height = strip_header
        .page_height()
        .or_else(|| strip_header.height().map(|h| h / pages))
        .ok_or("no page height")?;
    let delays = strip_header.delays();
    let frames = ctx.path("frames");
    tokio::fs::create_dir_all(&frames).await?;
    let mut list = String::from("ffconcat version 1.0\n");
    let mut last = String::new();
    for page in 0..pages {
        let frame = frames.join(format!("f{page:05}.png"));
        tools::run(
            "vips",
            [
                OsString::from("crop"),
                strip.as_os_str().to_owned(),
                frame.as_os_str().to_owned(),
                "0".into(),
                (page * page_height).to_string().into(),
                width.to_string().into(),
                page_height.to_string().into(),
            ],
            ctx.deadline,
        )
        .await?;
        // Browsers show delays of 10 ms or less as 100 ms.
        let delay = delays.get(page as usize).copied().unwrap_or(100);
        let delay = if delay <= 10 { 100 } else { delay };
        // A 1 kHz input clock keeps millisecond delays exact.
        let quoted = frame.display().to_string().replace('\'', "'\\''");
        last = format!("file '{quoted}'\noption framerate 1000\n");
        let _ = writeln!(list, "{last}duration {}.{:03}", delay / 1000, delay % 1000);
    }
    // The concat demuxer only honours the final duration if the last file
    // is listed again.
    list.push_str(&last);
    let path = ctx.path("frames.ffconcat");
    tokio::fs::write(&path, list).await?;
    Ok(path)
}

/// Converts an animated GIF/WebP/PNG/AVIF to a silent H.264 MP4 that clients
/// play muted and looping.
pub async fn animated(
    input: &Path,
    format: ImageFormat,
    header: &VipsHeader,
    ctx: &Ctx<'_>,
) -> Result<Outcome, Error> {
    let settings = ctx.settings();
    let mut args = Vec::new();
    match format {
        ImageFormat::WebP => {
            let list = webp_frames(input, header, ctx).await?;
            args.extend(os(["-f", "concat", "-safe", "0", "-i"]));
            args.push(list.into_os_string());
        }
        ImageFormat::Gif => args.extend(os(["-f", "gif", "-i"])),
        ImageFormat::Png => args.extend(os(["-f", "apng", "-i"])),
        _ => args.extend(os(["-i"])),
    }
    if format != ImageFormat::WebP {
        args.push(input.as_os_str().to_owned());
    }
    let width = header.width().ok_or("no width")?;
    let height = header
        .page_height()
        .or(header.height())
        .ok_or("no height")?;
    let (tw, th) = target_size(width, height, settings.video_max_height);
    // Transparent pixels become black (premultiplied), the dark UI colour.
    let filter = format!(
        "format=rgba,premultiply=inplace=1,scale={tw}:{th}:flags=lanczos,setsar=1,format=yuv420p"
    );
    args.extend(os([
        "-map",
        "0:v:0",
        "-an",
        "-sn",
        "-dn",
        "-fps_mode",
        "passthrough",
        "-vf",
    ]));
    args.push(filter.into());
    args.extend(h264_args(&settings.video_preset, settings.video_crf));
    // Without B-frames the MP4 keeps every variable frame delay, including
    // the last one (reordering breaks per-frame durations with passthrough).
    args.extend(os(["-bf", "0"]));
    let out = ctx.path("animated.mp4");
    args.push(out.clone().into_os_string());
    ffmpeg_with_progress(args, None, false, &mut Throttle::default(), ctx).await?;
    let (w, h, duration) = measure(&out, ctx).await;
    let preview = image::preview(input, ctx, "preview")
        .await
        .map(|(path, _, _)| path);
    Ok(Outcome::new(Kind::Video, "video/mp4", out, ctx.job)?
        .size(w, h)
        .duration(duration)
        .animated()
        .preview(preview))
}

// ---------------------------------------------------------------- video

/// Poster frame → WebP preview, published before the encode starts.
async fn poster(input: &Path, probe: &Probe, ctx: &Ctx<'_>) -> Option<PathBuf> {
    let duration = probe.duration_seconds().unwrap_or(0.0);
    let frame = ctx.path("poster.png");
    let at = if duration > 2.0 {
        (duration * 0.1).min(1.0)
    } else {
        0.0
    };
    for seek in [at, 0.0] {
        let result = tools::run(
            "ffmpeg",
            os([
                "-hide_banner".to_owned(),
                "-loglevel".to_owned(),
                "error".to_owned(),
                "-nostdin".to_owned(),
                "-y".to_owned(),
                "-ss".to_owned(),
                format!("{seek:.3}"),
                "-i".to_owned(),
                input.display().to_string(),
                "-map".to_owned(),
                "0:v:0".to_owned(),
                "-frames:v".to_owned(),
                "1".to_owned(),
                "-update".to_owned(),
                "1".to_owned(),
                frame.display().to_string(),
            ]),
            ctx.deadline,
        )
        .await;
        if result.is_ok() && size_of(&frame).is_some_and(|s| s > 0) {
            break;
        }
        if seek == 0.0 {
            return None;
        }
    }
    image::preview(&frame, ctx, "preview")
        .await
        .map(|(path, _, _)| path)
}

/// Source that can be stored remuxed when re-encoding does not help:
/// browser-playable H.264 (8-bit 4:2:0) in MP4/MOV with AAC/MP3 audio.
pub fn remuxable(probe: &Probe) -> bool {
    let Some(video) = probe.video() else {
        return false;
    };
    probe.is_iso_media()
        && video.codec_name == "h264"
        && matches!(video.pix_fmt.as_deref(), Some("yuv420p" | "yuvj420p"))
        && probe
            .audio()
            .all(|a| matches!(a.codec_name.as_str(), "aac" | "mp3"))
}

async fn remux(input: &Path, ctx: &Ctx<'_>) -> Result<PathBuf, ToolError> {
    let out = ctx.path("remux.mp4");
    let mut args = os(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"]);
    args.push(input.as_os_str().to_owned());
    args.extend(os([
        "-map",
        "0:v:0",
        "-map",
        "0:a?",
        "-c",
        "copy",
        "-map_metadata",
        "-1",
        "-map_chapters",
        "-1",
        "-movflags",
        "+faststart",
    ]));
    args.push(out.clone().into_os_string());
    tools::run("ffmpeg", args, ctx.deadline).await?;
    Ok(out)
}

pub async fn video(input: &Path, probe: &Probe, ctx: &Ctx<'_>) -> Result<Outcome, Error> {
    let settings = ctx.settings();
    let stream = probe.video().ok_or("no video stream")?;
    let (dw, dh) = stream.display_size().ok_or("no video size")?;
    let duration = probe.duration_seconds();
    let duration_ms = probe.duration_ms();

    let preview = poster(input, probe, ctx).await;
    if let Some(path) = &preview
        && let Some(byte_size) = size_of(path)
    {
        let early = EarlyPreview {
            content_type: "image/webp".into(),
            byte_size,
            width: dw,
            height: dh,
            duration_ms,
        };
        ctx.hooks.preview(path, &early).await;
    }

    let (tw, th) = target_size(dw, dh, settings.video_max_height);
    let encoded = ctx.path("video.mp4");
    let mut throttle = Throttle::default();
    let first = preset(settings, duration).to_owned();
    // e.g. slow → superfast → ultrafast, each tried only when the previous
    // one is projected to miss the deadline.
    let mut attempts = vec![first];
    while let Some(fallback) = attempts.last().and_then(|p| fallback_preset(p)) {
        attempts.push(fallback.to_owned());
    }
    let mut result: Result<(), EncodeError> = Err(EncodeError::TooSlow);
    for (index, preset) in attempts.iter().enumerate() {
        // ffmpeg autorotates before the filter graph, so scale to the
        // displayed size and the output carries no rotation flag.
        let mut args = os(["-i"]);
        args.push(input.as_os_str().to_owned());
        args.extend(os(["-map", "0:v:0", "-map", "0:a?", "-sn", "-dn", "-vf"]));
        args.push(format!("scale={tw}:{th},setsar=1,format=yuv420p").into());
        args.extend(h264_args(preset, settings.video_crf));
        args.extend(os([
            "-c:a".to_owned(),
            "aac".to_owned(),
            "-b:a".to_owned(),
            format!("{}k", settings.audio_kbps),
            "-max_muxing_queue_size".to_owned(),
            "4096".to_owned(),
        ]));
        args.push(encoded.clone().into_os_string());
        let adaptive = index + 1 < attempts.len();
        let started = Instant::now();
        result = ffmpeg_with_progress(args, duration, adaptive, &mut throttle, ctx).await;
        tracing::info!(
            preset,
            elapsed_ms = started.elapsed().as_millis(),
            ok = result.is_ok(),
            "video encode"
        );
        if !matches!(result, Err(EncodeError::TooSlow)) {
            break;
        }
    }

    let input_size = size_of(input).unwrap_or(0);
    let mut chosen = match &result {
        Ok(()) => Some(encoded),
        Err(error) => {
            tracing::warn!(error = %error, "video encode did not complete");
            None
        }
    };
    let encoded_size = chosen.as_deref().and_then(size_of);
    if remuxable(probe) && encoded_size.is_none_or(|size| size >= input_size) {
        match remux(input, ctx).await {
            Ok(path) => chosen = Some(path),
            Err(error) => tracing::warn!(error = %error, "remux failed"),
        }
    }
    let Some(chosen) = chosen else {
        return Err(match result {
            Err(error) => error.to_string().into(),
            Ok(()) => "no video result".into(),
        });
    };
    let (w, h, measured) = measure(&chosen, ctx).await;
    Ok(Outcome::new(Kind::Video, "video/mp4", chosen, ctx.job)?
        .size(w.or(Some(dw)), h.or(Some(dh)))
        .duration(measured.or(duration_ms))
        .preview(preview))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    #[test]
    fn throttle_emits_increasing_integers_every_three_seconds() {
        let start = Instant::now();
        let mut t = Throttle::default();
        assert_eq!(t.update(start, 0), Some(0));
        assert_eq!(t.update(start + Duration::from_secs(1), 5), None);
        assert_eq!(t.update(start + Duration::from_secs(3), 5), Some(5));
        assert_eq!(t.update(start + Duration::from_secs(7), 5), None);
        assert_eq!(t.update(start + Duration::from_secs(8), 4), None);
        assert_eq!(t.update(start + Duration::from_secs(9), 6), Some(6));
        assert_eq!(t.update(start + Duration::from_secs(10), 50), None);
    }

    #[test]
    fn progress_lines() {
        assert_eq!(progress_seconds("out_time_us=1500000"), Some(1.5));
        assert_eq!(progress_seconds("out_time_ms=2000000"), Some(2.0));
        assert_eq!(progress_seconds("out_time_us=N/A"), None);
        assert_eq!(progress_seconds("out_time_us=-5"), None);
        assert_eq!(progress_seconds("frame=12"), None);
        assert_eq!(percent(5.0, 10.0), 50);
        assert_eq!(percent(12.0, 10.0), 99);
        assert_eq!(percent(1.0, 0.0), 0);
    }

    #[test]
    fn sizes_are_even_and_capped_on_the_short_edge() {
        assert_eq!(target_size(3840, 2160, 1080), (1920, 1080));
        assert_eq!(target_size(2160, 3840, 1080), (1080, 1920));
        assert_eq!(target_size(1279, 719, 1080), (1280, 720));
        assert_eq!(target_size(1, 1, 1080), (2, 2));
        assert_eq!(target_size(4000, 3000, 0), (4000, 3000));
        assert_eq!(target_size(1440, 1080, 720), (960, 720));
    }

    #[test]
    fn long_videos_use_the_long_preset() {
        let s = Settings::default();
        assert_eq!(preset(&s, Some(299.0)), "slow");
        assert_eq!(preset(&s, Some(300.5)), "veryfast");
        assert_eq!(preset(&s, None), "slow");
    }

    #[test]
    fn fallback_presets() {
        assert_eq!(fallback_preset("slow"), Some("superfast"));
        assert_eq!(fallback_preset("veryfast"), Some("superfast"));
        assert_eq!(fallback_preset("superfast"), Some("ultrafast"));
        assert_eq!(fallback_preset("ultrafast"), None);
    }

    #[test]
    fn projection() {
        let budget = Duration::from_secs(600);
        assert!(!too_slow(Duration::from_secs(5), 0.001, budget));
        assert!(!too_slow(Duration::from_secs(60), 0.5, budget));
        assert!(too_slow(Duration::from_secs(60), 0.05, budget));
    }

    #[test]
    fn remuxable_sources() {
        let probe = |format: &str, codec: &str, pix: &str, audio: &str| -> Probe {
            serde_json::from_value(serde_json::json!({
                "streams": [
                    {"codec_type":"video","codec_name":codec,"width":640,"height":480,"pix_fmt":pix},
                    {"codec_type":"audio","codec_name":audio}
                ],
                "format": {"format_name": format}
            }))
            .unwrap()
        };
        let mov = "mov,mp4,m4a,3gp,3g2,mj2";
        assert!(remuxable(&probe(mov, "h264", "yuv420p", "aac")));
        assert!(!remuxable(&probe(mov, "hevc", "yuv420p", "aac")));
        assert!(!remuxable(&probe(mov, "h264", "yuv420p10le", "aac")));
        assert!(!remuxable(&probe(mov, "h264", "yuv420p", "pcm_s16le")));
        assert!(!remuxable(&probe(
            "matroska,webm",
            "h264",
            "yuv420p",
            "aac"
        )));
    }
}
