//! # mnemo-storage
//!
//! Redis + Qdrant + Blob storage implementations for Mnemo.
//!
//! - `RedisStateStore`: State storage + full-text search (RediSearch)
//! - `QdrantVectorStore`: Vector embedding storage and semantic search
//! - `LocalBlobStore`: Local filesystem blob storage for attachments
//! - `S3BlobStore`: S3-compatible blob storage (AWS S3, MinIO, R2)

pub mod local_blob_store;
pub mod qdrant_store;
pub mod redis_store;
pub mod redisearch;
pub mod s3_blob_store;

pub use local_blob_store::LocalBlobStore;
pub use qdrant_store::QdrantVectorStore;
pub use redis_store::RedisStateStore;
pub use s3_blob_store::{S3BlobStore, S3BlobStoreConfig};
