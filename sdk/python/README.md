# mnemo-client

Production-grade Python SDK for the [Mnemo](https://github.com/anjaustin/mnemo) memory API.

Covers all memory, knowledge graph, LLM span tracing, memory digest, governance, webhooks, operator,
import, and session-message endpoints. Zero runtime dependencies for the sync client.
Drop-in LangChain and LlamaIndex adapters included.

## Install

```bash
pip install mnemo-client
```

Or from source:

```bash
# Core sync client (zero dependencies)
pip install -e sdk/python

# With async support (aiohttp)
pip install -e "sdk/python[async]"

# With LangChain adapter
pip install -e "sdk/python[langchain]"

# With LlamaIndex adapter
pip install -e "sdk/python[llamaindex]"

# Everything
pip install -e "sdk/python[all]"
```

## Quick start

```python
from mnemo import Mnemo

client = Mnemo("http://localhost:8080")

# Store a memory
result = client.add("alice", "I love hiking in Colorado and skiing in Utah.")
print(result.session_id)  # server-assigned UUID

# Retrieve context
ctx = client.context("alice", "What does Alice enjoy outdoors?")
print(ctx.text)           # assembled context string
print(ctx.token_count)    # token count of the context
```

## Knowledge Graph API

```python
# List entities in the user's knowledge graph
entities = client.graph_entities("alice", limit=50)
for e in entities.data:
    print(e.name, e.entity_type, e.mention_count)

# Get entity with its adjacency (outgoing/incoming edges)
entity = client.graph_entity("alice", entity_id="<uuid>")

# List edges
edges = client.graph_edges("alice", valid_only=True, limit=100)
for e in edges.data:
    print(e.fact)

# 1-hop neighborhood of an entity
neighbors = client.graph_neighbors("alice", "<entity_uuid>", depth=2)
print(f"{neighbors.entities_visited} entities visited")

# Community detection
communities = client.graph_community("alice")
print(f"{communities.community_count} communities detected")
```

## Memory Digest (sleep-time compute)

```python
# Get or generate a memory digest (prose summary + topic extraction)
digest = client.memory_digest("alice")
print(digest.summary)
print("Topics:", digest.dominant_topics)
print(f"{digest.entity_count} entities, {digest.edge_count} edges, model: {digest.model}")

# Force regeneration
digest = client.memory_digest("alice", refresh=True)
```

## LLM Span Tracing

```python
# Look up all LLM calls made during a specific request
spans = client.spans_by_request("019cc15a-5470-7711-8d51-a3af1ace5522")
print(f"{spans.count} spans, {spans.total_tokens} tokens total")
for s in spans.spans:
    print(s.operation, s.model, s.total_tokens, s.latency_ms, "ms")
```

## Production client options

```python
from mnemo import Mnemo

client = Mnemo(
    base_url="https://mnemo.example.com",
    api_key="sk-...",        # sent as Authorization: Bearer <key>
    timeout_s=20.0,
    max_retries=3,
    retry_backoff_s=0.5,
    request_id="req-abc123", # default correlation ID for all calls
)
```

## Async client

```python
import asyncio
from mnemo import AsyncMnemo

async def main():
    async with AsyncMnemo("http://localhost:8080") as client:
        result = await client.add("alice", "I prefer mountains over beaches.")
        ctx = await client.context("alice", "What scenery does Alice prefer?")
        print(ctx.text)

asyncio.run(main())
```

## API reference

### Memory

```python
# Write a memory (creates user and session on first call)
result = client.add(
    user="alice",
    text="I love hiking in Colorado.",
    session="chat-2024-01",   # optional session name
    role="user",              # "user" | "assistant" | "system" | "tool"
)
# result: RememberResult(ok, user_id, session_id, episode_id, request_id)

# Retrieve context
ctx = client.context(
    user="alice",
    query="What does Alice enjoy outdoors?",
    session=None,             # restrict to a session (UUID)
    max_tokens=2000,
    min_relevance=0.3,
    mode="hybrid",            # "hybrid" | "historical" | "head"
    contract="support_safe",  # memory contract name
    retrieval_policy="precision",
    time_intent="last_week",
    as_of="2024-11-01T00:00:00Z",
    temporal_weight=0.7,
    filters={"role": "user"},
)
# ctx: ContextResult(text, token_count, entities, facts, episodes, latency_ms,
#                    sources, mode, head, contract_applied,
#                    retrieval_policy_applied, temporal_diagnostics,
#                    retrieval_policy_diagnostics, request_id)

# Head context (fast path — most recent session only)
ctx = client.context_head("alice", "What is Alice working on?")

# Changes since a timestamp
changes = client.changes_since("alice", from_dt="2024-11-01T00:00:00Z", to_dt="2024-12-01T00:00:00Z")
# changes: ChangesSinceResult(added_facts, superseded_facts, confidence_deltas,
#           head_changes, added_episodes, summary, from_dt, to_dt, request_id)

# Conflict radar
conflicts = client.conflict_radar("alice")
# conflicts: ConflictRadarResult(conflicts, user_id, request_id)

# Causal recall
chains = client.causal_recall("alice", "Why did Alice change jobs?")
# chains: CausalRecallResult(chains, query, request_id)

# Time-travel trace (snapshot diff over a time window)
tt = client.time_travel_trace(
    "alice", "What changed about Alice's preferences?",
    from_dt="2024-10-01T00:00:00Z",
    to_dt="2024-12-01T00:00:00Z",
)
# tt: TimeTravelTraceResult(snapshots, from_dt, to_dt, request_id)

# Time-travel summary (lightweight delta counts)
summary = client.time_travel_summary("alice", "preference changes", from_dt="...", to_dt="...")
```

### Governance

```python
# Get policy
policy = client.get_policy("alice")

# Set policy
policy = client.set_policy(
    "alice",
    retention_days_message=90,
    retention_days_text=365,
    retention_days_json=30,
    webhook_domain_allowlist=["example.com"],
    default_memory_contract="support_safe",
    default_retrieval_policy="precision",
)
# policy: PolicyResult(user_id, retention_days_*, webhook_domain_allowlist,
#                      default_memory_contract, default_retrieval_policy,
#                      created_at, updated_at, request_id)

# Preview impact before applying
preview = client.preview_policy("alice", retention_days_message=30)
# preview: PolicyPreviewResult(estimated_episodes_affected, policy, request_id)

# Audit log
audit = client.get_policy_audit("alice", limit=50)      # list[AuditRecord]
violations = client.get_policy_violations("alice", from_dt="...", to_dt="...")
```

### Webhooks

```python
# Create a webhook
wh = client.create_webhook(
    "alice",
    target_url="https://hooks.example.com/mnemo",
    events=["fact_added", "fact_superseded"],
    signing_secret="my-secret",
)
# wh: WebhookResult(id, user_id, target_url, events, enabled, created_at, updated_at)

# Inspect
wh = client.get_webhook(wh.id)
events = client.get_webhook_events(wh.id, limit=50)     # list[WebhookEvent]
dl = client.get_dead_letter_events(wh.id)               # list[WebhookEvent]
stats = client.get_webhook_stats(wh.id, window_seconds=300)
# stats: WebhookStats(webhook_id, window_seconds, delivered, failed, dead_letter)

# Replay from cursor
replay = client.replay_events(wh.id, after_event_id="evt-abc", limit=100)
# replay: ReplayResult(replayed, events, request_id)

# Retry a specific failed event
retry = client.retry_event(wh.id, event_id="evt-xyz", force=False)

# Audit
audit = client.get_webhook_audit(wh.id, limit=20)       # list[AuditRecord]

# Delete
client.delete_webhook(wh.id)
```

### Operator

```python
# Ops summary (live metrics)
summary = client.ops_summary(window_seconds=300)
# summary: OpsSummaryResult(http_requests_total, http_responses_2xx/4xx/5xx,
#                           policy_updates, policy_violations, webhook_delivered,
#                           webhook_failed, webhook_dead_letter, governance_events)

# Cross-pipeline trace by request correlation ID
trace = client.trace_lookup("req-abc123", from_dt="...", to_dt="...", limit=100)
# trace: TraceLookupResult(request_id, episodes, webhook_events,
#                          webhook_audit, governance_audit, sdk_request_id)
```

### Import

```python
# Start an async chat history import job
job = client.import_chat_history(
    user="alice",
    source="ndjson",          # "ndjson" | "chatgpt_export" | "gemini_export"
    payload_data={...},
    idempotency_key="import-2024-11",
    dry_run=False,
    default_session="main",
)
# job: ImportJobResult(id, source, user, dry_run, status, total_messages,
#                      imported_messages, failed_messages, sessions_touched,
#                      errors, created_at, started_at, finished_at)

status = client.get_import_job(job.id)
```

### Session messages (framework adapter primitives)

```python
# Get messages for a session (UUID) in chronological order
msgs = client.get_messages(session_id, limit=100, after="episode-uuid")
# msgs: MessagesResult(messages=[Message(idx, id, role, content, created_at)], count, session_id)

# Clear all messages in a session (without deleting the session)
client.clear_messages(session_id)

# Delete a message at ordinal index (0-based)
client.delete_message(session_id, idx=1)
```

### Health

```python
h = client.health()
# h: HealthResult(status, version, request_id)
```

## LangChain adapter

```python
from mnemo import Mnemo
from mnemo.ext.langchain import MnemoChatMessageHistory
from langchain_core.runnables.history import RunnableWithMessageHistory

client = Mnemo("http://localhost:8080")

# Create history for a session
history = MnemoChatMessageHistory(
    session_name="chat-2024-11",
    user_id="alice",
    client=client,
)

history.add_user_message("Hello!")
history.add_ai_message("Hello! How can I help?")
print(history.messages)  # [HumanMessage(...), AIMessage(...)]
history.clear()

# Wire into a LangChain chain with session management
def get_history(session_id: str) -> MnemoChatMessageHistory:
    return MnemoChatMessageHistory(session_id, "alice", client)

chain_with_history = RunnableWithMessageHistory(
    chain,
    get_history,
    input_messages_key="input",
    history_messages_key="history",
)
```

**Session name vs UUID**: The `session_name` argument is the human-readable name
passed to `add()`. After the first write, the adapter caches the server-assigned
UUID so subsequent reads via `get_messages()` use the correct endpoint.

## LlamaIndex adapter

```python
from mnemo import Mnemo
from mnemo.ext.llamaindex import MnemoChatStore
from llama_index.core.memory import ChatMemoryBuffer
from llama_index.core.llms import ChatMessage, MessageRole

client = Mnemo("http://localhost:8080")
store = MnemoChatStore(client=client, user_id="alice")

# Wire into a LlamaIndex chat engine
memory = ChatMemoryBuffer.from_defaults(
    token_limit=3000,
    chat_store=store,
    chat_store_key="my-chat-session",
)

# Direct store usage
store.add_message("ses-1", ChatMessage(role=MessageRole.USER, content="Hi!"))
msgs = store.get_messages("ses-1")    # list[ChatMessage]
store.delete_message("ses-1", idx=0)
store.delete_messages("ses-1")        # clear all
keys = store.get_keys()               # list of session names written this instance
```

All 7 `BaseChatStore` abstract methods are implemented, plus async variants
(`aset_messages`, `aget_messages`, `aadd_message`, `adelete_messages`,
`adelete_message`, `adelete_last_message`, `aget_keys`).

## Error handling

```python
from mnemo._errors import (
    MnemoError,          # base
    MnemoHttpError,      # non-2xx with status_code + body
    MnemoRateLimitError, # 429; carries retry_after_ms
    MnemoNotFoundError,  # 404
    MnemoValidationError,# 400 with validation_error code
    MnemoConnectionError,# network failure
    MnemoTimeoutError,   # request timed out
)

try:
    client.delete_message(session_id, idx=99)
except MnemoValidationError as e:
    print(e.status_code, e.body)
except MnemoRateLimitError as e:
    print(f"Back off for {e.retry_after_ms}ms")
```

## Request-ID tracing

Every method accepts and returns a `request_id` for end-to-end correlation:

```python
result = client.add("alice", "Hello", request_id="req-abc123")
print(result.request_id)  # echoed from x-mnemo-request-id response header

# Later: find all pipeline events for that request
trace = client.trace_lookup("req-abc123")
print(trace.episodes, trace.webhook_events, trace.governance_audit)
```

## Running tests

```bash
# Against a running server on localhost:8080
make test-local

# Full Docker-backed run (builds server image, starts stack, tears down)
make test

# Async client unit tests (mocked transport, no server required)
pytest tests/test_async_client.py -v
```

## Extras

| Extra | Install | Provides |
|-------|---------|---------|
| `async` | `pip install mnemo-client[async]` | `AsyncMnemo` via aiohttp |
| `langchain` | `pip install mnemo-client[langchain]` | `MnemoChatMessageHistory` |
| `llamaindex` | `pip install mnemo-client[llamaindex]` | `MnemoChatStore` |
| `all` | `pip install mnemo-client[all]` | Everything above |
| `dev` | `pip install mnemo-client[dev]` | pytest, pytest-asyncio, requests |
