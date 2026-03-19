# SDK Setup

Install and configure the Python or TypeScript SDK.

---

## Python SDK

### Installation

```bash
pip install git+https://github.com/anjaustin/mnemo.git#subdirectory=sdk/python
```

Or with poetry:
```bash
poetry add git+https://github.com/anjaustin/mnemo.git#subdirectory=sdk/python
```

### Basic Usage

```python
from mnemo import Mnemo

# Connect to Mnemo
client = Mnemo("http://localhost:8080")

# Store a memory
result = client.add("alice", "I prefer dark mode for all my apps.")
print(f"Stored in episode: {result.episode_id}")

# Retrieve context
ctx = client.context("alice", "What are the user's preferences?")
print(ctx.text)
```

### With API Key

```python
client = Mnemo(
    "http://localhost:8080",
    api_key="your-api-key"
)
```

### Async Client

```python
from mnemo import AsyncMnemo
import asyncio

async def main():
    client = AsyncMnemo("http://localhost:8080")
    
    await client.add("alice", "Meeting at 3pm tomorrow.")
    ctx = await client.context("alice", "When is the meeting?")
    print(ctx.text)

asyncio.run(main())
```

### Session Management

```python
# Create a session
session = client.create_session("alice")

# Add to specific session
client.add("alice", "First message", session_id=session.id)
client.add("alice", "Second message", session_id=session.id)

# Get session context
ctx = client.context("alice", "What was discussed?", session_id=session.id)

# Get session HEAD (latest state summary)
head = client.context_head("alice", "Quick summary")
```

### Knowledge Graph Access

```python
# List entities
entities = client.entities("alice")
for entity in entities:
    print(f"{entity.name} ({entity.entity_type})")

# List facts (edges)
edges = client.edges("alice", current_only=True)
for edge in edges:
    print(f"{edge.source_entity} → {edge.label} → {edge.target_entity}")

# Graph traversal
neighbors = client.entity_neighbors("alice", entity_id, depth=2)
```

### Temporal Queries

```python
from datetime import datetime, timezone

# Point-in-time query
ctx = client.context(
    "alice",
    "What was the status?",
    as_of=datetime(2025, 1, 15, tzinfo=timezone.utc)
)

# Get changes since a date
changes = client.changes_since(
    "alice",
    since=datetime(2025, 1, 1, tzinfo=timezone.utc)
)
print(f"Gained: {len(changes.gained)} facts")
print(f"Superseded: {len(changes.superseded)} facts")
```

### Multi-Modal (v0.11.0+)

```python
# Upload an image
result = client.upload_attachment(
    episode_id="...",
    file_path="/path/to/image.png"
)
print(f"Attachment ID: {result['id']}")

# Get attachment with download URL
attachment = client.get_attachment(result['id'])
print(f"Download: {attachment['download_url']}")

# List attachments for an episode
attachments = client.list_attachments(episode_id)
```

---

## TypeScript SDK

### Installation

```bash
npm install mnemo-client
# or
yarn add mnemo-client
# or
pnpm add mnemo-client
```

### Basic Usage

```typescript
import { MnemoClient } from 'mnemo-client';

const client = new MnemoClient('http://localhost:8080');

// Store a memory
const result = await client.add('alice', 'I prefer TypeScript over JavaScript.');
console.log(`Stored in episode: ${result.episode_id}`);

// Retrieve context
const ctx = await client.context('alice', 'What languages does the user prefer?');
console.log(ctx.text);
```

### With API Key

```typescript
const client = new MnemoClient('http://localhost:8080', {
  apiKey: 'your-api-key'
});
```

### Session Management

```typescript
// Create a session
const session = await client.createSession('alice');

// Add to specific session
await client.add('alice', 'First message', { sessionId: session.id });
await client.add('alice', 'Second message', { sessionId: session.id });

// Get session context
const ctx = await client.context('alice', 'What was discussed?', {
  sessionId: session.id
});
```

### Knowledge Graph Access

```typescript
// List entities
const entities = await client.entities('alice');
for (const entity of entities.data) {
  console.log(`${entity.name} (${entity.entity_type})`);
}

// List facts
const edges = await client.edges('alice', { currentOnly: true });
for (const edge of edges.data) {
  console.log(`${edge.source_entity} → ${edge.label} → ${edge.target_entity}`);
}
```

### Temporal Queries

```typescript
// Point-in-time query
const ctx = await client.context('alice', 'What was the status?', {
  asOf: '2025-01-15T00:00:00Z'
});

// Get changes
const changes = await client.changesSince('alice', {
  since: '2025-01-01T00:00:00Z'
});
```

### Multi-Modal (v0.11.0+)

```typescript
// Get attachment with download URL
const attachment = await client.getAttachment(attachmentId);
console.log(`Download: ${attachment.download_url}`);

// List attachments
const attachments = await client.listAttachments(episodeId);
```

---

## Framework Integrations

### LangChain (Python)

```python
from mnemo.ext.langchain import MnemoMemory
from langchain.chains import ConversationChain
from langchain.llms import OpenAI

memory = MnemoMemory(
    base_url="http://localhost:8080",
    user_id="alice"
)

chain = ConversationChain(
    llm=OpenAI(),
    memory=memory
)

response = chain.predict(input="What do you know about me?")
```

### LlamaIndex (Python)

```python
from mnemo.ext.llamaindex import MnemoChatStore
from llama_index.core.memory import ChatMemoryBuffer

chat_store = MnemoChatStore(
    base_url="http://localhost:8080"
)

memory = ChatMemoryBuffer.from_defaults(
    chat_store=chat_store,
    chat_store_key="alice"
)
```

### LangChain.js

```typescript
import { MnemoMemory } from 'mnemo-client/langchain';
import { ChatOpenAI } from '@langchain/openai';
import { ConversationChain } from 'langchain/chains';

const memory = new MnemoMemory({
  baseUrl: 'http://localhost:8080',
  userId: 'alice'
});

const chain = new ConversationChain({
  llm: new ChatOpenAI(),
  memory
});
```

### Vercel AI SDK

```typescript
import { MnemoProvider } from 'mnemo-client/vercel';

const provider = new MnemoProvider({
  baseUrl: 'http://localhost:8080'
});

// Use with Vercel AI SDK
const result = await generateText({
  model: openai('gpt-4'),
  system: await provider.getContext('alice', 'current conversation')
});
```

---

## Configuration

### Environment Variables

Both SDKs respect these environment variables:

| Variable | Description |
|----------|-------------|
| `MNEMO_BASE_URL` | Server URL (default: `http://localhost:8080`) |
| `MNEMO_API_KEY` | API key for authentication |
| `MNEMO_TIMEOUT` | Request timeout in seconds (default: 30) |

### Python

```python
import os
os.environ["MNEMO_BASE_URL"] = "http://mnemo.example.com"
os.environ["MNEMO_API_KEY"] = "your-key"

from mnemo import Mnemo
client = Mnemo()  # Uses env vars
```

### TypeScript

```typescript
// Uses MNEMO_BASE_URL and MNEMO_API_KEY from environment
const client = new MnemoClient();
```

---

## Next Steps

- **[First Memory](first-memory.md)** - Detailed API walkthrough
- **[LangChain Integration](../guides/integrations/langchain.md)** - Full LangChain guide
- **[API Reference](../api/python-sdk.md)** - Complete SDK reference
