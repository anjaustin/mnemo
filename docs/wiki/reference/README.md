# Reference

Technical reference documentation.

---

## In This Section

| Reference | Description |
|-----------|-------------|
| **[Configuration](configuration.md)** | Environment variables and TOML settings |
| **[Architecture](architecture.md)** | System internals and design |
| **[Benchmarks](../../BENCHMARKS.md)** | Performance measurements |
| **[Troubleshooting](../../TROUBLESHOOTING.md)** | Common issues and solutions |
| **[Security](../../SECURITY_CONTROLS.md)** | Security controls and hardening |

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
| `MNEMO_SERVER_PORT` | `8080` | HTTP port |
| `MNEMO_REDIS_URL` | `redis://localhost:6379` | Redis connection |
| `MNEMO_QDRANT_URL` | `http://localhost:6334` | Qdrant connection |
| `MNEMO_LLM_PROVIDER` | `anthropic` | LLM provider |
| `MNEMO_EMBEDDING_PROVIDER` | `openai` | Embedding provider |
| `MNEMO_AUTH_ENABLED` | `false` | Enable API key auth |

See **[Configuration](configuration.md)** for the complete list.
