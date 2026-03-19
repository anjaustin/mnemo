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
- **[Sessions](concepts/sessions.md)** - Conversation threads
- **[Users](concepts/users.md)** - Multi-tenant isolation

---

## Feature Guides

How to use specific features:

### Memory & Retrieval
- **[Context Assembly](guides/context-assembly.md)** - Token-budgeted retrieval
- **[Hybrid Search](guides/hybrid-search.md)** - Semantic + graph + full-text
- **[Temporal Queries](guides/temporal-queries.md)** - Point-in-time recall
- **[Memory Contracts](guides/memory-contracts.md)** - Predefined retrieval policies
- **[Reranking](guides/reranking.md)** - RRF, MMR, GNN, Hyperbolic

### Multi-Modal
- **[Image Memory](guides/multi-modal/images.md)** - Vision processing
- **[Audio Memory](guides/multi-modal/audio.md)** - Transcription
- **[Document Memory](guides/multi-modal/documents.md)** - PDF and text parsing

### Agent Architecture
- **[Agent Identity](guides/agent-identity.md)** - Personality and versioning
- **[Agent Branching](guides/agent-branching.md)** - A/B testing personalities
- **[Agent Forking](guides/agent-forking.md)** - Creating derived agents
- **[Experience Weighting](guides/experience-weighting.md)** - EWC++ for memory importance
- **[Memory Regions](guides/memory-regions.md)** - Multi-agent shared memory

### Governance & Security
- **[API Keys & RBAC](guides/api-keys.md)** - Authentication and authorization
- **[Data Classification](guides/data-classification.md)** - Four-tier labeling
- **[Memory Views](guides/memory-views.md)** - Filtered access policies
- **[Guardrails](guides/guardrails.md)** - Content filtering rules

### Integrations
- **[LangChain](guides/integrations/langchain.md)** - Python integration
- **[LlamaIndex](guides/integrations/llamaindex.md)** - Python integration
- **[Vercel AI SDK](guides/integrations/vercel-ai.md)** - TypeScript integration
- **[MCP Server](guides/integrations/mcp.md)** - Claude Code integration
- **[Webhooks](guides/webhooks.md)** - Event notifications

---

## API Reference

Complete API documentation:

- **[REST API](api/rest.md)** - All 142 endpoints
- **[gRPC API](api/grpc.md)** - Protobuf service definitions
- **[Python SDK](api/python-sdk.md)** - `mnemo` package reference
- **[TypeScript SDK](api/typescript-sdk.md)** - `mnemo-client` package reference
- **[MCP Tools](api/mcp-tools.md)** - Model Context Protocol tools
- **[Error Codes](api/errors.md)** - Error handling reference

---

## Deployment

Production deployment guides:

- **[Docker](deployment/docker.md)** - Local and single-server
- **[Kubernetes](deployment/kubernetes.md)** - Helm chart deployment
- **[AWS](deployment/aws.md)** - CloudFormation templates
- **[GCP](deployment/gcp.md)** - Cloud Run deployment
- **[DigitalOcean](deployment/digitalocean.md)** - App Platform
- **[Other Providers](deployment/other-providers.md)** - Render, Railway, Vultr, etc.

---

## Reference

Configuration and operations:

- **[Configuration](reference/configuration.md)** - Environment variables and TOML
- **[Architecture](reference/architecture.md)** - System internals
- **[Performance](reference/performance.md)** - Benchmarks and tuning
- **[Troubleshooting](reference/troubleshooting.md)** - Common issues
- **[Security](reference/security.md)** - Security controls and hardening

---

## Contributing

Join the project:

- **[Development Setup](contributing/setup.md)** - Local dev environment
- **[Code Structure](contributing/code-structure.md)** - Crate organization
- **[Testing](contributing/testing.md)** - Test commands and falsification
- **[Pull Requests](contributing/pull-requests.md)** - PR guidelines

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
