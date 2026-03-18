//! Mnemo server binary entry point.
//!
//! Loads configuration from environment variables, initializes Redis and
//! Qdrant connections, sets up the Axum router with auth middleware, and
//! starts the HTTP listener.
//!
//! # Environment Variables
//!
//! Core:
//! - `MNEMO_SERVER_HOST` / `MNEMO_SERVER_PORT` — REST bind address (default `0.0.0.0:8080`)
//! - `MNEMO_GRPC_PORT` — Optional dedicated gRPC port (e.g. `50051`). When set, gRPC
//!   binds to a separate port like Qdrant's `:6334`. When unset, gRPC is multiplexed
//!   on the same port as REST.
//! - `MNEMO_REDIS_URL` — Redis connection string
//! - `MNEMO_QDRANT_URL` — Qdrant gRPC endpoint
//!
//! Auth:
//! - `MNEMO_AUTH_ENABLED` — Enable API key authentication (default `false`)
//! - `MNEMO_AUTH_API_KEYS` — Comma-separated bootstrap API keys
//!
//! LLM:
//! - `MNEMO_LLM_PROVIDER` — `openai`, `anthropic`, or `ollama`
//! - `MNEMO_LLM_MODEL` — Model name for extraction/summarization
//! - `MNEMO_EMBEDDING_PROVIDER` — `openai`, `local`, or `ollama`
//! - `MNEMO_EMBEDDING_MODEL` — Embedding model name
//! - `MNEMO_EMBEDDING_DIMENSIONS` — Embedding vector dimensions
//!
//! OpenTelemetry:
//! - `MNEMO_OTEL_ENABLED` — Enable OTLP trace export (default `false`)
//! - `MNEMO_OTEL_ENDPOINT` — OTLP gRPC endpoint (e.g. `http://localhost:4317`)
//! - `MNEMO_OTEL_SERVICE_NAME` — Service name for traces (default `mnemo-server`)

use std::sync::Arc;

use axum::middleware::from_fn_with_state;
use tokio::net::TcpListener;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::trace::TraceLayer;

use mnemo_server::config::MnemoConfig;
use mnemo_server::config::RerankerConfig;
use mnemo_server::grpc::GrpcState;
use mnemo_server::lora_handle::LoraEmbedderHandle;
use mnemo_server::middleware::{request_context_middleware, AuthConfig, AuthLayer};
use mnemo_server::routes::{build_router, restore_webhook_state};
use mnemo_server::state::{
    AppState, LlmHandle, MetadataPrefilterConfig, RerankerMode, ServerMetrics,
    WebhookDeliveryConfig,
};

use mnemo_graph::GraphEngine;
use mnemo_ingest::{IngestConfig, IngestWorker};
use mnemo_llm::{
    AnthropicProvider, EmbedderKind, OpenAiCompatibleEmbedder, OpenAiCompatibleProvider,
};
#[cfg(feature = "local-embed")]
use mnemo_llm::{FastEmbedder, DEFAULT_LOCAL_DIMENSIONS, DEFAULT_LOCAL_MODEL};
use mnemo_lora::LoraAdaptedEmbedder;
use mnemo_retrieval::RetrievalEngine;
use mnemo_storage::{QdrantVectorStore, RedisStateStore};

use mnemo_core::traits::fulltext::FullTextStore;
use mnemo_core::traits::llm::EmbeddingProvider;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config_path = std::env::var("MNEMO_CONFIG").ok();
    let config = MnemoConfig::load(config_path.as_deref())?;

    // Logging + OpenTelemetry
    let _otel_provider = mnemo_server::telemetry::init_telemetry(&config.observability);

    tracing::info!("Starting Mnemo v{}", env!("CARGO_PKG_VERSION"));

    // Storage
    tracing::info!(url = %config.redis.url, "Connecting to Redis");
    let mut state_store = RedisStateStore::new(&config.redis.url, &config.redis.prefix).await?;

    // BYOK envelope encryption
    if config.encryption.enabled {
        if config.encryption.master_key.is_empty() {
            return Err(anyhow::anyhow!(
                "MNEMO_ENCRYPTION_ENABLED=true but MNEMO_ENCRYPTION_MASTER_KEY is not set"
            ));
        }
        let mut encryptor = mnemo_core::encryption::EnvelopeEncryptor::from_base64(
            &config.encryption.master_key,
            config.encryption.key_id.clone(),
        )?;

        // Load retired keys for key rotation support
        if !config.encryption.retired_keys.is_empty() {
            for entry in config.encryption.retired_keys.split(',') {
                let entry = entry.trim();
                if entry.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = entry.splitn(2, ':').collect();
                if parts.len() != 2 {
                    return Err(anyhow::anyhow!(
                        "Invalid retired key format: expected 'key_id:base64_key', got '{entry}'"
                    ));
                }
                encryptor.add_retired_key(parts[1], parts[0].to_string())?;
                tracing::info!(key_id = parts[0], "Loaded retired encryption key");
            }
        }

        let key_count = encryptor.known_key_ids().len();
        state_store = state_store.with_encryption(encryptor);
        tracing::info!(
            active_key_id = %config.encryption.key_id,
            total_keys = key_count,
            "BYOK envelope encryption enabled"
        );
    }

    let state_store = Arc::new(state_store);
    tracing::info!(url = %config.qdrant.url, "Connecting to Qdrant");
    let vector_store = Arc::new(
        QdrantVectorStore::new(
            &config.qdrant.url,
            &config.qdrant.collection_prefix,
            config.embedding.dimensions,
            config.qdrant.api_key.as_deref(),
        )
        .await?,
    );

    // Embedder — choose backend based on MNEMO_EMBEDDING_PROVIDER
    let embedder: Arc<EmbedderKind> = match config.embedding.provider.as_str() {
        #[cfg(feature = "local-embed")]
        "local" => {
            let model_str = if config.embedding.model.is_empty()
                || config.embedding.model == "text-embedding-3-small"
            {
                DEFAULT_LOCAL_MODEL.to_string()
            } else {
                config.embedding.model.clone()
            };
            let dims = if config.embedding.dimensions == 1536 {
                DEFAULT_LOCAL_DIMENSIONS
            } else {
                config.embedding.dimensions
            };
            tracing::info!(model = %model_str, dims = dims, "Using local fastembed provider");
            let fe = tokio::task::spawn_blocking(move || FastEmbedder::new(&model_str, dims))
                .await
                .map_err(|e| anyhow::anyhow!("spawn_blocking error: {}", e))??;
            Arc::new(EmbedderKind::Local(fe))
        }
        _ => {
            tracing::info!(
                provider = %config.embedding.provider,
                model = %config.embedding.model,
                "Using OpenAI-compatible embedding provider"
            );
            Arc::new(EmbedderKind::OpenAiCompat(OpenAiCompatibleEmbedder::new(
                config.embedding_config(),
            )))
        }
    };

    // Ensure RediSearch indexes exist
    tracing::info!("Ensuring RediSearch indexes");
    state_store.ensure_indexes().await?;

    // TinyLoRA: optionally wrap the base embedder with a per-agent adapter layer.
    // When enabled, all embed_for_agent() calls apply the learned LoRA residual.
    let (lora_embedder_opt, active_embedder): (
        Option<Arc<LoraAdaptedEmbedder<EmbedderKind, RedisStateStore>>>,
        Arc<LoraEmbedderHandle>,
    ) = if config.extraction.lora_enabled {
        tracing::info!("TinyLoRA enabled — wrapping embedder with LoraAdaptedEmbedder");
        let lora = Arc::new(LoraAdaptedEmbedder::new(
            embedder.clone(),
            state_store.clone(),
            true,
        ));
        let handle = Arc::new(LoraEmbedderHandle::Lora(lora.clone()));
        (Some(lora), handle)
    } else {
        tracing::debug!("TinyLoRA disabled (MNEMO_LORA_ENABLED=false)");
        let handle = Arc::new(LoraEmbedderHandle::Base(embedder.clone()));
        (None, handle)
    };

    // Engines (don't need LLM, only embedder)
    let retrieval = Arc::new(RetrievalEngine::new(
        state_store.clone(),
        vector_store.clone(),
        active_embedder.clone(),
    ));
    let graph = Arc::new(GraphEngine::new(state_store.clone()));

    // Ingest config
    let ingest_config = IngestConfig {
        poll_interval_ms: config.extraction.poll_interval_ms,
        batch_size: config.extraction.batch_size,
        concurrency: config.extraction.concurrency,
        max_retries: config.extraction.max_retries,
        session_summary_threshold: config.extraction.session_summary_threshold,
        sleep_enabled: config.extraction.sleep_enabled,
        sleep_idle_window_seconds: config.extraction.sleep_idle_window_seconds,
    };

    // Shared digest cache — passed to both AppState and IngestWorker.
    // Warm the cache from Redis so previously-generated digests survive restarts.
    let digest_cache: mnemo_ingest::DigestCache = {
        use mnemo_core::traits::storage::DigestStore as _;
        let persisted = state_store.list_digests().await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to load persisted digests from Redis");
            Vec::new()
        });
        let count = persisted.len();
        let mut map = std::collections::HashMap::with_capacity(count);
        for d in persisted {
            map.insert(d.user_id, d);
        }
        if count > 0 {
            tracing::info!(count, "Loaded persisted memory digests from Redis");
        }
        Arc::new(tokio::sync::RwLock::new(map))
    };

    // Create shared span ring buffer before spawning the worker so both
    // the server routes and the ingest worker record into the same VecDeque.
    let llm_spans = Arc::new(tokio::sync::RwLock::new(std::collections::VecDeque::new()));

    // Channel for proactive fact_added / fact_superseded webhook events.
    // The ingest worker sends events after creating or invalidating edges;
    // a receiver task (spawned after AppState is built) translates them
    // into webhook deliveries.
    let (webhook_tx, webhook_rx) =
        tokio::sync::mpsc::channel::<mnemo_core::models::webhook_event::IngestWebhookEvent>(256);

    // Spawn ingestion worker with provider-specific LLM type
    // (generics require concrete types, so we branch here).
    // We also keep an LlmHandle in AppState for on-demand extraction
    // (e.g. POST /api/v1/memory/extract).
    let llm_for_state: Option<LlmHandle> = match config.llm.provider.as_str() {
        "anthropic" => {
            tracing::info!(model = %config.llm.model, "Using Anthropic provider");
            let llm = Arc::new(AnthropicProvider::new(config.llm_config()));
            let handle = LlmHandle::Anthropic(llm.clone());
            let worker = IngestWorker::new(
                state_store.clone(),
                vector_store.clone(),
                llm,
                active_embedder.clone(),
                ingest_config,
            )
            .with_digest_cache(digest_cache.clone())
            .with_span_sink(llm_spans.clone())
            .with_webhook_sender(webhook_tx);
            tokio::spawn(async move { worker.run().await });
            Some(handle)
        }
        _ => {
            tracing::info!(provider = %config.llm.provider, model = %config.llm.model, "Using OpenAI-compatible provider");
            let llm = Arc::new(OpenAiCompatibleProvider::new(config.llm_config()));
            let handle = LlmHandle::OpenAiCompat(llm.clone());
            let worker = IngestWorker::new(
                state_store.clone(),
                vector_store.clone(),
                llm,
                active_embedder.clone(),
                ingest_config,
            )
            .with_digest_cache(digest_cache.clone())
            .with_span_sink(llm_spans.clone())
            .with_webhook_sender(webhook_tx);
            tokio::spawn(async move { worker.run().await });
            Some(handle)
        }
    };
    tracing::info!("Ingestion worker started");

    // Keep-warm task: fire a no-op embedding every 3 minutes so the embedding
    // model stays loaded in Ollama (or any provider with idle eviction).
    // Eliminates cold-start latency spikes on queries after idle periods.
    // Belt-and-suspenders alongside keep_alive:-1 in embed requests.
    {
        let warm_embedder = active_embedder.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(180));
            interval.tick().await; // skip immediate first tick
            loop {
                interval.tick().await;
                if let Err(e) = warm_embedder.embed("warmup").await {
                    tracing::debug!(error = %e, "keep-warm embed ping failed (non-fatal)");
                } else {
                    tracing::debug!("keep-warm embed ping ok");
                }
            }
        });
    }

    // Auth — shared between REST and gRPC via Arc
    let auth_config = Arc::new(if config.auth.enabled {
        tracing::info!(
            keys = config.auth.api_keys.len(),
            "API key auth enabled (scoped keys via Redis)"
        );
        AuthConfig::with_keys_and_store(config.auth.api_keys.clone(), state_store.clone())
    } else {
        tracing::warn!("API key auth DISABLED");
        AuthConfig::disabled()
    });

    // P3-3: Background task for auth cache cleanup (every 60 seconds)
    {
        let auth_for_cleanup = auth_config.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
            interval.tick().await; // skip immediate first tick
            loop {
                interval.tick().await;
                auth_for_cleanup.evict_stale_cache_entries().await;
            }
        });
    }

    // HTTP server
    let webhook_redis = if config.webhooks.persistence_enabled {
        match redis::Client::open(config.redis.url.as_str()) {
            Ok(client) => match redis::aio::ConnectionManager::new(client).await {
                Ok(conn) => Some(conn),
                Err(err) => {
                    tracing::warn!(error = %err, "webhook persistence disabled: redis connection failed");
                    None
                }
            },
            Err(err) => {
                tracing::warn!(error = %err, "webhook persistence disabled: redis client init failed");
                None
            }
        }
    } else {
        None
    };

    let app_state = AppState {
        state_store,
        vector_store,
        retrieval,
        lora_embedder: lora_embedder_opt,
        graph,
        llm: llm_for_state,
        metadata_prefilter: MetadataPrefilterConfig {
            enabled: config.retrieval.metadata_prefilter_enabled,
            scan_limit: config.retrieval.metadata_scan_limit,
            relax_if_empty: config.retrieval.metadata_relax_if_empty,
        },
        reranker: match config.retrieval.reranker {
            RerankerConfig::Mmr => RerankerMode::Mmr,
            RerankerConfig::Rrf => RerankerMode::Rrf,
        },
        import_jobs: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        import_idempotency: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        // P1-2: Limit concurrent import jobs to prevent DoS
        import_semaphore: Arc::new(tokio::sync::Semaphore::new(10)),
        memory_webhooks: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        memory_webhook_events: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        memory_webhook_audit: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        user_policies: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        governance_audit: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        webhook_runtime: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        webhook_delivery: WebhookDeliveryConfig {
            enabled: config.webhooks.enabled,
            max_attempts: config.webhooks.max_attempts,
            base_backoff_ms: config.webhooks.base_backoff_ms,
            request_timeout_ms: config.webhooks.request_timeout_ms,
            max_events_per_webhook: config.webhooks.max_events_per_webhook,
            rate_limit_per_minute: config.webhooks.rate_limit_per_minute,
            circuit_breaker_threshold: config.webhooks.circuit_breaker_threshold,
            circuit_breaker_cooldown_ms: config.webhooks.circuit_breaker_cooldown_ms,
            persistence_enabled: config.webhooks.persistence_enabled,
        },
        // P0-3 SSRF Protection: Disable redirects to prevent redirect-based SSRF bypasses
        webhook_http: Arc::new(
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("Failed to build webhook HTTP client"),
        ),
        webhook_redis,
        webhook_redis_prefix: format!(
            "{}:{}",
            config.redis.prefix, config.webhooks.persistence_prefix
        ),
        metrics: Arc::new(ServerMetrics::default()),
        llm_spans,
        memory_digests: digest_cache,
        require_tls: config.server.require_tls,
        audit_signing_secret: config.server.audit_signing_secret.clone(),
        compression_config: {
            let mut cc = mnemo_retrieval::compression::CompressionConfig::default();
            if let Ok(v) = std::env::var("MNEMO_EMBEDDING_COMPRESSION_ENABLED") {
                cc.enabled = v == "true" || v == "1";
            }
            if let Ok(v) = std::env::var("MNEMO_COMPRESSION_TIER1_DAYS") {
                if let Ok(d) = v.parse() {
                    cc.tier1_days = d;
                }
            }
            if let Ok(v) = std::env::var("MNEMO_COMPRESSION_TIER2_DAYS") {
                if let Ok(d) = v.parse() {
                    cc.tier2_days = d;
                }
            }
            if let Ok(v) = std::env::var("MNEMO_COMPRESSION_TIER3_DAYS") {
                if let Ok(d) = v.parse() {
                    cc.tier3_days = d;
                }
            }
            if let Ok(v) = std::env::var("MNEMO_COMPRESSION_SWEEP_INTERVAL_SECS") {
                if let Ok(d) = v.parse() {
                    cc.sweep_interval_secs = d;
                }
            }
            cc
        },
        compression_stats: Arc::new(mnemo_retrieval::compression::CompressionStats::default()),
        embedding_dimensions: config.embedding.dimensions,
        hyperbolic_config: {
            let mut hc = mnemo_retrieval::hyperbolic::HyperbolicConfig::default();
            if let Ok(v) = std::env::var("MNEMO_HYPERBOLIC_GRAPH_ENABLED") {
                hc.enabled = v == "true" || v == "1";
            }
            if let Ok(v) = std::env::var("MNEMO_HYPERBOLIC_CURVATURE") {
                if let Ok(c) = v.parse() {
                    hc.curvature = c;
                }
            }
            if let Ok(v) = std::env::var("MNEMO_HYPERBOLIC_ALPHA") {
                if let Ok(a) = v.parse() {
                    hc.alpha = a;
                }
            }
            hc
        },
        pipeline_metrics: Arc::new(mnemo_ingest::dag::PipelineMetrics::new({
            let mut dc = mnemo_ingest::dag::DagConfig::default();
            if let Ok(v) = std::env::var("MNEMO_PIPELINE_RETRY_MAX") {
                if let Ok(n) = v.parse() {
                    dc.max_retries = n;
                }
            }
            if let Ok(v) = std::env::var("MNEMO_PIPELINE_DEAD_LETTER_ENABLED") {
                dc.dead_letter_enabled = v == "true" || v == "1";
            }
            if let Ok(v) = std::env::var("MNEMO_PIPELINE_DEAD_LETTER_MAX_SIZE") {
                if let Ok(n) = v.parse() {
                    dc.dead_letter_max_size = n;
                }
            }
            dc
        })),
        sync_status: Arc::new(tokio::sync::RwLock::new({
            let node_id_str =
                std::env::var("MNEMO_SYNC_NODE_ID").unwrap_or_else(|_| "standalone".to_string());
            let enabled = std::env::var("MNEMO_SYNC_ENABLED")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false);
            if enabled {
                mnemo_core::sync::SyncStatus {
                    node_id: mnemo_core::sync::NodeId::new(node_id_str),
                    vector_clock: mnemo_core::sync::VectorClock::new(),
                    known_peers: Vec::new(),
                    deltas_produced: 0,
                    deltas_received: 0,
                    conflicts_resolved: 0,
                    last_sync: std::collections::BTreeMap::new(),
                    enabled: true,
                }
            } else {
                mnemo_core::sync::SyncStatus::disabled()
            }
        })),
        auth_config: auth_config.clone(),
    };

    if let Err(err) = restore_webhook_state(&app_state).await {
        tracing::warn!(error = %err, "failed to restore persisted webhook state");
    }

    // Spawn receiver task that translates ingest webhook events into
    // webhook deliveries via emit_memory_webhook_event.
    {
        use mnemo_core::models::webhook_event::IngestWebhookEvent;
        use mnemo_server::routes::emit_memory_webhook_event;
        use mnemo_server::state::MemoryWebhookEventType;

        let state_for_rx = app_state.clone();
        let mut rx = webhook_rx;
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    IngestWebhookEvent::FactAdded {
                        user_id,
                        edge_id,
                        source_entity,
                        target_entity,
                        label,
                        fact,
                        episode_id,
                        request_id,
                    } => {
                        emit_memory_webhook_event(
                            &state_for_rx,
                            user_id,
                            MemoryWebhookEventType::FactAdded,
                            request_id,
                            serde_json::json!({
                                "edge_id": edge_id,
                                "source_entity": source_entity,
                                "target_entity": target_entity,
                                "label": label,
                                "fact": fact,
                                "episode_id": episode_id,
                            }),
                        )
                        .await;
                    }
                    IngestWebhookEvent::FactSuperseded {
                        user_id,
                        old_edge_id,
                        invalidated_by_episode_id,
                        source_entity,
                        target_entity,
                        label,
                        old_fact,
                        request_id,
                    } => {
                        emit_memory_webhook_event(
                            &state_for_rx,
                            user_id,
                            MemoryWebhookEventType::FactSuperseded,
                            request_id,
                            serde_json::json!({
                                "old_edge_id": old_edge_id,
                                "invalidated_by_episode_id": invalidated_by_episode_id,
                                "source_entity": source_entity,
                                "target_entity": target_entity,
                                "label": label,
                                "old_fact": old_fact,
                            }),
                        )
                        .await;
                    }
                }
            }
        });
    }

    // Temporal tensor compression background sweep
    if app_state.compression_config.enabled {
        let sweep_state = app_state.clone();
        let interval_secs = app_state.compression_config.sweep_interval_secs;
        tracing::info!(
            interval_secs = interval_secs,
            "Temporal compression sweep enabled"
        );
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));
            interval.tick().await; // skip immediate first tick
            loop {
                interval.tick().await;
                match mnemo_server::routes::run_compression_sweep(&sweep_state).await {
                    Ok(compressed) => {
                        tracing::info!(compressed = compressed, "Compression sweep complete");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Compression sweep failed (non-fatal)");
                    }
                }
            }
        });
    } else {
        tracing::debug!(
            "Temporal compression disabled (MNEMO_EMBEDDING_COMPRESSION_ENABLED=false)"
        );
    }

    // P3-3: Background task for import job cleanup (every 10 minutes, evict jobs older than 1 hour)
    {
        let cleanup_state = app_state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(600));
            interval.tick().await; // skip immediate first tick
            loop {
                interval.tick().await;
                mnemo_server::routes::evict_stale_import_jobs(
                    &cleanup_state,
                    chrono::Duration::hours(1),
                )
                .await;
            }
        });
    }

    // ─── REST (Axum) router ───────────────────────────────────────
    let rest = build_router(app_state.clone())
        .layer(from_fn_with_state(
            app_state.clone(),
            request_context_middleware,
        ))
        .layer(AuthLayer::new((*auth_config).clone()))
        .layer(TraceLayer::new_for_http())
        .layer({
            let origins = &config.server.cors_allowed_origins;
            if origins.len() == 1 && origins[0] == "*" {
                CorsLayer::permissive()
            } else {
                let parsed: Vec<axum::http::HeaderValue> =
                    origins.iter().filter_map(|o| o.parse().ok()).collect();
                CorsLayer::new()
                    .allow_origin(AllowOrigin::list(parsed))
                    .allow_methods(Any)
                    .allow_headers(Any)
            }
        });

    // ─── OpenAPI spec + Swagger UI ────────────────────────────────
    use mnemo_server::openapi::MnemoApiDoc;
    use utoipa::OpenApi;
    let openapi_json = MnemoApiDoc::openapi()
        .to_json()
        .expect("OpenAPI JSON serialization");
    let openapi_json_clone = openapi_json.clone();

    let openapi_routes = axum::Router::new()
        .route(
            "/api/v1/openapi.json",
            axum::routing::get(move || async move {
                axum::response::Response::builder()
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(openapi_json_clone))
                    .unwrap()
            }),
        )
        .route(
            "/swagger-ui",
            axum::routing::get(move || async move { axum::response::Html(SWAGGER_UI_HTML) }),
        );

    // Merge OpenAPI routes (auth-exempt via middleware path checks)
    let rest = rest.merge(openapi_routes);

    // ─── gRPC (tonic) router ────────────────────────────────────
    // gRPC handlers enforce auth internally via validate_grpc_auth(),
    // using the same AuthConfig shared with the REST middleware.
    // P2-2: Control policy enforcement via config (default: enabled for security).
    let grpc_state = GrpcState::from_app_state_with_config(
        &app_state,
        auth_config.clone(),
        config.server.grpc_enforce_policies,
    );

    // Health check service
    let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<mnemo_proto::proto::memory_service_server::MemoryServiceServer<GrpcState>>()
        .await;
    health_reporter
        .set_serving::<mnemo_proto::proto::user_service_server::UserServiceServer<GrpcState>>()
        .await;
    health_reporter
        .set_serving::<mnemo_proto::proto::session_service_server::SessionServiceServer<GrpcState>>(
        )
        .await;
    health_reporter
        .set_serving::<mnemo_proto::proto::entity_service_server::EntityServiceServer<GrpcState>>()
        .await;
    health_reporter
        .set_serving::<mnemo_proto::proto::edge_service_server::EdgeServiceServer<GrpcState>>()
        .await;
    health_reporter
        .set_serving::<mnemo_proto::proto::agent_service_server::AgentServiceServer<GrpcState>>()
        .await;

    // Reflection service (exposes all registered protos to grpcurl / Postman)
    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(mnemo_proto::FILE_DESCRIPTOR_SET)
        .build_v1()
        .expect("failed to build gRPC reflection service");

    // Message size limits: 4 MiB decode (incoming), 16 MiB encode (outgoing)
    const GRPC_MAX_DECODE_SIZE: usize = 4 * 1024 * 1024;
    const GRPC_MAX_ENCODE_SIZE: usize = 16 * 1024 * 1024;

    let grpc_routes = tonic::service::Routes::new(health_service)
        .add_service(reflection)
        .add_service(
            mnemo_proto::proto::memory_service_server::MemoryServiceServer::new(grpc_state.clone())
                .max_decoding_message_size(GRPC_MAX_DECODE_SIZE)
                .max_encoding_message_size(GRPC_MAX_ENCODE_SIZE),
        )
        .add_service(
            mnemo_proto::proto::user_service_server::UserServiceServer::new(grpc_state.clone())
                .max_decoding_message_size(GRPC_MAX_DECODE_SIZE)
                .max_encoding_message_size(GRPC_MAX_ENCODE_SIZE),
        )
        .add_service(
            mnemo_proto::proto::session_service_server::SessionServiceServer::new(
                grpc_state.clone(),
            )
            .max_decoding_message_size(GRPC_MAX_DECODE_SIZE)
            .max_encoding_message_size(GRPC_MAX_ENCODE_SIZE),
        )
        .add_service(
            mnemo_proto::proto::entity_service_server::EntityServiceServer::new(grpc_state.clone())
                .max_decoding_message_size(GRPC_MAX_DECODE_SIZE)
                .max_encoding_message_size(GRPC_MAX_ENCODE_SIZE),
        )
        .add_service(
            mnemo_proto::proto::edge_service_server::EdgeServiceServer::new(grpc_state.clone())
                .max_decoding_message_size(GRPC_MAX_DECODE_SIZE)
                .max_encoding_message_size(GRPC_MAX_ENCODE_SIZE),
        )
        .add_service(
            mnemo_proto::proto::agent_service_server::AgentServiceServer::new(grpc_state)
                .max_decoding_message_size(GRPC_MAX_DECODE_SIZE)
                .max_encoding_message_size(GRPC_MAX_ENCODE_SIZE),
        );

    tracing::info!(
        "gRPC services registered: MemoryService, UserService, SessionService, EntityService, EdgeService, AgentService + health + reflection"
    );

    let rest_addr = format!("{}:{}", config.server.host, config.server.port);

    // ─── Dedicated gRPC port (optional) ─────────────────────────
    // When MNEMO_GRPC_PORT is set, gRPC binds to its own port (like Qdrant :6334)
    // and REST binds to MNEMO_SERVER_PORT. When unset, gRPC is multiplexed on REST.
    if let Some(grpc_port) = config.server.grpc_port {
        let grpc_addr = format!("{}:{}", config.server.host, grpc_port);
        let grpc_listener = TcpListener::bind(&grpc_addr).await?;
        let rest_listener = TcpListener::bind(&rest_addr).await?;

        tracing::info!(rest_addr = %rest_addr, grpc_addr = %grpc_addr, "Mnemo server listening");
        println!(
            r#"
  __  __
 |  \/  |_ __   ___ _ __ ___   ___
 | |\/| | '_ \ / _ \ '_ ` _ \ / _ \
 | |  | | | | |  __/ | | | | | (_) |
 |_|  |_|_| |_|\___|_| |_| |_|\___/

 v{} | REST {} | gRPC {}
"#,
            env!("CARGO_PKG_VERSION"),
            rest_addr,
            grpc_addr
        );

        // Build the tonic tower service for the dedicated gRPC port
        // P3-1 & P3-2: Apply concurrency limits (rate limiting requires Clone which
        // tower::limit::RateLimit doesn't provide; concurrency limiting is sufficient
        // for DoS protection and is Clone-compatible)
        let grpc_service = {
            let router = grpc_routes.into_axum_router();
            if config.server.grpc_max_connections > 0 {
                tracing::info!(
                    max_connections = config.server.grpc_max_connections,
                    "gRPC concurrency limiting enabled"
                );
                router.layer(tower::limit::ConcurrencyLimitLayer::new(
                    config.server.grpc_max_connections,
                ))
            } else {
                router
            }
        };

        tokio::select! {
            result = axum::serve(rest_listener, rest.into_make_service()).with_graceful_shutdown(shutdown_signal()) => {
                if let Err(e) = result { tracing::error!("REST server error: {e}"); }
            }
            result = axum::serve(grpc_listener, grpc_service.into_make_service()).with_graceful_shutdown(shutdown_signal()) => {
                if let Err(e) = result { tracing::error!("gRPC server error: {e}"); }
            }
        }
    } else {
        // ─── Multiplex: gRPC + REST on the same port ────────────
        // P3-1 & P3-2: Apply concurrency limits
        let grpc = {
            let router = grpc_routes.into_axum_router();
            if config.server.grpc_max_connections > 0 {
                tracing::info!(
                    max_connections = config.server.grpc_max_connections,
                    "gRPC concurrency limiting enabled (multiplexed)"
                );
                router.layer(tower::limit::ConcurrencyLimitLayer::new(
                    config.server.grpc_max_connections,
                ))
            } else {
                router
            }
        };
        let app = multiplex_grpc_rest(rest, grpc);

        let listener = TcpListener::bind(&rest_addr).await?;
        tracing::info!(addr = %rest_addr, "Mnemo server listening (REST + gRPC multiplexed)");

        println!(
            r#"
  __  __
 |  \/  |_ __   ___ _ __ ___   ___
 | |\/| | '_ \ / _ \ '_ ` _ \ / _ \
 | |  | | | | |  __/ | | | | | (_) |
 |_|  |_|_| |_|\___|_| |_| |_|\___/

 v{} | {} | REST + gRPC
"#,
            env!("CARGO_PKG_VERSION"),
            rest_addr
        );

        axum::serve(listener, app.into_make_service())
            .with_graceful_shutdown(shutdown_signal())
            .await?;
    }

    // Flush any pending OTel spans before exit
    mnemo_server::telemetry::shutdown_telemetry(_otel_provider);
    Ok(())
}

/// Wait for SIGTERM or Ctrl+C to initiate graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { tracing::info!("Ctrl+C received, shutting down"); },
        _ = terminate => { tracing::info!("SIGTERM received, shutting down"); },
    }
}

/// Build a combined router that serves both REST and gRPC on the same port.
/// gRPC paths (e.g. `/mnemo.v1.MemoryService/GetContext`) are disjoint from
/// REST paths (`/api/v1/...`) so a simple merge works.
fn multiplex_grpc_rest(rest: axum::Router, grpc: axum::Router) -> axum::Router {
    rest.merge(grpc)
}

/// Swagger UI HTML page served from CDN. Loads the OpenAPI spec from `/api/v1/openapi.json`.
const SWAGGER_UI_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
  <title>Mnemo API — Swagger UI</title>
  <meta charset="utf-8"/>
  <meta name="viewport" content="width=device-width, initial-scale=1"/>
  <link rel="stylesheet" type="text/css" href="https://unpkg.com/swagger-ui-dist@5.18.2/swagger-ui.css"
        crossorigin="anonymous" referrerpolicy="no-referrer" />
</head>
<body>
  <div id="swagger-ui"></div>
  <script src="https://unpkg.com/swagger-ui-dist@5.18.2/swagger-ui-bundle.js"
          crossorigin="anonymous" referrerpolicy="no-referrer"></script>
  <script>
    SwaggerUIBundle({
      url: '/api/v1/openapi.json',
      dom_id: '#swagger-ui',
      deepLinking: true,
      presets: [SwaggerUIBundle.presets.apis, SwaggerUIBundle.SwaggerUIStandalonePreset],
      layout: 'StandaloneLayout'
    });
  </script>
</body>
</html>"#;
