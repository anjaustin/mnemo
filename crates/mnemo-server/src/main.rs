use std::sync::Arc;

use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use mnemo_server::config::MnemoConfig;
use mnemo_server::middleware::{AuthConfig, AuthLayer};
use mnemo_server::routes::build_router;
use mnemo_server::state::{AppState, MetadataPrefilterConfig};

use mnemo_graph::GraphEngine;
use mnemo_ingest::{IngestConfig, IngestWorker};
use mnemo_llm::{AnthropicProvider, OpenAiCompatibleEmbedder, OpenAiCompatibleProvider};
use mnemo_retrieval::RetrievalEngine;
use mnemo_storage::{QdrantVectorStore, RedisStateStore};

use mnemo_core::traits::fulltext::FullTextStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config_path = std::env::var("MNEMO_CONFIG").ok();
    let config = MnemoConfig::load(config_path.as_deref())?;

    // Logging
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.observability.log_level));
    if config.observability.log_format == "json" {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    }

    tracing::info!("Starting Mnemo v{}", env!("CARGO_PKG_VERSION"));

    // Storage
    tracing::info!(url = %config.redis.url, "Connecting to Redis");
    let state_store =
        Arc::new(RedisStateStore::new(&config.redis.url, &config.redis.prefix).await?);
    tracing::info!(url = %config.qdrant.url, "Connecting to Qdrant");
    let vector_store = Arc::new(
        QdrantVectorStore::new(
            &config.qdrant.url,
            &config.qdrant.collection_prefix,
            config.embedding.dimensions,
        )
        .await?,
    );

    // Embedder
    let embedder = Arc::new(OpenAiCompatibleEmbedder::new(config.embedding_config()));

    // Ensure RediSearch indexes exist
    tracing::info!("Ensuring RediSearch indexes");
    state_store.ensure_indexes().await?;

    // Engines (don't need LLM, only embedder)
    let retrieval = Arc::new(RetrievalEngine::new(
        state_store.clone(),
        vector_store.clone(),
        embedder.clone(),
    ));
    let graph = Arc::new(GraphEngine::new(state_store.clone()));

    // Ingest config
    let ingest_config = IngestConfig {
        poll_interval_ms: config.extraction.poll_interval_ms,
        batch_size: config.extraction.batch_size,
        concurrency: config.extraction.concurrency,
        max_retries: config.extraction.max_retries,
    };

    // Spawn ingestion worker with provider-specific LLM type
    // (generics require concrete types, so we branch here)
    match config.llm.provider.as_str() {
        "anthropic" => {
            tracing::info!(model = %config.llm.model, "Using Anthropic provider");
            let llm = Arc::new(AnthropicProvider::new(config.llm_config()));
            let worker = IngestWorker::new(
                state_store.clone(),
                vector_store.clone(),
                llm,
                embedder.clone(),
                ingest_config,
            );
            tokio::spawn(async move { worker.run().await });
        }
        _ => {
            tracing::info!(provider = %config.llm.provider, model = %config.llm.model, "Using OpenAI-compatible provider");
            let llm = Arc::new(OpenAiCompatibleProvider::new(config.llm_config()));
            let worker = IngestWorker::new(
                state_store.clone(),
                vector_store.clone(),
                llm,
                embedder.clone(),
                ingest_config,
            );
            tokio::spawn(async move { worker.run().await });
        }
    }
    tracing::info!("Ingestion worker started");

    // Auth
    let auth_config = if config.auth.enabled {
        tracing::info!(keys = config.auth.api_keys.len(), "API key auth enabled");
        AuthConfig::with_keys(config.auth.api_keys.clone())
    } else {
        tracing::warn!("API key auth DISABLED");
        AuthConfig::disabled()
    };

    // HTTP server
    let app_state = AppState {
        state_store,
        vector_store,
        retrieval,
        graph,
        metadata_prefilter: MetadataPrefilterConfig {
            enabled: config.retrieval.metadata_prefilter_enabled,
            scan_limit: config.retrieval.metadata_scan_limit,
            relax_if_empty: config.retrieval.metadata_relax_if_empty,
        },
        import_jobs: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
    };
    let app = build_router(app_state)
        .layer(AuthLayer::new(auth_config))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());

    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!(addr = %addr, "Mnemo server listening");

    println!(
        r#"
  __  __
 |  \/  |_ __   ___ _ __ ___   ___
 | |\/| | '_ \ / _ \ '_ ` _ \ / _ \
 | |  | | | | |  __/ | | | | | (_) |
 |_|  |_|_| |_|\___|_| |_| |_|\___/

 v{} | {}
"#,
        env!("CARGO_PKG_VERSION"),
        addr
    );

    axum::serve(listener, app).await?;
    Ok(())
}
