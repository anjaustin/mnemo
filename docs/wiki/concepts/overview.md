# Core Concepts

An overview of Mnemo's data model and architecture.

---

## The Big Picture

Mnemo is a **memory infrastructure** for AI agents. It solves three problems:

1. **Storage** - Where do agent memories go?
2. **Retrieval** - How do you find relevant memories efficiently?
3. **Temporality** - How do you handle facts that change over time?

```
┌─────────────────────────────────────────────────────────────┐
│                        Your AI Agent                         │
└─────────────────────────────────────────────────────────────┘
                              │
                    remember() │ recall()
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                         Mnemo                                │
│                                                              │
│  ┌──────────┐   ┌──────────────┐   ┌──────────────────────┐ │
│  │ Episodes │──▶│ Knowledge    │──▶│ Context Assembly     │ │
│  │ (raw)    │   │ Graph        │   │ (token-budgeted)     │ │
│  └──────────┘   │ (extracted)  │   └──────────────────────┘ │
│                 └──────────────┘                             │
│                                                              │
│  ┌──────────┐   ┌──────────────┐   ┌──────────────────────┐ │
│  │ Sessions │   │ Temporal     │   │ Hybrid Search        │ │
│  │          │   │ Model        │   │ (semantic+graph+FTS) │ │
│  └──────────┘   └──────────────┘   └──────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

---

## Data Hierarchy

```
User
 └── Session (conversation thread)
      └── Episode (message/event/document)
           └── Attachment (image/audio/document file)

User
 └── Entity (person/org/product/concept)
      └── Edge (fact connecting two entities)
```

### User

The top-level tenant. All data is isolated per-user.

- Users have an `external_id` (your system's user ID)
- A user can have many sessions and entities
- Data never leaks between users

### Session

A conversation thread or context window.

- Sessions contain ordered episodes
- Sessions can have a narrative summary (auto-generated)
- Use session scope for conversation-specific retrieval

### Episode

The atomic unit of memory ingestion.

- **message** - Chat messages with role (user/assistant/system/tool)
- **json** - Structured events (CRM data, telemetry, etc.)
- **text** - Unstructured text (documents, transcripts)

### Entity

A node in the knowledge graph.

- Automatically extracted from episodes
- Types: person, organization, product, location, event, concept, custom
- Deduplicated across the user's history
- Tracks mention count and aliases

### Edge

A temporal fact connecting two entities.

- Has `valid_at` (when it became true) and `invalid_at` (when superseded)
- Contains the natural language fact and a structured label
- Confidence score with decay over time
- This is where Mnemo's temporal model lives

---

## The Knowledge Graph

When you store an episode, Mnemo:

1. **Extracts entities** - People, products, concepts mentioned
2. **Extracts relationships** - How entities relate
3. **Creates edges** - Temporal facts with validity windows
4. **Generates embeddings** - Vector representations for search

```
Episode: "Sarah from marketing loves the new Notion database views."

Extracts:
  Entities: Sarah (person), marketing (organization), Notion (product)
  
  Edges:
    Sarah → works_in → marketing
    Sarah → likes → Notion database views
```

The graph grows as more episodes are added. Entities are deduplicated (if "Sarah" is mentioned again, the same entity is reused).

---

## Temporal Model

Most databases overwrite data. Mnemo keeps history.

### Fact Supersession

When a fact changes, the old edge is marked `invalid_at` and a new edge is created:

```
March 1:  "Team uses Notion"
          → Edge: team → uses → Notion (valid_at: March 1)

March 15: "Team switched to Asana"
          → Edge: team → uses → Notion (invalid_at: March 15)  ← superseded
          → Edge: team → uses → Asana  (valid_at: March 15)    ← current
```

### Point-in-Time Queries

Use `as_of` to query historical state:

```bash
# What tool does the team use now?
GET /context?query=What PM tool?
→ "Asana"

# What tool did they use on March 10?
GET /context?query=What PM tool?&as_of=2025-03-10T00:00:00Z
→ "Notion"
```

### Temporal Intent

Mnemo detects temporal intent from queries:

- "What **is** the status?" → Current facts only
- "What **was** the status last month?" → Historical query
- "What **changed** recently?" → Recent modifications

---

## Retrieval Pipeline

Context assembly combines multiple retrieval strategies:

```
Query: "What does Sarah think about the project tools?"
                    │
    ┌───────────────┼───────────────┐
    ▼               ▼               ▼
┌────────┐    ┌──────────┐    ┌─────────┐
│Semantic│    │Full-Text │    │ Graph   │
│Search  │    │Search    │    │Traversal│
└───┬────┘    └────┬─────┘    └────┬────┘
    │              │               │
    └──────────────┴───────────────┘
                   │
                   ▼
            ┌──────────────┐
            │   Reranking   │
            │  (RRF/MMR/GNN)│
            └──────┬───────┘
                   │
                   ▼
            ┌──────────────┐
            │Token Budgeting│
            │ (max_tokens)  │
            └──────┬───────┘
                   │
                   ▼
            Context Block
```

### Search Types

- **Semantic** - Vector similarity (embedding cosine distance)
- **Full-Text** - Keyword matching (RediSearch)
- **Graph** - BFS traversal from query entities
- **Hybrid** - All three combined (default)

### Reranking

- **RRF** - Reciprocal Rank Fusion (default, merges ranked lists)
- **MMR** - Maximal Marginal Relevance (diversity-focused)
- **GNN** - Graph Attention Network on local subgraphs
- **Hyperbolic** - Poincare ball for hierarchical structures

---

## Multi-Tenancy

Mnemo supports multiple isolation levels:

### User Isolation

All data is partitioned by user ID. No cross-user data access.

### Agent Isolation

Optional agent-scoped memory within a user:

```bash
# Store memory as a specific agent
POST /memory?agent_id=support-bot

# Retrieve only that agent's memories
GET /context?agent_id=support-bot
```

### Memory Regions

Named memory partitions with ACLs:

```bash
# Create a shared region
POST /regions
{"name": "team-knowledge", "owner": "agent-a"}

# Grant access to another agent
POST /regions/{id}/access
{"agent_id": "agent-b", "permission": "read"}
```

---

## Storage Architecture

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   Mnemo     │────▶│    Redis    │     │   Qdrant    │
│   Server    │     │  (state)    │     │  (vectors)  │
└─────────────┘     └─────────────┘     └─────────────┘
                           │
                           ▼
                    ┌─────────────┐
                    │ RediSearch  │
                    │ (full-text) │
                    └─────────────┘
```

- **Redis** - Users, sessions, episodes, entities, edges, sessions state
- **RediSearch** - Full-text search index (module in Redis)
- **Qdrant** - Vector embeddings for semantic search

Both services can be scaled independently. See [Deployment](../../DEPLOY.md).

---

## Next Steps

- **[Episodes](episodes.md)** - Deep dive into episode types
- **[Entities & Edges](entities-and-edges.md)** - Knowledge graph details
- **[Temporal Model](temporal-model.md)** - Bi-temporal facts
- **[Architecture](../reference/architecture.md)** - Full system internals
