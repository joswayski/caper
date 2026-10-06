//! S3 (incoming originals) and R2 (stored results) through the S3 API.

use std::path::Path;

use aws_sdk_s3::Client;
use aws_sdk_s3::config::{
    Credentials, Region, RequestChecksumCalculation, ResponseChecksumValidation,
};
use aws_sdk_s3::primitives::ByteStream;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// The keys this worker reads from the shared Secrets Manager secret (it
/// holds many others, which are ignored). Not `Debug`, so it cannot be
/// logged by accident.
#[derive(Deserialize)]
#[allow(non_snake_case)]
pub struct Secret {
    pub R2_ACCOUNT_ID: String,
    pub R2_BUCKET: String,
    pub R2_ACCESS_KEY_ID: String,
    pub R2_SECRET_ACCESS_KEY: String,
    pub MEDIA_WORKER_SECRET: String,
}

impl Secret {
    pub fn parse(json: &str) -> Result<Self, Error> {
        let secret: Self =
            serde_json::from_str(json).map_err(|_| "media secret is missing required keys")?;
        if secret.R2_ACCOUNT_ID.is_empty()
            || !secret
                .R2_ACCOUNT_ID
                .bytes()
                .all(|b| b.is_ascii_alphanumeric())
        {
            return Err("R2_ACCOUNT_ID is invalid".into());
        }
        if secret.R2_BUCKET.is_empty()
            || secret.R2_ACCESS_KEY_ID.is_empty()
            || secret.R2_SECRET_ACCESS_KEY.is_empty()
        {
            return Err("R2 settings are incomplete".into());
        }
        if secret.MEDIA_WORKER_SECRET.len() < 32 {
            return Err("MEDIA_WORKER_SECRET must be at least 32 characters".into());
        }
        Ok(secret)
    }
}

pub struct Storage {
    s3: Client,
    r2: Client,
    r2_bucket: String,
}

/// Optional object headers for an R2 upload.
#[derive(Default)]
pub struct Put<'a> {
    pub content_type: &'a str,
    pub disposition: Option<String>,
    pub encoding: Option<&'a str>,
}

impl Storage {
    pub fn new(sdk: &aws_config::SdkConfig, secret: &Secret) -> Self {
        let s3 = Client::new(sdk);
        let credentials = Credentials::new(
            secret.R2_ACCESS_KEY_ID.clone(),
            secret.R2_SECRET_ACCESS_KEY.clone(),
            None,
            None,
            "caper-r2",
        );
        let r2_config = aws_sdk_s3::config::Builder::from(sdk)
            .region(Region::new("auto"))
            .endpoint_url(format!(
                "https://{}.r2.cloudflarestorage.com",
                secret.R2_ACCOUNT_ID
            ))
            .force_path_style(true)
            .credentials_provider(credentials)
            // R2 does not need the SDK's default flexible checksums.
            .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
            .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
            .build();
        Self {
            s3,
            r2: Client::from_conf(r2_config),
            r2_bucket: secret.R2_BUCKET.clone(),
        }
    }

    /// Streams `bucket/key` to `path`. `Ok(None)` when the object is gone.
    pub async fn download(
        &self,
        bucket: &str,
        key: &str,
        path: &Path,
    ) -> Result<Option<u64>, Error> {
        let response = match self.s3.get_object().bucket(bucket).key(key).send().await {
            Ok(response) => response,
            Err(error)
                if error.as_service_error().is_some_and(
                    aws_sdk_s3::operation::get_object::GetObjectError::is_no_such_key,
                ) =>
            {
                return Ok(None);
            }
            Err(error) => {
                return Err(format!(
                    "download failed: {}",
                    aws_sdk_s3::error::DisplayErrorContext(error)
                )
                .into());
            }
        };
        let mut body = response.body.into_async_read();
        let mut file = tokio::fs::File::create(path).await?;
        let written = tokio::io::copy(&mut body, &mut file).await?;
        file.flush().await?;
        file.sync_all().await?;
        Ok(Some(written))
    }

    pub async fn delete_original(&self, bucket: &str, key: &str) -> Result<(), Error> {
        self.s3
            .delete_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                format!(
                    "delete failed: {}",
                    aws_sdk_s3::error::DisplayErrorContext(e)
                )
            })?;
        Ok(())
    }

    pub async fn put(&self, key: &str, path: &Path, put: Put<'_>) -> Result<(), Error> {
        let body = ByteStream::from_path(path).await?;
        let length = i64::try_from(tokio::fs::metadata(path).await?.len())?;
        self.r2
            .put_object()
            .bucket(&self.r2_bucket)
            .key(key)
            .content_type(put.content_type)
            .content_length(length)
            .set_content_disposition(put.disposition)
            .set_content_encoding(put.encoding.map(str::to_owned))
            .body(body)
            .send()
            .await
            .map_err(|e| {
                format!(
                    "R2 upload failed: {}",
                    aws_sdk_s3::error::DisplayErrorContext(e)
                )
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_reads_only_the_needed_keys() {
        let json = format!(
            r#"{{"R2_ACCOUNT_ID":"abc123","R2_BUCKET":"caper","R2_ACCESS_KEY_ID":"id","R2_SECRET_ACCESS_KEY":"key","MEDIA_WORKER_SECRET":"{}","DATABASE_URL":"postgres://x"}}"#,
            "s".repeat(32)
        );
        let secret = Secret::parse(&json).unwrap();
        assert_eq!(secret.R2_BUCKET, "caper");
        assert!(Secret::parse(&json.replace("abc123", "abc.evil.com/")).is_err());
        assert!(Secret::parse(&json.replace(&"s".repeat(32), "short")).is_err());
        assert!(Secret::parse("{}").is_err());
    }
}
