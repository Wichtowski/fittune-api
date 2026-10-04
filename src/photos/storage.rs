use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Credentials, Region, timeout::TimeoutConfig},
};
use axum::body::Body;

use crate::config::PhotoStorageConfig;

/// Storage for private progress photos and public catalog media
#[async_trait]
pub trait PhotoStore: Send + Sync {
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<()>;
    async fn exists(&self, key: &str) -> Result<bool>;
    /// Streams an object, returning `None` when storage has no such key
    async fn get(&self, key: &str) -> Result<Option<Body>>;
    /// Deletes an object idempotently, including when the key is already absent
    async fn delete(&self, key: &str) -> Result<()>;
}

/// Path-style S3 storage, compatible with the private RustFS bucket
pub struct S3PhotoStore {
    client: Client,
    bucket: String,
}

impl S3PhotoStore {
    pub fn new(config: &PhotoStorageConfig) -> Self {
        let credentials = Credentials::new(
            &config.access_key,
            &config.secret_key,
            None,
            None,
            "fittune",
        );
        let sdk = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(&config.endpoint)
            .credentials_provider(credentials)
            .timeout_config(
                TimeoutConfig::builder()
                    .connect_timeout(Duration::from_secs(3))
                    .operation_timeout(Duration::from_secs(15))
                    .build(),
            )
            .force_path_style(true)
            .build();
        Self {
            client: Client::from_conf(sdk),
            bucket: config.bucket.clone(),
        }
    }

    /// Queues unreferenced progress-photo keys older than an hour, leaving catalog media untouched
    pub async fn queue_orphans(&self, db: &sqlx::PgPool) -> Result<usize> {
        let cutoff = chrono::Utc::now().timestamp() - 3600;
        let mut continuation = None;
        let mut queued = 0;
        loop {
            let page = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix("photos/")
                .set_continuation_token(continuation)
                .send()
                .await
                .context("RustFS list_objects_v2 failed")?;
            for object in page.contents() {
                if object
                    .last_modified()
                    .is_none_or(|modified| modified.secs() >= cutoff)
                {
                    continue;
                }
                let Some(key) = object.key().and_then(|key| {
                    key.strip_suffix("/full.jpg")
                        .or_else(|| key.strip_suffix("/thumb.jpg"))
                }) else {
                    continue;
                };
                queued += super::cleanup::queue_orphan(db, key).await? as usize;
            }
            if page.is_truncated() != Some(true) {
                break;
            }
            continuation = Some(
                page.next_continuation_token()
                    .context("missing S3 continuation token")?
                    .to_owned(),
            );
        }
        Ok(queued)
    }
}

#[async_trait]
impl PhotoStore for S3PhotoStore {
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type("image/jpeg")
            .body(bytes.into())
            .send()
            .await
            .context("RustFS put_object failed")?;
        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(err) if err.as_service_error().is_some_and(|err| err.is_not_found()) => Ok(false),
            Err(err) => Err(anyhow::Error::new(err).context("RustFS head_object failed")),
        }
    }

    async fn get(&self, key: &str) -> Result<Option<Body>> {
        match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(response) => Ok(Some(Body::new(response.body.into_inner()))),
            Err(error)
                if error
                    .as_service_error()
                    .is_some_and(|error| error.is_no_such_key()) =>
            {
                Ok(None)
            }
            Err(error) => Err(anyhow::Error::new(error).context("RustFS get_object failed")),
        }
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .context("RustFS delete_object failed")?;
        Ok(())
    }
}
