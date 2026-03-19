# Temporal Model

How Mnemo handles facts that change over time.

---

## Overview

Most databases treat data as current state - you update a row and the old value is gone. Mnemo is different: it's a **bi-temporal** knowledge graph where every fact has two time dimensions.

This enables:
- **Point-in-time queries** - "What did we know on January 15th?"
- **Change tracking** - "What changed in the last week?"
- **Audit trails** - "When did we learn this? When did it become true?"

---

## Two Time Dimensions

Every edge (fact) in Mnemo has two timestamps:

### Event Time (`valid_at`)

When the fact **became true in the real world**.

Example: "The project deadline is March 15th" - the deadline was set on some date, maybe when the project was planned.

### Ingestion Time (`created_at`)

When Mnemo **learned about the fact**.

Example: You told Mnemo about the deadline on February 1st, even though it was set in January.

```
Real world:     Jan 10: Deadline set to March 15
Mnemo learns:   Feb 1:  "The project deadline is March 15th"

Edge:
  valid_at: 2025-01-10 (when it became true)
  created_at: 2025-02-01 (when Mnemo learned it)
```

---

## Fact Lifecycle

### 1. Creation

A new fact is created with `valid_at` and no `invalid_at`:

```json
{
  "subject": "project",
  "predicate": "has_deadline",
  "object": "March 15th",
  "valid_at": "2025-01-10T00:00:00Z",
  "invalid_at": null,
  "confidence": 0.95
}
```

### 2. Supersession

When a contradicting fact arrives, the old edge is **superseded** (not deleted):

```
Old edge:
  "project → has_deadline → March 15th"
  valid_at: Jan 10
  invalid_at: Feb 20  ← now set

New edge:
  "project → has_deadline → March 30th"
  valid_at: Feb 20
  invalid_at: null    ← current
```

### 3. Confidence Decay

Over time, unreinforced facts lose confidence:

```
confidence(t) = initial_confidence × e^(-t/half_life) + fisher_floor
```

- `half_life` defaults to 30 days
- `fisher_floor` protects important facts from full decay
- Facts below threshold trigger revalidation suggestions

---

## Temporal Queries

### Current State (default)

Without parameters, Mnemo returns currently-valid facts:

```bash
POST /api/v1/users/{id}/context
{
  "messages": [{"role": "user", "content": "When is the deadline?"}]
}
```

Returns: "March 30th" (the current deadline)

### Point-in-Time (`as_of`)

Query the state at a specific moment:

```bash
POST /api/v1/users/{id}/context
{
  "messages": [{"role": "user", "content": "When is the deadline?"}],
  "as_of": "2025-02-15T00:00:00Z"
}
```

Returns: "March 15th" (the deadline as of Feb 15)

### Temporal Intent Detection

Mnemo analyzes query language to detect intent:

| Query | Detected Intent | Behavior |
|-------|-----------------|----------|
| "What **is** the status?" | `current` | Only current facts |
| "What **was** the status in January?" | `historical` | Auto-sets `as_of` |
| "What **changed** last week?" | `recent` | Prioritizes recent modifications |
| "Tell me about the project" | `auto` | Balanced retrieval |

Override with `time_intent` parameter:
```json
{"time_intent": "current"}   // Only currently-valid
{"time_intent": "historical"} // Use as_of or recent history
{"time_intent": "recent"}     // Prioritize recent changes
```

---

## Change Tracking

### Changes Since

Get facts that changed after a timestamp:

```bash
GET /api/v1/users/{id}/changes?since=2025-02-01T00:00:00Z
```

```json
{
  "gained": [
    {
      "fact": "project has deadline March 30th",
      "valid_at": "2025-02-20T00:00:00Z",
      "source_episode_id": "..."
    }
  ],
  "superseded": [
    {
      "fact": "project has deadline March 15th",
      "valid_at": "2025-01-10T00:00:00Z",
      "invalid_at": "2025-02-20T00:00:00Z",
      "superseded_by": "project has deadline March 30th"
    }
  ]
}
```

### Belief Changes Query

More detailed change history:

```bash
GET /api/v1/users/{id}/edges/belief-changes?limit=20
```

Returns pairs of (old_edge, new_edge) showing what changed and when.

---

## Temporal Scopes

Facts can have different temporal behaviors:

### Mutable (default)

Can be superseded by new information. Most facts are mutable.

```json
{"temporal_scope": "mutable"}
```

### Stable

Resists decay and supersession. Use for foundational facts.

```json
{"temporal_scope": "stable"}
```

### Time-Bounded

Has an expected expiry:

```json
{
  "temporal_scope": {
    "type": "time_bounded",
    "expected_duration_days": 30,
    "expires_at": "2025-04-15T00:00:00Z"
  }
}
```

---

## Temporal Scoring

During retrieval, facts are scored by temporal relevance:

```
temporal_score = base_relevance × temporal_weight × recency_factor
```

Where:
- `temporal_weight` (0.0-1.0) - How much to weight recency
- `recency_factor` - Decay based on age

### Temporal Weight Override

```json
{
  "messages": [...],
  "temporal_weight": 0.8  // Strong recency preference
}
```

---

## Memory Contracts

Predefined temporal retrieval policies:

| Contract | Behavior |
|----------|----------|
| `default` | Balanced hybrid retrieval |
| `current_strict` | Only currently-valid facts |
| `historical_strict` | Requires explicit `as_of` |
| `support_safe` | Conservative, avoids uncertain facts |

```json
{"contract": "current_strict"}
```

---

## Temporal Diagnostics

Get insight into temporal scoring:

```json
{
  "messages": [...],
  "explain": true
}
```

Response includes:
```json
{
  "temporal_diagnostics": {
    "resolved_intent": "current",
    "temporal_weight": 0.3,
    "as_of": null,
    "entities_scored": 5,
    "facts_scored": 12,
    "episodes_scored": 8
  }
}
```

---

## Best Practices

### 1. Include timestamps when relevant

If you know when something happened, include it:

```
"The project deadline was changed to March 30th on February 20th."
```

Mnemo will extract `valid_at: Feb 20` instead of using ingestion time.

### 2. Use structured events for precision

For business events, use JSON episodes:

```json
{
  "episode_type": "json",
  "content": {
    "event": "deadline_changed",
    "project_id": "proj-123",
    "old_deadline": "2025-03-15",
    "new_deadline": "2025-03-30",
    "changed_at": "2025-02-20T14:30:00Z"
  }
}
```

### 3. Query with appropriate intent

Don't force `current` if you need history. Let Mnemo detect intent or specify explicitly.

### 4. Use contracts for consistency

If your app always needs current state, use `current_strict` contract.

---

## Next Steps

- **[Entities & Edges](entities-and-edges.md)** - Knowledge graph structure
- **[Architecture](../reference/architecture.md)** - Retrieval pipeline details
- **[Capabilities](../../CAPABILITIES.md)** - Memory contracts and retrieval policies
