//! The processing step: one original in, one stored result (plus an optional
//! preview) out, described by the body of `POST …/finish`.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use crate::detect::{self, Sniffed};
use crate::filename;
use crate::settings::Settings;
use crate::{av, file, image};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// What the API told the worker about the upload (`/start`).
#[derive(Clone, Debug)]
pub struct Job {
    pub filename: String,
    pub declared_content_type: String,
    pub settings: Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Image,
    Video,
    Audio,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRef {
    pub content_type: String,
    pub byte_size: u64,
}

/// Body of `POST …/preview`: the video poster, sent before the encode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EarlyPreview {
    pub content_type: String,
    pub byte_size: u64,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Body of `POST …/finish`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finish {
    pub kind: Kind,
    pub content_type: String,
    pub filename: String,
    /// Stored bytes (the gzip size when `content_encoding` is set).
    pub byte_size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub animated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_encoding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<PreviewRef>,
}

#[derive(Debug)]
pub struct Outcome {
    pub finish: Finish,
    /// The bytes to store at `original/{id}` (may be the input itself).
    pub result: PathBuf,
    /// WebP to store at `preview/{id}`.
    pub preview: Option<PathBuf>,
}

impl Outcome {
    pub fn new(
        kind: Kind,
        content_type: &str,
        result: PathBuf,
        job: &Job,
    ) -> std::io::Result<Self> {
        let byte_size = std::fs::metadata(&result)?.len();
        let filename = match kind {
            Kind::File => filename::sanitize(&job.filename),
            _ => filename::with_extension(&job.filename, content_type),
        };
        Ok(Self {
            finish: Finish {
                kind,
                content_type: content_type.to_owned(),
                filename,
                byte_size,
                width: None,
                height: None,
                duration_ms: None,
                animated: false,
                content_encoding: None,
                preview: None,
            },
            result,
            preview: None,
        })
    }

    #[must_use]
    pub fn size(mut self, width: Option<u32>, height: Option<u32>) -> Self {
        self.finish.width = width;
        self.finish.height = height;
        self
    }

    #[must_use]
    pub fn duration(mut self, duration_ms: Option<u64>) -> Self {
        self.finish.duration_ms = duration_ms;
        self
    }

    #[must_use]
    pub fn animated(mut self) -> Self {
        self.finish.animated = true;
        self
    }

    /// Attaches a WebP preview (ignored when it cannot be read).
    #[must_use]
    pub fn preview(mut self, preview: Option<PathBuf>) -> Self {
        if let Some(path) = preview
            && let Ok(meta) = std::fs::metadata(&path)
        {
            self.finish.preview = Some(PreviewRef {
                content_type: "image/webp".into(),
                byte_size: meta.len(),
            });
            self.preview = Some(path);
        }
        self
    }
}

/// Side effects the pipeline triggers while it runs (R2 + API in Lambda,
/// nothing in local mode).
#[async_trait::async_trait]
pub trait Hooks: Send + Sync {
    /// A video poster is ready before the full encode.
    async fn preview(&self, path: &Path, preview: &EarlyPreview);
    /// Encode progress, already throttled.
    async fn progress(&self, percent: u8);
}

pub struct NoHooks;

#[async_trait::async_trait]
impl Hooks for NoHooks {
    async fn preview(&self, _: &Path, _: &EarlyPreview) {}
    async fn progress(&self, _: u8) {}
}

pub struct Ctx<'a> {
    pub job: &'a Job,
    /// Scratch directory for this file (removed by the caller).
    pub work: &'a Path,
    /// Latest moment the processing step may use (Lambda deadline minus the
    /// time reserved for uploading and callbacks).
    pub deadline: Option<Instant>,
    pub hooks: &'a dyn Hooks,
}

impl Ctx<'_> {
    pub fn settings(&self) -> &Settings {
        &self.job.settings
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.work.join(name)
    }
}

fn head(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(8192);
    std::fs::File::open(path)?
        .take(8192)
        .read_to_end(&mut head)?;
    Ok(head)
}

/// Detects the real type of `input` and stores it per the compression rules.
/// Media that cannot be processed falls back to being stored as a file;
/// only I/O failures are returned as errors.
pub async fn process(input: &Path, ctx: &Ctx<'_>) -> std::io::Result<Outcome> {
    let head = head(input)?;
    let compressed = match detect::sniff(&head) {
        Sniffed::Image { format, animated } => {
            match image::process(input, format, animated, ctx).await {
                Ok(outcome) => return Ok(outcome),
                Err(error) => {
                    tracing::warn!(error = %error, format = ?format, "image could not be processed; storing as a file");
                }
            }
            format.compressed()
        }
        Sniffed::AudioVideo => {
            match av::process(input, ctx).await {
                Ok(Some(outcome)) => return Ok(outcome),
                Ok(None) => tracing::info!("no audio or video stream; storing as a file"),
                Err(error) => {
                    tracing::warn!(error = %error, "audio/video could not be processed; storing as a file");
                }
            }
            detect::av_compressed(&head)
        }
        Sniffed::Compressed(content_type) => {
            return file::unchanged(input, ctx, Some(content_type));
        }
        Sniffed::Document(content_type) => {
            return file::gzip_or_unchanged(input, ctx, Some(content_type)).await;
        }
        Sniffed::Unknown => false,
    };
    if compressed {
        file::unchanged(input, ctx, None)
    } else {
        file::gzip_or_unchanged(input, ctx, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_body_shape() {
        let finish = Finish {
            kind: Kind::Image,
            content_type: "image/avif".into(),
            filename: "photo.avif".into(),
            byte_size: 10,
            width: Some(4),
            height: Some(3),
            duration_ms: None,
            animated: false,
            content_encoding: None,
            preview: Some(PreviewRef {
                content_type: "image/webp".into(),
                byte_size: 5,
            }),
        };
        assert_eq!(
            serde_json::to_value(&finish).unwrap(),
            serde_json::json!({"kind":"image","contentType":"image/avif","filename":"photo.avif","byteSize":10,"width":4,"height":3,"animated":false,"preview":{"contentType":"image/webp","byteSize":5}})
        );
    }
}
