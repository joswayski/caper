//! Documents and everything else: gzip when it saves enough, otherwise the
//! original bytes unchanged.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

use flate2::Compression;

use crate::process::{Ctx, Kind, Outcome};

/// Files larger than this are sampled first so incompressible data does not
/// spend minutes in gzip -9.
const SAMPLE_ABOVE: u64 = 32 * 1024 * 1024;
const SAMPLE_BYTES: u64 = 8 * 1024 * 1024;

/// Content type recorded for a stored file. The declared type is kept when
/// it does not claim to be media (the CDN serves files as downloads anyway);
/// media claims that failed to decode become `application/octet-stream`.
pub fn content_type(declared: &str, detected: Option<&str>) -> String {
    let declared = declared
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let valid = declared.len() <= 127
        && declared.split_once('/').is_some_and(|(top, sub)| {
            let token = |part: &str| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b))
            };
            token(top) && token(sub)
        });
    let media = ["image/", "video/", "audio/"]
        .iter()
        .any(|prefix| declared.starts_with(prefix));
    if valid && !media && declared != "application/octet-stream" {
        declared
    } else {
        detected.unwrap_or("application/octet-stream").to_owned()
    }
}

/// Whether a compressed size saves at least `min_percent` of the original.
pub fn worth_keeping(original: u64, compressed: u64, min_percent: f64) -> bool {
    #[allow(clippy::cast_precision_loss)]
    let (original, compressed) = (original as f64, compressed as f64);
    original > 0.0 && compressed <= original * (1.0 - min_percent / 100.0)
}

struct Counter(u64);

impl Write for Counter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn gzip_into<W: Write>(input: impl Read, output: W) -> io::Result<W> {
    // No name and a zero mtime: the stream depends only on the bytes.
    let mut encoder = flate2::GzBuilder::new().write(output, Compression::best());
    io::copy(&mut BufReader::with_capacity(1 << 20, input), &mut encoder)?;
    encoder.finish()
}

/// Gzips `input` to `output` when it saves at least `min_percent`, returning
/// the compressed size.
pub fn gzip_file(input: &Path, output: &Path, min_percent: f64) -> io::Result<Option<u64>> {
    let original = std::fs::metadata(input)?.len();
    if original > SAMPLE_ABOVE {
        let sample = gzip_into(File::open(input)?.take(SAMPLE_BYTES), Counter(0))?;
        if !worth_keeping(SAMPLE_BYTES, sample.0, min_percent) {
            return Ok(None);
        }
    }
    let writer = gzip_into(File::open(input)?, BufWriter::new(File::create(output)?))?;
    writer
        .into_inner()
        .map_err(io::IntoInnerError::into_error)?
        .sync_all()?;
    let compressed = std::fs::metadata(output)?.len();
    if worth_keeping(original, compressed, min_percent) {
        Ok(Some(compressed))
    } else {
        std::fs::remove_file(output)?;
        Ok(None)
    }
}

pub fn unchanged(input: &Path, ctx: &Ctx<'_>, detected: Option<&str>) -> io::Result<Outcome> {
    let content_type = content_type(&ctx.job.declared_content_type, detected);
    Outcome::new(Kind::File, &content_type, input.to_path_buf(), ctx.job)
}

pub async fn gzip_or_unchanged(
    input: &Path,
    ctx: &Ctx<'_>,
    detected: Option<&str>,
) -> io::Result<Outcome> {
    let output = ctx.path("result.gz");
    let min = ctx.settings().file_min_savings_percent;
    let (source, target) = (input.to_path_buf(), output.clone());
    let compressed = tokio::task::spawn_blocking(move || gzip_file(&source, &target, min))
        .await
        .map_err(io::Error::other)??;
    if compressed.is_none() {
        return unchanged(input, ctx, detected);
    }
    let content_type = content_type(&ctx.job.declared_content_type, detected);
    let mut outcome = Outcome::new(Kind::File, &content_type, output, ctx.job)?;
    outcome.finish.content_encoding = Some("gzip".into());
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;

    #[test]
    fn content_types() {
        assert_eq!(
            content_type("text/plain; charset=utf-8", None),
            "text/plain"
        );
        assert_eq!(content_type("image/png", None), "application/octet-stream");
        assert_eq!(
            content_type("video/mp4", Some("application/zip")),
            "application/zip"
        );
        assert_eq!(
            content_type(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                Some("application/zip")
            ),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
        assert_eq!(content_type("", Some("application/pdf")), "application/pdf");
        assert_eq!(content_type("not a type", None), "application/octet-stream");
        assert_eq!(
            content_type("application/octet-stream", Some("application/pdf")),
            "application/pdf"
        );
    }

    #[test]
    fn savings_threshold() {
        assert!(worth_keeping(100, 90, 10.0));
        assert!(!worth_keeping(100, 91, 10.0));
        assert!(!worth_keeping(0, 0, 10.0));
        assert!(worth_keeping(100, 100, 0.0));
    }

    #[test]
    fn gzip_round_trip_and_skip() {
        let dir = std::env::temp_dir().join(format!("caper-media-gzip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text = dir.join("text");
        std::fs::write(&text, "hello caper ".repeat(1000)).unwrap();
        let out = dir.join("text.gz");
        let size = gzip_file(&text, &out, 10.0).unwrap().unwrap();
        let mut decoded = Vec::new();
        GzDecoder::new(File::open(&out).unwrap())
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, std::fs::read(&text).unwrap());
        assert_eq!(size, std::fs::metadata(&out).unwrap().len());

        let noise = dir.join("noise");
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let bytes: Vec<u8> = (0..4096)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state.to_le_bytes()[0]
            })
            .collect();
        std::fs::write(&noise, bytes).unwrap();
        let out = dir.join("noise.gz");
        assert_eq!(gzip_file(&noise, &out, 10.0).unwrap(), None);
        assert!(!out.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
