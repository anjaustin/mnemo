/// Local embedding provider backed by `fastembed-rs`.
///
/// Downloads ONNX model weights from HuggingFace on first use (cached in
/// `$FASTEMBED_CACHE_PATH` or `~/.cache/fastembed`).  No external API keys
/// or network calls after the initial model download.
///
/// Activated when `MNEMO_EMBEDDING_PROVIDER=local`.
/// Default model: `BGEBaseENV15` (768-dim, ~44 MB).
/// Override model with `MNEMO_EMBEDDING_MODEL` using the variant name
/// (e.g. `AllMiniLML6V2`, `BGESmallENV15`, `BGEBaseENV15`, `BGELargeENV15`).
#[cfg(feature = "local-embed")]
pub mod inner {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
    use mnemo_core::error::MnemoError;
    use mnemo_core::traits::llm::{EmbeddingProvider, LlmResult};

    /// Default fastembed model: BGE-Base-EN v1.5 (768-dim, ~44 MB).
    pub const DEFAULT_LOCAL_MODEL: &str = "BGEBaseENV15";
    /// Embedding dimensions for the default model.
    pub const DEFAULT_LOCAL_DIMENSIONS: u32 = 768;

    pub struct FastEmbedder {
        /// Inner `TextEmbedding` wrapped in Arc so it can be shared across
        /// async tasks.  It is `!Send` inside, so we keep it in an
        /// `Arc<tokio::sync::Mutex<…>>` and offload each call to
        /// `spawn_blocking`.
        model: Arc<std::sync::Mutex<TextEmbedding>>,
        dimensions: u32,
        model_name: String,
    }

    impl FastEmbedder {
        /// Build a `FastEmbedder` synchronously.  Call this inside
        /// `tokio::task::spawn_blocking` or before the async runtime starts.
        pub fn new(model_str: &str, dimensions: u32) -> LlmResult<Self> {
            configure_ort_dylib_path();
            let embedding_model = model_from_str(model_str)?;
            let te = TextEmbedding::try_new(
                InitOptions::new(embedding_model).with_show_download_progress(true),
            )
            .map_err(|e| MnemoError::EmbeddingProvider {
                provider: "local".into(),
                message: format!(
                    "Failed to initialise FastEmbed model '{}': {}",
                    model_str, e
                ),
            })?;

            Ok(Self {
                model: Arc::new(std::sync::Mutex::new(te)),
                dimensions,
                model_name: model_str.to_string(),
            })
        }
    }

    fn configure_ort_dylib_path() {
        if std::env::var_os("ORT_DYLIB_PATH").is_some() {
            return;
        }

        if let Some(path) = discover_ort_dylib_path() {
            std::env::set_var("ORT_DYLIB_PATH", path);
        }
    }

    fn discover_ort_dylib_path() -> Option<PathBuf> {
        let mut candidates = Vec::new();

        if let Some(paths) = std::env::var_os("LD_LIBRARY_PATH") {
            candidates.extend(std::env::split_paths(&paths));
        }

        candidates.extend([
            PathBuf::from("/usr/local/lib"),
            PathBuf::from("/usr/lib"),
            PathBuf::from("/usr/lib64"),
            PathBuf::from("/lib"),
            PathBuf::from("/lib64"),
        ]);

        let mut matches = candidates
            .into_iter()
            .filter(|dir| dir.is_dir())
            .flat_map(|dir| ort_candidates_in_dir(&dir))
            .collect::<Vec<_>>();

        matches.sort_by_key(|b| std::cmp::Reverse(ort_candidate_rank(b)));
        matches.into_iter().next()
    }

    fn ort_candidates_in_dir(dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };

        entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("libonnxruntime.so"))
                    && path.is_file()
            })
            .collect()
    }

    fn ort_candidate_rank(path: &Path) -> (bool, Vec<u32>, usize) {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let version = name
            .strip_prefix("libonnxruntime.so")
            .unwrap_or_default()
            .strip_prefix('.')
            .map(|suffix| {
                suffix
                    .split('.')
                    .filter_map(|part| part.parse::<u32>().ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let meets_minimum = version.as_slice() >= [1, 23].as_slice();
        (meets_minimum, version, name.len())
    }

    impl EmbeddingProvider for FastEmbedder {
        async fn embed(&self, text: &str) -> LlmResult<Vec<f32>> {
            let batch = self.embed_batch(&[text.to_string()]).await?;
            batch
                .into_iter()
                .next()
                .ok_or_else(|| MnemoError::EmbeddingProvider {
                    provider: "local".into(),
                    message: "Empty embedding batch result".into(),
                })
        }

        async fn embed_batch(&self, texts: &[String]) -> LlmResult<Vec<Vec<f32>>> {
            let model = self.model.clone();
            let owned: Vec<String> = texts.to_vec();
            let model_name = self.model_name.clone();

            tokio::task::spawn_blocking(move || {
                let mut guard = model.lock().map_err(|e| MnemoError::EmbeddingProvider {
                    provider: "local".into(),
                    message: format!("Model lock poisoned: {}", e),
                })?;
                let docs: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
                guard
                    .embed(docs, None)
                    .map_err(|e| MnemoError::EmbeddingProvider {
                        provider: "local".into(),
                        message: format!("Embedding failed with model '{}': {}", model_name, e),
                    })
            })
            .await
            .map_err(|e| MnemoError::EmbeddingProvider {
                provider: "local".into(),
                message: format!("spawn_blocking join error: {}", e),
            })?
        }

        fn dimensions(&self) -> u32 {
            self.dimensions
        }

        fn provider_name(&self) -> &str {
            "local"
        }
    }

    /// Map a model string to a `fastembed::EmbeddingModel` variant.
    fn model_from_str(s: &str) -> LlmResult<EmbeddingModel> {
        match s {
            "AllMiniLML6V2" => Ok(EmbeddingModel::AllMiniLML6V2),
            "AllMiniLML6V2Q" => Ok(EmbeddingModel::AllMiniLML6V2Q),
            "BGESmallENV15" => Ok(EmbeddingModel::BGESmallENV15),
            "BGESmallENV15Q" => Ok(EmbeddingModel::BGESmallENV15Q),
            "BGEBaseENV15" => Ok(EmbeddingModel::BGEBaseENV15),
            "BGEBaseENV15Q" => Ok(EmbeddingModel::BGEBaseENV15Q),
            "BGELargeENV15" => Ok(EmbeddingModel::BGELargeENV15),
            "BGELargeENV15Q" => Ok(EmbeddingModel::BGELargeENV15Q),
            "BGEM3" => Ok(EmbeddingModel::BGEM3),
            _ => Err(MnemoError::EmbeddingProvider {
                provider: "local".into(),
                message: format!(
                    "Unknown local embedding model '{}'. \
                     Supported: AllMiniLML6V2, AllMiniLML6V2Q, BGESmallENV15, BGESmallENV15Q, \
                     BGEBaseENV15, BGEBaseENV15Q, BGELargeENV15, BGELargeENV15Q, BGEM3",
                    s
                ),
            }),
        }
    }
}

#[cfg(feature = "local-embed")]
pub use inner::FastEmbedder;
#[cfg(feature = "local-embed")]
pub use inner::{DEFAULT_LOCAL_DIMENSIONS, DEFAULT_LOCAL_MODEL};
