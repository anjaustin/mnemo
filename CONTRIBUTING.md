# Contributing to Mnemo

Thank you for your interest in contributing! This guide covers everything you need to get started.

---

## Development Setup

### Prerequisites

- Rust 1.85+ (`rustup update stable`)
- `protoc` (Protocol Buffers compiler, v25+ recommended — needed by `mnemo-proto`)
- Docker and Docker Compose
- An LLM API key (optional — Mnemo works without one using rule-based extraction)
- Python 3.11+ (only if contributing to the Python SDK)
- Node.js 20+ (only if contributing to the TypeScript SDK)

### Getting Started

```bash
git clone https://github.com/anjaustin/mnemo.git
cd mnemo

# Start dependencies
docker compose up -d redis qdrant

# Run the server
cargo run --bin mnemo-server

# Run tests
cargo test --workspace

# Run a specific crate's tests
cargo test -p mnemo-core
```

### Project Structure

```
mnemo/
├── crates/
│   ├── mnemo-core/       # Domain types, traits, errors (no external deps)
│   ├── mnemo-storage/    # Redis + Qdrant implementations
│   ├── mnemo-llm/        # LLM & embedding providers
│   ├── mnemo-ingest/     # Background ingestion pipeline
│   ├── mnemo-retrieval/  # Search + context assembly + compression
│   ├── mnemo-graph/      # Graph traversal, community detection
│   ├── mnemo-gnn/        # GNN-inspired retrieval feedback
│   ├── mnemo-mcp/        # Model Context Protocol (MCP) server
│   ├── mnemo-proto/      # gRPC proto definitions (tonic-build)
│   └── mnemo-server/     # HTTP/gRPC server, config, routes, dashboard
├── config/               # Default configuration (TOML)
├── deploy/               # Kubernetes (Helm), DigitalOcean, Linode
├── sdk/
│   ├── python/           # Python SDK (async + sync)
│   └── typescript/       # TypeScript SDK (Vercel AI adapter)
├── proto/                # Proto3 service definitions
├── docs/                 # Architecture, PRDs, design docs
└── docker-compose.yml    # Development stack
```

The dependency graph is deliberately clean:

```
mnemo-server
├── mnemo-ingest
│   ├── mnemo-core
│   └── mnemo-llm
├── mnemo-retrieval
│   └── mnemo-core
├── mnemo-graph
│   └── mnemo-core
├── mnemo-gnn
│   └── mnemo-core
├── mnemo-storage
│   └── mnemo-core
├── mnemo-llm
│   └── mnemo-core
├── mnemo-mcp
│   └── mnemo-core
└── mnemo-proto
    └── (standalone, tonic-build)
```

`mnemo-core` depends on nothing in-workspace. Everything else depends on `mnemo-core` for types and traits. The server crate pulls it all together.

---

## Code Style

### Rust Conventions

- Follow standard `rustfmt` formatting (`cargo fmt`)
- Fix all `clippy` warnings (`cargo clippy --workspace --all-targets -- -D warnings`)
- Add doc comments (`///`) to all public types and functions
- Use `tracing` for logging, not `println!` or `log`
- Prefer `thiserror` for error types, `anyhow` only in `main()`

### Naming

- Types: `PascalCase`
- Functions and methods: `snake_case`
- Constants: `SCREAMING_SNAKE_CASE`
- Redis keys: `prefix:resource:id` (e.g., `mnemo:user:019...`)
- Qdrant collections: `prefix_resource` (e.g., `mnemo_entities`)

### Error Handling

All fallible functions return `Result<T, MnemoError>`. The `MnemoError` enum in `mnemo-core` covers every error case. When adding new error variants, include an HTTP status code mapping and an error code string.

### Testing

- Unit tests live alongside the code in `#[cfg(test)] mod tests`
- Integration tests go in `tests/` directories
- Test names follow `test_<what>_<scenario>` (e.g., `test_entity_alias_no_duplicates`)
- Use `Uuid::now_v7()` for test IDs (time-ordered, unique)

---

## Making Changes

### Small Changes

For bug fixes, documentation improvements, and small features:

1. Fork the repository
2. Create a branch: `git checkout -b fix/description`
3. Make your changes
4. Run `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --lib --bins`
5. Open a pull request

For API or retrieval changes, also run the memory falsification suite:

```bash
cargo test -p mnemo-server --test memory_api -- --test-threads=1
```

This test hits the high-level memory endpoints and checks validation, identifier resolution, immediate-recall fallback behavior, and chat-history import falsification cases.
CI runs the same suite in `.github/workflows/memory-falsification.yml`.

If you want an isolated local dependency stack for integration tests:

```bash
docker compose -f docker-compose.test.yml up -d
cargo test -p mnemo-storage --test storage -- --test-threads=1
cargo test -p mnemo-ingest --test ingest -- --test-threads=1
cargo test -p mnemo-server --test memory_api -- --test-threads=1
bash tests/e2e_smoke.sh http://localhost:8080
docker compose -f docker-compose.test.yml down
```

### Larger Changes

For new features or architectural changes, please open an issue first to discuss the approach. This helps avoid wasted effort and ensures alignment with the project's direction.

### Commit Messages

Use conventional commits:

```
feat: add full-text search to retrieval engine
fix: prevent double-processing of episodes
docs: add API reference for edge filtering
refactor: split MnemoStore into StateStore + VectorStore
test: add integration tests for Redis entity dedup
```

---

## Areas Where Help Is Needed

These are the highest-impact areas for contributions right now:

- **Documentation**: Tutorials, worked examples, domain-specific integration guides
- **Benchmarks**: DMR benchmark implementation (LongMemEval shipped in v0.3.7)
- **Multi-modal memory**: Text-only today — image/audio/video memory is a genuine gap
- **Automatic re-encryption**: BYOK key rotation decrypts with old keys but has no bulk re-encrypt workflow
- **CRDT sync**: Multi-node sync protocol is scaffolded but not battle-tested at scale

Already shipped:
- ~~RediSearch full-text search~~ — shipped in v0.2
- ~~Python SDK~~ — shipped in v0.3, full async + sync coverage
- ~~TypeScript SDK~~ — shipped in v0.3, with Vercel AI adapter
- ~~LangChain and LlamaIndex adapters~~ — shipped in v0.3
- ~~Integration tests~~ — 244 tests in `tests/memory_api.rs`
- ~~Progressive summarization~~ — shipped, configurable via `MNEMO_SESSION_SUMMARY_THRESHOLD`
- ~~Helm chart~~ — shipped in v0.7.0, with Redis/Qdrant subcharts and NetworkPolicy
- ~~OpenTelemetry~~ — shipped in v0.7.0, with TLS and auth header support
- ~~gRPC API~~ — shipped in v0.6.0, 3 services, 8 RPCs
- ~~MCP server~~ — shipped in v0.6.0, stdio transport

---

## License

By contributing, you agree that your contributions will be licensed under the Apache 2.0 License.
