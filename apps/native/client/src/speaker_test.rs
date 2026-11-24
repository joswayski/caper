use rodio::Decoder;
use rodio::source::UniformSourceIterator;
use std::io::Cursor;

const WAV: &[u8] = include_bytes!("../../../web/public/audio/speaker-test.wav");

/// Web's speaker test sound as 48 kHz mono PCM for the WebRTC output path.
pub fn speaker_test_pcm() -> Option<Vec<i16>> {
    let decoder = Decoder::try_from(Cursor::new(WAV)).ok()?;
    let stereo: Vec<f32> = UniformSourceIterator::new(decoder, 2, 48_000).collect();
    Some(
        stereo
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| ((pair[0] + pair[1]) / 2.0 * 32_767.0).clamp(-32_768.0, 32_767.0) as i16)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::Source;

    #[test]
    fn speaker_test_is_the_web_sound_at_48_khz_mono() {
        let pcm = speaker_test_pcm().expect("bundled speaker-test.wav");
        let source = Decoder::try_from(Cursor::new(WAV)).unwrap();
        let seconds = source.total_duration().unwrap().as_secs_f64();
        assert!((pcm.len() as f64 / 48_000.0 - seconds).abs() < 0.01);
        assert!(pcm.iter().any(|value| value.unsigned_abs() > 300));
    }
}
