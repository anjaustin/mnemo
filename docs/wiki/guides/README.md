# Guides

How-to guides for specific features and use cases.

---

## Memory & Retrieval

| Guide | Description |
|-------|-------------|
| **[Context Assembly](context-assembly.md)** | Token-budgeted retrieval |
| **[Hybrid Search](hybrid-search.md)** | Semantic + graph + full-text |
| **[Temporal Queries](temporal-queries.md)** | Point-in-time recall |
| **[Memory Contracts](memory-contracts.md)** | Predefined retrieval policies |
| **[Reranking](reranking.md)** | RRF, MMR, GNN, Hyperbolic |

---

## Multi-Modal

| Guide | Description |
|-------|-------------|
| **[Image Memory](multi-modal/images.md)** | Vision processing |
| **[Audio Memory](multi-modal/audio.md)** | Transcription |
| **[Document Memory](multi-modal/documents.md)** | PDF and text parsing |

---

## Agent Architecture

| Guide | Description |
|-------|-------------|
| **[Agent Identity](agent-identity.md)** | Personality and versioning |
| **[Agent Branching](agent-branching.md)** | A/B testing personalities |
| **[Agent Forking](agent-forking.md)** | Creating derived agents |
| **[Experience Weighting](experience-weighting.md)** | EWC++ for memory importance |
| **[Memory Regions](memory-regions.md)** | Multi-agent shared memory |

---

## Governance & Security

| Guide | Description |
|-------|-------------|
| **[API Keys & RBAC](api-keys.md)** | Authentication and authorization |
| **[Data Classification](data-classification.md)** | Four-tier labeling |
| **[Memory Views](memory-views.md)** | Filtered access policies |
| **[Guardrails](guardrails.md)** | Content filtering rules |

---

## Integrations

| Guide | Description |
|-------|-------------|
| **[LangChain](integrations/langchain.md)** | Python integration |
| **[LlamaIndex](integrations/llamaindex.md)** | Python integration |
| **[Vercel AI SDK](integrations/vercel-ai.md)** | TypeScript integration |
| **[MCP Server](integrations/mcp.md)** | Claude Code integration |
| **[Webhooks](webhooks.md)** | Event notifications |

---

## Common Tasks

### Store and Retrieve

```python
client.add("user", "Important information")
ctx = client.context("user", "What do you know?")
```

### Query Historical State

```python
ctx = client.context("user", "What was true?", as_of="2025-01-15")
```

### Filter by Modality

```python
ctx = client.context("user", "Show images", include_modalities=["image"])
```

### Use a Memory Contract

```python
ctx = client.context("user", "Current state only", contract="current_strict")
```
