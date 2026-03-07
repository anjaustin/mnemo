# Contributing to Mnemo

Thank you for your interest in contributing! This guide covers everything you need to get started.

---

## Development Setup

### Prerequisites

- Rust 1.85+ (`rustup update stable`)
- Docker and Docker Compose
- An LLM API key (optional — Mnemo works without one using rule-based extraction)

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
│   ├── mnemo-retrieval/  # Search + context assembly
│   ├── mnemo-graph/      # Graph traversal, community detection
│   └── mnemo-server/     # HTTP server, config, routes
├── config/               # Default configuration
├── docs/                 # Documentation
└── docker-compose.yml    # Development stack
```

The dependency graph is deliberately clean:

```
mnemo-server
├── mnemo-ingest
│   ├── mnemo-core
│   └── (uses traits from mnemo-core)
├── mnemo-retrieval
│   └── mnemo-core
├── mnemo-graph
│   └── mnemo-core
├── mnemo-storage
│   └── mnemo-core
└── mnemo-llm
    └── mnemo-core
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

- **Progressive summarization**: Auto-summarize long sessions to stay within token budgets
- **Benchmarks**: DMR and LongMemEval benchmark implementations
- **TypeScript SDK**: Client library mirroring the Python SDK surface
- **Helm chart**: Kubernetes deployment (architecture is stateless — ready to scale)
- **Documentation**: Tutorials, worked examples, domain-specific integration guides

Already shipped and no longer needed:
- ~~RediSearch full-text search integration~~ — shipped in v0.2
- ~~Python SDK~~ — shipped in v0.3, full async + sync coverage
- ~~LangChain and LlamaIndex adapters~~ — shipped in v0.3
- ~~Integration tests with real Redis and Qdrant~~ — 91 tests in `tests/memory_api.rs`

---

## License

By contributing, you agree that your contributions will be licensed under the Apache 2.0 License.
