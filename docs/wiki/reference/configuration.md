# Configuration Reference

Complete reference for all Mnemo configuration options.

---

## Configuration Sources

Mnemo configuration is loaded in order (later sources override earlier):

1. **Built-in defaults**
2. **TOML file** (`config/default.toml` or `MNEMO_CONFIG_PATH`)
3. **Environment variables** (prefix: `MNEMO_`)

---

## Core Settings

### Server

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `MNEMO_HOST` | `server.host` | `0.0.0.0` | Bind address |
| `MNEMO_PORT` | `server.port` | `8080` | HTTP/gRPC port |
| `MNEMO_WORKERS` | `server.workers` | CPU count | Async workers |
| `MNEMO_REQUEST_TIMEOUT` | `server.request_timeout` | `30` | Request timeout (seconds) |
| `MNEMO_CORS_ORIGINS` | `server.cors_origins` | `*` | CORS allowed origins (CSV) |

### Redis

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `REDIS_URL` | `redis.url` | `redis://localhost:6379` | Redis connection URL |
| `REDIS_POOL_SIZE` | `redis.pool_size` | `10` | Connection pool size |
| `REDIS_KEY_PREFIX` | `redis.key_prefix` | `mnemo` | Key prefix for all data |

### Qdrant

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `QDRANT_URL` | `qdrant.url` | `http://localhost:6333` | Qdrant server URL |
| `QDRANT_API_KEY` | `qdrant.api_key` | None | Qdrant API key (if auth enabled) |
| `QDRANT_COLLECTION` | `qdrant.collection` | `mnemo_episodes` | Collection name |

---

## Authentication

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `MNEMO_AUTH_ENABLED` | `auth.enabled` | `false` | Enable API key auth |
| `MNEMO_AUTH_KEYS` | `auth.keys` | None | Bootstrap API keys (CSV) |
| `MNEMO_AUTH_ADMIN_KEY` | `auth.admin_key` | None | Admin bootstrap key |

### API Key Format

Keys can be specified as:
- Plain key: `mnemo_abc123...`
- Key with role: `mnemo_abc123...:admin`
- Key with scope: `mnemo_abc123...:write:user_id`

---

## LLM Providers

### Provider Selection

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `LLM_PROVIDER` | `llm.provider` | `anthropic` | Primary LLM provider |
| `EMBEDDING_PROVIDER` | `embedding.provider` | `fastembed` | Embedding provider |
| `VISION_PROVIDER` | `vision.provider` | `openai` | Vision provider |
| `TRANSCRIPTION_PROVIDER` | `transcription.provider` | `openai` | Transcription provider |

### Anthropic

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `ANTHROPIC_API_KEY` | `llm.anthropic.api_key` | Required | API key |
| `ANTHROPIC_MODEL` | `llm.anthropic.model` | `claude-sonnet-4-20250514` | Model name |
| `ANTHROPIC_MAX_TOKENS` | `llm.anthropic.max_tokens` | `4096` | Max response tokens |

### OpenAI

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `OPENAI_API_KEY` | `llm.openai.api_key` | Required | API key |
| `OPENAI_MODEL` | `llm.openai.model` | `gpt-4o` | Model name |
| `OPENAI_EMBEDDING_MODEL` | `embedding.openai.model` | `text-embedding-3-small` | Embedding model |

### Ollama

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `OLLAMA_URL` | `llm.ollama.url` | `http://localhost:11434` | Ollama server URL |
| `OLLAMA_MODEL` | `llm.ollama.model` | `llama3.2` | Model name |
| `OLLAMA_EMBEDDING_MODEL` | `embedding.ollama.model` | `nomic-embed-text` | Embedding model |

### FastEmbed (Local Embeddings)

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `FASTEMBED_MODEL` | `embedding.fastembed.model` | `BAAI/bge-small-en-v1.5` | Model name |
| `FASTEMBED_CACHE_DIR` | `embedding.fastembed.cache_dir` | `~/.cache/fastembed` | Model cache |

---

## Ingestion Pipeline

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `MNEMO_INGEST_WORKERS` | `ingest.workers` | `4` | Background workers |
| `MNEMO_INGEST_BATCH_SIZE` | `ingest.batch_size` | `10` | Episodes per batch |
| `MNEMO_INGEST_RETRY_MAX` | `ingest.retry_max` | `3` | Max retry attempts |
| `MNEMO_INGEST_RETRY_DELAY` | `ingest.retry_delay_ms` | `1000` | Initial retry delay (ms) |

---

## Retrieval

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `MNEMO_DEFAULT_MAX_TOKENS` | `retrieval.default_max_tokens` | `500` | Default context tokens |
| `MNEMO_DEFAULT_MIN_RELEVANCE` | `retrieval.min_relevance` | `0.3` | Minimum relevance score |
| `MNEMO_DEFAULT_RERANKER` | `retrieval.reranker` | `rrf` | Default reranker (rrf/mmr/gnn) |
| `MNEMO_SEMANTIC_TOP_K` | `retrieval.semantic_top_k` | `50` | Semantic search candidates |
| `MNEMO_FTS_TOP_K` | `retrieval.fts_top_k` | `30` | Full-text search candidates |
| `MNEMO_GRAPH_DEPTH` | `retrieval.graph_depth` | `2` | Graph traversal depth |

### Temporal Settings

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `MNEMO_DECAY_HALF_LIFE_DAYS` | `temporal.decay_half_life_days` | `30` | Confidence decay half-life |
| `MNEMO_TEMPORAL_WEIGHT` | `temporal.default_weight` | `0.3` | Default temporal weight |

---

## Multi-Modal

### Vision

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `VISION_MAX_TOKENS` | `vision.max_tokens` | `500` | Max description tokens |
| `VISION_DETAIL` | `vision.detail` | `auto` | Image detail level (low/high/auto) |

### Transcription

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `TRANSCRIPTION_MODEL` | `transcription.model` | `whisper-1` | Whisper model |
| `TRANSCRIPTION_LANGUAGE` | `transcription.language` | `en` | Default language |

### Document Parsing

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `DOCUMENT_CHUNK_STRATEGY` | `document.chunk_strategy` | `paragraph` | Chunking method |
| `DOCUMENT_MAX_CHUNK_SIZE` | `document.max_chunk_size` | `1000` | Max chars per chunk |

### Blob Storage

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `BLOB_STORAGE_PROVIDER` | `blob.provider` | `local` | Storage backend (local/s3) |
| `BLOB_STORAGE_PATH` | `blob.local.path` | `./data/blobs` | Local storage path |
| `AWS_S3_BUCKET` | `blob.s3.bucket` | None | S3 bucket name |
| `AWS_S3_REGION` | `blob.s3.region` | `us-east-1` | S3 region |
| `AWS_S3_ENDPOINT` | `blob.s3.endpoint` | None | Custom S3 endpoint |

---

## Webhooks

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `MNEMO_WEBHOOK_ENABLED` | `webhook.enabled` | `false` | Enable webhooks |
| `MNEMO_WEBHOOK_URL` | `webhook.url` | None | Webhook endpoint URL |
| `MNEMO_WEBHOOK_SECRET` | `webhook.secret` | None | HMAC signing secret |
| `MNEMO_WEBHOOK_TIMEOUT` | `webhook.timeout_ms` | `5000` | Request timeout (ms) |
| `MNEMO_WEBHOOK_RETRY_MAX` | `webhook.retry_max` | `3` | Max retries |

---

## Security

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `MNEMO_ENCRYPTION_KEY` | `security.encryption_key` | None | AES-256-GCM key (base64) |
| `MNEMO_RATE_LIMIT_RPS` | `security.rate_limit_rps` | `100` | Requests per second |
| `MNEMO_RATE_LIMIT_BURST` | `security.rate_limit_burst` | `200` | Burst capacity |
| `MNEMO_MAX_REQUEST_SIZE` | `security.max_request_size` | `10485760` | Max request body (bytes) |

---

## Observability

| Variable | TOML Key | Default | Description |
|----------|----------|---------|-------------|
| `RUST_LOG` | - | `info` | Log level (trace/debug/info/warn/error) |
| `MNEMO_LOG_FORMAT` | `logging.format` | `json` | Log format (json/pretty) |
| `MNEMO_METRICS_ENABLED` | `metrics.enabled` | `true` | Enable Prometheus metrics |
| `MNEMO_METRICS_PATH` | `metrics.path` | `/metrics` | Metrics endpoint path |

---

## TOML Configuration Example

```toml
# config/production.toml

[server]
host = "0.0.0.0"
port = 8080
workers = 8
cors_origins = "https://app.example.com,https://admin.example.com"

[redis]
url = "redis://redis.internal:6379"
pool_size = 20

[qdrant]
url = "http://qdrant.internal:6333"
collection = "mnemo_production"

[auth]
enabled = true

[llm.anthropic]
model = "claude-sonnet-4-20250514"
max_tokens = 8192

[embedding]
provider = "openai"

[embedding.openai]
model = "text-embedding-3-large"

[retrieval]
default_max_tokens = 1000
reranker = "gnn"
semantic_top_k = 100

[temporal]
decay_half_life_days = 60

[webhook]
enabled = true
timeout_ms = 10000
retry_max = 5

[security]
rate_limit_rps = 200
rate_limit_burst = 500

[metrics]
enabled = true
```

---

## Environment Override Examples

```bash
# Production-like configuration via env vars
export MNEMO_PORT=8080
export MNEMO_AUTH_ENABLED=true
export REDIS_URL="redis://redis.internal:6379"
export QDRANT_URL="http://qdrant.internal:6333"
export ANTHROPIC_API_KEY="sk-ant-..."
export OPENAI_API_KEY="sk-..."
export LLM_PROVIDER=anthropic
export EMBEDDING_PROVIDER=openai
export MNEMO_WEBHOOK_ENABLED=true
export MNEMO_WEBHOOK_URL="https://hooks.example.com/mnemo"
export MNEMO_WEBHOOK_SECRET="whsec_..."
export RUST_LOG="mnemo_server=info,mnemo_ingest=debug"
```

---

## Next Steps

- **[Architecture](architecture.md)** - System internals
- **[Deployment](../deployment/docker.md)** - Production setup
- **[Troubleshooting](troubleshooting.md)** - Common issues
