# Entities & Edges

The knowledge graph structure in Mnemo.

---

## Overview

Mnemo automatically extracts a **knowledge graph** from your episodes:

- **Entities** - Nodes representing people, products, concepts, etc.
- **Edges** - Relationships between entities (temporal facts)

This graph enables semantic queries like "What does Alice think about Project X?" without explicit tagging.

---

## Entities

An entity is a distinct concept worth tracking. Mnemo extracts entities automatically from episode content.

### Entity Types

| Type | Examples |
|------|----------|
| `person` | Alice, Bob, Dr. Smith |
| `organization` | Acme Corp, Marketing Team |
| `product` | iPhone, Notion, Project Alpha |
| `location` | New York, Conference Room A |
| `event` | Q3 Review, Product Launch |
| `concept` | Machine Learning, Budget Constraints |
| `custom` | Any domain-specific type |

### Entity Structure

```json
{
  "id": "019abc12-...",
  "name": "Alice",
  "entity_type": "person",
  "summary": "VP of Sales at Acme Corp",
  "aliases": ["Alice Smith", "A. Smith"],
  "mention_count": 15,
  "classification": "internal",
  "community_id": "019xyz...",
  "created_at": "2025-01-15T10:00:00Z",
  "updated_at": "2025-03-15T14:30:00Z"
}
```

### Entity Deduplication

Mnemo merges entities that refer to the same thing:

```
Episode 1: "Alice mentioned the project."
Episode 2: "Alice Smith from Sales called."
Episode 3: "A. Smith sent the report."

→ Single entity "Alice" with aliases ["Alice Smith", "A. Smith"]
```

### Querying Entities

```bash
# List all entities for a user
GET /api/v1/users/{user_id}/entities?limit=50

# Filter by type
GET /api/v1/users/{user_id}/entities?entity_type=person

# Get a specific entity
GET /api/v1/entities/{entity_id}

# Get entity neighbors (graph traversal)
GET /api/v1/entities/{entity_id}/neighbors?depth=2
```

---

## Edges (Facts)

An edge represents a **temporal fact** connecting two entities.

### Edge Structure

```json
{
  "id": "019def34-...",
  "source_entity_id": "019abc12-...",
  "target_entity_id": "019cde56-...",
  "label": "works_at",
  "fact": "Alice works at Acme Corp as VP of Sales",
  "confidence": 0.95,
  "valid_at": "2024-06-01T00:00:00Z",
  "invalid_at": null,
  "source_episode_id": "019ghi78-...",
  "classification": "internal",
  "temporal_scope": "mutable",
  "corroboration_count": 3,
  "access_count": 12
}
```

### Key Fields

| Field | Description |
|-------|-------------|
| `source_entity_id` | The "from" entity |
| `target_entity_id` | The "to" entity |
| `label` | Structured relationship type |
| `fact` | Natural language description |
| `confidence` | 0.0 - 1.0 certainty score |
| `valid_at` | When this became true |
| `invalid_at` | When superseded (null if current) |

### Common Labels

| Label | Example |
|-------|---------|
| `works_at` | Alice works at Acme |
| `manages` | Bob manages the Sales team |
| `owns` | Acme owns Product X |
| `prefers` | Alice prefers dark mode |
| `located_in` | Acme is located in NYC |
| `related_to` | Project X related to Budget |
| `deadline_is` | Project X deadline is March 15 |

### Querying Edges

```bash
# All current edges for a user
GET /api/v1/users/{user_id}/edges?current_only=true

# Filter by entity
GET /api/v1/users/{user_id}/edges?entity_id={id}

# Filter by label
GET /api/v1/users/{user_id}/edges?label=works_at

# Include historical (superseded) edges
GET /api/v1/users/{user_id}/edges?current_only=false
```

---

## Temporal Facts

Edges are **temporal** - they track when facts were true.

### Current vs. Superseded

```
January: "Alice works at Acme"
  → Edge: valid_at=Jan, invalid_at=null (current)

March: "Alice left Acme and joined Beta Corp"
  → Edge 1: valid_at=Jan, invalid_at=Mar (superseded)
  → Edge 2: valid_at=Mar, invalid_at=null (current)
```

### Querying Historical State

```bash
# Current facts only
GET /api/v1/users/{id}/edges?current_only=true

# All facts including superseded
GET /api/v1/users/{id}/edges?current_only=false

# Facts valid at a specific time
GET /api/v1/users/{id}/edges?as_of=2025-02-01T00:00:00Z
```

---

## Confidence Scoring

Edge confidence comes from multiple signals:

### Initial Confidence

Set by the extraction model based on linguistic certainty:
- "Alice definitely works at Acme" → 0.95
- "Alice might work at Acme" → 0.6
- "I think Alice works at Acme" → 0.7

### Corroboration

When multiple episodes mention the same fact:
```
effective_confidence = min(1.0, base_confidence × (1 + 0.1 × corroboration_count))
```

### Decay

Unreinforced facts decay over time:
```
confidence(t) = confidence_0 × e^(-t/half_life)
```

Default `half_life` is 30 days. High-importance facts resist decay.

### Fisher Importance

Facts with high "Fisher importance" (frequently accessed, highly connected) decay slower.

---

## Graph Traversal

Explore the knowledge graph:

### Neighbors

Get entities connected to a given entity:

```bash
GET /api/v1/entities/{entity_id}/neighbors?depth=2
```

```json
{
  "nodes": [
    {"id": "...", "name": "Alice", "entity_type": "person", "depth": 0},
    {"id": "...", "name": "Acme Corp", "entity_type": "organization", "depth": 1},
    {"id": "...", "name": "Sales Team", "entity_type": "organization", "depth": 2}
  ],
  "edges": [
    {"source": "Alice", "target": "Acme Corp", "label": "works_at"},
    {"source": "Acme Corp", "target": "Sales Team", "label": "contains"}
  ]
}
```

### Shortest Path

Find connection between two entities:

```bash
GET /api/v1/entities/{entity_id_1}/path/{entity_id_2}
```

```json
{
  "path": [
    {"entity": "Alice", "edge": "works_at"},
    {"entity": "Acme Corp", "edge": "owns"},
    {"entity": "Product X"}
  ],
  "length": 2
}
```

### Community Detection

Entities are clustered into communities:

```bash
GET /api/v1/users/{user_id}/communities
```

---

## Manual Edge Management

While edges are usually auto-extracted, you can manage them manually:

### Create Edge

```bash
POST /api/v1/edges
{
  "user_id": "...",
  "source_entity_id": "...",
  "target_entity_id": "...",
  "label": "reports_to",
  "fact": "Alice reports to Bob",
  "confidence": 0.9
}
```

### Update Edge

```bash
PATCH /api/v1/edges/{edge_id}
{
  "confidence": 0.95
}
```

### Invalidate Edge

Mark an edge as no longer true:

```bash
POST /api/v1/edges/{edge_id}/invalidate
{
  "reason": "Alice no longer reports to Bob"
}
```

### Delete Edge

Permanently remove (use sparingly - prefer invalidation):

```bash
DELETE /api/v1/edges/{edge_id}
```

---

## Classification

Entities and edges can be classified for access control:

| Level | Description |
|-------|-------------|
| `public` | Visible to all |
| `internal` | Default, normal access |
| `confidential` | Restricted access |
| `restricted` | Highly sensitive |

Classification affects retrieval based on caller's access level.

---

## Best Practices

### 1. Rich Content = Better Extraction

```
Good: "In the Q3 review meeting, Sarah (VP Engineering) announced 
       that Project Phoenix would launch in April."

Poor: "Project launches in April."
```

### 2. Consistent Entity References

Use consistent names to help deduplication:
- "Sarah Chen" not sometimes "S. Chen", sometimes "Sarah"
- "Project Phoenix" not "Phoenix project" or "the project"

### 3. Explicit Relationships

State relationships clearly:
- "Alice manages the Sales team" (clear)
- "Alice, Sales team" (ambiguous)

### 4. Use Labels for Queries

When querying, use labels for precision:
```bash
GET /edges?label=works_at  # More precise
GET /edges?entity_type=person  # Broader
```

---

## Next Steps

- **[Temporal Model](temporal-model.md)** - How facts change over time
- **[Architecture](../reference/architecture.md)** - Retrieval pipeline details
- **[Capabilities](../../CAPABILITIES.md)** - Full feature list including classification
