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
    } else if h.starts_with(b"fLaC") {
        c.flac(0)
    } else if h.starts_with(b"OggS") {
        c.ogg()
    } else if h.starts_with(b"RIFF") && h.get(8..12) == Some(b"WAVE") {
        c.chunk_audio(false)
    } else if h.starts_with(b"FORM") && matches!(h.get(8..12), Some(b"AIFF" | b"AIFC")) {
        c.chunk_audio(true)
    } else if h.starts_with(b"ID3") || h.len() >= 2 && h[0] == 255 && h[1] & 224 == 224 {
        c.mpeg()
    } else if h.starts_with(&[255, 10]) {
        Ok(())
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
}
