//! Lossless metadata stripping for files uploaded as originals (anything not
//! re-encoded, including every video, since desktop has no transcoder), so
//! location and camera metadata never leave the device. Pixels and media
//! samples are never touched.
//!
//! - MP4/QuickTime: every `udta` and `meta` box directly under `moov` and
//!   under each `moov/trak` becomes a zero-filled `free` box of the same size,
//!   so no offset moves and sample data is untouched. That removes `©xyz` and
//!   `com.apple.quicktime.location.ISO6709`. Only box headers are read (`moov`
//!   may sit at the end of a large file), and the upload streams the original
//!   with those bytes substituted.
//! - JPEG: APPn segments other than APP0 (JFIF), APP2 `ICC_PROFILE` and
//!   APP14 (Adobe), and comments, are removed. A non-default EXIF orientation
//!   is written back as a minimal Exif segment so the photo stays upright.
//! - PNG: `eXIf`, `tEXt`, `zTXt` and `iTXt` chunks are removed.

use crate::compress::box_header;
use std::io::{Read, Seek, SeekFrom};

/// The box types that become `free`.
const MP4_METADATA: [&[u8; 4]; 2] = [b"udta", b"meta"];

/// One metadata box to blank: its 4-byte type at `kind` becomes `free` and
/// its contents (after the size fields) become zeros.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    pub kind: u64,
    pub body: std::ops::Range<u64>,
}

impl Patch {
    fn new(position: u64, header: u64, size: u64) -> Self {
        Self {
            kind: position + 4,
            body: position + header..position + size,
        }
    }

    fn byte(&self, at: u64) -> Option<u8> {
        if (self.kind..self.kind + 4).contains(&at) {
            Some(b"free"[(at - self.kind) as usize])
        } else if self.body.contains(&at) {
            Some(0)
        } else {
            None
        }
    }
}

/// The metadata boxes to blank. Anything unexpected ends the walk with what
/// was found so far.
pub fn mp4_metadata_patches<R: Read + Seek>(reader: &mut R) -> Vec<Patch> {
    let mut patches = Vec::new();
    let Ok(end) = reader.seek(SeekFrom::End(0)) else {
        return patches;
    };
    for (kind, position, header, size) in boxes(reader, 0, end) {
        if &kind != b"moov" {
            continue;
        }
        for (child, at, child_header, child_size) in
            boxes(reader, position + header, position + size)
        {
            if MP4_METADATA.contains(&&child) {
                patches.push(Patch::new(at, child_header, child_size));
            } else if &child == b"trak" {
                for (grandchild, at, header, size) in
                    boxes(reader, at + child_header, at + child_size)
                {
                    if MP4_METADATA.contains(&&grandchild) {
                        patches.push(Patch::new(at, header, size));
                    }
                }
            }
        }
    }
    patches
}

/// `(type, offset, header length, size)` of each box in `start..end`, read
/// by seeking from header to header.
fn boxes<R: Read + Seek>(reader: &mut R, start: u64, end: u64) -> Vec<([u8; 4], u64, u64, u64)> {
    let mut found = Vec::new();
    let mut position = start;
    while position + 8 <= end {
        let Some((kind, header, size)) = box_header(reader, position, end) else {
            break;
        };
        found.push((kind, position, header, size));
        position += size;
    }
    found
}

/// Streams `inner` with each [`Patch`] applied.
pub struct Patched<R> {
    inner: R,
    position: u64,
    patches: Vec<Patch>,
}

impl<R> Patched<R> {
    pub fn new(inner: R, patches: Vec<Patch>) -> Self {
        Self {
            inner,
            position: 0,
            patches,
        }
    }
}

impl<R: Read> Read for Patched<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        let (start, end) = (self.position, self.position + read as u64);
        for patch in &self.patches {
            let from = patch.kind.max(start);
            let to = patch.body.end.min(end);
            for at in from..to {
                if let Some(byte) = patch.byte(at) {
                    buffer[(at - start) as usize] = byte;
                }
            }
        }
        self.position = end;
        Ok(read)
    }
}

/// The EXIF orientation (1–8) in an APP1 Exif payload, if it says.
fn exif_orientation(payload: &[u8]) -> Option<u16> {
    let tiff = payload.strip_prefix(b"Exif\0\0")?;
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |at: usize| {
        let bytes: [u8; 2] = tiff.get(at..at + 2)?.try_into().ok()?;
        Some(if little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    };
    let u32_at = |at: usize| {
        let bytes: [u8; 4] = tiff.get(at..at + 4)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    };
    let ifd = u32_at(4)? as usize;
    let count = usize::from(u16_at(ifd)?);
    (0..count)
        .map(|entry| ifd + 2 + entry * 12)
        .find(|&entry| u16_at(entry) == Some(0x0112) && u16_at(entry + 2) == Some(3))
        .and_then(|entry| u16_at(entry + 8))
        .filter(|orientation| (1..=8).contains(orientation))
}

/// A complete APP1 segment holding only an orientation tag.
fn orientation_segment(orientation: u16) -> Vec<u8> {
    let mut payload = b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01".to_vec();
    payload.extend_from_slice(&0x0112_u16.to_be_bytes());
    payload.extend_from_slice(&3_u16.to_be_bytes());
    payload.extend_from_slice(&1_u32.to_be_bytes());
    payload.extend_from_slice(&orientation.to_be_bytes());
    payload.extend_from_slice(&[0, 0]);
    // No further IFD.
    payload.extend_from_slice(&[0, 0, 0, 0]);
    let mut segment = vec![0xff, 0xe1];
    segment.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    segment.extend(payload);
    segment
}

/// The JPEG without metadata segments, or `None` when its structure is not
/// understood (the caller then keeps the original).
pub fn strip_jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.get(..2)? != [0xff, 0xd8] {
        return None;
    }
    let mut output = vec![0xff, 0xd8];
    let mut orientation = None;
    // Where the orientation segment goes: after a leading APP0, else after SOI.
    let mut insert_at = 2;
    let mut offset = 2;
    loop {
        if *bytes.get(offset)? != 0xff {
            return None;
        }
        let marker = *bytes.get(offset + 1)?;
        match marker {
            // Fill bytes before a marker.
            0xff => {
                offset += 1;
                continue;
            }
            // Standalone markers carry no length.
            0x01 | 0xd0..=0xd7 => {
                output.extend_from_slice(&bytes[offset..offset + 2]);
                offset += 2;
                continue;
            }
            // Start of scan or end of image: the rest is entropy-coded data
            // and later markers, copied unchanged.
            0xda | 0xd9 => {
                output.extend_from_slice(&bytes[offset..]);
                break;
            }
            _ => {}
        }
        let length = usize::from(u16::from_be_bytes([
            *bytes.get(offset + 2)?,
            *bytes.get(offset + 3)?,
        ]));
        if length < 2 {
            return None;
        }
        let segment = bytes.get(offset..offset + 2 + length)?;
        let payload = &segment[4..];
        let keep = match marker {
            0xe0 | 0xee => true,
            0xe2 => payload.starts_with(b"ICC_PROFILE\0"),
            0xe1 => {
                orientation = orientation.or_else(|| exif_orientation(payload));
                false
            }
            0xe3..=0xef | 0xfe => false,
            _ => true,
        };
        if keep {
            output.extend_from_slice(segment);
            if marker == 0xe0 && insert_at == 2 && output.len() == 2 + segment.len() {
                insert_at = output.len();
            }
        }
        offset += 2 + length;
    }
    if let Some(orientation) = orientation.filter(|orientation| *orientation != 1) {
        output.splice(insert_at..insert_at, orientation_segment(orientation));
    }
    Some(output)
}

/// The PNG without textual and EXIF chunks, or `None` when malformed.
pub fn strip_png(bytes: &[u8]) -> Option<Vec<u8>> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.get(..8)? != SIGNATURE {
        return None;
    }
    let mut output = SIGNATURE.to_vec();
    let mut offset = 8;
    while offset < bytes.len() {
        let length = u32::from_be_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?) as usize;
        let chunk = bytes.get(offset..offset.checked_add(12 + length)?)?;
        let kind = &chunk[4..8];
        if !matches!(kind, b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt") {
            output.extend_from_slice(chunk);
        }
        offset += chunk.len();
        if kind == b"IEND" {
            break;
        }
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(body);
        bytes
    }

    fn large_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = 1_u32.to_be_bytes().to_vec();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(&((body.len() + 16) as u64).to_be_bytes());
        bytes.extend_from_slice(body);
        bytes
    }

    const LOCATION: &[u8] = b"+40.6892-074.0445/";

    /// An iPhone-style file: samples first (in a 64-bit `mdat`), `moov` at
    /// the end with location in `moov/udta`, `moov/meta` and `trak/udta`.
    fn video() -> Vec<u8> {
        let mut file = mp4_box(b"ftyp", b"qt  \0\0\0\0");
        file.extend(large_box(b"mdat", &[7; 4096]));
        let mut moov = mp4_box(b"mvhd", &[0; 100]);
        let mut trak = mp4_box(b"tkhd", &[0; 84]);
        trak.extend(mp4_box(b"udta", &mp4_box(b"\xa9xyz", LOCATION)));
        trak.extend(mp4_box(b"mdia", &mp4_box(b"udta", b"nested stays")));
        moov.extend(mp4_box(b"trak", &trak));
        moov.extend(mp4_box(b"udta", &mp4_box(b"\xa9xyz", LOCATION)));
        let mut keys = b"com.apple.quicktime.location.ISO6709".to_vec();
        keys.extend_from_slice(LOCATION);
        moov.extend(mp4_box(b"meta", &mp4_box(b"keys", &keys)));
        file.extend(mp4_box(b"moov", &moov));
        file
    }

    fn patched(file: &[u8], patches: Vec<Patch>, chunk: usize) -> Vec<u8> {
        let mut reader = Patched::new(Cursor::new(file.to_vec()), patches);
        let mut output = Vec::new();
        let mut buffer = vec![0; chunk];
        loop {
            let read = reader.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            output.extend_from_slice(&buffer[..read]);
        }
        output
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
        haystack
            .windows(needle.len())
            .enumerate()
            .filter(|(_, window)| *window == needle)
            .map(|(at, _)| at)
            .collect()
    }

    #[test]
    fn mp4_location_boxes_become_free_without_moving_samples() {
        let file = video();
        let patches = mp4_metadata_patches(&mut Cursor::new(&file));
        assert_eq!(patches.len(), 3);
        // Odd read sizes split the patched type fields across reads.
        for chunk in [1, 3, 7, 4096] {
            let output = patched(&file, patches.clone(), chunk);
            assert_eq!(output.len(), file.len());
            for patch in &patches {
                let kind = patch.kind as usize;
                assert_eq!(&output[kind..kind + 4], b"free");
                // Box sizes are unchanged.
                assert_eq!(output[kind - 4..kind], file[kind - 4..kind]);
                let body = patch.body.start as usize..patch.body.end as usize;
                assert!(output[body].iter().all(|byte| *byte == 0));
            }
            let blanked: usize = patches
                .iter()
                .map(|patch| 4 + (patch.body.end - patch.body.start) as usize)
                .sum();
            let differing = (0..file.len()).filter(|&at| output[at] != file[at]).count();
            assert!(differing <= blanked);
            // The samples are byte-identical.
            assert_eq!(output[32..32 + 4096], file[32..32 + 4096]);
            assert!(find(&output, LOCATION).is_empty());
            assert!(find(&output, b"\xa9xyz").is_empty());
            assert!(find(&output, b"ISO6709").is_empty());
            assert!(find(&output, b"meta").is_empty());
            // Deeper boxes (here mdia/udta) are left alone.
            assert_eq!(find(&output, b"nested stays").len(), 1);
            assert_eq!(
                crate::compress::probe_mp4(&mut Cursor::new(&output)),
                crate::compress::probe_mp4(&mut Cursor::new(&file))
            );
        }
        assert!(mp4_metadata_patches(&mut Cursor::new(b"junk".to_vec())).is_empty());
    }

    fn jpeg(orientation: Option<u16>) -> Vec<u8> {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(16, 8, |x, y| {
            image::Rgb([(x * 16) as u8, (y * 32) as u8, 9])
        }));
        let mut plain = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut plain, 90)
            .encode_image(&image)
            .unwrap();
        // SOI, then camera-style segments: Exif with GPS text, XMP, an ICC
        // profile, a vendor APP and a comment.
        let mut file = plain[..2].to_vec();
        let mut exif = match orientation {
            Some(orientation) => orientation_segment(orientation)[4..].to_vec(),
            None => b"Exif\0\0MM\0\x2a\0\0\0\x08\0\0".to_vec(),
        };
        exif.extend_from_slice(b"GPS 37.3349 -122.0090");
        let segment = |marker: u8, payload: &[u8]| {
            let mut segment = vec![0xff, marker];
            segment.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
            segment.extend_from_slice(payload);
            segment
        };
        file.extend(segment(0xe1, &exif));
        file.extend(segment(
            0xe1,
            b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta GPS/>",
        ));
        file.extend(segment(0xe2, b"ICC_PROFILE\0\x01\x01profile"));
        file.extend(segment(0xe9, b"vendor GPS"));
        file.extend(segment(0xfe, b"comment GPS"));
        file.extend_from_slice(&plain[2..]);
        file
    }

    #[test]
    fn jpeg_metadata_is_removed_but_orientation_and_profile_kept() {
        for orientation in [None, Some(1), Some(6)] {
            let original = jpeg(orientation);
            let stripped = strip_jpeg(&original).unwrap();
            assert!(find(&stripped, b"GPS").is_empty());
            assert!(find(&stripped, b"xmpmeta").is_empty());
            assert_eq!(find(&stripped, b"ICC_PROFILE\0").len(), 1);
            assert_eq!(
                find(&stripped, b"Exif\0\0").len(),
                usize::from(orientation == Some(6))
            );
            if let Some(at) = find(&stripped, b"Exif\0\0").first() {
                assert_eq!(exif_orientation(&stripped[*at..]), Some(6));
            }
            // Pixels are untouched.
            let pixels = |bytes: &[u8]| {
                image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)
                    .unwrap()
                    .to_rgb8()
            };
            assert_eq!(pixels(&stripped), pixels(&original));
            let mut decoder =
                image::codecs::jpeg::JpegDecoder::new(Cursor::new(&stripped)).unwrap();
            use image::ImageDecoder;
            assert_eq!(
                decoder.orientation().unwrap(),
                if orientation == Some(6) {
                    image::metadata::Orientation::Rotate90
                } else {
                    image::metadata::Orientation::NoTransforms
                }
            );
        }
        assert_eq!(strip_jpeg(b"not a jpeg"), None);
        assert_eq!(strip_jpeg(b"\xff\xd8\xff\xe1\x00"), None);
    }

    #[test]
    fn png_text_and_exif_chunks_are_removed() {
        let image = image::RgbaImage::from_fn(9, 5, |x, y| image::Rgba([x as u8, y as u8, 3, 200]));
        let mut plain = Vec::new();
        image::DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut Cursor::new(&mut plain), image::ImageFormat::Png)
            .unwrap();
        let chunk = |kind: &[u8; 4], data: &[u8]| {
            let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
            chunk.extend_from_slice(kind);
            chunk.extend_from_slice(data);
            // The stripper never reads CRCs, and these chunks are removed.
            chunk.extend_from_slice(&[0, 0, 0, 0]);
            chunk
        };
        // Signature + IHDR are the first 33 bytes.
        let mut file = plain[..33].to_vec();
        file.extend(chunk(b"tEXt", b"Location\0GPS 37.3349"));
        file.extend(chunk(b"eXIf", b"MM\0\x2aGPS"));
        file.extend(chunk(b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0GPS"));
        file.extend(chunk(b"zTXt", b"Comment\0\0GPS"));
        file.extend_from_slice(&plain[33..]);
        let stripped = strip_png(&file).unwrap();
        assert_eq!(stripped, plain);
        assert!(find(&stripped, b"GPS").is_empty());
        assert_eq!(
            image::load_from_memory(&stripped).unwrap().to_rgba8(),
            image
        );
        assert_eq!(strip_png(b"nope"), None);
    }
}
