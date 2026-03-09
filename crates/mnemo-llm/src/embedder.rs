/// `EmbedderKind` — a concrete enum that wraps every supported embedding
/// backend.  Using an enum avoids boxing (`Box<dyn Trait>`) and the
/// associated object-safety complications while still giving the codebase a
/// single concrete type to monomorphize against.
///
/// Add a new variant here when adding a new embedding backend.
use mnemo_core::traits::llm::{EmbeddingProvider, LlmResult};

use crate::OpenAiCompatibleEmbedder;

#[cfg(feature = "local-embed")]
use crate::FastEmbedder;

pub enum EmbedderKind {
    /// OpenAI-compatible HTTP embedder (OpenAI, Ollama, etc.)
    OpenAiCompat(OpenAiCompatibleEmbedder),
    /// Local fastembed-rs embedder — no external API required.
    #[cfg(feature = "local-embed")]
    Local(FastEmbedder),
}

impl EmbeddingProvider for EmbedderKind {
    async fn embed(&self, text: &str) -> LlmResult<Vec<f32>> {
        match self {
            EmbedderKind::OpenAiCompat(e) => e.embed(text).await,
            #[cfg(feature = "local-embed")]
            EmbedderKind::Local(e) => e.embed(text).await,
        }
    }

    async fn embed_batch(&self, texts: &[String]) -> LlmResult<Vec<Vec<f32>>> {
        match self {
            EmbedderKind::OpenAiCompat(e) => e.embed_batch(texts).await,
            #[cfg(feature = "local-embed")]
            EmbedderKind::Local(e) => e.embed_batch(texts).await,
        }
    }

    fn dimensions(&self) -> u32 {
        match self {
            EmbedderKind::OpenAiCompat(e) => e.dimensions(),
            #[cfg(feature = "local-embed")]
            EmbedderKind::Local(e) => e.dimensions(),
        }
    }

    fn provider_name(&self) -> &str {
        match self {
            EmbedderKind::OpenAiCompat(e) => e.provider_name(),
            #[cfg(feature = "local-embed")]
            EmbedderKind::Local(e) => e.provider_name(),
        }
    }
}
