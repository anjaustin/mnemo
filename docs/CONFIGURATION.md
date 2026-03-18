# Configuration

Mnemo reads `config/default.toml` and overrides with environment variables.

## Core Settings

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_SERVER_HOST` | Server bind address | `0.0.0.0` |
| `MNEMO_SERVER_PORT` | Server port | `8080` |
| `MNEMO_CONFIG` | Path to custom TOML config file | (none) |
| `MNEMO_CORS_ALLOWED_ORIGINS` | Comma-separated allowed CORS origins (`*` = all) | `*` |

## LLM Provider

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_LLM_PROVIDER` | `anthropic`, `openai`, `ollama`, `liquid` | `anthropic` |
| `MNEMO_LLM_API_KEY` | API key for entity extraction | (none) |
| `MNEMO_LLM_MODEL` | Model for extraction | `claude-sonnet-4-20250514` |
| `MNEMO_LLM_BASE_URL` | Base URL for LLM provider | Provider default |

## Embeddings

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_EMBEDDING_PROVIDER` | `openai`-compatible or `local` fastembed | `openai` |
| `MNEMO_EMBEDDING_API_KEY` | Embedding API key | (none) |
| `MNEMO_EMBEDDING_MODEL` | Model for embedding generation | `text-embedding-3-small` |
| `MNEMO_EMBEDDING_BASE_URL` | Base URL for embedding provider | Provider default |
| `MNEMO_EMBEDDING_DIMENSIONS` | Embedding vector dimensions | `1536` |

### Local Embeddings

For fully offline operation:

```bash
MNEMO_EMBEDDING_PROVIDER=local
MNEMO_EMBEDDING_MODEL=AllMiniLML6V2
MNEMO_EMBEDDING_DIMENSIONS=384
```

## Storage

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_REDIS_URL` | Redis connection | `redis://localhost:6379` |
| `MNEMO_QDRANT_URL` | Qdrant connection | `http://localhost:6334` |
| `MNEMO_QDRANT_PREFIX` | Qdrant collection prefix / namespace | `mnemo_` |
| `MNEMO_QDRANT_API_KEY` | Qdrant API key for authenticated access | (none) |

## Authentication

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_AUTH_ENABLED` | Require API key auth (`true`/`false`) | `false` |
| `MNEMO_AUTH_API_KEYS` | Comma-separated accepted API keys | (none) |

## Encryption (BYOK)

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_ENCRYPTION_ENABLED` | Enable AES-256-GCM at-rest encryption | `false` |
| `MNEMO_ENCRYPTION_MASTER_KEY` | Base64-encoded 32-byte master key | (none) |
| `MNEMO_ENCRYPTION_KEY_ID` | Identifier for active encryption key | `kek-001` |
| `MNEMO_ENCRYPTION_RETIRED_KEYS` | Retired keys for rotation (`key_id:base64key,...`) | (none) |

## Retrieval

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_METADATA_PREFILTER_ENABLED` | Enable metadata prefilter planner | `true` |
| `MNEMO_METADATA_SCAN_LIMIT` | Candidate scan limit for prefilter planner | `400` |
| `MNEMO_METADATA_RELAX_IF_EMPTY` | Relax strict metadata filters when empty | `false` |
| `reranker` (TOML only) | Retrieval reranking strategy: `rrf` or `mmr` | `rrf` |

## Sleep-Time Compute

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_SLEEP_ENABLED` | Enable background sleep-time compute | `true` |
| `MNEMO_SLEEP_IDLE_WINDOW_SECONDS` | Seconds of inactivity before triggering | `300` |
| `MNEMO_SESSION_SUMMARY_THRESHOLD` | Episodes before progressive summarization (0 = disabled) | `10` |

## Webhooks

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_WEBHOOKS_ENABLED` | Enable outbound webhook delivery | `true` |
| `MNEMO_WEBHOOKS_MAX_ATTEMPTS` | Retry attempts before dead-lettering | `3` |
| `MNEMO_WEBHOOKS_BASE_BACKOFF_MS` | Base backoff duration for retries | `200` |
| `MNEMO_WEBHOOKS_TIMEOUT_MS` | Per-attempt request timeout | `3000` |
| `MNEMO_WEBHOOKS_MAX_EVENTS_PER_WEBHOOK` | Max retained event rows per webhook | `1000` |
| `MNEMO_WEBHOOKS_RATE_LIMIT_PER_MINUTE` | Max outbound sends per webhook per minute | `120` |
| `MNEMO_WEBHOOKS_CIRCUIT_BREAKER_THRESHOLD` | Consecutive failures before opening circuit | `5` |
| `MNEMO_WEBHOOKS_CIRCUIT_BREAKER_COOLDOWN_MS` | Circuit cooldown before retrying sends | `60000` |
| `MNEMO_WEBHOOKS_PERSISTENCE_ENABLED` | Persist webhook state in Redis | `true` |
| `MNEMO_WEBHOOKS_PERSISTENCE_PREFIX` | Redis key suffix for webhook state | `webhooks` |
| `MNEMO_REQUIRE_TLS` | Reject non-HTTPS webhook targets | `false` |

## Observability

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_OTEL_ENABLED` | Enable OpenTelemetry OTLP trace export | `false` |
| `MNEMO_OTEL_ENDPOINT` | OTLP gRPC collector endpoint | `http://localhost:4317` |
| `MNEMO_OTEL_SERVICE_NAME` | Service name reported in traces | `mnemo` |
| `MNEMO_OTEL_TLS_ENABLED` | Enable TLS for OTLP gRPC connection | `false` |
| `MNEMO_OTEL_TLS_CA_PATH` | Path to CA certificate (PEM) | (none) |
| `MNEMO_OTEL_AUTH_HEADER` | Authorization header for OTLP collector | (none) |
| `MNEMO_AUDIT_SIGNING_SECRET` | HMAC secret for signing audit export responses | (none) |

## Embedding Compression

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_EMBEDDING_COMPRESSION_ENABLED` | Enable temporal embedding compression | `true` |
| `MNEMO_COMPRESSION_TIER1_DAYS` | Days before Tier 1 (quantized) | `30` |
| `MNEMO_COMPRESSION_TIER2_DAYS` | Days before Tier 2 (dimensionality-reduced) | `90` |
| `MNEMO_COMPRESSION_TIER3_DAYS` | Days before Tier 3 (archived) | `365` |
| `MNEMO_COMPRESSION_SWEEP_INTERVAL_SECS` | Background sweep interval | `3600` |

## Hyperbolic Geometry

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_HYPERBOLIC_GRAPH_ENABLED` | Enable hyperbolic geometry for graph embeddings | `false` |
| `MNEMO_HYPERBOLIC_CURVATURE` | Poincare ball curvature parameter | `1.0` |
| `MNEMO_HYPERBOLIC_ALPHA` | Mixing weight between Euclidean and hyperbolic | `0.3` |

## Pipeline

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_PIPELINE_RETRY_MAX` | Max retries for failed pipeline stages | `3` |
| `MNEMO_PIPELINE_DEAD_LETTER_ENABLED` | Enable dead-letter queue | `true` |
| `MNEMO_PIPELINE_DEAD_LETTER_MAX_SIZE` | Max items before oldest are evicted | `1000` |

## TinyLoRA

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_LORA_ENABLED` | Enable per-agent embedding personalization | `false` |

## Multi-Node Sync

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_SYNC_ENABLED` | Enable multi-node CRDT sync protocol | `false` |
| `MNEMO_SYNC_NODE_ID` | Unique node identifier (must differ per replica) | (auto-generated) |

## gRPC

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_GRPC_PORT` | Dedicated gRPC port (e.g. `50051`). When unset, gRPC is multiplexed on REST port. | (multiplexed) |

## Example .env

```bash
# Required for LLM extraction (optional - works without it)
MNEMO_LLM_API_KEY=sk-...

# Storage
MNEMO_REDIS_URL=redis://localhost:6379
MNEMO_QDRANT_URL=http://localhost:6334

# Production auth
MNEMO_AUTH_ENABLED=true
MNEMO_AUTH_API_KEYS=mnk_prod_key_1,mnk_prod_key_2

# Encryption
MNEMO_ENCRYPTION_ENABLED=true
MNEMO_ENCRYPTION_MASTER_KEY=base64-encoded-32-byte-key

# Observability
MNEMO_OTEL_ENABLED=true
MNEMO_OTEL_ENDPOINT=http://otel-collector:4317

# Webhook security
MNEMO_REQUIRE_TLS=true
```

See `config/default.toml` for all options with inline comments.
