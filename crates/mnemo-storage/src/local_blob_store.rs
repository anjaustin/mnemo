//! Local filesystem blob storage implementation.
//!
//! Stores blobs as files on the local filesystem. Suitable for development,
//! single-node deployments, or edge deployments with local storage.

use std::path::{Path, PathBuf};

use mnemo_core::error::MnemoError;
use mnemo_core::traits::blob::{BlobMetadata, BlobResult, BlobStore, PresignOptions};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, instrument};

/// Local filesystem blob store.
///
/// Stores blobs as files in a directory structure. Not suitable for
/// distributed deployments; use S3BlobStore for multi-node setups.
#[derive(Debug, Clone)]
pub struct LocalBlobStore {
    /// Base path for blob storage.
    base_path: PathBuf,
}

impl LocalBlobStore {
    /// Create a new local blob store.
    ///
    /// # Arguments
    /// * `base_path` - Base directory for storing blobs.
    ///
    /// # Returns
    /// A new `LocalBlobStore` instance.
    ///
    /// Note: The directory is created lazily on first write.
    pub fn new<P: AsRef<Path>>(base_path: P) -> Self {
        Self {
            base_path: base_path.as_ref().to_path_buf(),
        }
    }

    /// Get the full filesystem path for a blob key.
    ///
    /// SECURITY: Validates that the resolved path stays within base_path
    /// to prevent path traversal attacks via ".." sequences.
    fn full_path(&self, key: &str) -> BlobResult<PathBuf> {
        // First, reject keys with obvious path traversal patterns
        if key.contains("..") || key.starts_with('/') || key.starts_with('\\') {
            return Err(MnemoError::Validation(
                "Invalid storage key: path traversal detected".to_string(),
            ));
        }

        let path = self.base_path.join(key);

        // For existing files, canonicalize and verify containment
        if path.exists() {
            let canonical = path.canonicalize().map_err(|e| {
                MnemoError::Storage(format!("Failed to canonicalize path: {}", e))
            })?;
            let base_canonical = self.base_path.canonicalize().map_err(|e| {
                MnemoError::Storage(format!("Failed to canonicalize base path: {}", e))
            })?;

            if !canonical.starts_with(&base_canonical) {
                return Err(MnemoError::Validation(
                    "Invalid storage key: path traversal detected".to_string(),
                ));
            }
        }

        Ok(path)
    }

    /// Ensure the parent directory exists for a given path.
    async fn ensure_parent_dir(&self, path: &Path) -> BlobResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                MnemoError::Storage(format!("Failed to create directory {:?}: {}", parent, e))
            })?;
        }
        Ok(())
    }

    /// Read content type from sidecar metadata file.
    async fn read_content_type(&self, key: &str) -> BlobResult<String> {
        let meta_path = self.full_path(&format!("{}.meta", key))?;
        match fs::read_to_string(&meta_path).await {
            Ok(content_type) => Ok(content_type),
            Err(_) => Ok("application/octet-stream".to_string()),
        }
    }

    /// Write content type to sidecar metadata file.
    async fn write_content_type(&self, key: &str, content_type: &str) -> BlobResult<()> {
        let meta_path = self.full_path(&format!("{}.meta", key))?;
        self.ensure_parent_dir(&meta_path).await?;
        fs::write(&meta_path, content_type).await.map_err(|e| {
            MnemoError::Storage(format!(
                "Failed to write content type for {:?}: {}",
                meta_path, e
            ))
        })
    }
}

impl BlobStore for LocalBlobStore {
    #[instrument(skip(self, data), fields(key = %key, size = data.len()))]
    async fn put(&self, key: &str, data: Vec<u8>, content_type: &str) -> BlobResult<BlobMetadata> {
        let path = self.full_path(key)?;
        self.ensure_parent_dir(&path).await?;

        let size_bytes = data.len() as u64;

        fs::write(&path, &data).await.map_err(|e| {
            MnemoError::Storage(format!("Failed to write blob {:?}: {}", path, e))
        })?;

        // Store content type in sidecar file
        self.write_content_type(key, content_type).await?;

        debug!("Stored blob at {:?}, {} bytes", path, size_bytes);

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
        mut stream: Box<
            dyn futures::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send + Unpin,
        >,
        content_type: &str,
        content_length: u64,
    ) -> BlobResult<BlobMetadata> {
        use futures::StreamExt;

        let path = self.full_path(key)?;
        self.ensure_parent_dir(&path).await?;

        let mut file = fs::File::create(&path).await.map_err(|e| {
            MnemoError::Storage(format!("Failed to create file {:?}: {}", path, e))
        })?;

        let mut total_bytes = 0u64;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| {
                MnemoError::Storage(format!("Failed to read stream chunk: {}", e))
            })?;
            total_bytes += chunk.len() as u64;
            file.write_all(&chunk).await.map_err(|e| {
                MnemoError::Storage(format!("Failed to write chunk to {:?}: {}", path, e))
            })?;
        }

        file.flush().await.map_err(|e| {
            MnemoError::Storage(format!("Failed to flush file {:?}: {}", path, e))
        })?;

        // Store content type in sidecar file
        self.write_content_type(key, content_type).await?;

        debug!(
            "Stored blob stream at {:?}, {} bytes (expected {})",
            path, total_bytes, content_length
        );

        Ok(BlobMetadata {
            key: key.to_string(),
            size_bytes: total_bytes,
            content_type: content_type.to_string(),
            etag: None,
        })
    }

    #[instrument(skip(self), fields(key = %key))]
    async fn get(&self, key: &str) -> BlobResult<(Vec<u8>, BlobMetadata)> {
        let path = self.full_path(key)?;

        let data = fs::read(&path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                MnemoError::NotFound {
                    resource_type: "Blob".to_string(),
                    id: key.to_string(),
                }
            } else {
                MnemoError::Storage(format!("Failed to read blob {:?}: {}", path, e))
            }
        })?;

        let content_type = self.read_content_type(key).await?;

        let metadata = BlobMetadata {
            key: key.to_string(),
            size_bytes: data.len() as u64,
            content_type,
            etag: None,
        };

        Ok((data, metadata))
    }

    #[instrument(skip(self), fields(key = %key))]
    async fn delete(&self, key: &str) -> BlobResult<()> {
        let path = self.full_path(key)?;
        let meta_path = self.full_path(&format!("{}.meta", key))?;

        // Delete the main blob file
        fs::remove_file(&path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                MnemoError::NotFound {
                    resource_type: "Blob".to_string(),
                    id: key.to_string(),
                }
            } else {
                MnemoError::Storage(format!("Failed to delete blob {:?}: {}", path, e))
            }
        })?;

        // Delete the metadata sidecar (best effort)
        let _ = fs::remove_file(&meta_path).await;

        debug!("Deleted blob {:?}", path);
        Ok(())
    }

    #[instrument(skip(self), fields(key = %key))]
    async fn exists(&self, key: &str) -> BlobResult<bool> {
        let path = self.full_path(key)?;
        // P3-3: Use async tokio fs instead of blocking exists()
        Ok(tokio::fs::try_exists(&path).await.unwrap_or(false))
    }

    #[instrument(skip(self), fields(key = %key))]
    async fn head(&self, key: &str) -> BlobResult<BlobMetadata> {
        let path = self.full_path(key)?;

        let metadata = fs::metadata(&path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                MnemoError::NotFound {
                    resource_type: "Blob".to_string(),
                    id: key.to_string(),
                }
            } else {
                MnemoError::Storage(format!("Failed to get metadata for {:?}: {}", path, e))
            }
        })?;

        let content_type = self.read_content_type(key).await?;

        Ok(BlobMetadata {
            key: key.to_string(),
            size_bytes: metadata.len(),
            content_type,
            etag: None,
        })
    }

    /// Local blob store does not support pre-signed URLs.
    /// Use a reverse proxy or serve files directly for local access.
    async fn presign_get(&self, _key: &str, _options: PresignOptions) -> BlobResult<Option<String>> {
        Ok(None)
    }

    /// Local blob store does not support pre-signed URLs.
    async fn presign_put(
        &self,
        _key: &str,
        _content_type: &str,
        _options: PresignOptions,
    ) -> BlobResult<Option<String>> {
        Ok(None)
    }

    #[instrument(skip(self), fields(prefix = %prefix))]
    async fn list(&self, prefix: &str, max_keys: u32) -> BlobResult<Vec<BlobMetadata>> {
        let base = self.full_path(prefix)?;

        // If the prefix doesn't exist as a directory, return empty
        if !tokio::fs::try_exists(&base).await.unwrap_or(false) {
            return Ok(Vec::new());
        }

        // P1-3: Get canonical base path for symlink containment checks
        let base_canonical = self.base_path.canonicalize().map_err(|e| {
            MnemoError::Storage(format!("Failed to canonicalize base path: {}", e))
        })?;

        let mut results = Vec::new();
        let mut stack = vec![base];

        while let Some(dir) = stack.pop() {
            if results.len() >= max_keys as usize {
                break;
            }

            let mut entries = match fs::read_dir(&dir).await {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            while let Ok(Some(entry)) = entries.next_entry().await {
                if results.len() >= max_keys as usize {
                    break;
                }

                let path = entry.path();

                // Skip metadata sidecar files
                if path.extension().map_or(false, |ext| ext == "meta") {
                    continue;
                }

                // P1-3: Skip symlinks to prevent traversal attacks
                if path.is_symlink() {
                    continue;
                }

                // P1-3: Verify path is still under base_path after resolving
                if let Ok(canonical) = path.canonicalize() {
                    if !canonical.starts_with(&base_canonical) {
                        continue; // Skip paths that escape the base directory
                    }
                }

                if path.is_dir() {
                    stack.push(path);
                } else if let Ok(metadata) = fs::metadata(&path).await {
                    // Convert path back to key
                    let key = path
                        .strip_prefix(&self.base_path)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .to_string();

                    let content_type = self.read_content_type(&key).await.unwrap_or_default();

                    results.push(BlobMetadata {
                        key,
                        size_bytes: metadata.len(),
                        content_type,
                        etag: None,
                    });
                }
            }
        }

        Ok(results)
    }

    #[instrument(skip(self), fields(source = %source_key, dest = %dest_key))]
    async fn copy(&self, source_key: &str, dest_key: &str) -> BlobResult<BlobMetadata> {
        let source_path = self.full_path(source_key)?;
        let dest_path = self.full_path(dest_key)?;

        self.ensure_parent_dir(&dest_path).await?;

        fs::copy(&source_path, &dest_path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                MnemoError::NotFound {
                    resource_type: "Blob".to_string(),
                    id: source_key.to_string(),
                }
            } else {
                MnemoError::Storage(format!(
                    "Failed to copy {:?} to {:?}: {}",
                    source_path, dest_path, e
                ))
            }
        })?;

        // Copy metadata sidecar
        let source_meta = self.full_path(&format!("{}.meta", source_key))?;
        let dest_meta = self.full_path(&format!("{}.meta", dest_key))?;
        let _ = fs::copy(&source_meta, &dest_meta).await;

        self.head(dest_key).await
    }

    /// Local blob store does not have public URLs.
    fn public_url(&self, _key: &str) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn create_test_store() -> (LocalBlobStore, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let store = LocalBlobStore::new(temp_dir.path());
        (store, temp_dir)
    }

    #[tokio::test]
    async fn test_put_and_get() {
        let (store, _temp) = create_test_store().await;

        let data = b"Hello, World!".to_vec();
        let metadata = store.put("test/hello.txt", data.clone(), "text/plain").await.unwrap();

        assert_eq!(metadata.key, "test/hello.txt");
        assert_eq!(metadata.size_bytes, 13);
        assert_eq!(metadata.content_type, "text/plain");

        let (retrieved, meta) = store.get("test/hello.txt").await.unwrap();
        assert_eq!(retrieved, data);
        assert_eq!(meta.content_type, "text/plain");
    }

    #[tokio::test]
    async fn test_get_not_found() {
        let (store, _temp) = create_test_store().await;

        let result = store.get("nonexistent.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_exists() {
        let (store, _temp) = create_test_store().await;

        assert!(!store.exists("test.txt").await.unwrap());

        store.put("test.txt", b"data".to_vec(), "text/plain").await.unwrap();

        assert!(store.exists("test.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_delete() {
        let (store, _temp) = create_test_store().await;

        store.put("test.txt", b"data".to_vec(), "text/plain").await.unwrap();
        assert!(store.exists("test.txt").await.unwrap());

        store.delete("test.txt").await.unwrap();
        assert!(!store.exists("test.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_head() {
        let (store, _temp) = create_test_store().await;

        store.put("test.txt", b"Hello".to_vec(), "text/plain").await.unwrap();

        let metadata = store.head("test.txt").await.unwrap();
        assert_eq!(metadata.size_bytes, 5);
        assert_eq!(metadata.content_type, "text/plain");
    }

    #[tokio::test]
    async fn test_copy() {
        let (store, _temp) = create_test_store().await;

        store.put("source.txt", b"data".to_vec(), "text/plain").await.unwrap();

        let metadata = store.copy("source.txt", "dest.txt").await.unwrap();
        assert_eq!(metadata.key, "dest.txt");
        assert_eq!(metadata.size_bytes, 4);

        let (data, _) = store.get("dest.txt").await.unwrap();
        assert_eq!(data, b"data");
    }

    #[tokio::test]
    async fn test_list() {
        let (store, _temp) = create_test_store().await;

        store.put("prefix/a.txt", b"a".to_vec(), "text/plain").await.unwrap();
        store.put("prefix/b.txt", b"b".to_vec(), "text/plain").await.unwrap();
        store.put("other/c.txt", b"c".to_vec(), "text/plain").await.unwrap();

        let results = store.list("prefix", 100).await.unwrap();
        assert_eq!(results.len(), 2);

        let keys: Vec<_> = results.iter().map(|m| m.key.as_str()).collect();
        assert!(keys.contains(&"prefix/a.txt"));
        assert!(keys.contains(&"prefix/b.txt"));
    }

    #[tokio::test]
    async fn test_nested_directories() {
        let (store, _temp) = create_test_store().await;

        let data = b"nested data".to_vec();
        store.put("a/b/c/d/file.txt", data.clone(), "text/plain").await.unwrap();

        let (retrieved, _) = store.get("a/b/c/d/file.txt").await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn test_presign_not_supported() {
        let (store, _temp) = create_test_store().await;

        let url = store.presign_get("test.txt", PresignOptions::default()).await.unwrap();
        assert!(url.is_none());

        let url = store.presign_put("test.txt", "text/plain", PresignOptions::default()).await.unwrap();
        assert!(url.is_none());
    }
}
