//! The Lambda handler: SQS (S3 notifications) → process → R2 → API.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use aws_lambda_events::event::sqs::{SqsBatchResponse, SqsEvent, SqsMessage};
use lambda_runtime::LambdaEvent;

use crate::api::{Api, Start};
use crate::event::{self, Notification, Upload};
use crate::filename;
use crate::process::{self, Ctx, EarlyPreview, Job};
use crate::settings::Settings;
use crate::storage::{Error, Put, Storage};

/// Lambda ephemeral storage.
const WORK_ROOT: &str = "/tmp/caper-media";
/// Time kept after processing for the R2 upload and callbacks: a fixed
/// margin plus the upload at a conservative 40 MB/s.
const RESERVE_FIXED: Duration = Duration::from_secs(30);
const RESERVE_BYTES_PER_SECOND: u64 = 40_000_000;
/// Final margin before the hard Lambda timeout for the `fail` callback.
const FINAL_MARGIN: Duration = Duration::from_secs(8);
/// Matches the queue's redrive policy (`maxReceiveCount`).
const DEFAULT_MAX_RECEIVES: u32 = 3;

pub struct Worker {
    api: Api,
    storage: Arc<Storage>,
    max_receives: u32,
}

/// Removes the per-file scratch directory however processing ends.
struct WorkDir(PathBuf);

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn reserve(bytes: u64) -> Duration {
    RESERVE_FIXED + Duration::from_secs(bytes / RESERVE_BYTES_PER_SECOND)
}

/// Converts the Lambda deadline (Unix ms) into an `Instant`.
fn deadline_instant(deadline_ms: u64) -> Instant {
    let deadline = SystemTime::UNIX_EPOCH + Duration::from_millis(deadline_ms);
    let left = deadline
        .duration_since(SystemTime::now())
        .unwrap_or(Duration::ZERO);
    Instant::now() + left
}

struct LambdaHooks {
    api: Api,
    storage: Arc<Storage>,
    id: String,
}

#[async_trait::async_trait]
impl process::Hooks for LambdaHooks {
    async fn preview(&self, path: &Path, preview: &EarlyPreview) {
        let put = Put {
            content_type: "image/webp",
            ..Put::default()
        };
        if let Err(error) = self
            .storage
            .put(&format!("preview/{}", self.id), path, put)
            .await
        {
            tracing::warn!(error = %error, "early preview upload failed");
            return;
        }
        if let Err(error) = self.api.preview(&self.id, preview).await {
            tracing::warn!(error = %error, "preview callback failed");
        }
    }

    async fn progress(&self, percent: u8) {
        // Never hold up the encoder's progress pipe on the network.
        let (api, id) = (self.api.clone(), self.id.clone());
        tokio::spawn(async move { api.progress(&id, percent).await });
    }
}

impl Worker {
    pub async fn from_env() -> Result<Self, Error> {
        let origin =
            std::env::var("MEDIA_API_ORIGIN").map_err(|_| "MEDIA_API_ORIGIN is not set")?;
        if !(origin.starts_with("https://") || origin.starts_with("http://")) {
            return Err("MEDIA_API_ORIGIN must be an http(s) origin".into());
        }
        let secret_id =
            std::env::var("MEDIA_SECRET_ID").map_err(|_| "MEDIA_SECRET_ID is not set")?;
        let sdk = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let value = aws_sdk_secretsmanager::Client::new(&sdk)
            .get_secret_value()
            .secret_id(secret_id)
            .send()
            .await
            .map_err(|e| {
                format!(
                    "could not read the media secret: {}",
                    aws_sdk_secretsmanager::error::DisplayErrorContext(e)
                )
            })?;
        let secret = crate::storage::Secret::parse(
            value
                .secret_string()
                .ok_or("media secret is not a string")?,
        )?;
        let max_receives = std::env::var("MEDIA_MAX_RECEIVES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_RECEIVES);
        Ok(Self {
            api: Api::new(&origin, secret.MEDIA_WORKER_SECRET.clone())?,
            storage: Arc::new(Storage::new(&sdk, &secret)),
            max_receives,
        })
    }

    pub async fn handle(
        &self,
        event: LambdaEvent<SqsEvent>,
    ) -> Result<SqsBatchResponse, lambda_runtime::Error> {
        let deadline = deadline_instant(event.context.deadline);
        let mut response = SqsBatchResponse::default();
        for message in event.payload.records {
            let message_id = message.message_id.clone().unwrap_or_default();
            if let Err(error) = self.message(&message, deadline).await {
                tracing::error!(message_id, error = %error, "message failed; SQS will retry it");
                response.add_failure(message_id);
            }
        }
        Ok(response)
    }

    async fn message(&self, message: &SqsMessage, deadline: Instant) -> Result<(), Error> {
        let receives: u32 = message
            .attributes
            .get("ApproximateReceiveCount")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let body = message.body.as_deref().unwrap_or_default();
        let uploads = match event::parse(body) {
            Ok(Notification::Test) => {
                tracing::info!("ignoring s3:TestEvent");
                return Ok(());
            }
            Ok(Notification::Uploads { uploads, ignored }) => {
                if ignored > 0 {
                    tracing::warn!(ignored, "ignoring records outside incoming/{{assetId}}");
                }
                uploads
            }
            Err(error) => {
                // Retrying cannot fix a malformed body.
                tracing::error!(error = %error, "dropping message");
                return Ok(());
            }
        };
        for upload in uploads {
            let id = upload.asset_id.clone();
            let final_attempt = receives >= self.max_receives;
            match self.upload(&upload, deadline).await {
                Ok(()) => {}
                Err(error) if final_attempt => {
                    tracing::error!(id, error = %error, "processing failed on the final attempt");
                    if let Err(fail) = self.api.fail(&id, "processing failed").await {
                        tracing::error!(id, error = %fail, "fail callback failed");
                    }
                    return Err(error);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    async fn upload(&self, upload: &Upload, deadline: Instant) -> Result<(), Error> {
        let id = upload.asset_id.as_str();
        let hard_stop = deadline.checked_sub(FINAL_MARGIN).unwrap_or(deadline);
        let work = tokio::time::timeout_at(hard_stop.into(), self.process(upload, deadline)).await;
        match work {
            Ok(result) => result,
            Err(_) => {
                tracing::error!(id, "processing timed out");
                self.api.fail(id, "processing timed out").await?;
                Ok(())
            }
        }
    }

    async fn process(&self, upload: &Upload, deadline: Instant) -> Result<(), Error> {
        let id = upload.asset_id.as_str();
        let started = Instant::now();
        let job = match self.api.start(id).await? {
            Start::Gone => {
                tracing::info!(id, "asset no longer needs processing");
                self.storage
                    .delete_original(&upload.bucket, &upload.key)
                    .await?;
                return Ok(());
            }
            Start::Job(job) => job,
        };
        if job.id != id {
            return Err("start returned a different asset id".into());
        }

        let work = PathBuf::from(WORK_ROOT).join(id);
        let _ = tokio::fs::remove_dir_all(&work).await;
        tokio::fs::create_dir_all(&work).await?;
        let _cleanup = WorkDir(work.clone());
        let input = work.join("input");

        let Some(size) = self
            .storage
            .download(&upload.bucket, &upload.key, &input)
            .await?
        else {
            tracing::warn!(id, "original is missing");
            self.api.fail(id, "upload missing").await?;
            return Ok(());
        };
        if i64::try_from(size).ok() != Some(job.upload_byte_size) {
            tracing::warn!(id, size, expected = job.upload_byte_size, "size mismatch");
            self.api.fail(id, "size mismatch").await?;
            self.storage
                .delete_original(&upload.bucket, &upload.key)
                .await?;
            return Ok(());
        }

        let job_input = Job {
            filename: job.filename,
            declared_content_type: job.declared_content_type,
            settings: Settings::from_value(&job.settings),
        };
        let hooks = LambdaHooks {
            api: self.api.clone(),
            storage: Arc::clone(&self.storage),
            id: id.to_owned(),
        };
        let ctx = Ctx {
            job: &job_input,
            work: &work,
            deadline: deadline.checked_sub(reserve(size)),
            hooks: &hooks,
        };
        let outcome = process::process(&input, &ctx).await?;
        let finish = &outcome.finish;
        tracing::info!(
            id,
            kind = ?finish.kind,
            content_type = finish.content_type,
            original_bytes = size,
            stored_bytes = finish.byte_size,
            gzip = finish.content_encoding.is_some(),
            elapsed_ms = started.elapsed().as_millis(),
            "processed"
        );

        self.storage
            .put(
                &format!("original/{id}"),
                &outcome.result,
                Put {
                    content_type: &finish.content_type,
                    disposition: Some(filename::disposition(&finish.filename)),
                    encoding: finish.content_encoding.as_deref(),
                },
            )
            .await?;
        if let Some(preview) = &outcome.preview {
            let put = Put {
                content_type: "image/webp",
                ..Put::default()
            };
            self.storage
                .put(&format!("preview/{id}"), preview, put)
                .await?;
        }
        self.api.finish(id, finish).await?;
        self.storage
            .delete_original(&upload.bucket, &upload.key)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserve_scales_with_size() {
        assert_eq!(reserve(0), Duration::from_secs(30));
        assert_eq!(reserve(2_000_000_000), Duration::from_secs(80));
    }

    #[test]
    fn deadline_in_the_past_is_now() {
        let before = Instant::now();
        assert!(deadline_instant(0) >= before);
        assert!(deadline_instant(0) <= Instant::now());
    }
}
