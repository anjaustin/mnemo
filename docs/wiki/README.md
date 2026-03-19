# Mnemo Documentation

**Memory infrastructure for production AI agents.**

Mnemo is a free, open-source, self-hosted memory and context engine for agent systems. Built in Rust with Redis and Qdrant, it focuses on temporal correctness, fast recall, and operational simplicity.

---

## Quick Navigation

| Section | Description |
|---------|-------------|
| [Getting Started](getting-started/) | Installation, quickstart, first steps |
| [Concepts](concepts/) | Core ideas: episodes, entities, edges, temporal model |
| [Guides](guides/) | How-to guides for specific features |
| [API Reference](api/) | REST, gRPC, and SDK documentation |
| [Deployment](deployment/) | Docker, Kubernetes, cloud providers |
| [Reference](reference/) | Configuration, CLI, environment variables |
| [Contributing](contributing/) | Development setup, architecture, PR process |

---

## Getting Started

New to Mnemo? Start here:

1. **[Quickstart](getting-started/quickstart.md)** - Get running in 5 minutes
2. **[First Memory](getting-started/first-memory.md)** - Store and retrieve your first memory
3. **[Core Concepts](concepts/overview.md)** - Understand the data model
4. **[SDK Setup](getting-started/sdk-setup.md)** - Python and TypeScript clients

---

## Core Concepts

Understanding Mnemo's data model:

- **[Overview](concepts/overview.md)** - The big picture
- **[Episodes](concepts/episodes.md)** - The atomic unit of memory
- **[Entities & Edges](concepts/entities-and-edges.md)** - The knowledge graph
- **[Temporal Model](concepts/temporal-model.md)** - How facts change over time

For sessions and users, see the [Overview](concepts/overview.md).

---

## Feature Guides

For detailed feature documentation, see:

- **[Capabilities](../CAPABILITIES.md)** - Full feature list
- **[Usage Guide](../USAGE.md)** - API examples and integrations
- **[Architecture](reference/architecture.md)** - System internals

### Key Topics (in existing docs)
- Memory & Retrieval - See [Capabilities](../CAPABILITIES.md#memory--retrieval)
- Multi-Modal - See [Multi-Modal PRD](../MULTI_MODAL_PRD.md)
- Agent Identity - See [Agent Identity](../AGENT_IDENTITY_SUBSTRATE.md)
- Webhooks - See [Webhooks](../WEBHOOKS.md)
- Security - See [Security Controls](../SECURITY_CONTROLS.md)

---

## API Reference

Complete API documentation:

- **[REST API Reference](../API.md)** - All endpoints with examples
- **[Usage Guide](../USAGE.md)** - SDK examples and integrations
- **[Configuration](reference/configuration.md)** - Environment variables

---

## Deployment

Production deployment guides:

- **[Docker](deployment/docker.md)** - Local and single-server
- **[Kubernetes & Cloud](../DEPLOY.md)** - Helm chart and cloud providers

See the main [deploy/](../../deploy/) directory for platform-specific guides.

---

## Reference

Configuration and operations:

- **[Configuration](reference/configuration.md)** - Environment variables and TOML
- **[Architecture](reference/architecture.md)** - System internals
- **[Benchmarks](../BENCHMARKS.md)** - Performance measurements
- **[Troubleshooting](../TROUBLESHOOTING.md)** - Common issues
- **[Security](../SECURITY_CONTROLS.md)** - Security controls and hardening

---

## Contributing

Join the project:

- **[Quick Start](contributing/)** - Development setup
- **[Testing Guide](../TESTING.md)** - Test commands and falsification
- **[Contributing Guide](../../CONTRIBUTING.md)** - Full PR guidelines

---

## Version

This documentation covers **Mnemo v0.11.0**.

| Component | Version |
|-----------|---------|
| mnemo-server | 0.9.0 |
| Python SDK | 0.9.0 |
| TypeScript SDK | 0.9.0 |

---

## License

Apache 2.0 - see [LICENSE](../../LICENSE).
