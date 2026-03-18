# Usage

Mnemo exposes a REST API, gRPC API (6 services / 30 RPCs), MCP server (stdio transport), and Python/TypeScript SDKs.

## Quick Start: High-Level Memory API

The simplest way to use Mnemo is with the high-level memory endpoints.

### Remember

```bash
curl -X POST http://localhost:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"jordan","text":"Acme Corp renewal is due on 2025-09-30 and procurement requires SOC 2 Type II before signature."}'
```

### Recall

```bash
curl -X POST http://localhost:8080/api/v1/memory/jordan/context \
  -H "Content-Type: application/json" \
  -d '{"query":"What are the renewal blockers for Acme?"}'
```

Inject the returned `context` string into your agent's system prompt.

## Memory Operations

### Diff Changes

```bash
curl -X POST http://localhost:8080/api/v1/memory/jordan/changes_since \
  -H "Content-Type: application/json" \
  -d '{"from":"2025-02-01T00:00:00Z","to":"2025-04-01T00:00:00Z"}'
```

### Detect Conflicts

```bash
curl -X POST http://localhost:8080/api/v1/memory/jordan/conflict_radar \
  -H "Content-Type: application/json" \
  -d '{}'
```

### Explain Retrieval

```bash
curl -X POST http://localhost:8080/api/v1/memory/jordan/causal_recall \
  -H "Content-Type: application/json" \
  -d '{"query":"Why do we think Acme has legal risk this quarter?"}'
```

### Time Travel Trace

```bash
curl -X POST http://localhost:8080/api/v1/memory/jordan/time_travel/trace \
  -H "Content-Type: application/json" \
  -d '{"query":"How did Acme renewal risk evolve?","from":"2025-02-01T00:00:00Z","to":"2025-04-01T00:00:00Z"}'
```

## Full Workflow: Users, Sessions, Episodes

Use this flow when you need explicit user/session lifecycle control.

```bash
# 1. Create a user
curl -X POST http://localhost:8080/api/v1/users \
  -H "Content-Type: application/json" \
  -d '{"name": "Jordan Lee", "email": "jordan.lee@acme.com"}'

# 2. Start a session
curl -X POST http://localhost:8080/api/v1/sessions \
  -H "Content-Type: application/json" \
  -d '{"user_id": "USER_ID_FROM_STEP_1"}'

# 3. Add messages
curl -X POST http://localhost:8080/api/v1/sessions/SESSION_ID/episodes \
  -H "Content-Type: application/json" \
  -d '{"type":"message","role":"user","name":"Jordan","content":"Acme legal approved redlines."}'

# 4. Get context (wait for processing)
curl -X POST http://localhost:8080/api/v1/users/USER_ID/context \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"What still blocks Acme renewal?"}]}'
```

## Webhooks

```bash
# Register a webhook
curl -X POST http://localhost:8080/api/v1/memory/webhooks \
  -H "Content-Type: application/json" \
  -d '{
    "user":"jordan",
    "target_url":"https://example.com/hooks/memory",
    "signing_secret":"whsec_demo",
    "events":["head_advanced","conflict_detected"]
  }'

# Check event delivery status
curl http://localhost:8080/api/v1/memory/webhooks/WEBHOOK_ID/events?limit=10
```

## Governance

```bash
# Set user policy
curl -X PUT http://localhost:8080/api/v1/policies/jordan \
  -H "Content-Type: application/json" \
  -d '{"webhook_domain_allowlist":["hooks.acme.example"],"retention_days_message":365}'

# Preview policy impact
curl -X POST http://localhost:8080/api/v1/policies/jordan/preview \
  -H "Content-Type: application/json" \
  -d '{"retention_days_message":30}'
```

## Import Chat History

Supported sources: `ndjson`, `chatgpt_export`, `gemini_export`.

```bash
curl -X POST http://localhost:8080/api/v1/import/chat-history \
  -H "Content-Type: application/json" \
  -d '{
    "user": "jordan",
    "source": "ndjson",
    "idempotency_key": "import-001",
    "payload": [
      {"role": "user", "content": "Acme requested SOC 2 report.", "created_at": "2025-02-01T10:00:00Z"},
      {"role": "assistant", "content": "Acknowledged.", "created_at": "2025-02-01T10:00:05Z"}
    ]
  }'

# Poll job status
curl http://localhost:8080/api/v1/import/jobs/JOB_ID
```

## Python SDK

### Install

```bash
pip install git+https://github.com/anjaustin/mnemo.git#subdirectory=sdk/python

# With extras
pip install "mnemo-client[async,langchain,llamaindex] @ git+https://github.com/anjaustin/mnemo.git#subdirectory=sdk/python"
```

### Basic Usage

```python
from mnemo import Mnemo

client = Mnemo("http://localhost:8080")

# Remember
client.add("jordan", "Acme renewal is at risk.")

# Recall
ctx = client.context("jordan", "What blocks Acme renewal?")
print(ctx.text)  # inject into agent system prompt
```

### Async Client

```python
from mnemo import AsyncMnemo

async with AsyncMnemo("http://localhost:8080") as client:
    await client.add("jordan", "Acme renewal is at risk.")
    ctx = await client.context("jordan", "What blocks renewal?")
```

### LangChain Adapter

```python
from mnemo import Mnemo
from mnemo.ext.langchain import MnemoChatMessageHistory

client = Mnemo("http://localhost:8080")
history = MnemoChatMessageHistory(
    session_name="acme-chat",
    user_id="jordan",
    client=client
)

history.add_user_message("What are the renewal blockers?")
history.add_ai_message("SOC 2 evidence is required.")

print(history.messages)
history.clear()
```

### LlamaIndex Adapter

```python
from mnemo import Mnemo
from mnemo.ext.llamaindex import MnemoChatStore
from llama_index.core.llms import ChatMessage, MessageRole

client = Mnemo("http://localhost:8080")
store = MnemoChatStore(client=client, user_id="jordan")

store.add_message("session", ChatMessage(role=MessageRole.USER, content="Hello"))
msgs = store.get_messages("session")
```

## TypeScript SDK

### Install

```bash
npm install mnemo-client
# or
yarn add mnemo-client
```

### Basic Usage

```typescript
import { MnemoClient } from 'mnemo-client';

const client = new MnemoClient('http://localhost:8080');

// Remember
await client.add('jordan', 'Acme renewal is at risk.');

// Recall
const ctx = await client.context('jordan', 'What blocks Acme renewal?');
console.log(ctx.text);
```

### LangChain.js Adapter

```typescript
import { MnemoChatMessageHistory } from 'mnemo-client/langchain';

const history = new MnemoChatMessageHistory({
  sessionName: 'acme-chat',
  userId: 'jordan',
  baseUrl: 'http://localhost:8080'
});

await history.addUserMessage('What are the blockers?');
await history.addAIMessage('SOC 2 evidence is required.');
```

### Vercel AI SDK

```typescript
import { mnemoRemember, mnemoRecall, mnemoDigest } from 'mnemo-client/vercel-ai';

const tools = {
  remember: mnemoRemember({ baseUrl: 'http://localhost:8080' }),
  recall: mnemoRecall({ baseUrl: 'http://localhost:8080' }),
  digest: mnemoDigest({ baseUrl: 'http://localhost:8080' })
};
```

## gRPC API

6 services with 30 RPCs. Proto schema at `proto/mnemo/v1/memory.proto`.

```bash
# Using grpcurl
grpcurl -plaintext localhost:8080 list

# Get context
grpcurl -plaintext -d '{"user_id":"jordan","query":"What blocks renewal?"}' \
  localhost:8080 mnemo.v1.MemoryService/GetContext
```

Set `MNEMO_GRPC_PORT=50051` for a dedicated gRPC port.

## MCP Server

Model Context Protocol for Claude Code and compatible clients.

```json
{
  "mcpServers": {
    "mnemo": {
      "command": "mnemo-mcp",
      "args": ["--base-url", "http://localhost:8080"]
    }
  }
}
```

Available tools: `mnemo_remember`, `mnemo_recall`, `mnemo_graph_query`, `mnemo_agent_identity`, `mnemo_digest`, `mnemo_coherence`, `mnemo_health`.

## API Reference

See [docs/API.md](API.md) for the complete endpoint reference.

## OpenAPI / Swagger

- OpenAPI 3.1 spec: `GET /api/v1/openapi.json`
- Swagger UI: `GET /swagger-ui/`
