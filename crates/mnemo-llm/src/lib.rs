//! # mnemo-llm
//!
//! LLM and embedding provider implementations for Mnemo.
//!
//! - `OpenAiCompatibleProvider` — works with OpenAI, Ollama, Liquid AI, vLLM, etc.
//! - `AnthropicProvider` — native Anthropic Messages API
//! - `OpenAiCompatibleEmbedder` — embedding generation via OpenAI-compatible API
//! - `FastEmbedder` — local embedding via fastembed-rs (no external API required)
//!   Enabled by the `local-embed` feature (on by default).
//! - `EmbedderKind` — enum wrapper for unified concrete type across backends.

pub mod anthropic;
pub mod openai_compat;
#[cfg(feature = "local-embed")]
pub mod local_embed;
pub mod embedder;

pub use anthropic::AnthropicProvider;
pub use openai_compat::{OpenAiCompatibleEmbedder, OpenAiCompatibleProvider};
#[cfg(feature = "local-embed")]
pub use local_embed::{FastEmbedder, DEFAULT_LOCAL_DIMENSIONS, DEFAULT_LOCAL_MODEL};
pub use embedder::EmbedderKind;
