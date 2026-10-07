//! AVIF photo encoding: rav1e (pure-Rust AV1, BSD-2-Clause) wrapped in an
//! AVIF container by avif-serialize (BSD-3-Clause). 8-bit 4:2:0, full-range
//! BT.601 YCbCr, colour signalled with CICP (`nclx`) instead of an ICC
//! profile, like the Android client: sRGB or Display P3 primaries with the
//! sRGB transfer curve. Other colour profiles cannot be signalled this way,
//! so those photos keep the WebP path, which embeds the profile.
//!
//! The server's `avifQuality` is on libavif's `quality` scale (`avifenc -q`,
//! libaom); rav1e takes an AV1 quantizer index instead (see [`quantizer`]).

use image::RgbImage;
use rav1e::prelude::{
    ChromaSamplePosition, ChromaSampling, ColorDescription, ColorPrimaries, Config, Context,
    EncoderConfig, EncoderStatus, FrameType, MatrixCoefficients, PixelRange, SceneDetectionSpeed,
    SpeedSettings, TransferCharacteristics,
};

/// rav1e speed preset (0 slowest – 10 fastest). On the benchmark photos
/// (see [`quantizer`]) speed 10 takes 0.9 s for 6 MP and 1.4 s for 14 MP
/// with four threads and rav1e's assembly (feature `avif-asm`), 2.6 s and
/// 4.6 s without it; `avifenc -s 6` takes 2.1 s for the 14 MP photo. Speed 9
/// saves 3% of the bytes at 1.8× the time, speed 8 5% at 5×.
const SPEED: u8 = 10;
/// Encoder threads; rav1e parallelises a still image across tiles.
const THREADS: usize = 4;
/// Tiles narrower or shorter than this cost more in compression than they
/// gain in speed.
const MIN_TILE_PIXELS: usize = 256 * 256;

/// The primaries a photo's pixels are in, from its embedded ICC profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primaries {
    Srgb,
    DisplayP3,
}

/// Untagged photos are sRGB. A profile is recognised by its description
/// (`desc` tag, ICC v2 `desc` or v4 `mluc` text): sRGB and Display P3 are
/// the profiles cameras and phones embed. `None` means "keep the profile",
/// i.e. use the WebP path.
pub fn primaries(icc: Option<&[u8]>) -> Option<Primaries> {
    let Some(icc) = icc else {
        return Some(Primaries::Srgb);
    };
    let description = icc_description(icc)?;
    if description.contains("sRGB") {
        Some(Primaries::Srgb)
    } else if description.contains("Display P3") {
        Some(Primaries::DisplayP3)
    } else {
        None
    }
}

fn be32(bytes: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_be_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?) as usize)
}

/// The profile description text, from the `desc` tag.
fn icc_description(icc: &[u8]) -> Option<String> {
    let count = be32(icc, 128)?.min(256);
    let (offset, size) = (0..count).find_map(|index| {
        let entry = 132 + index * 12;
        if icc.get(entry..entry + 4)? != b"desc" {
            return None;
        }
        Some((be32(icc, entry + 4)?, be32(icc, entry + 8)?))
    })?;
    let tag = icc.get(offset..offset.checked_add(size)?)?;
    match tag.get(..4)? {
        // textDescriptionType: ASCII count (including the NUL), then text.
        b"desc" => {
            let length = be32(tag, 8)?;
            let text = tag.get(12..12usize.checked_add(length)?)?;
            Some(
                String::from_utf8_lossy(text)
                    .trim_end_matches('\0')
                    .to_owned(),
            )
        }
        // multiLocalizedUnicodeType: the first record's UTF-16BE text.
        b"mluc" => {
            let length = be32(tag, 20)?;
            let start = be32(tag, 24)?;
            let text = tag.get(start..start.checked_add(length)?)?;
            let units: Vec<u16> = text
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    }
}

/// The AV1 quantizer index (0–255) for an `avifQuality` (1–100).
///
/// libavif 1.0 maps `quality` to libaom's 0–63 quantizer as
/// `((100 - quality) * 63 + 50) / 100`, which libaom turns into a quantizer
/// index (about four steps per level; quality 85 is level 9, index 36).
/// rav1e (speed 10, psychovisual tuning) spends more bits at the same
/// index than libaom with `tune=ssim`, so the index is offset to land on
/// the same SSIMULACRA2. Calibrated at quality 85 on the five 6–14 MP
/// benchmark photos against `avifenc -q 85 -s 6 -y 420` (libavif 1.0.4,
/// aom 3.8.2), 8-bit 4:2:0, by SSIMULACRA2 of the decoded file:
///
/// | rav1e index | mean Δ vs avifenc | per photo        | bytes vs avifenc |
/// |-------------|-------------------|------------------|------------------|
/// | 50          | +0.82             | −0.50 … +1.63    | 106.8%           |
/// | 55 (used)   | −0.05             | −1.29 … +0.64    | 99.9%            |
/// | 60          | −0.86             | −2.09 … −0.22    | 93.8%            |
///
/// Other qualities keep libavif's spacing with the same offset.
pub fn quantizer(quality: u8) -> u8 {
    const OFFSET: u16 = 19;
    let level = (u16::from(100 - quality.clamp(1, 100)) * 63 + 50) / 100;
    let index = if level == 63 { 255 } else { level * 4 };
    (index + OFFSET).min(255) as u8
}

/// Encode opaque 8-bit RGB as a still AVIF at `quality` (libavif scale).
pub fn encode(image: &RgbImage, quality: u8, primaries: Primaries) -> Result<Vec<u8>, String> {
    // rav1e reports most problems as errors but may assert on unusual
    // input; any failure falls back to WebP.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        encode_av1(image, quantizer(quality), primaries)
    }))
    .map_err(|_| "AV1 encoder panicked".to_owned())?
    .map(|av1| {
        let mut container = avif_serialize::Aviffy::new();
        container
            .set_chroma_subsampling((true, true))
            .set_full_color_range(true)
            .set_matrix_coefficients(avif_serialize::constants::MatrixCoefficients::Bt601)
            .set_transfer_characteristics(avif_serialize::constants::TransferCharacteristics::Srgb)
            .set_color_primaries(match primaries {
                Primaries::Srgb => avif_serialize::constants::ColorPrimaries::Bt709,
                Primaries::DisplayP3 => avif_serialize::constants::ColorPrimaries::DisplayP3,
            });
        container.to_vec(&av1, None, image.width(), image.height(), 8)
    })
}

/// Full-range BT.601 Y′CbCr with 2×2-averaged chroma, as libavif computes
/// 4:2:0 from RGB.
fn ycbcr420(image: &RgbImage) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    const KR: f32 = 0.299;
    const KB: f32 = 0.114;
    const KG: f32 = 1.0 - KR - KB;
    let (width, height) = (image.width() as usize, image.height() as usize);
    let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
    let pixels = image.as_raw();
    let rgb = |x: usize, y: usize| {
        let at = (y * width + x) * 3;
        (
            f32::from(pixels[at]),
            f32::from(pixels[at + 1]),
            f32::from(pixels[at + 2]),
        )
    };
    let to_u8 = |value: f32| value.round().clamp(0.0, 255.0) as u8;
    let mut luma = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let (r, g, b) = rgb(x, y);
            luma.push(to_u8(KR * r + KG * g + KB * b));
        }
    }
    let mut cb = Vec::with_capacity(chroma_width * chroma_height);
    let mut cr = Vec::with_capacity(chroma_width * chroma_height);
    for chroma_y in 0..chroma_height {
        for chroma_x in 0..chroma_width {
            let (mut sum_b, mut sum_r, mut count) = (0.0, 0.0, 0.0);
            for y in chroma_y * 2..(chroma_y * 2 + 2).min(height) {
                for x in chroma_x * 2..(chroma_x * 2 + 2).min(width) {
                    let (r, g, b) = rgb(x, y);
                    let luma = KR * r + KG * g + KB * b;
                    sum_b += (b - luma) / (2.0 * (1.0 - KB));
                    sum_r += (r - luma) / (2.0 * (1.0 - KR));
                    count += 1.0;
                }
            }
            cb.push(to_u8(sum_b / count + 128.0));
            cr.push(to_u8(sum_r / count + 128.0));
        }
    }
    (luma, cb, cr)
}

fn encode_av1(image: &RgbImage, quantizer: u8, primaries: Primaries) -> Result<Vec<u8>, String> {
    let (width, height) = (image.width() as usize, image.height() as usize);
    let mut speed_settings = SpeedSettings::from_preset(SPEED);
    // A single still frame: no reference frames to search or scenes to cut.
    speed_settings.multiref = false;
    speed_settings.rdo_lookahead_frames = 1;
    speed_settings.scene_detection_mode = SceneDetectionSpeed::None;
    let config = Config::new()
        .with_encoder_config(EncoderConfig {
            width,
            height,
            bit_depth: 8,
            chroma_sampling: ChromaSampling::Cs420,
            chroma_sample_position: ChromaSamplePosition::Unknown,
            pixel_range: PixelRange::Full,
            color_description: Some(ColorDescription {
                color_primaries: match primaries {
                    Primaries::Srgb => ColorPrimaries::BT709,
                    Primaries::DisplayP3 => ColorPrimaries::SMPTE432,
                },
                transfer_characteristics: TransferCharacteristics::SRGB,
                matrix_coefficients: MatrixCoefficients::BT601,
            }),
            still_picture: true,
            quantizer: usize::from(quantizer),
            min_quantizer: quantizer,
            // A fixed tile count keeps the output the same on every machine.
            tiles: THREADS.min(width * height / MIN_TILE_PIXELS).max(1),
            speed_settings,
            ..EncoderConfig::with_speed_preset(SPEED)
        })
        .with_threads(THREADS);
    let mut context: Context<u8> = config.new_context().map_err(|error| error.to_string())?;
    let mut frame = context.new_frame();
    let (luma, cb, cr) = ycbcr420(image);
    for (plane, data, plane_width) in [
        (0, &luma, width),
        (1, &cb, width.div_ceil(2)),
        (2, &cr, width.div_ceil(2)),
    ] {
        let mut slice = frame.planes[plane].mut_slice(Default::default());
        for (row, source) in slice.rows_iter_mut().zip(data.chunks_exact(plane_width)) {
            row[..plane_width].copy_from_slice(source);
        }
    }
    context
        .send_frame(frame)
        .map_err(|error| error.to_string())?;
    context.flush();
    let mut av1 = Vec::new();
    loop {
        match context.receive_packet() {
            Ok(mut packet) if packet.frame_type == FrameType::KEY => av1.append(&mut packet.data),
            Ok(_) => {}
            // Encoded: a frame is done but its packet comes next.
            Err(EncoderStatus::Encoded) => {}
            Err(EncoderStatus::LimitReached) => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    if av1.is_empty() {
        return Err("AV1 encoder produced no frame".into());
    }
    Ok(av1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_maps_onto_the_libavif_quantizer_scale() {
        // The calibrated point: avifenc -q 85 is libaom index 36.
        assert_eq!(quantizer(85), 55);
        assert_eq!(quantizer(0), quantizer(1), "clamped to 1");
        assert!(quantizer(100) < quantizer(85));
        assert!(quantizer(85) < quantizer(60));
        assert!(quantizer(60) < quantizer(1));
    }

    /// An ICC profile with one `desc` tag of the given type and payload.
    fn profile(tag: &[u8]) -> Vec<u8> {
        let mut icc = vec![0_u8; 128];
        icc[16..20].copy_from_slice(b"RGB ");
        icc.extend_from_slice(&1_u32.to_be_bytes());
        icc.extend_from_slice(b"desc");
        icc.extend_from_slice(&144_u32.to_be_bytes());
        icc.extend_from_slice(&(tag.len() as u32).to_be_bytes());
        icc.extend_from_slice(tag);
        icc
    }

    fn desc_v2(text: &str) -> Vec<u8> {
        let mut tag = b"desc\0\0\0\0".to_vec();
        tag.extend_from_slice(&(text.len() as u32 + 1).to_be_bytes());
        tag.extend_from_slice(text.as_bytes());
        tag.push(0);
        tag
    }

    fn mluc(text: &str) -> Vec<u8> {
        let units: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let mut tag = b"mluc\0\0\0\0".to_vec();
        tag.extend_from_slice(&1_u32.to_be_bytes());
        tag.extend_from_slice(&12_u32.to_be_bytes());
        tag.extend_from_slice(b"enUS");
        tag.extend_from_slice(&(units.len() as u32).to_be_bytes());
        tag.extend_from_slice(&28_u32.to_be_bytes());
        tag.extend(units);
        tag
    }

    #[test]
    fn profiles_are_recognised_by_description() {
        assert_eq!(primaries(None), Some(Primaries::Srgb));
        assert_eq!(
            primaries(Some(&profile(&desc_v2("sRGB IEC61966-2.1")))),
            Some(Primaries::Srgb)
        );
        assert_eq!(
            primaries(Some(&profile(&mluc("Display P3")))),
            Some(Primaries::DisplayP3)
        );
        assert_eq!(primaries(Some(&profile(&mluc("Adobe RGB (1998)")))), None);
        assert_eq!(primaries(Some(&[0; 64])), None, "malformed");
        let mut truncated = profile(&mluc("sRGB"));
        truncated.truncate(150);
        assert_eq!(primaries(Some(&truncated)), None);
    }

    #[test]
    fn encodes_a_valid_420_still() {
        // Odd dimensions exercise the partial chroma blocks.
        for (width, height) in [(1, 1), (37, 21), (300, 200)] {
            let image = RgbImage::from_fn(width, height, |x, y| {
                image::Rgb([(x * 5 % 256) as u8, (y * 3 % 256) as u8, 90])
            });
            let avif = encode(&image, 85, Primaries::Srgb).unwrap();
            assert_eq!(&avif[4..8], b"ftyp");
            assert_eq!(&avif[8..12], b"avif");
            assert!(crate::compress::sniff("image/avif", &avif));
        }
    }

    #[test]
    fn chroma_is_averaged_over_two_by_two_blocks() {
        let image = RgbImage::from_fn(3, 1, |x, _| {
            image::Rgb(if x == 0 { [255, 0, 0] } else { [0, 0, 255] })
        });
        let (luma, cb, cr) = ycbcr420(&image);
        assert_eq!(luma, vec![76, 29, 29]);
        assert_eq!((cb.len(), cr.len()), (2, 2));
        // Red and blue share a block; the last column is blue alone.
        assert_eq!((cb[0], cr[0]), (170, 181));
        assert_eq!((cb[1], cr[1]), (255, 107));
    }
}
