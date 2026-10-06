//! Audio/video containers: ffprobe decides between video, audio, or
//! neither (stored as a file).

use std::ffi::OsString;
use std::path::Path;

use crate::process::{Ctx, Error, Kind, Outcome};
use crate::tools::{self, Probe};
use crate::video;

/// PCM layouts FLAC stores bit-exactly (ffmpeg's FLAC encoder takes up to
/// 24-bit integers; float or 32-bit PCM stays WAV/AIFF).
const LOSSLESS_PCM: [&str; 5] = ["pcm_u8", "pcm_s16le", "pcm_s16be", "pcm_s24le", "pcm_s24be"];

/// Content type for audio stored unchanged, from the probed container.
pub fn audio_type(probe: &Probe) -> Option<&'static str> {
    let format = probe.format.format_name.as_str();
    let codec = probe.audio().next()?.codec_name.as_str();
    Some(match format {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "aiff" => "audio/aiff",
        "aac" => "audio/aac",
        "amr" => "audio/amr",
        "caf" => "audio/x-caf",
        "matroska,webm" if matches!(codec, "opus" | "vorbis") => "audio/webm",
        _ if probe.is_iso_media() => "audio/mp4",
        _ => return None,
    })
}

/// Whether a WAV/AIFF is converted to FLAC.
pub fn flac_candidate(probe: &Probe) -> bool {
    let Some(audio) = probe.audio().next() else {
        return false;
    };
    matches!(probe.format.format_name.as_str(), "wav" | "aiff")
        && LOSSLESS_PCM.contains(&audio.codec_name.as_str())
        && audio.channels.is_some_and(|c| (1..=8).contains(&c))
}

async fn audio(input: &Path, probe: &Probe, ctx: &Ctx<'_>) -> Result<Outcome, Error> {
    let duration = probe.duration_ms();
    if flac_candidate(probe) {
        let out = ctx.path("audio.flac");
        let result = tools::run(
            "ffmpeg",
            [
                OsString::from("-hide_banner"),
                "-loglevel".into(),
                "error".into(),
                "-nostdin".into(),
                "-y".into(),
                "-i".into(),
                input.as_os_str().to_owned(),
                "-map".into(),
                "0:a:0".into(),
                "-c:a".into(),
                "flac".into(),
                "-compression_level".into(),
                "8".into(),
                out.as_os_str().to_owned(),
            ],
            ctx.deadline,
        )
        .await;
        match result {
            Ok(_) => {
                let (flac, original) = (
                    std::fs::metadata(&out)?.len(),
                    std::fs::metadata(input)?.len(),
                );
                if flac > 0 && flac < original {
                    return Ok(
                        Outcome::new(Kind::Audio, "audio/flac", out, ctx.job)?.duration(duration)
                    );
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "FLAC encode failed; keeping the original")
            }
        }
    }
    let content_type = audio_type(probe).ok_or("unrecognised audio container")?;
    Ok(Outcome::new(Kind::Audio, content_type, input.to_path_buf(), ctx.job)?.duration(duration))
}

/// `Ok(None)` when the container has neither video nor audio.
pub async fn process(input: &Path, ctx: &Ctx<'_>) -> Result<Option<Outcome>, Error> {
    let probe = tools::ffprobe(input, ctx.deadline).await?;
    if probe.video().is_some() {
        return video::video(input, &probe, ctx).await.map(Some);
    }
    if probe.audio().next().is_some() {
        return audio(input, &probe, ctx).await.map(Some);
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(format: &str, codec: &str, channels: u32) -> Probe {
        serde_json::from_value(serde_json::json!({
            "streams": [{"codec_type":"audio","codec_name":codec,"channels":channels}],
            "format": {"format_name": format}
        }))
        .unwrap()
    }

    #[test]
    fn flac_only_for_integer_pcm() {
        assert!(flac_candidate(&probe("wav", "pcm_s16le", 2)));
        assert!(flac_candidate(&probe("aiff", "pcm_s24be", 1)));
        assert!(!flac_candidate(&probe("wav", "pcm_f32le", 2)));
        assert!(!flac_candidate(&probe("wav", "pcm_s32le", 2)));
        assert!(!flac_candidate(&probe("wav", "pcm_s16le", 12)));
        assert!(!flac_candidate(&probe("mp3", "mp3", 2)));
    }

    #[test]
    fn unchanged_audio_types() {
        assert_eq!(audio_type(&probe("mp3", "mp3", 2)), Some("audio/mpeg"));
        assert_eq!(
            audio_type(&probe("mov,mp4,m4a,3gp,3g2,mj2", "aac", 2)),
            Some("audio/mp4")
        );
        assert_eq!(audio_type(&probe("ogg", "opus", 2)), Some("audio/ogg"));
        assert_eq!(
            audio_type(&probe("matroska,webm", "opus", 2)),
            Some("audio/webm")
        );
        assert_eq!(audio_type(&probe("matroska,webm", "ac3", 2)), None);
        assert_eq!(audio_type(&probe("wav", "pcm_f32le", 2)), Some("audio/wav"));
    }
}
