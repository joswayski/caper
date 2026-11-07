//! Dependency-free, lossless upload metadata cleaning shared by native clients.
//! Offsets refer to the original source. Parsing never loads audio/video samples.
use std::io::{self, Read, Seek, SeekFrom};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    Zero {
        offset: u64,
        len: u64,
    },
    Replace {
        offset: u64,
        len: u64,
        bytes: Vec<u8>,
    },
}

impl Edit {
    fn offset(&self) -> u64 {
        match self {
            Self::Zero { offset, .. } | Self::Replace { offset, .. } => *offset,
        }
    }
    fn len(&self) -> u64 {
        match self {
            Self::Zero { len, .. } | Self::Replace { len, .. } => *len,
        }
    }
    fn output_len(&self) -> u64 {
        match self {
            Self::Zero { len, .. } => *len,
            Self::Replace { bytes, .. } => bytes.len() as u64,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Unsupported,
    Malformed,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Clean(Vec<Edit>),
    Refused(Refusal),
}

fn malformed() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Malformed media container")
}
fn require(ok: bool) -> io::Result<()> {
    if ok { Ok(()) } else { Err(malformed()) }
}

/// The output length for a valid sorted, non-overlapping edit list.
pub fn cleaned_len(len: u64, edits: &[Edit]) -> u64 {
    edits
        .iter()
        .fold(len, |len, e| len - e.len() + e.output_len())
}

enum Span {
    Source(u64),
    Zero,
    Bytes(Vec<u8>),
}
struct Piece {
    start: u64,
    len: u64,
    span: Span,
}

/// A seekable edited stream. Seeks (including multipart ranges) use output offsets.
pub struct Patched<R> {
    source: R,
    pieces: Vec<Piece>,
    len: u64,
    position: u64,
}

impl<R: Read + Seek> Patched<R> {
    pub fn new(source: R, len: u64, edits: Vec<Edit>) -> io::Result<Self> {
        let mut pieces = Vec::new();
        let (mut original, mut output) = (0, 0u64);
        for edit in edits {
            let at = edit.offset();
            require(at >= original && at <= len && edit.len() <= len - at)?;
            if at > original {
                pieces.push(Piece {
                    start: output,
                    len: at - original,
                    span: Span::Source(original),
                });
                output = output.checked_add(at - original).ok_or_else(malformed)?;
            }
            original = at + edit.len();
            let size = edit.output_len();
            if size != 0 {
                let span = match edit {
                    Edit::Zero { .. } => Span::Zero,
                    Edit::Replace { bytes, .. } => Span::Bytes(bytes),
                };
                pieces.push(Piece {
                    start: output,
                    len: size,
                    span,
                });
                output = output.checked_add(size).ok_or_else(malformed)?;
            }
        }
        if original < len {
            pieces.push(Piece {
                start: output,
                len: len - original,
                span: Span::Source(original),
            });
            output = output.checked_add(len - original).ok_or_else(malformed)?;
        }
        Ok(Self {
            source,
            pieces,
            len: output,
            position: 0,
        })
    }
}

impl<R: Read + Seek> Read for Patched<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.position >= self.len {
            return Ok(0);
        }
        let i = self
            .pieces
            .partition_point(|p| p.start + p.len <= self.position);
        let piece = &self.pieces[i];
        let offset = self.position - piece.start;
        let count = (piece.len - offset).min(out.len() as u64) as usize;
        let count = match &piece.span {
            Span::Zero => {
                out[..count].fill(0);
                count
            }
            Span::Bytes(bytes) => {
                out[..count].copy_from_slice(&bytes[offset as usize..offset as usize + count]);
                count
            }
            Span::Source(start) => {
                self.source.seek(SeekFrom::Start(start + offset))?;
                let read = self.source.read(&mut out[..count])?;
                if read == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                read
            }
        };
        self.position += count as u64;
        Ok(count)
    }
}

impl<R: Read + Seek> Seek for Patched<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(at) => i128::from(at),
            SeekFrom::Current(delta) => i128::from(self.position) + i128::from(delta),
            SeekFrom::End(delta) => i128::from(self.len) + i128::from(delta),
        };
        self.position =
            u64::try_from(position).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        Ok(self.position)
    }
}

struct Cleaner<'a, R> {
    source: &'a mut R,
    len: u64,
    edits: Vec<Edit>,
}
impl<R: Read + Seek> Cleaner<'_, R> {
    fn bytes(&mut self, at: u64, len: u64) -> io::Result<Vec<u8>> {
        require(at <= self.len && len <= self.len - at)?;
        let size = usize::try_from(len).map_err(|_| malformed())?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        bytes.resize(size, 0);
        self.source.seek(SeekFrom::Start(at))?;
        self.source.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    fn number(&mut self, at: u64, size: u64, little: bool) -> io::Result<u64> {
        require(size <= 8)?;
        let bytes = self.bytes(at, size)?;
        Ok(if little {
            bytes.iter().rev().fold(0, |v, b| v << 8 | u64::from(*b))
        } else {
            bytes.iter().fold(0, |v, b| v << 8 | u64::from(*b))
        })
    }
    fn is(&mut self, at: u64, value: &[u8]) -> io::Result<bool> {
        Ok(at <= self.len
            && value.len() as u64 <= self.len - at
            && self.bytes(at, value.len() as u64)? == value)
    }
    fn zero(&mut self, offset: u64, len: u64) {
        if len > 0 {
            self.edits.push(Edit::Zero { offset, len });
        }
    }
    fn replace(&mut self, offset: u64, len: u64, bytes: &[u8]) {
        if len != 0 || !bytes.is_empty() {
            self.edits.push(Edit::Replace {
                offset,
                len,
                bytes: bytes.to_vec(),
            });
        }
    }
    fn delete(&mut self, offset: u64, len: u64) {
        self.replace(offset, len, &[]);
    }
    fn finish(mut self) -> io::Result<Vec<Edit>> {
        self.edits.sort_by_key(Edit::offset);
        let mut result: Vec<Edit> = Vec::new();
        for edit in self.edits {
            require(edit.offset() <= self.len && edit.len() <= self.len - edit.offset())?;
            if let Some(previous) = result.last_mut() {
                if let (Edit::Zero { offset: a, len: n }, Edit::Zero { offset: b, len: m }) =
                    (&mut *previous, &edit)
                    && *b <= *a + *n
                {
                    *n = (*a + *n).max(*b + *m) - *a;
                    continue;
                }
                require(previous.offset() + previous.len() <= edit.offset())?;
            }
            result.push(edit);
        }
        Ok(result)
    }

    fn trailing(&mut self, mut end: u64) -> io::Result<()> {
        loop {
            let total = if end >= 128 && self.is(end - 128, b"TAG")? {
                if end >= 355 && self.is(end - 355, b"TAG+")? {
                    355
                } else {
                    128
                }
            } else if end >= 32 && self.is(end - 32, b"APETAGEX")? {
                let size = self.number(end - 20, 4, true)?;
                let flags = self.number(end - 12, 4, true)?;
                require(size >= 32)?;
                size + if flags & 0x80000000 != 0 { 32 } else { 0 }
            } else if end >= 10 && self.is(end - 10, b"3DI")? {
                syncsafe(&self.bytes(end - 4, 4)?)? + 20
            } else if end >= 15 && self.is(end - 9, b"LYRICS200")? {
                let digits = self.bytes(end - 15, 6)?;
                require(digits.iter().all(u8::is_ascii_digit))?;
                digits
                    .iter()
                    .fold(0u64, |v, b| v * 10 + u64::from(b - b'0'))
                    + 15
            } else {
                break;
            };
            require(total <= end)?;
            self.zero(end - total, total);
            end -= total;
        }
        Ok(())
    }

    fn mpeg(&mut self) -> io::Result<()> {
        let mut at = 0;
        while self.is(at, b"ID3")? {
            let h = self.bytes(at, 10)?;
            if !(2..=4).contains(&h[3]) || h[6..10].iter().any(|v| v & 128 != 0) {
                break;
            }
            let total =
                10 + syncsafe(&h[6..10])? + if h[3] == 4 && h[5] & 0x10 != 0 { 10 } else { 0 };
            require(total <= self.len - at && total - 10 < 1 << 28)?;
            let size = total - 10;
            let mut header = h;
            header[5] = 0;
            for i in 0..4 {
                header[6 + i] = ((size >> (21 - i * 7)) & 127) as u8;
            }
            self.replace(at, 10, &header);
            self.zero(at + 10, total - 10);
            at += total;
        }
        if self.is(at, b"fLaC")? {
            self.flac(at)?;
        } else {
            self.trailing(self.len)?;
        }
        Ok(())
    }

    fn flac(&mut self, start: u64) -> io::Result<()> {
        let mut at = start + 4;
        loop {
            let h = self.bytes(at, 4)?;
            let len = u64::from(h[1]) << 16 | u64::from(h[2]) << 8 | u64::from(h[3]);
            require(len <= self.len - at - 4 && h[0] & 127 != 127)?;
            if !matches!(h[0] & 127, 0 | 3) {
                self.replace(at, 1, &[h[0] & 128 | 1]);
                self.zero(at + 4, len);
            }
            at += 4 + len;
            if h[0] & 128 != 0 {
                break;
            }
        }
        self.trailing(self.len)
    }

    fn chunk_audio(&mut self, aiff: bool) -> io::Result<()> {
        let mut at = 12;
        while at < self.len {
            let h = self.bytes(at, 8)?;
            let size = self.number(at + 4, 4, !aiff)?;
            let kind = &h[..4];
            if size > self.len - at - 8 {
                require(kind == if aiff { b"SSND" } else { b"data" })?;
                break;
            }
            let keep = if aiff {
                matches!(kind, b"COMM" | b"SSND" | b"FVER" | b"CHAN" | b"FLLR")
            } else {
                matches!(
                    kind,
                    b"fmt "
                        | b"data"
                        | b"fact"
                        | b"cue "
                        | b"smpl"
                        | b"inst"
                        | b"plst"
                        | b"acid"
                        | b"ds64"
                        | b"JUNK"
                        | b"junk"
                        | b"PAD "
                        | b"FLLR"
                )
            };
            if !keep {
                self.replace(at, 4, if aiff { b"FLLR" } else { b"JUNK" });
                self.zero(at + 8, size);
            }
            at += 8 + size + (size & 1);
        }
        Ok(())
    }
}

#[derive(Default)]
struct OggStream {
    packet: u64,
    codec: u8,
    spans: Vec<(u64, u64)>,
}

impl<R: Read + Seek> Cleaner<'_, R> {
    fn packet_bytes(&mut self, spans: &[(u64, u64)], offset: u64, len: u64) -> io::Result<Vec<u8>> {
        let mut result = Vec::new();
        let mut at = 0;
        for &(start, size) in spans {
            let a = at.max(offset);
            let b = (at + size).min(offset + len);
            if a < b {
                result.extend(self.bytes(start + a - at, b - a)?);
            }
            at += size;
        }
        require(result.len() as u64 == len)?;
        Ok(result)
    }

    fn packet_edit(&mut self, spans: &[(u64, u64)], offset: u64, len: u64, bytes: Option<&[u8]>) {
        let mut at = 0;
        for &(start, size) in spans {
            let a = at.max(offset);
            let b = (at + size).min(offset + len);
            if a < b {
                if let Some(bytes) = bytes {
                    self.replace(
                        start + a - at,
                        b - a,
                        &bytes[(a - offset) as usize..(b - offset) as usize],
                    );
                } else {
                    self.zero(start + a - at, b - a);
                }
            }
            at += size;
        }
    }

    fn ogg_packet(&mut self, stream: &mut OggStream) -> io::Result<()> {
        let len: u64 = stream.spans.iter().map(|s| s.1).sum();
        let h = self.packet_bytes(&stream.spans, 0, len.min(16))?;
        if stream.packet == 0 {
            stream.codec = if h.starts_with(b"\x01vorbis") {
                1
            } else if h.starts_with(b"OpusHead") {
                2
            } else if h.starts_with(b"\x80theora") {
                3
            } else if h.starts_with(b"Speex   ") {
                4
            } else if h.starts_with(b"\x7fFLAC") {
                5
            } else {
                0
            };
        } else if stream.packet == 1 && (1..=4).contains(&stream.codec) {
            let prefix: &[u8] = match stream.codec {
                1 => b"\x03vorbis",
                2 => b"OpusTags",
                3 => b"\x81theora",
                _ => b"",
            };
            require(h.starts_with(prefix))?;
            let p = prefix.len() as u64;
            let vendor = self.packet_bytes(&stream.spans, p, 4)?;
            let vendor = u32::from_le_bytes(vendor.try_into().unwrap()) as u64;
            let count = p + 4 + vendor;
            require(count + 4 + u64::from(stream.codec == 1) <= len)?;
            if stream.codec == 1 {
                self.packet_edit(&stream.spans, count, 4, Some(&[0; 4]));
                self.packet_edit(&stream.spans, count + 4, 1, Some(&[1]));
                self.packet_edit(&stream.spans, count + 5, len - count - 5, None);
            } else {
                self.packet_edit(&stream.spans, count, len - count, None);
            }
        } else if stream.codec == 5 {
            require(h.len() >= 4 && h[0] & 127 != 127)?;
            let size = u64::from(h[1]) << 16 | u64::from(h[2]) << 8 | u64::from(h[3]);
            require(size + 4 <= len)?;
            if !matches!(h[0] & 127, 0 | 3) {
                self.packet_edit(&stream.spans, 0, 1, Some(&[h[0] & 128 | 1]));
                self.packet_edit(&stream.spans, 4, size, None);
            }
            if h[0] & 128 != 0 {
                stream.codec = 0;
            }
        }
        stream.packet += 1;
        stream.spans.clear();
        Ok(())
    }

    fn ogg(&mut self) -> io::Result<()> {
        let mut streams = std::collections::HashMap::<u64, OggStream>::new();
        let mut pages = Vec::new();
        let mut at = 0;
        while at + 27 <= self.len {
            if !self.is(at, b"OggS")? || self.number(at + 4, 1, false)? != 0 {
                at += 1;
                continue;
            }
            let count = self.number(at + 26, 1, false)?;
            if at + 27 + count > self.len {
                at += 1;
                continue;
            }
            let lacing = self.bytes(at + 27, count)?;
            let size: u64 = lacing.iter().map(|v| u64::from(*v)).sum();
            let mut body = at + 27 + count;
            if size > self.len - body {
                at += 1;
                continue;
            }
            let serial = self.number(at + 14, 4, true)?;
            let flags = self.number(at + 5, 1, false)?;
            if flags & 2 != 0 {
                streams.insert(serial, OggStream::default());
            }
            if let Some(stream) = streams.get_mut(&serial) {
                if flags & 1 == 0 {
                    stream.spans.clear();
                }
                for size in lacing {
                    if stream.packet < 2 || stream.codec == 5 {
                        stream.spans.push((body, u64::from(size)));
                    }
                    body += u64::from(size);
                    if size < 255 {
                        if !stream.spans.is_empty() {
                            self.ogg_packet(stream)?;
                        } else {
                            stream.packet += 1;
                        }
                    }
                }
            }
            let end = at + 27 + count + size;
            pages.push((at, end));
            at = end;
        }
        // Each page is at most 65,307 bytes. Recompute only changed pages.
        for (start, end) in pages {
            if !self
                .edits
                .iter()
                .any(|e| e.offset() < end && e.offset() + e.len() > start)
            {
                continue;
            }
            let mut page = self.bytes(start, end - start)?;
            for e in &self.edits {
                let a = e.offset().max(start);
                let b = (e.offset() + e.len()).min(end);
                if a >= b {
                    continue;
                }
                let out = &mut page[(a - start) as usize..(b - start) as usize];
                match e {
                    Edit::Zero { .. } => out.fill(0),
                    Edit::Replace { bytes, .. } => out.copy_from_slice(
                        &bytes[(a - e.offset()) as usize..(b - e.offset()) as usize],
                    ),
                }
            }
            page[22..26].fill(0);
            self.replace(start + 22, 4, &ogg_crc(&page).to_le_bytes());
        }
        Ok(())
    }
}

fn orientation(tiff: &[u8]) -> Option<u8> {
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let n = |at: usize, size: usize| -> Option<u64> {
        let b = tiff.get(at..at.checked_add(size)?)?;
        Some(if little {
            b.iter().rev().fold(0, |v, b| v << 8 | u64::from(*b))
        } else {
            b.iter().fold(0, |v, b| v << 8 | u64::from(*b))
        })
    };
    let ifd = usize::try_from(n(4, 4)?).ok()?;
    for i in 0..n(ifd, 2)? as usize {
        let at = ifd + 2 + 12 * i;
        if n(at, 2)? == 0x112 && n(at + 2, 2)? == 3 && n(at + 4, 4)? == 1 {
            return u8::try_from(n(at + 8, 2)?)
                .ok()
                .filter(|n| (2..=8).contains(n));
        }
    }
    None
}

fn orientation_tiff(n: u8) -> Vec<u8> {
    vec![
        b'M', b'M', 0, 42, 0, 0, 0, 8, 0, 1, 1, 18, 0, 3, 0, 0, 0, 1, 0, n, 0, 0, 0, 0, 0, 0,
    ]
}

impl<R: Read + Seek> Cleaner<'_, R> {
    fn jpeg(&mut self) -> io::Result<()> {
        let bytes = self.bytes(0, self.len)?;
        let (mut at, mut insertion, mut orient) = (2usize, 2u64, None);
        let (mut seen_exif, mut scanned, mut entropy) = (false, false, false);
        while at < bytes.len() {
            if entropy {
                while at + 1 < bytes.len()
                    && !(bytes[at] == 255 && !matches!(bytes[at + 1], 0 | 255 | 0xd0..=0xd7))
                {
                    at += 1;
                }
                if at + 1 >= bytes.len() {
                    break;
                }
                entropy = false;
            }
            require(bytes[at] == 255)?;
            let start = at;
            while at + 1 < bytes.len() && bytes[at + 1] == 255 {
                at += 1;
            }
            if at > start {
                self.delete(start as u64, (at - start) as u64);
            }
            require(at + 2 <= bytes.len())?;
            let marker = bytes[at + 1];
            if marker == 0xd9 && scanned {
                self.delete((at + 2) as u64, self.len - at as u64 - 2);
                break;
            }
            require(!matches!(marker, 1 | 0xd0..=0xd9) && at + 4 <= bytes.len())?;
            let size = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
            require(size >= 2 && size <= bytes.len() - at - 2)?;
            let end = at + 2 + size;
            let payload = &bytes[at + 4..end];
            if marker == 0xe1 && payload.starts_with(b"Exif\0\0") && !seen_exif {
                seen_exif = true;
                orient = orientation(&payload[6..]);
            }
            let keep = match marker {
                0xe0 | 0xee => true,
                0xe2 => payload.starts_with(b"ICC_PROFILE\0"),
                0xe1..=0xef | 0xfe => false,
                _ => true,
            };
            if !keep {
                self.delete(at as u64, (end - at) as u64);
            }
            if marker == 0xe0 && !scanned {
                insertion = end as u64;
            }
            if marker == 0xda {
                entropy = true;
                scanned = true;
            }
            at = end;
        }
        if let Some(n) = orient {
            let mut segment = b"\xff\xe1\0\x22Exif\0\0".to_vec();
            segment.extend(orientation_tiff(n));
            // Insertions sort before deletions beginning at the same source byte.
            self.edits.insert(
                0,
                Edit::Replace {
                    offset: insertion,
                    len: 0,
                    bytes: segment,
                },
            );
        }
        Ok(())
    }

    fn png(&mut self) -> io::Result<()> {
        let mut at = 8;
        let mut oriented = false;
        while at < self.len {
            let h = self.bytes(at, 8)?;
            let size = u32::from_be_bytes(h[..4].try_into().unwrap()) as u64;
            require(size + 12 <= self.len - at)?;
            let kind = &h[4..];
            if kind == b"eXIf" {
                let n = if oriented {
                    None
                } else {
                    orientation(&self.bytes(at + 8, size)?)
                };
                if let Some(n) = n {
                    let mut chunk = 26u32.to_be_bytes().to_vec();
                    chunk.extend(b"eXIf");
                    chunk.extend(orientation_tiff(n));
                    chunk.extend(png_crc(&chunk[4..]).to_be_bytes());
                    self.replace(at, size + 12, &chunk);
                    oriented = true;
                } else {
                    self.delete(at, size + 12);
                }
            } else if matches!(kind, b"tEXt" | b"zTXt" | b"iTXt" | b"tIME" | b"caBX") {
                self.delete(at, size + 12);
            }
            at += size + 12;
            if kind == b"IEND" {
                self.delete(at, self.len - at);
                return Ok(());
            }
        }
        Err(malformed())
    }

    fn gif_subblocks(&mut self, mut at: u64) -> io::Result<u64> {
        loop {
            let size = self.number(at, 1, false)?;
            at += 1;
            require(size <= self.len - at)?;
            at += size;
            if size == 0 {
                return Ok(at);
            }
        }
    }

    fn gif(&mut self) -> io::Result<()> {
        let h = self.bytes(0, 13)?;
        let mut at = 13
            + if h[10] & 128 != 0 {
                3 << ((h[10] & 7) + 1)
            } else {
                0
            };
        loop {
            let start = at;
            match self.number(at, 1, false)? {
                0x3b => {
                    self.delete(at + 1, self.len - at - 1);
                    return Ok(());
                }
                0x2c => {
                    let h = self.bytes(at, 10)?;
                    at += 10
                        + if h[9] & 128 != 0 {
                            3 << ((h[9] & 7) + 1)
                        } else {
                            0
                        };
                    self.bytes(at, 1)?;
                    at = self.gif_subblocks(at + 1)?;
                }
                0x21 => {
                    let label = self.number(at + 1, 1, false)?;
                    let keep = matches!(label, 0xf9 | 1)
                        || label == 255
                            && self.number(at + 2, 1, false)? == 11
                            && matches!(
                                self.bytes(at + 3, 11)?.as_slice(),
                                b"NETSCAPE2.0" | b"ANIMEXTS1.0" | b"ICCRGBG1012"
                            );
                    at = self.gif_subblocks(at + 2)?;
                    if !keep {
                        self.delete(start, at - start);
                    }
                }
                _ => return Err(malformed()),
            }
        }
    }

    fn webp(&mut self) -> io::Result<()> {
        let end = self.number(4, 4, true)? + 8;
        require((12..=self.len).contains(&end))?;
        let (mut at, mut oriented) = (12, false);
        let mut flags = Vec::new();
        while at < end {
            require(end - at >= 8)?;
            let kind = self.bytes(at, 4)?;
            let size = self.number(at + 4, 4, true)?;
            let total = 8 + size + (size & 1);
            require(total <= end - at)?;
            if kind == b"EXIF" {
                let payload = self.bytes(at + 8, size)?;
                if let Some(n) = orientation(payload.strip_prefix(b"Exif\0\0").unwrap_or(&payload))
                {
                    let mut chunk = b"EXIF\x1a\0\0\0".to_vec();
                    chunk.extend(orientation_tiff(n));
                    self.replace(at, total, &chunk);
                    oriented = true;
                } else {
                    self.delete(at, total);
                }
            } else if !matches!(
                kind.as_slice(),
                b"VP8X" | b"VP8 " | b"VP8L" | b"ALPH" | b"ANIM" | b"ANMF" | b"ICCP"
            ) {
                self.delete(at, total);
            }
            if kind == b"VP8X" {
                require(size > 0)?;
                flags.push(at + 8);
            }
            at += total;
        }
        self.delete(end, self.len - end);
        for at in flags {
            let value = self.number(at, 1, false)? as u8 & !4 & if oriented { 255 } else { !8 };
            self.replace(at, 1, &[value]);
        }
        let size =
            u32::try_from(cleaned_len(self.len, &self.edits) - 8).map_err(|_| malformed())?;
        self.replace(4, 4, &size.to_le_bytes());
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct IsoBox {
    start: u64,
    body: u64,
    end: u64,
    kind: [u8; 4],
}

impl<R: Read + Seek> Cleaner<'_, R> {
    fn boxes(&mut self, mut at: u64, end: u64, tolerance: bool) -> io::Result<Vec<IsoBox>> {
        require(end <= self.len)?;
        let mut result = Vec::new();
        while at < end {
            require(end - at >= 8)?;
            let header = self.bytes(at, 8)?;
            let kind: [u8; 4] = header[4..8].try_into().unwrap();
            let mut size = u32::from_be_bytes(header[..4].try_into().unwrap()) as u64;
            let head = if size == 1 {
                require(end - at >= 16)?;
                size = self.number(at + 8, 8, false)?;
                16
            } else {
                8
            };
            if size == 0 {
                size = end - at;
            }
            if size > end - at && tolerance && matches!(&kind, b"mdat" | b"free" | b"skip") {
                size = end - at;
            }
            require(size >= head && size <= end - at)?;
            result.push(IsoBox {
                start: at,
                body: at + head,
                end: at + size,
                kind,
            });
            at += size;
        }
        Ok(result)
    }

    fn neutral_box(&mut self, b: &IsoBox) {
        self.replace(b.start + 4, 4, b"free");
        self.zero(b.body, b.end - b.body);
    }

    fn iso_metadata(&mut self, boxes: &[IsoBox]) {
        for b in boxes {
            if matches!(&b.kind, b"udta" | b"meta" | b"uuid") {
                self.neutral_box(b);
            }
        }
    }

    fn child(&mut self, parent: &IsoBox, kind: &[u8; 4]) -> io::Result<Option<IsoBox>> {
        Ok(self
            .boxes(parent.body, parent.end, false)?
            .into_iter()
            .find(|b| &b.kind == kind))
    }

    fn sample_bytes(&mut self, mdia: &IsoBox) -> io::Result<()> {
        let minf = self.child(mdia, b"minf")?.ok_or_else(malformed)?;
        let stbl = self.child(&minf, b"stbl")?.ok_or_else(malformed)?;
        let boxes = self.boxes(stbl.body, stbl.end, false)?;
        let find = |kind: &[u8; 4]| boxes.iter().find(|b| &b.kind == kind).ok_or_else(malformed);
        let sizes = find(b"stsz").or_else(|_| find(b"stz2"))?;
        require(sizes.end - sizes.body >= 12)?;
        let bits = if &sizes.kind == b"stz2" {
            self.number(sizes.body + 7, 1, false)?
        } else {
            32
        };
        require(matches!(bits, 4 | 8 | 16 | 32) && (&sizes.kind != b"stz2" || bits != 32))?;
        let fixed = if bits == 32 {
            self.number(sizes.body + 4, 4, false)?
        } else {
            0
        };
        let count = self.number(sizes.body + 8, 4, false)?;
        if fixed == 0 {
            require((count * bits).div_ceil(8) <= sizes.end - sizes.body - 12)?;
        }
        let sc = find(b"stsc")?;
        require(sc.end - sc.body >= 8)?;
        let entries = self.number(sc.body + 4, 4, false)?;
        require(entries <= (sc.end - sc.body - 8) / 12)?;
        let chunks = find(b"stco").or_else(|_| find(b"co64"))?;
        let width = if &chunks.kind == b"co64" { 8 } else { 4 };
        require(chunks.end - chunks.body >= 8)?;
        let n = self.number(chunks.body + 4, 4, false)?;
        require(n <= (chunks.end - chunks.body - 8) / width)?;
        let (mut entry, mut samples, mut next, mut sample) = (0, 0, 1, 0);
        for chunk in 1..=n {
            if chunk == next {
                require(entry < entries)?;
                let at = sc.body + 8 + entry * 12;
                require(self.number(at, 4, false)? == chunk)?;
                samples = self.number(at + 4, 4, false)?;
                entry += 1;
                next = if entry < entries {
                    self.number(at + 12, 4, false)?
                } else {
                    n + 1
                };
                require(next > chunk)?;
            }
            require(samples <= count - sample)?;
            let mut length = 0u64;
            if fixed != 0 {
                length = fixed.checked_mul(samples).ok_or_else(malformed)?;
            } else {
                for i in sample..sample + samples {
                    let size = if bits == 4 {
                        let byte = self.number(sizes.body + 12 + i / 2, 1, false)?;
                        if i % 2 == 0 { byte >> 4 } else { byte & 15 }
                    } else {
                        self.number(sizes.body + 12 + i * (bits / 8), bits / 8, false)?
                    };
                    length = length.checked_add(size).ok_or_else(malformed)?;
                }
            }
            let offset = self.number(chunks.body + 8 + (chunk - 1) * width, width, false)?;
            require(offset <= self.len && length <= self.len - offset)?;
            self.zero(offset, length);
            sample += samples;
        }
        require(sample == count)
    }

    fn mp4(&mut self, top: &[IsoBox], keep_tracks: bool) -> io::Result<()> {
        let moovs: Vec<_> = top.iter().filter(|b| &b.kind == b"moov").collect();
        require(if keep_tracks {
            moovs.len() <= 1
        } else {
            moovs.len() == 1
        })?;
        if !keep_tracks {
            self.iso_metadata(top);
        }
        let mut removed = Vec::new();
        let mut fragmented = false;
        for moov in moovs {
            let children = self.boxes(moov.body, moov.end, false)?;
            self.iso_metadata(&children);
            fragmented = children.iter().any(|b| &b.kind == b"mvex");
            for trak in children.iter().filter(|b| &b.kind == b"trak") {
                let tracks = self.boxes(trak.body, trak.end, false)?;
                if !keep_tracks {
                    let mdia = tracks
                        .iter()
                        .find(|b| &b.kind == b"mdia")
                        .ok_or_else(malformed)?;
                    let hdlr = self.child(mdia, b"hdlr")?.ok_or_else(malformed)?;
                    require(hdlr.end - hdlr.body >= 12)?;
                    if !matches!(self.bytes(hdlr.body + 8, 4)?.as_slice(), b"vide" | b"soun") {
                        self.sample_bytes(mdia)?;
                        if let Some(tkhd) = tracks.iter().find(|b| &b.kind == b"tkhd") {
                            let offset = if self.number(tkhd.body, 1, false)? == 1 {
                                20
                            } else {
                                12
                            };
                            require(tkhd.end - tkhd.body >= offset + 4)?;
                            removed.push(self.number(tkhd.body + offset, 4, false)?);
                        }
                        self.neutral_box(trak);
                        continue;
                    }
                }
                self.iso_metadata(&tracks);
            }
        }
        if fragmented && !removed.is_empty() {
            for moof in top.iter().filter(|b| &b.kind == b"moof") {
                for traf in self
                    .boxes(moof.body, moof.end, false)?
                    .iter()
                    .filter(|b| &b.kind == b"traf")
                {
                    if let Some(tfhd) = self.child(traf, b"tfhd")? {
                        require(tfhd.end - tfhd.body >= 8)?;
                        if removed.contains(&self.number(tfhd.body + 4, 4, false)?) {
                            self.neutral_box(traf);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn zstring(&mut self, at: &mut u64, end: u64) -> io::Result<Vec<u8>> {
        let mut result = Vec::new();
        while *at < end {
            let b = self.number(*at, 1, false)? as u8;
            *at += 1;
            if b == 0 {
                return Ok(result);
            }
            result.push(b);
        }
        Err(malformed())
    }

    fn heif(&mut self, top: &[IsoBox]) -> io::Result<()> {
        for meta in top.iter().filter(|b| &b.kind == b"meta") {
            require(meta.end - meta.body >= 4)?;
            let boxes = self.boxes(meta.body + 4, meta.end, false)?;
            let mut ids = Vec::new();
            for iinf in boxes.iter().filter(|b| &b.kind == b"iinf") {
                let width = if self.number(iinf.body, 1, false)? == 0 {
                    2
                } else {
                    4
                };
                require(iinf.end - iinf.body >= 4 + width)?;
                for infe in self
                    .boxes(iinf.body + 4 + width, iinf.end, false)?
                    .iter()
                    .filter(|b| &b.kind == b"infe")
                {
                    let version = self.number(infe.body, 1, false)?;
                    if !matches!(version, 2 | 3) {
                        continue;
                    }
                    let width = if version == 2 { 2 } else { 4 };
                    require(infe.end - infe.body >= 10 + width)?;
                    let id = self.number(infe.body + 4, width, false)?;
                    let kind = self.bytes(infe.body + 6 + width, 4)?;
                    let mut at = infe.body + 10 + width;
                    self.zstring(&mut at, infe.end)?;
                    if kind == b"Exif"
                        || kind == b"mime"
                            && self.zstring(&mut at, infe.end)? == b"application/rdf+xml"
                    {
                        ids.push(id);
                    }
                }
            }
            if ids.is_empty() {
                continue;
            }
            let iloc = boxes
                .iter()
                .find(|b| &b.kind == b"iloc")
                .ok_or_else(malformed)?;
            require(iloc.end - iloc.body >= 8)?;
            let version = self.number(iloc.body, 1, false)?;
            require(version <= 2)?;
            let fields = self.bytes(iloc.body + 4, 2)?;
            let (offset_size, length_size, base_size, index_size) = (
                u64::from(fields[0] >> 4),
                u64::from(fields[0] & 15),
                u64::from(fields[1] >> 4),
                if version == 0 {
                    0
                } else {
                    u64::from(fields[1] & 15)
                },
            );
            require(
                [offset_size, length_size, base_size, index_size]
                    .iter()
                    .all(|n| *n <= 8),
            )?;
            let width = if version == 2 { 4 } else { 2 };
            let mut at = iloc.body + 6;
            let read = |c: &mut Self, at: &mut u64, width| -> io::Result<u64> {
                require(*at <= iloc.end && width <= iloc.end - *at)?;
                let n = c.number(*at, width, false)?;
                *at += width;
                Ok(n)
            };
            let count = read(self, &mut at, width)?;
            for _ in 0..count {
                let id = read(self, &mut at, width)?;
                let method = if version == 0 {
                    0
                } else {
                    read(self, &mut at, 2)? & 15
                };
                let reference = read(self, &mut at, 2)?;
                let base = read(self, &mut at, base_size)?;
                let extents = read(self, &mut at, 2)?;
                for _ in 0..extents {
                    read(self, &mut at, index_size)?;
                    let offset = read(self, &mut at, offset_size)?;
                    let length = read(self, &mut at, length_size)?;
                    if ids.contains(&id) {
                        require(reference == 0 && method <= 1 && length != 0)?;
                        let (origin, limit) = if method == 1 {
                            let idat = boxes
                                .iter()
                                .find(|b| &b.kind == b"idat")
                                .ok_or_else(malformed)?;
                            (idat.body, idat.end)
                        } else {
                            (0, self.len)
                        };
                        let start = origin
                            .checked_add(base)
                            .and_then(|n| n.checked_add(offset))
                            .ok_or_else(malformed)?;
                        require(start <= limit && length <= limit - start)?;
                        self.zero(start, length);
                    }
                }
            }
        }
        self.mp4(top, true)
    }

    fn iso(&mut self, heif: bool) -> io::Result<()> {
        let top = self.boxes(0, self.len, !heif)?;
        if heif {
            self.heif(&top)
        } else {
            self.mp4(&top, false)
        }
    }

    fn boxed_image(&mut self, jxl: bool) -> io::Result<()> {
        for b in self.boxes(0, self.len, false)? {
            let drop = if jxl {
                matches!(&b.kind, b"Exif" | b"xml " | b"jumb")
                    || &b.kind == b"brob"
                        && b.end - b.body >= 4
                        && matches!(
                            self.bytes(b.body, 4)?.as_slice(),
                            b"Exif" | b"xml " | b"jumb"
                        )
            } else {
                matches!(&b.kind, b"uuid" | b"xml " | b"uinf")
            };
            if drop {
                self.delete(b.start, b.end - b.start);
            }
        }
        Ok(())
    }

    fn psd(&mut self) -> io::Result<()> {
        let color = self.number(26, 4, false)?;
        let length_at = 30 + color;
        let size = self.number(length_at, 4, false)?;
        let mut at = length_at + 4;
        require(size <= self.len - at)?;
        let end = at + size;
        let mut removed = 0;
        while at < end {
            require(end - at >= 7)?;
            let id = self.number(at + 4, 2, false)?;
            let name = 1 + self.number(at + 6, 1, false)?;
            let size_at = at + 6 + name + (name & 1);
            require(size_at <= end && end - size_at >= 4)?;
            let len = self.number(size_at, 4, false)?;
            let next = size_at + 4 + len + (len & 1);
            require(next <= end)?;
            if matches!(id, 0x0404 | 0x0422..=0x0425) {
                self.delete(at, next - at);
                removed += next - at;
            }
            at = next;
        }
        self.replace(length_at, 4, &((size - removed) as u32).to_be_bytes());
        Ok(())
    }
}

#[derive(Clone)]
struct Ebml {
    start: u64,
    body: u64,
    end: u64,
    id: u64,
    unknown: bool,
}
fn segment_child(id: u64) -> bool {
    matches!(
        id,
        0x1f43b675
            | 0x1c53bb6b
            | 0x1254c367
            | 0x1941a469
            | 0x1043a770
            | 0x1549a966
            | 0x1654ae6b
            | 0x114d9b74
    )
}

impl<R: Read + Seek> Cleaner<'_, R> {
    fn vint(&mut self, at: u64, end: u64, id: bool) -> io::Result<(u64, u64, bool)> {
        require(at < end)?;
        let first = self.number(at, 1, false)? as u8;
        require(first != 0)?;
        let width = u64::from(first.leading_zeros()) + 1;
        require(width <= if id { 4 } else { 8 } && width <= end - at)?;
        let raw = self.number(at, width, false)?;
        let value = if id {
            raw
        } else {
            raw & ((1 << (7 * width)) - 1)
        };
        Ok((value, width, !id && value == (1 << (7 * width)) - 1))
    }

    fn ebml(&mut self, start: u64, end: u64) -> io::Result<Ebml> {
        let (id, width, _) = self.vint(start, end, true)?;
        let (size, sw, unknown) = self.vint(start + width, end, false)?;
        let body = start + width + sw;
        require(unknown || size <= end - body)?;
        Ok(Ebml {
            start,
            body,
            end: if unknown { end } else { body + size },
            id,
            unknown,
        })
    }

    fn elements(&mut self, mut at: u64, end: u64) -> io::Result<Vec<Ebml>> {
        let mut out = Vec::new();
        while at < end {
            let mut element = self.ebml(at, end)?;
            if element.unknown && element.id == 0x1f43b675 {
                let mut next = element.body;
                while next < end {
                    let child = self.ebml(next, end)?;
                    if segment_child(child.id) {
                        break;
                    }
                    require(!child.unknown)?;
                    next = child.end;
                }
                element.end = next;
            } else {
                require(!element.unknown || element.id == 0x18538067)?;
            }
            at = element.end;
            out.push(element);
        }
        Ok(out)
    }

    fn void(&mut self, e: &Ebml) -> io::Result<()> {
        let len = e.end - e.start;
        let width = (1..=8)
            .find(|w| len > *w && len - 1 - w <= (1u64 << (7 * w)) - 2)
            .ok_or_else(malformed)?;
        let encoded = ((len - 1 - width) | (1 << (7 * width))).to_be_bytes();
        let mut h = vec![0xec];
        h.extend(&encoded[(8 - width) as usize..]);
        self.replace(e.start, 1 + width, &h);
        self.zero(e.start + 1 + width, len - 1 - width);
        Ok(())
    }

    fn crc_children(&mut self, children: &[Ebml], before: usize) -> io::Result<()> {
        if self.edits.len() > before {
            for child in children.iter().filter(|e| e.id == 0xbf) {
                self.void(child)?;
            }
        }
        Ok(())
    }

    fn webm(&mut self) -> io::Result<()> {
        for segment in self
            .elements(0, self.len)?
            .iter()
            .filter(|e| e.id == 0x18538067)
        {
            let before = self.edits.len();
            let children = self.elements(segment.body, segment.end)?;
            let mut removed = Vec::new();
            for tracks in children.iter().filter(|e| e.id == 0x1654ae6b) {
                let before = self.edits.len();
                let entries = self.elements(tracks.body, tracks.end)?;
                for entry in entries.iter().filter(|e| e.id == 0xae) {
                    let fields = self.elements(entry.body, entry.end)?;
                    let number = fields.iter().find(|e| e.id == 0xd7).ok_or_else(malformed)?;
                    let kind = fields.iter().find(|e| e.id == 0x83).ok_or_else(malformed)?;
                    if !matches!(self.number(kind.body, kind.end - kind.body, false)?, 1 | 2) {
                        removed.push(self.number(number.body, number.end - number.body, false)?);
                        self.void(entry)?;
                    } else {
                        let before = self.edits.len();
                        for field in fields.iter().filter(|e| e.id == 0x536e) {
                            self.void(field)?;
                        }
                        self.crc_children(&fields, before)?;
                    }
                }
                self.crc_children(&entries, before)?;
            }
            for child in &children {
                match child.id {
                    0x1254c367 | 0x1941a469 | 0x1043a770 => self.void(child)?,
                    0x1549a966 => {
                        let before = self.edits.len();
                        let fields = self.elements(child.body, child.end)?;
                        for field in fields.iter().filter(|e| matches!(e.id, 0x7ba9 | 0x4461)) {
                            self.void(field)?;
                        }
                        self.crc_children(&fields, before)?;
                    }
                    0x1f43b675 if !removed.is_empty() => {
                        let before = self.edits.len();
                        let blocks = self.elements(child.body, child.end)?;
                        for block in &blocks {
                            let sample = if block.id == 0xa3 {
                                Some(block.clone())
                            } else if block.id == 0xa0 {
                                self.elements(block.body, block.end)?
                                    .into_iter()
                                    .find(|e| e.id == 0xa1)
                            } else {
                                None
                            };
                            if let Some(sample) = sample
                                && removed.contains(&self.vint(sample.body, sample.end, false)?.0)
                            {
                                self.void(block)?;
                            }
                        }
                        self.crc_children(&blocks, before)?;
                    }
                    _ => {}
                }
            }
            self.crc_children(&children, before)?;
        }
        Ok(())
    }

    fn riff_chunks(&mut self, mut at: u64, end: u64) -> io::Result<Vec<IsoBox>> {
        require(end <= self.len)?;
        let mut chunks = Vec::new();
        while at < end {
            require(end - at >= 8)?;
            let kind = self.bytes(at, 4)?.try_into().unwrap();
            let size = self.number(at + 4, 4, true)?;
            require(size + 8 <= end - at)?;
            chunks.push(IsoBox {
                start: at,
                body: at + 8,
                end: at + 8 + size,
                kind,
            });
            at += 8 + size + (size & 1);
        }
        Ok(chunks)
    }

    fn junk(&mut self, chunk: &IsoBox) {
        self.replace(chunk.start, 4, b"JUNK");
        self.zero(chunk.body, chunk.end - chunk.body);
    }

    fn avi(&mut self) -> io::Result<()> {
        let end = 8 + self.number(4, 4, true)?;
        require(end >= 12)?;
        let top = self.riff_chunks(12, end)?;
        let mut removed = Vec::new();
        let mut stream = 0u64;
        let mut movie_lists = Vec::new();
        for chunk in top {
            let list = if &chunk.kind == b"LIST" {
                require(chunk.end - chunk.body >= 4)?;
                self.bytes(chunk.body, 4)?
            } else {
                vec![]
            };
            if list == b"hdrl" {
                for h in self.riff_chunks(chunk.body + 4, chunk.end)? {
                    let list = if &h.kind == b"LIST" {
                        require(h.end - h.body >= 4)?;
                        self.bytes(h.body, 4)?
                    } else {
                        vec![]
                    };
                    if list == b"strl" {
                        for field in self.riff_chunks(h.body + 4, h.end)? {
                            if &field.kind == b"strh" {
                                require(field.end - field.body >= 4)?;
                                if !matches!(
                                    self.bytes(field.body, 4)?.as_slice(),
                                    b"vids" | b"auds"
                                ) {
                                    removed.push(stream);
                                }
                            }
                            if !matches!(
                                &field.kind,
                                b"strh" | b"strf" | b"strd" | b"indx" | b"vprp" | b"JUNK"
                            ) {
                                self.junk(&field);
                            }
                        }
                        stream += 1;
                    } else if list != b"odml" && !matches!(&h.kind, b"avih" | b"JUNK") {
                        self.junk(&h);
                    }
                }
            } else if list == b"movi" {
                movie_lists.push((chunk.body + 4, chunk.end));
            } else if !matches!(&chunk.kind, b"idx1" | b"JUNK") {
                self.junk(&chunk);
            }
        }
        let mut at = end + (end & 1);
        while at < self.len {
            require(self.is(at, b"RIFF")?)?;
            let end = at + 8 + self.number(at + 4, 4, true)?;
            require(end <= self.len && end - at >= 12)?;
            if self.is(at + 8, b"AVIX")? {
                for chunk in self.riff_chunks(at + 12, end)? {
                    if &chunk.kind == b"LIST"
                        && chunk.end - chunk.body >= 4
                        && self.is(chunk.body, b"movi")?
                    {
                        movie_lists.push((chunk.body + 4, chunk.end));
                    }
                }
            }
            at = end + (end & 1);
        }
        while let Some((start, end)) = movie_lists.pop() {
            for chunk in self.riff_chunks(start, end)? {
                if &chunk.kind == b"LIST"
                    && chunk.end - chunk.body >= 4
                    && self.is(chunk.body, b"rec ")?
                {
                    movie_lists.push((chunk.body + 4, chunk.end));
                } else if chunk.kind[..2].iter().all(u8::is_ascii_digit)
                    && removed.contains(
                        &(u64::from(chunk.kind[0] - b'0') * 10 + u64::from(chunk.kind[1] - b'0')),
                    )
                {
                    self.junk(&chunk);
                }
            }
        }
        Ok(())
    }
}

fn identifying(tag: u64) -> bool {
    matches!(
        tag,
        315 | 316
            | 700
            | 33723
            | 34377
            | 40093
            | 42016
            | 42032
            | 42033
            | 42037
            | 50735
            | 65000
            | 65001
    )
}
fn tiff_size(kind: u64) -> Option<u64> {
    match kind {
        1 | 2 | 6 | 7 => Some(1),
        3 | 8 => Some(2),
        4 | 9 | 11 | 13 => Some(4),
        5 | 10 | 12 => Some(8),
        _ => None,
    }
}

impl<R: Read + Seek> Cleaner<'_, R> {
    fn tiff_value(
        &mut self,
        entry: u64,
        base: u64,
        end: u64,
        little: bool,
    ) -> io::Result<Option<(u64, u64)>> {
        let kind = self.number(entry + 2, 2, little)?;
        let count = self.number(entry + 4, 4, little)?;
        let Some(size) = tiff_size(kind) else {
            return Ok(None);
        };
        let len = size * count;
        let at = if len <= 4 {
            entry + 8
        } else {
            base.checked_add(self.number(entry + 8, 4, little)?)
                .ok_or_else(malformed)?
        };
        require(at >= base && at <= end && len <= end - at)?;
        Ok(Some((at, len)))
    }

    fn tiff(&mut self, base: u64, end: u64) -> io::Result<()> {
        require(base <= end && end <= self.len && end - base >= 8)?;
        let little = match self.bytes(base, 2)?.as_slice() {
            b"II" => true,
            b"MM" => false,
            _ => return Err(malformed()),
        };
        let root = self.number(base + 4, 4, little)?;
        let mut queue = vec![(root, 0u8)];
        let mut visited = std::collections::HashSet::new();
        let mut make = Vec::new();
        while let Some((offset, role)) = queue.pop() {
            if offset == 0 || !visited.insert(offset) {
                continue;
            }
            require(visited.len() <= 1000 && offset <= end - base && end - base - offset >= 2)?;
            let at = base + offset;
            let count = self.number(at, 2, little)?;
            let size = 6 + count * 12;
            require(size <= end - at)?;
            let next = self.number(at + 2 + count * 12, 4, little)?;
            queue.push((next, role));
            if offset == root {
                for i in 0..count {
                    let entry = at + 2 + i * 12;
                    if self.number(entry, 2, little)? == 271
                        && let Some((start, len)) = self.tiff_value(entry, base, end, little)?
                    {
                        make = self.bytes(start, len.min(128))?;
                    }
                }
            }
            let mut kept = Vec::new();
            let mut deleted = 0;
            for i in 0..count {
                let entry = at + 2 + i * 12;
                let tag = self.number(entry, 2, little)?;
                let value = match self.tiff_value(entry, base, end, little) {
                    Err(error)
                        if tag == 37500
                            && role == 1
                            && error.kind() == io::ErrorKind::InvalidData =>
                    {
                        None
                    }
                    value => value?,
                };
                if role == 2 {
                    if let Some((start, len)) = value
                        && len > 4
                    {
                        self.zero(start, len);
                    }
                    continue;
                }
                if matches!(tag, 330 | 34665 | 34853)
                    && let Some((start, len)) = value
                {
                    for i in 0..len / 4 {
                        queue.push((
                            self.number(start + i * 4, 4, little)?,
                            if tag == 34853 {
                                2
                            } else if tag == 34665 {
                                1
                            } else {
                                role
                            },
                        ));
                    }
                }
                if tag == 37500
                    && role == 1
                    && let Some((start, len)) = value
                {
                    let before = self.edits.len();
                    if let Err(error) = self.maker(start, len, base, end, little, &make) {
                        self.edits.truncate(before);
                        if !matches!(
                            error.kind(),
                            io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
                        ) {
                            return Err(error);
                        }
                    }
                }
                if identifying(tag) || tag == 34853 {
                    if let Some((start, len)) = value
                        && len > 4
                    {
                        self.zero(start, len);
                    }
                    deleted += 1;
                } else {
                    kept.extend(self.bytes(entry, 12)?);
                }
            }
            if role == 2 {
                self.zero(at, size);
            } else if deleted != 0 {
                let n = (count - deleted) as u16;
                let mut bytes = if little {
                    n.to_le_bytes()
                } else {
                    n.to_be_bytes()
                }
                .to_vec();
                bytes.extend(kept);
                bytes.extend(if little {
                    (next as u32).to_le_bytes()
                } else {
                    (next as u32).to_be_bytes()
                });
                bytes.resize(size as usize, 0);
                self.replace(at, size, &bytes);
            }
        }
        Ok(())
    }

    fn maker(
        &mut self,
        start: u64,
        len: u64,
        tiff_base: u64,
        end: u64,
        mut little: bool,
        make: &[u8],
    ) -> io::Result<()> {
        let header = self.bytes(start, len.min(20))?;
        let make = String::from_utf8_lossy(make).to_ascii_uppercase();
        let (ifd, base, order, tags, olympus): (u64, u64, Option<usize>, &[u64], bool) =
            if header.starts_with(b"Nikon\0\x02") {
                (start + 18, start + 10, Some(10), &[29, 160], false)
            } else if header.starts_with(b"FUJIFILM") {
                little = true;
                (
                    start + self.number(start + 8, 4, true)?,
                    start,
                    None,
                    &[16],
                    false,
                )
            } else if header.starts_with(b"OLYMPUS\0") {
                (start + 12, start, Some(8), &[1028, 4122], true)
            } else if header.starts_with(b"OM SYSTEM\0\0\0") {
                (start + 16, start, Some(12), &[1028, 4122], true)
            } else if header.starts_with(b"OLYMP\0") {
                (start + 8, tiff_base, None, &[1028, 4122], true)
            } else if header.starts_with(b"Panasonic\0\0\0") {
                (start + 12, tiff_base, None, &[37, 82, 84], false)
            } else if header.starts_with(b"AOC\0") {
                (start + 6, start, Some(4), &[553], false)
            } else if header.starts_with(b"PENTAX \0") {
                (start + 10, start, Some(8), &[553], false)
            } else if [
                b"SONY DSC \0\0\0".as_slice(),
                b"SONY CAM \0\0\0",
                b"SONY MOBILE\0",
            ]
            .iter()
            .any(|p| header.starts_with(p))
            {
                (start + 12, tiff_base, None, &[8241], false)
            } else if make.starts_with("CANON") {
                (start, tiff_base, None, &[9, 12, 40, 150], false)
            } else if make.starts_with("NIKON") && !header.starts_with(b"Nikon\0") {
                (start, tiff_base, None, &[29, 160], false)
            } else if make.starts_with("SONY") {
                (start, tiff_base, None, &[8241], false)
            } else {
                return Ok(());
            };
        if let Some(order) = order {
            match header.get(order..order + 2) {
                Some(b"II") => little = true,
                Some(b"MM") => little = false,
                _ => {}
            }
        }
        let mut pending = vec![(ifd, false)];
        let mut seen = std::collections::HashSet::new();
        while let Some((at, equipment)) = pending.pop() {
            if !seen.insert(at) {
                continue;
            }
            require(seen.len() <= 1000 && at >= tiff_base && at <= end && end - at >= 2)?;
            let mut count = self.number(at, 2, little)?;
            if count == 0 || count > 500 {
                little = !little;
                count = self.number(at, 2, little)?;
            }
            require(2 + count * 12 <= end - at)?;
            for i in 0..count {
                let entry = at + 2 + i * 12;
                let tag = self.number(entry, 2, little)?;
                let value = self.tiff_value(entry, base, end, little)?;
                if olympus && !equipment && tag == 8208 {
                    pending.push((base + self.number(entry + 8, 4, little)?, true));
                }
                if if equipment {
                    [257, 258, 514, 770, 4099].contains(&tag)
                } else {
                    tags.contains(&tag)
                } {
                    match value {
                        Some((offset, len)) if len > 4 => self.zero(offset, len),
                        _ => self.zero(entry + 8, 4),
                    }
                }
            }
        }
        Ok(())
    }

    fn raf(&mut self) -> io::Result<()> {
        let start = self.number(84, 4, false)?;
        let size = self.number(88, 4, false)?;
        require(
            start <= self.len
                && size <= self.len - start
                && size >= 2
                && self.is(start, b"\xff\xd8")?,
        )?;
        let end = start + size;
        let mut at = start + 2;
        while at < end {
            require(end - at >= 4 && self.number(at, 1, false)? == 255)?;
            while at + 1 < end && self.number(at + 1, 1, false)? == 255 {
                at += 1;
            }
            let marker = self.number(at + 1, 1, false)?;
            if marker == 0xda {
                break;
            }
            let len = self.number(at + 2, 2, false)?;
            require(len >= 2 && len + 2 <= end - at)?;
            let payload = at + 4;
            if marker == 0xe1 && len >= 8 && self.is(payload, b"Exif\0\0")? {
                self.tiff(payload + 6, at + 2 + len)?;
            } else if matches!(marker, 0xed | 0xfe)
                || marker == 0xe1 && self.is(payload, b"http://ns.adobe.com/xap/1.0/\0")?
            {
                self.zero(payload, len - 2);
            }
            at += len + 2;
        }
        Ok(())
    }

    fn crw(&mut self) -> io::Result<()> {
        let root = self.number(2, 4, true)?;
        let mut pending = vec![(root, self.len)];
        let mut seen = std::collections::HashSet::new();
        while let Some((start, end)) = pending.pop() {
            if !seen.insert((start, end)) {
                continue;
            }
            require(seen.len() <= 1000 && end <= self.len && start <= end && end - start >= 4)?;
            let table = start
                .checked_add(self.number(end - 4, 4, true)?)
                .ok_or_else(malformed)?;
            require(table <= end && end - table >= 2)?;
            let count = self.number(table, 2, true)?;
            require(count * 10 + 2 <= end - table)?;
            for i in 0..count {
                let entry = table + 2 + 10 * i;
                let kind = self.number(entry, 2, true)?;
                let (offset, len) = if kind & 0xc000 == 0x4000 {
                    (entry + 2, 8)
                } else {
                    (
                        start + self.number(entry + 6, 4, true)?,
                        self.number(entry + 2, 4, true)?,
                    )
                };
                require(offset <= end && len <= end - offset)?;
                if matches!(kind & 0x3fff, 0x0810 | 0x180b) {
                    self.zero(offset, len);
                } else if matches!((kind >> 8) & 0x38, 0x28 | 0x30) {
                    pending.push((offset, offset + len));
                }
            }
        }
        Ok(())
    }
}

fn png_crc(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for b in bytes {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 };
        }
    }
    !crc
}

fn ogg_crc(bytes: &[u8]) -> u32 {
    let mut crc = 0u32;
    for b in bytes {
        crc ^= u32::from(*b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x80000000 != 0 {
                crc << 1 ^ 0x04c11db7
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn syncsafe(bytes: &[u8]) -> io::Result<u64> {
    require(bytes.len() == 4 && bytes.iter().all(|v| v & 128 == 0))?;
    Ok(bytes.iter().fold(0, |v, b| v << 7 | u64::from(*b)))
}

/// Analyze the source without modifying it. I/O errors remain distinct from a refusal.
pub fn clean<R: Read + Seek>(source: &mut R, len: u64, declared_type: &str) -> io::Result<Outcome> {
    let mut c = Cleaner {
        source,
        len,
        edits: Vec::new(),
    };
    let h = c.bytes(0, len.min(32))?;
    let declared = declared_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let parsed = if h.starts_with(b"\xff\xd8\xff") {
        c.jpeg()
    } else if h.starts_with(b"\x89PNG\r\n\x1a\n") {
        c.png()
    } else if h.starts_with(b"GIF87a") || h.starts_with(b"GIF89a") {
        c.gif()
    } else if h.starts_with(b"RIFF") && h.get(8..12) == Some(b"WEBP") {
        c.webp()
    } else if h.get(4..8) == Some(b"ftyp") {
        if h.get(8..12) == Some(b"crx ") {
            return Ok(Outcome::Refused(Refusal::Unsupported));
        }
        c.iso(matches!(
            h.get(8..12),
            Some(
                b"avif"
                    | b"avis"
                    | b"mif1"
                    | b"msf1"
                    | b"heic"
                    | b"heix"
                    | b"hevc"
                    | b"hevx"
                    | b"heim"
                    | b"heis"
                    | b"heif"
            )
        ))
    } else if matches!(h.get(4..8), Some(b"moov" | b"mdat" | b"wide" | b"free")) {
        c.iso(false)
    } else if h.starts_with(b"\0\0\0\x0cJXL \r\n\x87\n") {
        c.boxed_image(true)
    } else if h.starts_with(b"\0\0\0\x0cjP  ") {
        c.boxed_image(false)
    } else if h.starts_with(b"8BPS") {
        c.psd()
    } else if h.starts_with(b"\x1aE\xdf\xa3") {
        c.webm()
    } else if h.starts_with(b"fLaC") {
        c.flac(0)
    } else if h.starts_with(b"OggS") {
        c.ogg()
    } else if h.starts_with(b"RIFF") && h.get(8..12) == Some(b"WAVE") {
        c.chunk_audio(false)
    } else if h.starts_with(b"RIFF") && h.get(8..12) == Some(b"AVI ") {
        c.avi()
    } else if h.starts_with(b"FORM") && matches!(h.get(8..12), Some(b"AIFF" | b"AIFC")) {
        c.chunk_audio(true)
    } else if h.starts_with(b"ID3") || h.len() >= 2 && h[0] == 255 && h[1] & 224 == 224 {
        c.mpeg()
    } else if h.starts_with(&[255, 10]) {
        Ok(())
    } else if [
        b"II\x2a\0".as_slice(),
        b"MM\0\x2a",
        b"IIU\0",
        b"IIRO",
        b"IIRS",
        b"MMOR",
    ]
    .iter()
    .any(|magic| h.starts_with(magic))
    {
        c.tiff(0, len)
    } else if h.starts_with(b"FUJIFILMCCD-RAW") {
        c.raf()
    } else if h.starts_with(b"II\x1a\0\0\0HEAPCCDR") {
        c.crw()
    } else if ["image/", "video/", "audio/"]
        .iter()
        .any(|prefix| declared.starts_with(prefix))
    {
        return Ok(Outcome::Refused(Refusal::Unsupported));
    } else {
        Ok(())
    };
    match parsed.and_then(|()| c.finish()) {
        Ok(edits) => Ok(Outcome::Clean(edits)),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
            ) =>
        {
            Ok(Outcome::Refused(Refusal::Malformed))
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn cleaned(bytes: &[u8], mime: &str) -> Vec<u8> {
        let Outcome::Clean(edits) =
            clean(&mut Cursor::new(bytes), bytes.len() as u64, mime).unwrap()
        else {
            panic!("refused")
        };
        let size = cleaned_len(bytes.len() as u64, &edits);
        let mut reader = Patched::new(Cursor::new(bytes), bytes.len() as u64, edits).unwrap();
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(out.len() as u64, size);
        out
    }

    #[test]
    fn patched_seeks_in_output_coordinates() {
        let edits = vec![
            Edit::Replace {
                offset: 1,
                len: 2,
                bytes: b"long".to_vec(),
            },
            Edit::Zero { offset: 4, len: 2 },
            Edit::Replace {
                offset: 6,
                len: 2,
                bytes: vec![],
            },
            Edit::Replace {
                offset: 9,
                len: 0,
                bytes: b"!".to_vec(),
            },
        ];
        let expected = b"alongd\0\0i!j";
        let mut p = Patched::new(Cursor::new(b"abcdefghij"), 10, edits).unwrap();
        for start in 0..=expected.len() {
            p.seek(SeekFrom::Start(start as u64)).unwrap();
            let mut out = Vec::new();
            p.read_to_end(&mut out).unwrap();
            assert_eq!(out, expected[start..]);
        }
        p.seek(SeekFrom::End(-2)).unwrap();
        let mut out = [0; 2];
        p.read_exact(&mut out).unwrap();
        assert_eq!(&out, b"!j");
        assert!(p.seek(SeekFrom::Current(-100)).is_err());
    }

    #[test]
    fn audio_preserves_samples_and_removes_both_ends() {
        let mut mp3 = b"ID3\x04\x00\x10\0\0\0\x03abc0123456789\xff\xfbSAMPLES".to_vec();
        let tag = mp3.len();
        mp3.extend_from_slice(b"TAG");
        mp3.extend([b'x'; 125]);
        let out = cleaned(&mp3, "audio/mpeg");
        assert_eq!(&out[..10], b"ID3\x04\0\0\0\0\0\x0d");
        assert!(out[10..23].iter().all(|v| *v == 0));
        assert_eq!(&out[23..tag], b"\xff\xfbSAMPLES");
        assert!(out[tag..].iter().all(|v| *v == 0));
        let flac = b"fLaC\0\0\0\x02ab\x84\0\0\x04nameSAMPLES";
        assert_eq!(
            cleaned(flac, "audio/flac"),
            b"fLaC\0\0\0\x02ab\x81\0\0\x04\0\0\0\0SAMPLES"
        );
    }

    fn iso_box(kind: &[u8], payload: &[u8]) -> Vec<u8> {
        let mut bytes = (8 + payload.len() as u32).to_be_bytes().to_vec();
        bytes.extend(kind);
        bytes.extend(payload);
        bytes
    }

    #[test]
    fn large_metadata_and_sample_ranges_do_not_allocate_payloads() {
        struct Sparse {
            header: Cursor<Vec<u8>>,
            read: usize,
        }
        impl Seek for Sparse {
            fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
                self.header.seek(p)
            }
        }
        impl Read for Sparse {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                assert!(out.len() <= 32, "parser tried to load a payload");
                self.read += out.len();
                self.header.read(out)
            }
        }
        let size = 6u64 << 30;
        let mut bytes = iso_box(b"ftyp", b"isom0000");
        bytes.extend(iso_box(b"moov", &[]));
        bytes.extend(b"\0\0\0\x01uuid");
        bytes.extend(size.to_be_bytes());
        let mut source = Sparse {
            header: Cursor::new(bytes),
            read: 0,
        };
        let len = size + 24;
        let Outcome::Clean(edits) = clean(&mut source, len, "video/mp4").unwrap() else {
            panic!("refused")
        };
        assert_eq!(cleaned_len(len, &edits), len);
        assert!(source.read < 128);
        let mut patched = Patched::new(source, len, edits).unwrap();
        patched.seek(SeekFrom::End(-8)).unwrap();
        let mut zeros = [1; 8];
        patched.read_exact(&mut zeros).unwrap();
        assert_eq!(zeros, [0; 8]);
    }

    #[test]
    fn jpeg_removes_between_scan_metadata_and_trailing_motion_video() {
        let source = b"\xff\xd8\xff\xda\0\x02abc\xff\0def\xff\xd1g\xff\xfe\0\x08secret\xff\xda\0\x02ijk\xff\xd9PRIVATE_MOVIE";
        assert_eq!(
            cleaned(source, "image/jpeg"),
            b"\xff\xd8\xff\xda\0\x02abc\xff\0def\xff\xd1g\xff\xda\0\x02ijk\xff\xd9"
        );
        assert_eq!(
            cleaned(b"\xff\xd8\xff\xda\0\x02truncated", "image/jpeg"),
            b"\xff\xd8\xff\xda\0\x02truncated"
        );
    }

    #[test]
    fn ogg_comments_cross_pages_and_both_crcs_change() {
        fn page(flags: u8, seq: u32, lacing: &[u8], body: &[u8]) -> Vec<u8> {
            let mut page = b"OggS\0".to_vec();
            page.push(flags);
            page.extend([0; 8]);
            page.extend(17u32.to_le_bytes());
            page.extend(seq.to_le_bytes());
            page.extend([0; 4]);
            page.push(lacing.len() as u8);
            page.extend(lacing);
            page.extend(body);
            let crc = ogg_crc(&page);
            page[22..26].copy_from_slice(&crc.to_le_bytes());
            page
        }
        let mut comment = b"OpusTags\x03\0\0\0lib\x01\0\0\0".to_vec();
        comment.extend(300u32.to_le_bytes());
        comment.extend([b'x'; 300]);
        let first = page(2, 0, &[8], b"OpusHead");
        let second = page(0, 1, &[255], &comment[..255]);
        let third = page(
            1,
            2,
            &[(comment.len() - 255) as u8, 3],
            &[&comment[255..], b"snd"].concat(),
        );
        let source = [first.clone(), second.clone(), third.clone()].concat();
        let output = cleaned(&source, "audio/ogg");
        let mut expected = comment.clone();
        expected[15..].fill(0);
        assert_eq!(
            output,
            [
                first,
                page(0, 1, &[255], &expected[..255]),
                page(
                    1,
                    2,
                    &[(expected.len() - 255) as u8, 3],
                    &[&expected[255..], b"snd"].concat()
                )
            ]
            .concat()
        );
        assert_ne!(
            second[22..26],
            output[source.len() - third.len() - second.len() + 22
                ..source.len() - third.len() - second.len() + 26]
        );
    }

    #[test]
    fn mp4_non_av_samples_use_all_compact_sample_widths() {
        for bits in [4u8, 8, 16] {
            let mut size_table = vec![0, 0, 0, 0, 0, 0, 0, bits, 0, 0, 0, 3];
            size_table.extend(match bits {
                4 => vec![0x39, 0x50],
                8 => vec![3, 9, 5],
                _ => vec![0, 3, 0, 9, 0, 5],
            });
            let stbl = [
                iso_box(b"stz2", &size_table),
                iso_box(
                    b"stsc",
                    &[0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0, 1],
                ),
                iso_box(b"stco", &[0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 24]),
            ]
            .concat();
            let mdia = [
                iso_box(b"hdlr", b"\0\0\0\0\0\0\0\0meta"),
                iso_box(b"minf", &iso_box(b"stbl", &stbl)),
            ]
            .concat();
            let track_body = iso_box(b"mdia", &mdia);
            let source = [
                iso_box(b"ftyp", b"isom0000"),
                iso_box(b"mdat", b"123456789abcdefghKEEP"),
                iso_box(b"moov", &iso_box(b"trak", &track_body)),
            ]
            .concat();
            let expected = [
                iso_box(b"ftyp", b"isom0000"),
                iso_box(b"mdat", &[&[0; 17], b"KEEP".as_slice()].concat()),
                iso_box(b"moov", &iso_box(b"free", &vec![0; track_body.len()])),
            ]
            .concat();
            assert_eq!(cleaned(&source, "video/mp4"), expected, "stz2 {bits}");
        }
    }

    #[test]
    fn truncated_containers_never_panic() {
        let containers = [
            b"\x89PNG\r\n\x1a\n\0\0\0\0IEND\xaeB\x60\x82".to_vec(),
            b"RIFF\x0c\0\0\0WAVEfmt \xff\xff\xff\xff".to_vec(),
            b"II\x2a\0\x08\0\0\0\x01\0\x3b\x01\x02\0\xff\xff\xff\xff\xff\xff\xff\xff\0\0\0\0"
                .to_vec(),
            [
                iso_box(b"ftyp", b"isom0000"),
                iso_box(b"moov", &iso_box(b"udta", b"secret")),
            ]
            .concat(),
            b"\x1aE\xdf\xa3\x80\x18\x53\x80\x67\xff\x15\x49\xa9\x66\x84\x7b\xa9\x81x".to_vec(),
        ];
        for source in containers {
            for end in 0..=source.len() {
                let bytes = &source[..end];
                let result = clean(&mut Cursor::new(bytes), end as u64, "image/test").unwrap();
                if let Outcome::Clean(edits) = result {
                    assert!(Patched::new(Cursor::new(bytes), end as u64, edits).is_ok());
                }
            }
        }
    }
}
