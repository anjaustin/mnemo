# mnemo-client

Production-grade Python SDK for the [Mnemo](https://github.com/anjaustin/mnemo) memory API.

**Compatibility**: SDK v0.9.0 targets Mnemo server v0.9.0+.

Covers all memory, knowledge graph, LLM span tracing, memory digest, agent identity, governance,
webhooks, operator, import, and session-message endpoints. Zero runtime dependencies for the sync client.
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
result = client.add("jordan", "Acme Corp renewal is due 2025-09-30. Procurement requires SOC 2 Type II before signature.")
print(result.session_id)  # server-assigned UUID

# Retrieve context
ctx = client.context("jordan", "What are the Acme renewal blockers?")
print(ctx.text)           # assembled context string
print(ctx.token_count)    # token count of the context
```

## Knowledge Graph API

```python
# List entities in the user's knowledge graph
entities = client.graph_entities("jordan", limit=50)
for e in entities.data:
    print(e.name, e.entity_type, e.mention_count)

# Get entity with its adjacency (outgoing/incoming edges)
entity = client.graph_entity("jordan", entity_id="<uuid>")

# List edges
edges = client.graph_edges("jordan", valid_only=True, limit=100)
for e in edges.data:
    print(e.fact)

# 1-hop neighborhood of an entity
neighbors = client.graph_neighbors("jordan", "<entity_uuid>", depth=2)
print(f"{neighbors.entities_visited} entities visited")

# Community detection
communities = client.graph_community("jordan")
print(f"{communities.community_count} communities detected")
```

## Memory Digest (sleep-time compute)

```python
# Get or generate a memory digest (prose summary + topic extraction)
digest = client.memory_digest("jordan")
print(digest.summary)
print("Topics:", digest.dominant_topics)
print(f"{digest.entity_count} entities, {digest.edge_count} edges, model: {digest.model}")

# Force regeneration
digest = client.memory_digest("jordan", refresh=True)
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
        result = await client.add("jordan", "Acme renewal is blocked — procurement needs SOC 2 evidence before signature.")
        ctx = await client.context("jordan", "What is blocking the Acme renewal?")
        print(ctx.text)

asyncio.run(main())
```

## API reference

### Memory

```python
# Write a memory (creates user and session on first call)
result = client.add(
    user="jordan",
    text="Acme Corp procurement flagged SOC 2 Type II as a hard requirement before renewal signature.",
    session="acme-deal-room",   # optional session name
    role="user",              # "user" | "assistant" | "system" | "tool"
)
# result: RememberResult(ok, user_id, session_id, episode_id, request_id)

# Retrieve context
ctx = client.context(
    user="jordan",
    query="What still blocks the Acme renewal?",
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
ctx = client.context_head("jordan", "What is the latest on Acme?")

# Changes since a timestamp
changes = client.changes_since("jordan", from_dt="2025-01-01T00:00:00Z", to_dt="2025-03-01T00:00:00Z")
# changes: ChangesSinceResult(added_facts, superseded_facts, confidence_deltas,
#           head_changes, added_episodes, summary, from_dt, to_dt, request_id)

# Conflict radar
conflicts = client.conflict_radar("jordan")
# conflicts: ConflictRadarResult(conflicts, user_id, request_id)

# Causal recall
chains = client.causal_recall("jordan", "Why is the Acme renewal at risk?")
# chains: CausalRecallResult(chains, query, request_id)

# Time-travel trace (snapshot diff over a time window)
tt = client.time_travel_trace(
    "jordan", "How did Acme renewal risk evolve?",
    from_dt="2025-01-01T00:00:00Z",
    to_dt="2025-03-01T00:00:00Z",
)
# tt: TimeTravelTraceResult(snapshots, from_dt, to_dt, request_id)

# Time-travel summary (lightweight delta counts)
summary = client.time_travel_summary("jordan", "Acme deal status changes", from_dt="...", to_dt="...")
```

### Governance

```python
# Get policy
policy = client.get_policy("jordan")

# Set policy
policy = client.set_policy(
    "jordan",
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
preview = client.preview_policy("jordan", retention_days_message=30)
# preview: PolicyPreviewResult(estimated_episodes_affected, policy, request_id)

# Audit log
audit = client.get_policy_audit("jordan", limit=50)      # list[AuditRecord]
violations = client.get_policy_violations("jordan", from_dt="...", to_dt="...")
```

### Webhooks

```python
# Create a webhook
wh = client.create_webhook(
    "jordan",
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

### Agent Identity

```python
# Get or auto-create agent identity
identity = client.get_agent_identity("my-agent")
# identity: AgentIdentityResult(agent_id, version, core, created_at, updated_at, request_id)

# Update identity core (contamination-guarded: no user/session/email keys allowed)
identity = client.update_agent_identity("my-agent", core={
    "mission": "Assist account managers with deal intelligence and renewal risk tracking",
    "style": {"tone": "direct", "verbosity": "concise"},
    "boundaries": ["no financial advice", "no legal opinions"],
})

# Version history and audit trail
versions = client.list_agent_identity_versions("my-agent", limit=10)
audit = client.list_agent_identity_audit("my-agent", limit=20)

# Rollback to a previous version
identity = client.rollback_agent_identity("my-agent", target_version=2, reason="reverted experiment")

# Record an experience event (behavioral signal from runtime)
exp = client.add_agent_experience("my-agent",
    category="tone",
    signal="account manager requested deal-risk summaries before full context",
    confidence=0.85,
    weight=0.6,
    decay_half_life_days=30,
)
# exp: ExperienceEventResult(id, agent_id, category, signal, confidence, weight, ...)

# Promotion proposals (evidence-gated identity evolution)
proposal = client.create_promotion_proposal("my-agent",
    proposal="lead with risk summary before full deal context",
    candidate_core={"mission": "Assist account managers with deal intelligence", "style": {"tone": "direct", "lead_with": "risk_summary"}},
    reason="3+ sessions showed account managers act on risk flags first",
    source_event_ids=[exp1.id, exp2.id, exp3.id],  # must reference real experience events
)
# proposal: PromotionProposalResult(id, status="pending", ...)

proposals = client.list_promotion_proposals("my-agent", limit=10)
approved = client.approve_promotion("my-agent", proposal.id)    # applies candidate_core
rejected = client.reject_promotion("my-agent", proposal.id, reason="insufficient evidence")

# Full agent context (identity + experience + user memory in one call)
ctx = client.agent_context("my-agent",
    query="What are the open risks on Jordan's accounts?",
    user="jordan",
    max_tokens=500,
)
# ctx: AgentContextResult(identity_version, experience_events_used, experience_weight_sum,
#                         user_memory_items_used, context, identity, request_id)
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
    user="jordan",
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
    user_id="jordan",
    client=client,
)

history.add_user_message("What are the open blockers on the Acme renewal?")
history.add_ai_message("Procurement requires SOC 2 Type II evidence before signature. Legal has cleared redlines.")
print(history.messages)  # [HumanMessage(...), AIMessage(...)]
history.clear()

# Wire into a LangChain chain with session management
def get_history(session_id: str) -> MnemoChatMessageHistory:
    return MnemoChatMessageHistory(session_id, "jordan", client)

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
store = MnemoChatStore(client=client, user_id="jordan")

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
result = client.add("jordan", "Acme legal cleared redlines.", request_id="req-abc123")
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
| `dev` | `pip install mnemo-client[dev]` | pytest, pytest-asyncio, aioresponses, requests |
