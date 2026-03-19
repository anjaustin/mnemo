# MCP Server Guide

Mnemo includes a native [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) server that exposes memory operations as tools and resources for any MCP-compatible AI agent.

---

## Overview

The MCP server allows AI agents like Claude, GPT, and open-source models to:

- **Remember** information persistently across sessions
- **Recall** relevant context using semantic search
- **Relate** entities in a knowledge graph
- **Evolve** agent identity through experience recording

```
Agent (Claude/GPT/etc.)
        |
        v
   MCP Protocol (JSON-RPC 2.0)
        |
    +---+---+
    |       |
  stdio    SSE/HTTP
    |       |
    v       v
   mnemo-mcp-server
        |
        v
   Mnemo HTTP API
        |
        v
   Memory Storage (Redis + Qdrant)
```

The MCP server supports two transports:
- **stdio** — For local tool integration (Claude Code, Cursor)
- **SSE/HTTP** — For web-based agents and remote integrations

---

## Quick Start

### 1. Build the MCP Server

```bash
# Stdio transport (default)
cargo build --release -p mnemo-mcp

# With SSE transport support
cargo build --release -p mnemo-mcp --features sse
```

Binaries:
- `target/release/mnemo-mcp-server` — stdio transport
- `target/release/mnemo-mcp-sse` — SSE/HTTP transport (requires `sse` feature)

### 2. Configure Environment

```bash
export MNEMO_MCP_BASE_URL="http://localhost:8080"  # Mnemo server
export MNEMO_MCP_DEFAULT_USER="your-user-id"       # Default user for memory
export MNEMO_API_KEY="your-api-key"                # Optional: API key
export MNEMO_MCP_AGENT_ID="my-assistant"           # Optional: Agent identity
```

### 3. Run Standalone

**Stdio transport:**
```bash
mnemo-mcp-server
```

The server reads JSON-RPC messages from stdin and writes responses to stdout.

**SSE transport:**
```bash
# Start SSE server (requires --features sse build)
MNEMO_MCP_SSE_HOST=0.0.0.0 \
MNEMO_MCP_SSE_PORT=3000 \
mnemo-mcp-sse
```

SSE transport provides:
- `POST /message` — Send JSON-RPC requests
- `GET /sse` — SSE stream for notifications
- `GET /health` — Health check

### 4. Integrate with Claude Desktop

Add to your Claude Desktop MCP settings (`~/.config/claude/mcp.json` or equivalent):

```json
{
  "mcpServers": {
    "mnemo": {
      "command": "/path/to/mnemo-mcp-server",
      "env": {
        "MNEMO_MCP_BASE_URL": "http://localhost:8080",
        "MNEMO_MCP_DEFAULT_USER": "your-user-id",
        "MNEMO_MCP_AGENT_ID": "claude-assistant"
      }
    }
  }
}
```

---

## Configuration

| Environment Variable | Description | Default |
|---------------------|-------------|---------|
| `MNEMO_MCP_BASE_URL` | Mnemo HTTP server URL | `http://localhost:8080` |
| `MNEMO_API_KEY` | API key for Mnemo authentication | None |
| `MNEMO_MCP_DEFAULT_USER` | Default user identifier for memory operations | None (required per-call) |
| `MNEMO_MCP_AGENT_ID` | Agent identifier for identity binding | None |
| `MNEMO_MCP_SESSION` | Session name for grouping operations | `mcp-session` |
| `RUST_LOG` | Log level (logs go to stderr) | `warn` |

**SSE transport only:**

| Environment Variable | Description | Default |
|---------------------|-------------|---------|
| `MNEMO_MCP_SSE_HOST` | Host to bind SSE server | `127.0.0.1` |
| `MNEMO_MCP_SSE_PORT` | Port to bind SSE server | `3000` |
| `MNEMO_MCP_SSE_CORS` | Comma-separated CORS origins | Permissive |

---

## Tools

The MCP server exposes 13 tools:

### Core Memory Tools

#### `remember`
Store information in long-term memory.

```json
{
  "name": "remember",
  "arguments": {
    "text": "Alice works at Acme Corp as a software engineer",
    "user": "user-123",
    "session": "conversation-1"
  }
}
```

#### `recall`
Search memory for relevant context.

```json
{
  "name": "recall",
  "arguments": {
    "query": "Where does Alice work?",
    "user": "user-123",
    "max_tokens": 1000
  }
}
```

#### `forget`
Delete a specific memory episode (requires reason for audit).

```json
{
  "name": "forget",
  "arguments": {
    "episode_id": "550e8400-e29b-41d4-a716-446655440000",
    "reason": "User requested deletion",
    "user": "user-123"
  }
}
```

### Graph Tools

#### `graph`
Query the knowledge graph.

```json
{
  "name": "graph",
  "arguments": {
    "operation": "list_entities",
    "user": "user-123",
    "entity_type": "person",
    "limit": 50
  }
}
```

Operations: `list_entities`, `list_edges`, `communities`

#### `relate`
Create or query entity relationships.

```json
{
  "name": "relate",
  "arguments": {
    "action": "connect",
    "source": "Alice",
    "target": "Acme Corp",
    "relation": "works_at",
    "user": "user-123"
  }
}
```

Actions:
- `connect` — Create an edge between entities
- `neighbors` — List connected entities (with optional `depth`)
- `path` — Find shortest path between two entities

### Agent Identity Tools

#### `identity`
Get or update agent identity profile.

```json
{
  "name": "identity",
  "arguments": {
    "agent_id": "my-assistant",
    "action": "get"
  }
}
```

#### `experience`
Record a learning experience for identity evolution.

```json
{
  "name": "experience",
  "arguments": {
    "agent_id": "my-assistant",
    "category": "tone",
    "signal": "User prefers concise, direct responses",
    "confidence": 0.85,
    "evidence_episode_ids": ["episode-1", "episode-2"]
  }
}
```

Experience events feed into the EWC++ identity evolution pipeline. Over time, accumulated experiences can be promoted to identity updates through the approval workflow.

### Memory Scope Tools

#### `delegate`
Grant another agent access to a memory scope.

```json
{
  "name": "delegate",
  "arguments": {
    "region_name": "shared-knowledge",
    "target_agent_id": "helper-bot",
    "permission": "read",
    "user": "user-123"
  }
}
```

#### `revoke`
Revoke delegated access.

```json
{
  "name": "revoke",
  "arguments": {
    "region_id": "region-uuid",
    "target_agent_id": "helper-bot"
  }
}
```

#### `scopes`
List visible memory scopes.

```json
{
  "name": "scopes",
  "arguments": {
    "user": "user-123",
    "agent_id": "my-assistant"
  }
}
```

### Utility Tools

#### `digest`
Get or generate a prose memory digest.

```json
{
  "name": "digest",
  "arguments": {
    "action": "get",
    "user": "user-123"
  }
}
```

#### `coherence`
Get a coherence report for the knowledge graph.

```json
{
  "name": "coherence",
  "arguments": {
    "user": "user-123"
  }
}
```

#### `health`
Check Mnemo server health.

```json
{
  "name": "health",
  "arguments": {}
}
```

---

## Resources

The MCP server exposes 11 resource templates:

### User Memory Resources

| URI Template | Description |
|--------------|-------------|
| `mnemo://users/{user}/memory` | Knowledge graph summary and coherence |
| `mnemo://users/{user}/episodes` | Recent memory episodes (default: 20) |
| `mnemo://users/{user}/entities` | Entities in the knowledge graph (default: 50) |
| `mnemo://users/{user}/search?q={query}` | Semantic memory search |
| `mnemo://users/{user}/digest` | Prose summary of knowledge graph |

### Episode Resources

| URI Template | Description |
|--------------|-------------|
| `mnemo://episodes/{episode_id}` | Single episode by ID with all turns |

### Agent Identity Resources

| URI Template | Description |
|--------------|-------------|
| `mnemo://agents/{agent_id}/identity` | Agent identity profile |
| `mnemo://agents/{agent_id}/experience` | Recent experience events (default: 20) |
| `mnemo://agents/{agent_id}/promotions` | Pending promotion proposals |

### Graph Resources

| URI Template | Description |
|--------------|-------------|
| `mnemo://users/{user}/graph/edges` | Entity relationships in knowledge graph |
| `mnemo://users/{user}/graph/communities` | Detected graph communities |

### Example Resource Read

```json
{
  "method": "resources/read",
  "params": {
    "uri": "mnemo://users/user-123/memory"
  }
}
```

### Resource Subscriptions

Subscribe to resource changes (notifications delivered via SSE):

```json
{
  "method": "resources/subscribe",
  "params": {
    "uri": "mnemo://users/user-123/memory"
  }
}
```

Unsubscribe:

```json
{
  "method": "resources/unsubscribe",
  "params": {
    "uri": "mnemo://users/user-123/memory"
  }
}
```

**Note:** Subscriptions require SSE transport for notification delivery. With stdio transport, subscriptions are acknowledged but notifications cannot be pushed.

---

## Prompts

The MCP server provides 5 prompt templates for common memory workflows:

| Prompt | Description |
|--------|-------------|
| `memory-context` | Load relevant memories for a topic as conversation context |
| `memory-summary` | Generate a summary of what's known about a user |
| `identity-reflection` | Reflect on agent identity and suggest improvements |
| `entity-analysis` | Analyze an entity and its relationships in the graph |
| `remember-conversation` | Generate memory-optimized summary of a conversation |

### Listing Prompts

```json
{
  "method": "prompts/list",
  "id": 1
}
```

### Using a Prompt

```json
{
  "method": "prompts/get",
  "params": {
    "name": "memory-context",
    "arguments": {
      "topic": "project deadlines",
      "user": "alice"
    }
  }
}
```

### Example: Memory Context

The `memory-context` prompt retrieves relevant memories and formats them as conversation context:

```json
{
  "method": "prompts/get",
  "params": {
    "name": "memory-context",
    "arguments": {
      "topic": "Alice's work preferences",
      "user": "user-123"
    }
  }
}
```

Response includes a message with retrieved memories:

```json
{
  "result": {
    "description": "Memory context for topic: Alice's work preferences",
    "messages": [
      {
        "role": "user",
        "content": {
          "type": "text",
          "text": "Here is relevant context from memory about \"Alice's work preferences\":\n\n[retrieved memories]\n\nPlease use this context to inform your response."
        }
      }
    ]
  }
}
```

---

## Agent Identity Binding

When `MNEMO_MCP_AGENT_ID` is set, the MCP session is bound to an agent identity. This enables:

### 1. Agent-Scoped Memory

Without a user ID, the agent can have its own memory space:

```bash
export MNEMO_MCP_AGENT_ID="knowledge-bot"
# No MNEMO_MCP_DEFAULT_USER set
```

The agent gets a synthetic user ID derived from its agent_id, keeping its memories separate from user memories.

### 2. Experience Recording

Record experiences that influence identity over time:

```json
{
  "name": "experience",
  "arguments": {
    "agent_id": "knowledge-bot",
    "category": "domain",
    "signal": "Users frequently ask about Python async patterns",
    "confidence": 0.9
  }
}
```

### 3. Identity Evolution

Experiences accumulate and can be promoted to identity updates:

1. **Record experiences** via the `experience` tool
2. **Create proposals** via the Mnemo API
3. **Review and approve** through the governance workflow
4. **Identity updates** applied after approval

See [Agent Identity Substrate](../../AGENT_IDENTITY_SUBSTRATE.md) for details.

---

## Security

### Input Validation

All user-provided identifiers are validated:
- Path traversal prevention (`..`, `/`, `\` rejected)
- Maximum length limits (256 characters)
- Null byte injection prevention

### Authentication

The MCP server forwards API keys to the Mnemo server:

```bash
export MNEMO_API_KEY="your-api-key"
```

### Audit Trail

All operations are logged through the Mnemo audit system:
- Memory operations tracked per user
- Agent identity changes recorded with witness chain
- Deletions require a reason

---

## Examples

### Conversation with Memory

```
User: "Remember that my favorite programming language is Rust"

Agent: [Calls remember tool]
  remember({ text: "User's favorite programming language is Rust", user: "..." })

Agent: "Got it! I'll remember that Rust is your favorite programming language."

--- Later session ---

User: "What's my favorite language?"

Agent: [Calls recall tool]
  recall({ query: "favorite programming language", user: "..." })

  Returns: [{ content: "User's favorite programming language is Rust", ... }]

Agent: "Your favorite programming language is Rust!"
```

### Building a Knowledge Graph

```
User: "Alice is the CEO of TechCorp. Bob reports to Alice."

Agent: [Calls remember, then relate]
  remember({ text: "Alice is the CEO of TechCorp. Bob reports to Alice." })
  relate({ action: "connect", source: "Alice", target: "TechCorp", relation: "ceo_of" })
  relate({ action: "connect", source: "Bob", target: "Alice", relation: "reports_to" })

User: "Who does Bob report to?"

Agent: [Calls relate]
  relate({ action: "neighbors", source: "Bob" })

  Returns: [{ target: "Alice", relation: "reports_to" }]

Agent: "Bob reports to Alice."
```

### Agent Learning

```
[After observing user preferences across multiple sessions]

Agent: [Calls experience tool]
  experience({
    agent_id: "assistant-v1",
    category: "communication",
    signal: "User prefers detailed technical explanations with code examples",
    confidence: 0.88,
    evidence_episode_ids: ["ep-1", "ep-2", "ep-3"]
  })

[This experience accumulates with others, eventually becoming a promotion proposal
 that updates the agent's identity to prefer technical communication style]
```

---

## Troubleshooting

### "No user specified" Error

Set the default user:

```bash
export MNEMO_MCP_DEFAULT_USER="your-user-id"
```

Or provide `user` in each tool call.

### Connection Refused

Ensure Mnemo server is running:

```bash
curl http://localhost:8080/health
```

### Logs

Enable debug logging:

```bash
RUST_LOG=debug mnemo-mcp-server
```

Logs go to stderr, not stdout (stdout is reserved for MCP protocol).

---

## Protocol Details

The MCP server implements:

- **JSON-RPC 2.0** message format
- **Transports**: stdio (stdin/stdout) and SSE/HTTP
- **MCP Protocol Version**: 2025-03-26

### Supported Methods

| Method | Description |
|--------|-------------|
| `initialize` | Capability negotiation |
| `tools/list` | List available tools |
| `tools/call` | Execute a tool |
| `resources/list` | List static resources |
| `resources/templates/list` | List resource templates |
| `resources/read` | Read a resource |
| `resources/subscribe` | Subscribe to resource updates |
| `resources/unsubscribe` | Unsubscribe from resource updates |
| `ping` | Health check |

### Server Capabilities

```json
{
  "capabilities": {
    "tools": { "listChanged": false },
    "resources": { "listChanged": false, "subscribe": true }
  }
}
```

### SSE Transport Endpoints

When running with SSE transport:

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/message` | POST | Send JSON-RPC request, receive response |
| `/sse` | GET | SSE stream for server notifications |
| `/health` | GET | Health check endpoint |

Example SSE client (JavaScript):

```javascript
// Connect to SSE stream
const events = new EventSource('http://localhost:3000/sse');

events.addEventListener('connected', (e) => {
  const { sessionId } = JSON.parse(e.data);
  console.log('Connected:', sessionId);
});

events.onmessage = (e) => {
  const notification = JSON.parse(e.data);
  if (notification.method === 'notifications/resources/updated') {
    console.log('Resource updated:', notification.params.uri);
  }
};

// Send a request
fetch('http://localhost:3000/message', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    jsonrpc: '2.0',
    id: 1,
    method: 'tools/list'
  })
}).then(r => r.json()).then(console.log);
```

---

## Next Steps

- [API Reference](../api/README.md) — Full HTTP API documentation
- [Agent Identity](../../AGENT_IDENTITY_SUBSTRATE.md) — Identity evolution details
- [Architecture](../reference/architecture.md) — System internals
