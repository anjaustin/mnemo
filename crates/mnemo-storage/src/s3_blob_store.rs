//! S3-compatible blob storage implementation.
//!
//! Supports AWS S3, MinIO, Cloudflare R2, and other S3-compatible stores.
//! Provides pre-signed URLs for direct client uploads/downloads.

use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::config::{Builder as S3ConfigBuilder, Region};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use tracing::{debug, instrument};

use mnemo_core::error::MnemoError;
use mnemo_core::traits::blob::{BlobMetadata, BlobResult, BlobStore, PresignOptions};

/// S3-compatible blob store configuration.
#[derive(Debug, Clone)]
pub struct S3BlobStoreConfig {
    /// S3 bucket name.
    pub bucket: String,
    /// AWS region (e.g., "us-east-1").
    pub region: String,
    /// Custom endpoint URL (for MinIO, R2, etc.). If None, uses AWS default.
    pub endpoint: Option<String>,
    /// Access key ID. If None, uses environment/IAM credentials.
    pub access_key_id: Option<String>,
    /// Secret access key. If None, uses environment/IAM credentials.
    pub secret_access_key: Option<String>,
    /// Use path-style addressing (required for some MinIO setups).
    pub path_style: bool,
}

impl S3BlobStoreConfig {
    /// Create config from environment variables.
    ///
    /// Reads:
    /// - `MNEMO_BLOB_S3_BUCKET` (required)
    /// - `MNEMO_BLOB_S3_REGION` (default: "us-east-1")
    /// - `MNEMO_BLOB_S3_ENDPOINT` (optional, for MinIO/R2)
    /// - `AWS_ACCESS_KEY_ID` (optional)
    /// - `AWS_SECRET_ACCESS_KEY` (optional)
    /// - `MNEMO_BLOB_S3_PATH_STYLE` (optional, "true" for path-style)
    pub fn from_env() -> Result<Self, MnemoError> {
        let bucket = std::env::var("MNEMO_BLOB_S3_BUCKET")
            .map_err(|_| MnemoError::Config("MNEMO_BLOB_S3_BUCKET is required".to_string()))?;

        Ok(Self {
            bucket,
            region: std::env::var("MNEMO_BLOB_S3_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
            endpoint: std::env::var("MNEMO_BLOB_S3_ENDPOINT").ok(),
            access_key_id: std::env::var("AWS_ACCESS_KEY_ID").ok(),
            secret_access_key: std::env::var("AWS_SECRET_ACCESS_KEY").ok(),
            path_style: std::env::var("MNEMO_BLOB_S3_PATH_STYLE")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
        })
    }
}

/// S3-compatible blob store.
///
/// Provides binary storage with pre-signed URLs for direct uploads/downloads.
/// Suitable for production deployments with high availability requirements.
#[derive(Clone)]
pub struct S3BlobStore {
    client: Client,
    bucket: String,
}

impl S3BlobStore {
    /// Create a new S3 blob store from configuration.
    pub async fn new(config: S3BlobStoreConfig) -> Result<Self, MnemoError> {
        let mut s3_config_builder = S3ConfigBuilder::new()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(config.region.clone()))
            .force_path_style(config.path_style);

        // Set custom endpoint if provided
        if let Some(endpoint) = &config.endpoint {
            s3_config_builder = s3_config_builder.endpoint_url(endpoint);
        }

        // Set credentials if provided explicitly
        if let (Some(access_key), Some(secret_key)) =
            (&config.access_key_id, &config.secret_access_key)
        {
            let credentials = Credentials::new(
                access_key,
                secret_key,
                None, // session token
                None, // expiry
                "mnemo-s3-blob-store",
            );
            s3_config_builder = s3_config_builder.credentials_provider(credentials);
        } else {
            // Use default credential chain (env vars, IAM role, etc.)
            let sdk_config = aws_config::defaults(BehaviorVersion::latest())
                .region(Region::new(config.region.clone()))
                .load()
                .await;
            s3_config_builder =
                s3_config_builder.credentials_provider(sdk_config.credentials_provider().unwrap().clone());
        }

        let s3_config = s3_config_builder.build();
        let client = Client::from_conf(s3_config);

        // Verify bucket access
        client
            .head_bucket()
            .bucket(&config.bucket)
            .send()
            .await
            .map_err(|e| {
                MnemoError::Storage(format!(
                    "Failed to access S3 bucket '{}': {}",
                    config.bucket, e
                ))
            })?;

        debug!("S3 blob store initialized for bucket: {}", config.bucket);

        Ok(Self {
            client,
            bucket: config.bucket,
        })
    }

    /// Create from environment variables.
    pub async fn from_env() -> Result<Self, MnemoError> {
        let config = S3BlobStoreConfig::from_env()?;
        Self::new(config).await
    }
}

impl BlobStore for S3BlobStore {
    #[instrument(skip(self, data), fields(key = %key, size = data.len()))]
    async fn put(&self, key: &str, data: Vec<u8>, content_type: &str) -> BlobResult<BlobMetadata> {
        let size_bytes = data.len() as u64;

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(data))
            .content_type(content_type)
            .send()
            .await
            .map_err(|e| MnemoError::Storage(format!("S3 put failed for '{}': {}", key, e)))?;

        debug!("Stored blob in S3: {}, {} bytes", key, size_bytes);

        Ok(BlobMetadata {
            key: key.to_string(),
            size_bytes,
            content_type: content_type.to_string(),
            etag: None,
        })
    }

    #[instrument(skip(self, stream), fields(key = %key, size = content_length))]
    async fn put_stream(
        &self,
        key: &str,
        stream: Box<
            dyn futures::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send + Unpin,
        >,
        content_type: &str,
        content_length: u64,
    ) -> BlobResult<BlobMetadata> {
        use futures::StreamExt;

        // Collect stream into bytes (S3 SDK requires known length for single PUT)
        // For very large files, multipart upload would be better
        let mut data = Vec::with_capacity(content_length as usize);
        let mut stream = stream;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| {
                MnemoError::Storage(format!("Failed to read stream chunk: {}", e))
            })?;
            data.extend_from_slice(&chunk);
        }

        self.put(key, data, content_type).await
    }

    #[instrument(skip(self), fields(key = %key))]
    async fn get(&self, key: &str) -> BlobResult<(Vec<u8>, BlobMetadata)> {
        let response = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                let err_str = e.to_string();
                if err_str.contains("NoSuchKey") || err_str.contains("NotFound") {
                    MnemoError::NotFound {
                        resource_type: "Blob".to_string(),
                        id: key.to_string(),
                    }
                } else {
                    MnemoError::Storage(format!("S3 get failed for '{}': {}", key, e))
                }
            })?;

        let content_type = response
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();
        let content_length = response.content_length().unwrap_or(0) as u64;
        let etag = response.e_tag().map(|s| s.to_string());

        let data = response
            .body
            .collect()
            .await
            .map_err(|e| MnemoError::Storage(format!("Failed to read S3 body: {}", e)))?
            .into_bytes()
            .to_vec();

        Ok((
            data,
            BlobMetadata {
                key: key.to_string(),
                size_bytes: content_length,
                content_type,
                etag,
            },
        ))
    }

    #[instrument(skip(self), fields(key = %key))]
    async fn delete(&self, key: &str) -> BlobResult<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| MnemoError::Storage(format!("S3 delete failed for '{}': {}", key, e)))?;

        debug!("Deleted blob from S3: {}", key);
        Ok(())
    }

    #[instrument(skip(self), fields(key = %key))]
    async fn exists(&self, key: &str) -> BlobResult<bool> {
        match self.client.head_object().bucket(&self.bucket).key(key).send().await {
            Ok(_) => Ok(true),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("NotFound") || err_str.contains("NoSuchKey") {
                    Ok(false)
                } else {
                    Err(MnemoError::Storage(format!(
                        "S3 head failed for '{}': {}",
                        key, e
                    )))
                }
            }
        }
    }

    #[instrument(skip(self), fields(key = %key))]
    async fn head(&self, key: &str) -> BlobResult<BlobMetadata> {
        let response = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                let err_str = e.to_string();
                if err_str.contains("NotFound") || err_str.contains("NoSuchKey") {
                    MnemoError::NotFound {
                        resource_type: "Blob".to_string(),
                        id: key.to_string(),
                    }
                } else {
                    MnemoError::Storage(format!("S3 head failed for '{}': {}", key, e))
                }
            })?;

        Ok(BlobMetadata {
            key: key.to_string(),
            size_bytes: response.content_length().unwrap_or(0) as u64,
            content_type: response
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_string(),
            etag: response.e_tag().map(|s| s.to_string()),
        })
    }

    #[instrument(skip(self), fields(key = %key))]
    async fn presign_get(&self, key: &str, options: PresignOptions) -> BlobResult<Option<String>> {
        let presigning_config = PresigningConfig::builder()
            .expires_in(options.expires_in)
            .build()
            .map_err(|e| MnemoError::Storage(format!("Failed to build presign config: {}", e)))?;

        let mut request = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key);

        if let Some(disposition) = options.content_disposition {
            request = request.response_content_disposition(disposition);
        }

        let presigned = request
            .presigned(presigning_config)
            .await
            .map_err(|e| MnemoError::Storage(format!("Failed to presign GET: {}", e)))?;

        Ok(Some(presigned.uri().to_string()))
    }

    #[instrument(skip(self), fields(key = %key))]
    async fn presign_put(
        &self,
        key: &str,
        content_type: &str,
        options: PresignOptions,
    ) -> BlobResult<Option<String>> {
        let presigning_config = PresigningConfig::builder()
            .expires_in(options.expires_in)
            .build()
            .map_err(|e| MnemoError::Storage(format!("Failed to build presign config: {}", e)))?;

        let presigned = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .presigned(presigning_config)
            .await
            .map_err(|e| MnemoError::Storage(format!("Failed to presign PUT: {}", e)))?;

        Ok(Some(presigned.uri().to_string()))
    }

    #[instrument(skip(self), fields(prefix = %prefix))]
    async fn list(&self, prefix: &str, max_keys: u32) -> BlobResult<Vec<BlobMetadata>> {
        let response = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(prefix)
            .max_keys(max_keys as i32)
            .send()
            .await
            .map_err(|e| MnemoError::Storage(format!("S3 list failed for prefix '{}': {}", prefix, e)))?;

        let mut results: Vec<BlobMetadata> = Vec::new();
        for object in response.contents() {
            let obj_key = match object.key() {
                Some(k) => k.to_string(),
                None => continue,
            };
            let obj_etag = object.e_tag().map(|e| e.to_string());
            results.push(BlobMetadata {
                key: obj_key,
                size_bytes: object.size().unwrap_or(0) as u64,
                content_type: "application/octet-stream".to_string(), // S3 list doesn't return content-type
                etag: obj_etag,
            });
        }

        Ok(results)
    }

    #[instrument(skip(self), fields(source = %source_key, dest = %dest_key))]
    async fn copy(&self, source_key: &str, dest_key: &str) -> BlobResult<BlobMetadata> {
        let copy_source = format!("{}/{}", self.bucket, source_key);

        self.client
            .copy_object()
            .bucket(&self.bucket)
            .copy_source(&copy_source)
            .key(dest_key)
            .send()
            .await
            .map_err(|e| {
                let err_str = e.to_string();
                if err_str.contains("NoSuchKey") || err_str.contains("NotFound") {
                    MnemoError::NotFound {
                        resource_type: "Blob".to_string(),
                        id: source_key.to_string(),
                    }
                } else {
                    MnemoError::Storage(format!("S3 copy failed: {}", e))
                }
            })?;

        // Get metadata of the copied object
        self.head(dest_key).await
    }

    fn public_url(&self, _key: &str) -> Option<String> {
        // S3 public URLs require bucket policy configuration
        // Return None by default; use presign_get for access
        None
    }
}

impl std::fmt::Debug for S3BlobStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3BlobStore")
            .field("bucket", &self.bucket)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_env_missing_bucket() {
        // Clear env var if set
        std::env::remove_var("MNEMO_BLOB_S3_BUCKET");
        let result = S3BlobStoreConfig::from_env();
        assert!(result.is_err());
    }

    #[test]
    fn test_config_from_env_defaults() {
        std::env::set_var("MNEMO_BLOB_S3_BUCKET", "test-bucket");
        std::env::remove_var("MNEMO_BLOB_S3_REGION");
        std::env::remove_var("MNEMO_BLOB_S3_ENDPOINT");
        std::env::remove_var("MNEMO_BLOB_S3_PATH_STYLE");

        let config = S3BlobStoreConfig::from_env().unwrap();
        assert_eq!(config.bucket, "test-bucket");
        assert_eq!(config.region, "us-east-1");
        assert!(config.endpoint.is_none());
        assert!(!config.path_style);

        // Clean up
        std::env::remove_var("MNEMO_BLOB_S3_BUCKET");
    }

    #[test]
    fn test_config_path_style() {
        std::env::set_var("MNEMO_BLOB_S3_BUCKET", "test-bucket");
        std::env::set_var("MNEMO_BLOB_S3_PATH_STYLE", "true");

        let config = S3BlobStoreConfig::from_env().unwrap();
        assert!(config.path_style);

        // Clean up
        std::env::remove_var("MNEMO_BLOB_S3_BUCKET");
        std::env::remove_var("MNEMO_BLOB_S3_PATH_STYLE");
    }
}
