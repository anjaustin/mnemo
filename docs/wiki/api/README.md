# API Reference

Complete API documentation for Mnemo.

---

## In This Section

| Reference | Description |
|-----------|-------------|
| **[REST API](rest.md)** | All 142 HTTP endpoints |
| **[gRPC API](grpc.md)** | Protobuf service definitions |
| **[Python SDK](python-sdk.md)** | `mnemo` package reference |
| **[TypeScript SDK](typescript-sdk.md)** | `mnemo-client` package reference |
| **[MCP Tools](mcp-tools.md)** | Model Context Protocol tools |
| **[Error Codes](errors.md)** | Error handling reference |

---

## Quick Reference

### Base URL

```
http://localhost:8080
```

### Authentication

When `MNEMO_AUTH_ENABLED=true`:

```bash
# Bearer token (preferred)
curl -H "Authorization: Bearer YOUR_API_KEY" ...

# X-API-Key header
curl -H "X-API-Key: YOUR_API_KEY" ...
```

### Content Types

- REST: `application/json`
- gRPC: `application/grpc`

Both served on the same port.

---

## Common Endpoints

### Memory Operations

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/v1/memory` | Store memory (simple) |
| POST | `/api/v1/memory/{user}/context` | Get context (simple) |
| POST | `/api/v1/sessions/{id}/episodes` | Create episode |
| POST | `/api/v1/users/{id}/context` | Get context (full) |

### Knowledge Graph

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/users/{id}/entities` | List entities |
| GET | `/api/v1/users/{id}/edges` | List edges |
| GET | `/api/v1/entities/{id}/neighbors` | Graph traversal |

### Management

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/v1/users` | Create user |
| POST | `/api/v1/users/{id}/sessions` | Create session |
| GET | `/health` | Health check |

---

## SDK Quick Start

### Python

```python
from mnemo import Mnemo

client = Mnemo("http://localhost:8080", api_key="...")
client.add("user", "Remember this")
ctx = client.context("user", "What do you know?")
```

### TypeScript

```typescript
import { MnemoClient } from 'mnemo-client';

const client = new MnemoClient('http://localhost:8080', { apiKey: '...' });
await client.add('user', 'Remember this');
const ctx = await client.context('user', 'What do you know?');
```

---

## Response Format

All responses include:

```json
{
  "data": { ... },        // or array for list endpoints
  "request_id": "uuid"    // correlation ID
}
```

Error responses:

```json
{
  "error": {
    "code": "NOT_FOUND",
    "message": "User not found",
    "details": { ... }
  },
  "request_id": "uuid"
}
```

---

## Rate Limits

Default: 100 requests/second with burst of 200.

Rate-limited responses return `429 Too Many Requests` with:
```
Retry-After: <seconds>
X-RateLimit-Remaining: 0
```
