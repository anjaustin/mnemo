//! # mnemo-core
//!
//! Core domain types and traits for **Mnemo** — a high-performance memory
//! and context engine for AI agents.
//!
//! This crate contains:
//! - **Domain models**: User, Session, Episode, Entity, Edge, ContextBlock
//! - **Storage traits**: Interfaces for persistence (implemented by mnemo-storage)
//! - **LLM traits**: Interfaces for LLM/embedding providers (implemented by mnemo-llm)
//! - **Error types**: Unified error handling across all Mnemo crates
//!
//! ## Architecture
//!
//! `mnemo-core` has **zero external service dependencies**. It defines the
//! contracts that other crates implement. This enables:
//! - Clean separation of concerns
//! - Easy testing with mock implementations
//! - Swappable storage and LLM backends
//!
//! ## Example
//!
//! ```rust
//! use mnemo_core::models::user::{User, CreateUserRequest};
//!
//! let user = User::from_request(CreateUserRequest {
//!     id: None,
//!     external_id: Some("ext_123".to_string()),
//!     name: "Kendra".to_string(),
//!     email: Some("kendra@example.com".to_string()),
//!     metadata: serde_json::json!({}),
//! });
//!
//! assert_eq!(user.name, "Kendra");
//! ```

pub mod error;
pub mod models;
pub mod traits;

// Convenience re-exports
pub use error::MnemoError;
