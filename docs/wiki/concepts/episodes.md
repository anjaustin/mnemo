# Episodes

The atomic unit of memory in Mnemo.

---

## Overview

An **episode** is a single unit of information stored in Mnemo. It represents a moment in time - a message, an event, a document. Episodes are processed to extract entities and relationships that form the knowledge graph.

---

## Episode Types

### Message (`message`)

The most common type. Represents a chat message in a conversation.

```json
{
  "episode_type": "message",
  "role": "user",
  "content": "I just bought Nike running shoes for my marathon training.",
  "speaker_name": "Alice"
}
```

**Fields:**
- `role` - One of: `user`, `assistant`, `system`, `tool`
- `content` - The message text
- `speaker_name` - Optional speaker identifier

### JSON (`json`)

Structured event data. Ideal for CRM events, telemetry, or any structured input.

```json
{
  "episode_type": "json",
  "content": {
    "event": "purchase",
    "product": "Nike Air Max",
    "category": "running",
    "price": 149.99,
    "date": "2025-03-15"
  }
}
```

JSON episodes are converted to natural language for extraction:
> "A purchase event occurred: product Nike Air Max in category running for price 149.99 on date 2025-03-15."

### Text (`text`)

Unstructured text like documents, meeting notes, or transcripts.

```json
{
  "episode_type": "text",
  "content": "Meeting Notes - March 15, 2025\n\nAttendees: Alice, Bob, Carol\n\nDiscussed Q2 roadmap..."
}
```

---

## Creating Episodes

### Simplified API

The easiest way - auto-creates user and session:

```bash
POST /api/v1/memory
{
  "user": "alice",
  "text": "I prefer dark mode for all my apps."
}
```

### Full API

More control over structure:

```bash
# Create session first
POST /api/v1/users/{user_id}/sessions
{}

# Add episode to session
POST /api/v1/sessions/{session_id}/episodes
{
  "user_id": "...",
  "episode_type": "message",
  "role": "user",
  "content": "I prefer dark mode for all my apps."
}
```

---

## Episode Processing

When an episode is created, Mnemo's ingestion pipeline processes it:

```
Episode Created
      │
      ▼
┌──────────────┐
│   Digest     │  Summary for session context
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Extract    │  Entities and relationships
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Embed      │  Vector embeddings
└──────┬───────┘
       │
       ▼
┌──────────────┐
│ Graph Update │  Add to knowledge graph
└──────────────┘
```

### Processing Status

Check if an episode has been processed:

```bash
GET /api/v1/episodes/{episode_id}
```

```json
{
  "id": "...",
  "status": "completed",  // pending, processing, completed, failed
  "processed_at": "2025-03-15T10:00:05Z",
  "entity_ids": ["...", "..."],
  "edge_ids": ["...", "..."]
}
```

---

## Attachments (Multi-Modal)

Episodes can have file attachments (v0.11.0+):

### Image

```bash
POST /api/v1/episodes/{episode_id}/attachments
Content-Type: multipart/form-data

file=@screenshot.png
```

Images are processed with vision models to extract descriptions.

### Audio

```bash
POST /api/v1/episodes/{episode_id}/attachments
Content-Type: multipart/form-data

file=@meeting.mp3
```

Audio is transcribed and the transcript becomes searchable.

### Document

```bash
POST /api/v1/episodes/{episode_id}/attachments
Content-Type: multipart/form-data

file=@report.pdf
```

PDFs and text documents are parsed and chunked.

---

## Episode Metadata

Every episode includes:

| Field | Description |
|-------|-------------|
| `id` | UUIDv7 identifier |
| `user_id` | Owner user |
| `session_id` | Parent session |
| `episode_type` | `message`, `json`, or `text` |
| `content` | The raw content |
| `role` | For messages: user/assistant/system/tool |
| `speaker_name` | Optional speaker identifier |
| `status` | Processing status |
| `created_at` | When created |
| `processed_at` | When processing completed |
| `entity_ids` | Extracted entity IDs |
| `edge_ids` | Extracted edge IDs |

---

## Listing Episodes

### In a session

```bash
GET /api/v1/sessions/{session_id}/episodes?limit=20&offset=0
```

### For a user

```bash
GET /api/v1/users/{user_id}/episodes?limit=50
```

### With filters

```bash
GET /api/v1/users/{user_id}/episodes?role=user&since=2025-03-01
```

---

## Deleting Episodes

```bash
DELETE /api/v1/episodes/{episode_id}
```

This also removes associated entities and edges (if they have no other references).

---

## Episode Embedding

Each episode gets a vector embedding for semantic search:

1. Content is chunked if long
2. Each chunk is embedded using the configured provider
3. Embeddings are stored in Qdrant
4. Retrieval matches against these embeddings

### Embedding Providers

- **FastEmbed** (default) - Local, no API key needed
- **OpenAI** - `text-embedding-3-small` or `text-embedding-3-large`
- **Anthropic** - Via Voyage embeddings
- **Ollama** - Local models like `nomic-embed-text`

Configure via:
```bash
EMBEDDING_PROVIDER=openai
OPENAI_API_KEY=sk-...
```

---

## Best Practices

### 1. Use appropriate episode types

- Conversations → `message`
- Business events → `json`
- Documents → `text`

### 2. Include context

More context = better extraction:

```
Good: "In today's meeting, Sarah (VP of Sales) announced Q3 revenue of $5M."
Poor: "Revenue was $5M."
```

### 3. Use speaker names

For multi-party conversations:

```json
{
  "role": "user",
  "speaker_name": "Alice",
  "content": "I disagree with Bob's proposal."
}
```

### 4. Batch related messages

Keep related messages in the same session for better context.

---

## Next Steps

- **[Entities & Edges](entities-and-edges.md)** - What gets extracted
- **[Sessions](sessions.md)** - Episode containers
- **[Multi-Modal Guide](../guides/multi-modal/images.md)** - Attachments
