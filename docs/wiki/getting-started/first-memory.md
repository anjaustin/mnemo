# Your First Memory

A detailed walkthrough of storing and retrieving memories with Mnemo.

---

## Overview

This guide walks you through:

1. Creating a user and session
2. Storing messages as episodes
3. Understanding automatic extraction
4. Retrieving context
5. Exploring the knowledge graph

---

## Step 1: Create a User

Users are the top-level tenant in Mnemo. All data is isolated per-user.

```bash
curl -X POST http://localhost:8080/api/v1/users \
  -H "Content-Type: application/json" \
  -d '{"external_id": "alice"}'
```

Response:
```json
{
  "id": "019abc12-3456-7890-abcd-ef1234567890",
  "external_id": "alice",
  "created_at": "2025-03-18T10:00:00Z",
  "updated_at": "2025-03-18T10:00:00Z"
}
```

Save the `id` - you'll need it for subsequent calls.

> **Tip**: The simplified `/api/v1/memory` endpoint creates users automatically if they don't exist.

---

## Step 2: Create a Session

Sessions represent conversation threads. A user can have many sessions.

```bash
curl -X POST http://localhost:8080/api/v1/users/019abc12-3456-7890-abcd-ef1234567890/sessions \
  -H "Content-Type: application/json" \
  -d '{}'
```

Response:
```json
{
  "id": "019abc12-3456-7890-abcd-ef1234567891",
  "user_id": "019abc12-3456-7890-abcd-ef1234567890",
  "episode_count": 0,
  "created_at": "2025-03-18T10:00:00Z",
  "updated_at": "2025-03-18T10:00:00Z"
}
```

---

## Step 3: Add Episodes

Episodes are the atomic units of memory. Let's add a conversation:

### User message

```bash
curl -X POST http://localhost:8080/api/v1/sessions/019abc12-3456-7890-abcd-ef1234567891/episodes \
  -H "Content-Type: application/json" \
  -d '{
    "user_id": "019abc12-3456-7890-abcd-ef1234567890",
    "episode_type": "message",
    "role": "user",
    "content": "I just started using Notion for project management. We migrated from Asana last week."
  }'
```

### Assistant message

```bash
curl -X POST http://localhost:8080/api/v1/sessions/019abc12-3456-7890-abcd-ef1234567891/episodes \
  -H "Content-Type: application/json" \
  -d '{
    "user_id": "019abc12-3456-7890-abcd-ef1234567890",
    "episode_type": "message",
    "role": "assistant",
    "content": "Got it! Notion is a great choice for project management. How is the team adapting to the change from Asana?"
  }'
```

### Another user message

```bash
curl -X POST http://localhost:8080/api/v1/sessions/019abc12-3456-7890-abcd-ef1234567891/episodes \
  -H "Content-Type: application/json" \
  -d '{
    "user_id": "019abc12-3456-7890-abcd-ef1234567890",
    "episode_type": "message",
    "role": "user",
    "content": "Pretty well! Sarah from marketing especially loves the database views."
  }'
```

---

## Step 4: What Mnemo Extracted

Mnemo automatically processes each episode in the background. Let's see what it found.

### View entities

```bash
curl http://localhost:8080/api/v1/users/019abc12-3456-7890-abcd-ef1234567890/entities
```

```json
{
  "data": [
    {
      "id": "...",
      "name": "Notion",
      "entity_type": "product",
      "mention_count": 2
    },
    {
      "id": "...",
      "name": "Asana",
      "entity_type": "product",
      "mention_count": 1
    },
    {
      "id": "...",
      "name": "Sarah",
      "entity_type": "person",
      "mention_count": 1
    },
    {
      "id": "...",
      "name": "marketing",
      "entity_type": "organization",
      "mention_count": 1
    }
  ]
}
```

### View edges (facts)

```bash
curl "http://localhost:8080/api/v1/users/019abc12-3456-7890-abcd-ef1234567890/edges?current_only=true"
```

```json
{
  "data": [
    {
      "source_entity": "the team",
      "target_entity": "Notion",
      "label": "uses",
      "fact": "The team uses Notion for project management",
      "valid_at": "2025-03-18T10:00:00Z",
      "confidence": 0.95
    },
    {
      "source_entity": "the team",
      "target_entity": "Asana",
      "label": "migrated_from",
      "fact": "The team migrated from Asana to Notion last week",
      "valid_at": "2025-03-11T00:00:00Z",
      "confidence": 0.90
    },
    {
      "source_entity": "Sarah",
      "target_entity": "marketing",
      "label": "works_in",
      "fact": "Sarah works in marketing",
      "confidence": 0.85
    },
    {
      "source_entity": "Sarah",
      "target_entity": "Notion database views",
      "label": "likes",
      "fact": "Sarah loves the database views in Notion",
      "confidence": 0.90
    }
  ]
}
```

---

## Step 5: Retrieve Context

Now let's ask Mnemo a question:

```bash
curl -X POST http://localhost:8080/api/v1/users/019abc12-3456-7890-abcd-ef1234567890/context \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [{"role": "user", "content": "What project management tool do we use?"}],
    "max_tokens": 500
  }'
```

Response:
```json
{
  "text": "## Memory Context\n\n### Key Facts\n- The team uses Notion for project management\n- The team migrated from Asana to Notion last week\n- Sarah from marketing loves the database views\n\n### Entities\n- Notion (product)\n- Asana (product)\n- Sarah (person)",
  "token_count": 87,
  "entities": [...],
  "facts": [...],
  "episodes": [...],
  "latency_ms": 45,
  "sources": ["semantic_search", "graph_traversal"]
}
```

---

## Step 6: Temporal Queries

Now let's see temporal memory in action. Add a new fact that supersedes an old one:

```bash
curl -X POST http://localhost:8080/api/v1/sessions/019abc12-3456-7890-abcd-ef1234567891/episodes \
  -H "Content-Type: application/json" \
  -d '{
    "user_id": "019abc12-3456-7890-abcd-ef1234567890",
    "episode_type": "message",
    "role": "user",
    "content": "Actually, we switched back to Asana yesterday. Notion was too complex for the team."
  }'
```

### Query current state

```bash
curl -X POST http://localhost:8080/api/v1/users/019abc12-3456-7890-abcd-ef1234567890/context \
  -H "Content-Type: application/json" \
  -d '{"messages": [{"role": "user", "content": "What PM tool do we use?"}]}'
```

Now returns: "The team uses Asana"

### Query historical state

```bash
curl -X POST http://localhost:8080/api/v1/users/019abc12-3456-7890-abcd-ef1234567890/context \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [{"role": "user", "content": "What PM tool did we use last week?"}],
    "as_of": "2025-03-15T00:00:00Z"
  }'
```

Returns: "The team uses Notion" (the state at that point in time)

---

## Step 7: View Change History

Track how facts changed over time:

```bash
curl "http://localhost:8080/api/v1/users/019abc12-3456-7890-abcd-ef1234567890/changes?since=2025-03-01T00:00:00Z"
```

```json
{
  "gained": [
    {
      "fact": "The team uses Asana",
      "valid_at": "2025-03-17T00:00:00Z"
    }
  ],
  "superseded": [
    {
      "fact": "The team uses Notion for project management",
      "superseded_by": "The team uses Asana",
      "invalid_at": "2025-03-17T00:00:00Z"
    }
  ]
}
```

---

## Next Steps

- **[SDK Setup](sdk-setup.md)** - Use Python or TypeScript clients
- **[Temporal Model](../concepts/temporal-model.md)** - Deep dive into bi-temporal facts
- **[Architecture](../reference/architecture.md)** - Retrieval pipeline details
- **[Usage Guide](../../USAGE.md)** - Full API usage examples
