//! # mnemo-llm
//!
//! LLM, embedding, vision, transcription, and document parsing implementations for Mnemo.
//!
//! ## LLM Providers
//! - `OpenAiCompatibleProvider` — works with OpenAI, Ollama, Liquid AI, vLLM, etc.
//! - `AnthropicProvider` — native Anthropic Messages API
//!
//! ## Embedding Providers
//! - `OpenAiCompatibleEmbedder` — embedding generation via OpenAI-compatible API
//! - `FastEmbedder` — local embedding via fastembed-rs (no external API required)
//!   Enabled by the `local-embed` feature (on by default).
//! - `EmbedderKind` — enum wrapper for unified concrete type across backends.
//!
//! ## Vision Providers
//! - `AnthropicVisionProvider` — Claude Vision (claude-sonnet-4-20250514, claude-3-haiku)
//! - `OpenAIVisionProvider` — GPT-4V (gpt-4o, gpt-4-turbo)
//!
//! ## Transcription Providers
//! - `OpenAITranscriptionProvider` — OpenAI Whisper (whisper-1)
//!
//! ## Document Parsers
//! - `PdfParser` — PDF document parsing with structural chunking
//! - `TextDocumentParser` — Plain text, Markdown, HTML parsing

pub mod anthropic;
pub mod anthropic_vision;
pub mod embedder;
#[cfg(feature = "local-embed")]
pub mod local_embed;
pub mod openai_compat;
pub mod openai_transcription;
pub mod openai_vision;
pub mod pdf_parser;

pub use anthropic::AnthropicProvider;
pub use anthropic_vision::AnthropicVisionProvider;
pub use embedder::EmbedderKind;
#[cfg(feature = "local-embed")]
pub use local_embed::{FastEmbedder, DEFAULT_LOCAL_DIMENSIONS, DEFAULT_LOCAL_MODEL};
pub use openai_compat::{OpenAiCompatibleEmbedder, OpenAiCompatibleProvider};
pub use openai_transcription::OpenAITranscriptionProvider;
pub use openai_vision::OpenAIVisionProvider;
pub use pdf_parser::{PdfParser, TextDocumentParser};
