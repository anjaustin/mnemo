# Reference

Technical reference documentation.

---

## In This Section

| Reference | Description |
|-----------|-------------|
| **[Configuration](configuration.md)** | Environment variables and TOML settings |
| **[Architecture](architecture.md)** | System internals and design |
| **[Performance](performance.md)** | Benchmarks and tuning |
| **[Troubleshooting](troubleshooting.md)** | Common issues and solutions |
| **[Security](security.md)** | Security controls and hardening |

---

## Quick Links

### Configuration

- [Server Settings](configuration.md#core-settings)
- [LLM Providers](configuration.md#llm-providers)
- [Retrieval Settings](configuration.md#retrieval)
- [Security Settings](configuration.md#security)

### Architecture

- [System Overview](architecture.md#system-overview)
- [Workspace Crates](architecture.md#workspace-crates)
- [Ingestion Pipeline](architecture.md#ingestion-pipeline)
- [Retrieval Pipeline](architecture.md#retrieval-pipeline)

---

## Environment Variables

Common variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `MNEMO_PORT` | `8080` | HTTP/gRPC port |
| `REDIS_URL` | `redis://localhost:6379` | Redis connection |
| `QDRANT_URL` | `http://localhost:6333` | Qdrant connection |
| `LLM_PROVIDER` | `anthropic` | LLM provider |
| `EMBEDDING_PROVIDER` | `fastembed` | Embedding provider |
| `MNEMO_AUTH_ENABLED` | `false` | Enable API key auth |

See **[Configuration](configuration.md)** for the complete list.
