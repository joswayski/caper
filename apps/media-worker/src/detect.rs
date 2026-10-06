//! Real file type from the leading bytes. The declared content type and the
//! file name are never consulted for media decisions.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    WebP,
    Avif,
    Heif,
    Tiff,
    Bmp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sniffed {
    /// A still or animated image. `animated` is a hint from the container
    /// (APNG `acTL`, animated WebP flag, AVIF image sequence); GIF frame
    /// counts come from libvips.
    Image {
        format: ImageFormat,
        animated: bool,
    },
    /// Audio/video container worth probing with ffprobe.
    AudioVideo,
    /// Already compressed: stored unchanged without trying gzip.
    Compressed(&'static str),
    /// A recognised document type that may still compress (PDF).
    Document(&'static str),
    Unknown,
}

impl ImageFormat {
    /// Formats whose original may be kept (metadata stripped) when
    /// re-encoding does not save enough. HEIC/TIFF/BMP/GIF always convert.
    pub fn keepable(self) -> bool {
        matches!(self, Self::Png | Self::Jpeg | Self::WebP | Self::Avif)
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::WebP => "image/webp",
            Self::Avif => "image/avif",
            Self::Heif => "image/heic",
            Self::Tiff => "image/tiff",
            Self::Bmp => "image/bmp",
        }
    }

    /// Already entropy-coded: gzip is pointless if decoding fails.
    pub fn compressed(self) -> bool {
        !matches!(self, Self::Tiff | Self::Bmp)
    }
}

fn ftyp_brands(head: &[u8]) -> Option<Vec<&[u8]>> {
    if head.get(4..8)? != b"ftyp" {
        return None;
    }
    let size = u32::from_be_bytes(head.get(0..4)?.try_into().ok()?) as usize;
    let end = size.clamp(16, head.len());
    let mut brands = vec![head.get(8..12)?];
    let mut offset = 16;
    while offset + 4 <= end {
        brands.push(&head[offset..offset + 4]);
        offset += 4;
    }
    Some(brands)
}

/// Whether a PNG stream declares an animation (`acTL` before `IDAT`) with
/// more than one frame.
pub fn png_animated(data: &[u8]) -> bool {
    let mut offset = 8;
    while let Some(header) = data.get(offset..offset + 8) {
        let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        match &header[4..8] {
            b"acTL" => {
                return data
                    .get(offset + 8..offset + 12)
                    .is_some_and(|n| u32::from_be_bytes([n[0], n[1], n[2], n[3]]) > 1);
            }
            b"IDAT" | b"IEND" => return false,
            _ => {}
        }
        offset += 12 + len;
    }
    false
}

/// Classifies a file from (up to) its first few kilobytes.
pub fn sniff(head: &[u8]) -> Sniffed {
    let image = |format, animated| Sniffed::Image { format, animated };
    let starts = |magic: &[u8]| head.starts_with(magic);
    if starts(b"\x89PNG\r\n\x1a\n") {
        return image(ImageFormat::Png, png_animated(head));
    }
    if starts(b"\xff\xd8\xff") {
        return image(ImageFormat::Jpeg, false);
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return image(ImageFormat::Gif, false);
    }
    if starts(b"RIFF") && head.len() >= 12 {
        return match &head[8..12] {
            b"WEBP" => {
                // VP8X flags byte: bit 1 marks an animation.
                let animated = head.get(12..16) == Some(b"VP8X")
                    && head.get(20).is_some_and(|flags| flags & 0x02 != 0);
                image(ImageFormat::WebP, animated)
            }
            b"WAVE" | b"AVI " => Sniffed::AudioVideo,
            _ => Sniffed::Unknown,
        };
    }
    if starts(b"II*\0") || starts(b"MM\0*") {
        return image(ImageFormat::Tiff, false);
    }
    if starts(b"BM") && head.len() >= 26 && head[6..10] == [0, 0, 0, 0] {
        return image(ImageFormat::Bmp, false);
    }
    if let Some(brands) = ftyp_brands(head) {
        let has = |b: &[u8]| brands.contains(&b);
        if has(b"avis") {
            return image(ImageFormat::Avif, true);
        }
        if has(b"avif") {
            return image(ImageFormat::Avif, false);
        }
        if [
            b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx", b"mif1", b"msf1",
        ]
        .iter()
        .any(|b| has(*b))
        {
            return image(ImageFormat::Heif, false);
        }
        return Sniffed::AudioVideo;
    }
    if starts(b"FORM") && head.len() >= 12 && matches!(&head[8..12], b"AIFF" | b"AIFC") {
        return Sniffed::AudioVideo;
    }
    let av_magic: [&[u8]; 10] = [
        b"\x1a\x45\xdf\xa3", // Matroska / WebM
        b"fLaC",
        b"OggS",
        b"ID3",
        b"#!AMR",
        b"caff",
        b"FLV\x01",
        b"\x30\x26\xb2\x75\x8e\x66\xcf\x11", // ASF / WMV
        b"\x00\x00\x01\xba",                 // MPEG program stream
        b".snd",
    ];
    if av_magic.iter().any(|m| starts(m)) {
        return Sniffed::AudioVideo;
    }
    // MPEG audio frame sync / ADTS AAC, and MPEG-TS packets.
    if head.len() >= 4 && head[0] == 0xff && head[1] & 0xe0 == 0xe0 {
        return Sniffed::AudioVideo;
    }
    if head.len() >= 377 && head[0] == 0x47 && head[188] == 0x47 && head[376] == 0x47 {
        return Sniffed::AudioVideo;
    }
    let compressed: [(&[u8], &'static str); 10] = [
        (b"PK\x03\x04", "application/zip"),
        (b"PK\x05\x06", "application/zip"),
        (b"\x1f\x8b", "application/gzip"),
        (b"BZh", "application/x-bzip2"),
        (b"\xfd7zXZ\0", "application/x-xz"),
        (b"\x28\xb5\x2f\xfd", "application/zstd"),
        (b"7z\xbc\xaf\x27\x1c", "application/x-7z-compressed"),
        (b"Rar!\x1a\x07", "application/vnd.rar"),
        (b"wOF2", "font/woff2"),
        (b"\x04\x22\x4d\x18", "application/x-lz4"),
    ];
    if let Some((_, content_type)) = compressed.iter().find(|(m, _)| starts(m)) {
        return Sniffed::Compressed(content_type);
    }
    if starts(b"%PDF-") {
        return Sniffed::Document("application/pdf");
    }
    Sniffed::Unknown
}

/// Container formats whose payload is already compressed (gzip would not
/// help if ffmpeg cannot decode them).
pub fn av_compressed(head: &[u8]) -> bool {
    !(head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WAVE")
        || head.starts_with(b"FORM")
        || head.starts_with(b".snd"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in chunks {
            out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
            out.extend_from_slice(*kind);
            out.extend_from_slice(data);
            out.extend_from_slice(&[0; 4]);
        }
        out
    }

    #[test]
    fn images() {
        let still = png(&[(b"IHDR", &[0; 13]), (b"IDAT", &[1, 2])]);
        assert_eq!(
            sniff(&still),
            Sniffed::Image {
                format: ImageFormat::Png,
                animated: false
            }
        );
        let apng = png(&[
            (b"IHDR", &[0; 13]),
            (b"acTL", &[0, 0, 0, 3, 0, 0, 0, 0]),
            (b"IDAT", &[1]),
        ]);
        assert!(matches!(
            sniff(&apng),
            Sniffed::Image { animated: true, .. }
        ));
        let one_frame = png(&[
            (b"IHDR", &[0; 13]),
            (b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]),
            (b"IDAT", &[1]),
        ]);
        assert!(matches!(
            sniff(&one_frame),
            Sniffed::Image {
                animated: false,
                ..
            }
        ));
        assert!(matches!(
            sniff(b"\xff\xd8\xff\xe1rest"),
            Sniffed::Image {
                format: ImageFormat::Jpeg,
                ..
            }
        ));
        assert!(matches!(
            sniff(b"GIF89a...."),
            Sniffed::Image {
                format: ImageFormat::Gif,
                ..
            }
        ));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\x02\0\0\0".to_vec();
        assert_eq!(
            sniff(&webp),
            Sniffed::Image {
                format: ImageFormat::WebP,
                animated: true
            }
        );
        webp[20] = 0x10;
        assert!(matches!(
            sniff(&webp),
            Sniffed::Image {
                animated: false,
                ..
            }
        ));
        assert!(matches!(
            sniff(b"II*\0\x08\0\0\0"),
            Sniffed::Image {
                format: ImageFormat::Tiff,
                ..
            }
        ));
        let mut bmp = b"BM\0\0\0\0\0\0\0\0".to_vec();
        bmp.resize(30, 0);
        assert!(matches!(
            sniff(&bmp),
            Sniffed::Image {
                format: ImageFormat::Bmp,
                ..
            }
        ));
    }

    fn ftyp(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
        let size = 16 + 4 * compatible.len();
        let mut out = u32::try_from(size).unwrap().to_be_bytes().to_vec();
        out.extend_from_slice(b"ftyp");
        out.extend_from_slice(major);
        out.extend_from_slice(&[0; 4]);
        for brand in compatible {
            out.extend_from_slice(*brand);
        }
        out.extend_from_slice(b"\0\0\0\x08mdat");
        out
    }

    #[test]
    fn iso_media_brands() {
        assert_eq!(
            sniff(&ftyp(b"avif", &[b"mif1", b"miaf"])),
            Sniffed::Image {
                format: ImageFormat::Avif,
                animated: false
            }
        );
        assert_eq!(
            sniff(&ftyp(b"avis", &[b"avif", b"msf1"])),
            Sniffed::Image {
                format: ImageFormat::Avif,
                animated: true
            }
        );
        assert_eq!(
            sniff(&ftyp(b"heic", &[b"mif1"])),
            Sniffed::Image {
                format: ImageFormat::Heif,
                animated: false
            }
        );
        assert_eq!(
            sniff(&ftyp(b"mif1", &[b"heic"])),
            Sniffed::Image {
                format: ImageFormat::Heif,
                animated: false
            }
        );
        assert_eq!(sniff(&ftyp(b"isom", &[b"mp41"])), Sniffed::AudioVideo);
        assert_eq!(sniff(&ftyp(b"qt  ", &[b"qt  "])), Sniffed::AudioVideo);
        assert_eq!(sniff(&ftyp(b"M4A ", &[])), Sniffed::AudioVideo);
    }

    #[test]
    fn audio_video_and_files() {
        assert_eq!(sniff(b"RIFF\0\0\0\0WAVEfmt "), Sniffed::AudioVideo);
        assert_eq!(sniff(b"FORM\0\0\0\0AIFFCOMM"), Sniffed::AudioVideo);
        assert_eq!(sniff(b"\x1a\x45\xdf\xa3\x01"), Sniffed::AudioVideo);
        assert_eq!(sniff(b"ID3\x04\0"), Sniffed::AudioVideo);
        assert_eq!(sniff(b"\xff\xfb\x90\x00"), Sniffed::AudioVideo);
        assert_eq!(sniff(b"fLaC\0"), Sniffed::AudioVideo);
        assert_eq!(
            sniff(b"PK\x03\x04rest"),
            Sniffed::Compressed("application/zip")
        );
        assert_eq!(
            sniff(b"\x1f\x8b\x08"),
            Sniffed::Compressed("application/gzip")
        );
        assert_eq!(sniff(b"%PDF-1.7"), Sniffed::Document("application/pdf"));
        assert_eq!(sniff(b"hello world"), Sniffed::Unknown);
        assert_eq!(sniff(b""), Sniffed::Unknown);
        assert!(!av_compressed(b"RIFF\0\0\0\0WAVE"));
        assert!(av_compressed(b"ID3"));
    }
}
