//! Blob storage trait for multi-modal attachments.
//!
//! The [`BlobStore`] trait defines the interface for storing and retrieving
//! binary files (images, audio, video, documents). Implementations exist for
//! local filesystem storage and S3-compatible object stores.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::MnemoError;

/// Result type for blob storage operations.
pub type BlobResult<T> = Result<T, MnemoError>;

/// Configuration for blob storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BlobStorageConfig {
    /// Local filesystem storage.
    Local {
        /// Base path for blob storage.
        path: String,
    },
    /// S3-compatible object storage (S3, MinIO, R2, etc.).
    S3 {
        /// Bucket name.
        bucket: String,
        /// AWS region (optional for non-AWS endpoints).
        #[serde(skip_serializing_if = "Option::is_none")]
        region: Option<String>,
        /// Custom endpoint URL (for MinIO, R2, etc.).
        #[serde(skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
        /// Access key ID (can also come from environment).
        #[serde(skip_serializing_if = "Option::is_none")]
        access_key_id: Option<String>,
        /// Secret access key (can also come from environment).
        #[serde(skip_serializing_if = "Option::is_none")]
        secret_access_key: Option<String>,
        /// Use path-style addressing (required for some MinIO setups).
        #[serde(default)]
        path_style: bool,
    },
}

impl Default for BlobStorageConfig {
    fn default() -> Self {
        Self::Local {
            path: "/var/mnemo/blobs".to_string(),
        }
    }
}

/// Metadata about a stored blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobMetadata {
    /// Storage key (path within the blob store).
    pub key: String,
    /// Size in bytes.
    pub size_bytes: u64,
    /// Content type (MIME type).
    pub content_type: String,
    /// ETag or checksum (if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
}

/// Options for generating pre-signed URLs.
#[derive(Debug, Clone)]
pub struct PresignOptions {
    /// URL expiration time.
    pub expires_in: Duration,
    /// Content-Disposition header for downloads.
    pub content_disposition: Option<String>,
}

impl Default for PresignOptions {
    fn default() -> Self {
        Self {
            expires_in: Duration::from_secs(15 * 60), // 15 minutes
            content_disposition: None,
        }
    }
}

/// Trait for binary blob storage operations.
///
/// Implementations must be thread-safe (`Send + Sync`) for use in async contexts.
#[allow(async_fn_in_trait)]
pub trait BlobStore: Send + Sync {
    /// Store a blob from bytes.
    ///
    /// # Arguments
    /// * `key` - Storage key (path within the blob store)
    /// * `data` - Binary content
    /// * `content_type` - MIME type of the content
    ///
    /// # Returns
    /// Metadata about the stored blob.
    async fn put(&self, key: &str, data: Vec<u8>, content_type: &str) -> BlobResult<BlobMetadata>;

    /// Store a blob from a stream.
    ///
    /// For large files, this is more memory-efficient than loading into a Vec.
    /// The default implementation buffers the stream into memory.
    ///
    /// # Arguments
    /// * `key` - Storage key
    /// * `stream` - Async byte stream
    /// * `content_type` - MIME type
    /// * `content_length` - Total size in bytes (required for S3)
    async fn put_stream(
        &self,
        key: &str,
        stream: Box<
            dyn futures::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send + Unpin,
        >,
        content_type: &str,
        content_length: u64,
    ) -> BlobResult<BlobMetadata>;

    /// Retrieve a blob as bytes.
    ///
    /// # Arguments
    /// * `key` - Storage key
    ///
    /// # Returns
    /// The blob content and metadata, or `NotFound` if the key doesn't exist.
    async fn get(&self, key: &str) -> BlobResult<(Vec<u8>, BlobMetadata)>;

    /// Delete a blob.
    ///
    /// # Arguments
    /// * `key` - Storage key
    ///
    /// # Returns
    /// `Ok(())` if deleted, or `NotFound` if the key doesn't exist.
    async fn delete(&self, key: &str) -> BlobResult<()>;

    /// Check if a blob exists.
    ///
    /// # Arguments
    /// * `key` - Storage key
    ///
    /// # Returns
    /// `true` if the blob exists, `false` otherwise.
    async fn exists(&self, key: &str) -> BlobResult<bool>;

    /// Get blob metadata without downloading content.
    ///
    /// # Arguments
    /// * `key` - Storage key
    ///
    /// # Returns
    /// Metadata about the blob, or `NotFound` if it doesn't exist.
    async fn head(&self, key: &str) -> BlobResult<BlobMetadata>;

    /// Generate a pre-signed URL for downloading a blob.
    ///
    /// # Arguments
    /// * `key` - Storage key
    /// * `options` - URL generation options (expiration, content-disposition)
    ///
    /// # Returns
    /// A URL that can be used to download the blob without authentication.
    /// Returns `None` if the blob store doesn't support pre-signed URLs.
    async fn presign_get(&self, key: &str, options: PresignOptions) -> BlobResult<Option<String>>;

    /// Generate a pre-signed URL for uploading a blob.
    ///
    /// # Arguments
    /// * `key` - Storage key where the blob will be stored
    /// * `content_type` - Expected MIME type
    /// * `options` - URL generation options
    ///
    /// # Returns
    /// A URL that can be used to upload a blob directly.
    /// Returns `None` if the blob store doesn't support pre-signed uploads.
    async fn presign_put(
        &self,
        key: &str,
        content_type: &str,
        options: PresignOptions,
    ) -> BlobResult<Option<String>>;

    /// List blobs with a given prefix.
    ///
    /// # Arguments
    /// * `prefix` - Key prefix to filter by
    /// * `max_keys` - Maximum number of keys to return
    ///
    /// # Returns
    /// List of blob metadata for matching keys.
    async fn list(&self, prefix: &str, max_keys: u32) -> BlobResult<Vec<BlobMetadata>>;

    /// Copy a blob to a new key.
    ///
    /// # Arguments
    /// * `source_key` - Source storage key
    /// * `dest_key` - Destination storage key
    ///
    /// # Returns
    /// Metadata about the copied blob.
    async fn copy(&self, source_key: &str, dest_key: &str) -> BlobResult<BlobMetadata>;

    /// Get the public URL for a blob (if the store supports public access).
    ///
    /// Returns `None` if the store doesn't support public URLs.
    fn public_url(&self, key: &str) -> Option<String>;
}

/// Generate a storage key for an attachment.
///
/// Format: `{user_id}/attachments/{attachment_id}/{filename}`
pub fn attachment_key(user_id: &uuid::Uuid, attachment_id: &uuid::Uuid, filename: &str) -> String {
    format!("{}/attachments/{}/{}", user_id, attachment_id, filename)
}

/// Generate a storage key for a thumbnail.
///
/// Format: `{user_id}/attachments/{attachment_id}/thumbnail.jpg`
pub fn thumbnail_key(user_id: &uuid::Uuid, attachment_id: &uuid::Uuid) -> String {
    format!("{}/attachments/{}/thumbnail.jpg", user_id, attachment_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_attachment_key() {
        let user_id = Uuid::nil();
        let attachment_id = Uuid::nil();
        let key = attachment_key(&user_id, &attachment_id, "original.png");
        assert_eq!(
            key,
            "00000000-0000-0000-0000-000000000000/attachments/00000000-0000-0000-0000-000000000000/original.png"
        );
    }

    #[test]
    fn test_thumbnail_key() {
        let user_id = Uuid::nil();
        let attachment_id = Uuid::nil();
        let key = thumbnail_key(&user_id, &attachment_id);
        assert_eq!(
            key,
            "00000000-0000-0000-0000-000000000000/attachments/00000000-0000-0000-0000-000000000000/thumbnail.jpg"
        );
    }

    #[test]
    fn test_blob_storage_config_default() {
        let config = BlobStorageConfig::default();
        match config {
            BlobStorageConfig::Local { path } => {
                assert_eq!(path, "/var/mnemo/blobs");
            }
            _ => panic!("Expected Local config"),
        }
    }

    #[test]
    fn test_presign_options_default() {
        let options = PresignOptions::default();
        assert_eq!(options.expires_in, Duration::from_secs(15 * 60));
        assert!(options.content_disposition.is_none());
    }
}
