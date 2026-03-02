//! # mnemo-llm
//!
//! LLM and embedding provider implementations for Mnemo.
//!
//! - `OpenAiCompatibleProvider` — works with OpenAI, Ollama, Liquid AI, vLLM, etc.
//! - `AnthropicProvider` — native Anthropic Messages API
//! - `OpenAiCompatibleEmbedder` — embedding generation via OpenAI-compatible API

pub mod openai_compat;
pub mod anthropic;

pub use openai_compat::{OpenAiCompatibleProvider, OpenAiCompatibleEmbedder};
pub use anthropic::AnthropicProvider;
