# Contributing

Join the Mnemo project.

---

## In This Section

| Guide | Description |
|-------|-------------|
| **[Development Setup](setup.md)** | Local dev environment |
| **[Code Structure](code-structure.md)** | Crate organization |
| **[Testing](testing.md)** | Test commands and falsification |
| **[Pull Requests](pull-requests.md)** | PR guidelines |

---

## Quick Start

```bash
# Clone
git clone https://github.com/anjaustin/mnemo.git
cd mnemo

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Start dependencies
docker compose -f deploy/docker/docker-compose.dev.yml up -d

# Build
cargo build

# Test
cargo test

# Run
cargo run --bin mnemo-server
```

---

## Code of Conduct

Be respectful. We follow the [Contributor Covenant](https://www.contributor-covenant.org/).

---

## Reporting Issues

1. Check existing issues first
2. Include reproduction steps
3. Include version info (`curl localhost:8080/health`)
4. Include relevant logs

---

## Pull Request Process

1. Fork the repository
2. Create a feature branch
3. Make changes with tests
4. Run `cargo test` and `cargo clippy`
5. Submit PR with description
6. Address review feedback

---

## Development Tips

### Useful Commands

```bash
# Format code
cargo fmt

# Lint
cargo clippy --workspace

# Run specific test
cargo test test_name

# Run with debug logging
RUST_LOG=debug cargo run --bin mnemo-server

# Generate docs
cargo doc --open
```

### Local Testing

```bash
# Quick health check
curl http://localhost:8080/health

# Test memory storage
curl -X POST http://localhost:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"test","text":"Hello world"}'
```

---

## License

Contributions are licensed under Apache 2.0.
