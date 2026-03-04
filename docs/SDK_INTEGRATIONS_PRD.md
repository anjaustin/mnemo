# SDK Integrations PRD

Status: P0 active
Owner: Platform / DX
Priority: P0
Last updated: 2026-03-04

## 1) Executive Summary

Mnemo's backend is decisively ahead of Zep on temporal correctness, governance, operability, and explainability. But prospects first touch a memory system through its SDK — specifically through LangChain and LlamaIndex integrations. Today our Python SDK covers 2 of 62 endpoints (3.2% surface area), has zero tests, no async support, and no framework adapters.

This PRD defines the execution plan to ship a production-grade Python SDK, a LangChain `MnemoChatMessageHistory` adapter, and a LlamaIndex `MnemoChatStore` adapter — all falsifiable, all tested against a live server in Docker.

Manifold conclusion: the SDK is the single highest-leverage distribution surface. Every week without framework adapters is a week where Zep's LangChain integration is the default choice for teams evaluating memory backends.

## 2) Problem Statement

1. **3.2% API coverage.** The existing SDK wraps only `POST /api/v1/memory` and `POST /api/v1/memory/:user/context`. The other 60 endpoints require raw HTTP from Python callers.
2. **No framework adapters.** LangChain requires `BaseChatMessageHistory`; LlamaIndex requires `BaseChatStore`. Neither can be implemented without raw message retrieval APIs that don't exist yet.
3. **No async.** The SDK uses `urllib.request` (sync only). FastAPI, async Django, and any modern Python service framework will block the event loop on every call.
4. **No tests.** Zero test coverage means no regression detection on SDK changes.
5. **Missing parameters.** `context()` omits `contract`, `retrieval_policy`, and `filters` — the differentiating features that make Mnemo better than Zep are invisible to SDK users.
6. **No request-id propagation.** The SDK neither sends nor reads `x-mnemo-request-id`, making distributed tracing impossible through the SDK.

## 3) Product Goals

1. **Drop-in LangChain adapter.** `from mnemo.ext.langchain import MnemoChatMessageHistory` works with any LangChain chain/agent.
2. **Drop-in LlamaIndex adapter.** `from mnemo.ext.llamaindex import MnemoChatStore` works with any LlamaIndex chat engine.
3. **Full high-level API coverage.** Every memory, governance, webhook, time-travel, and operator endpoint has a typed SDK method.
4. **Async-first with sync wrapper.** `AsyncMnemo` for async callers; `Mnemo` for sync callers. Both share the same interface.
5. **Request-id propagation.** Every SDK call can accept and returns a request ID for end-to-end tracing.
6. **Docker-based falsification.** SDK tests run against a real Mnemo server in Docker Compose, not mocks.

## 4) Non-Goals

- JavaScript/TypeScript SDK (future, not this PRD).
- Auto-generated SDK from OpenAPI spec (manual implementation gives better DX).
- SDK for admin/CRUD operations on Users, Sessions, Episodes, Entities, Edges (lower priority — operators use the dashboard or curl).
- PyPI publication (local install and GitHub-based pip install first).

## 5) Architecture

### 5.1) Server-Side API Gaps

Before the framework adapters can work, the server needs three new endpoints:

| Endpoint | Method | Purpose | Framework Need |
|----------|--------|---------|----------------|
| `GET /api/v1/sessions/:session_id/messages` | GET | Return raw messages (role + content + created_at) for a session, ordered chronologically. Paginated. | LangChain `messages` property, LlamaIndex `get_messages()` |
| `DELETE /api/v1/sessions/:session_id/messages` | DELETE | Clear all episode content for a session without deleting the session itself. | LangChain `clear()`, LlamaIndex `delete_messages()` |
| `DELETE /api/v1/sessions/:session_id/messages/:idx` | DELETE | Delete a specific message by ordinal index within the session. | LlamaIndex `delete_message(key, idx)` |

These are thin wrappers over existing storage: `list_episodes` already returns session episodes; we need a message-shaped projection and delete-by-index.

### 5.2) SDK Package Structure

```
sdk/python/
├── pyproject.toml          # mnemo-client, version 0.3.1
├── mnemo/
│   ├── __init__.py          # Exports Mnemo, AsyncMnemo, result types, errors
│   ├── client.py            # Sync client (Mnemo class)
│   ├── async_client.py      # Async client (AsyncMnemo class)
│   ├── _transport.py        # Shared HTTP transport (sync: urllib, async: aiohttp)
│   ├── _models.py           # Typed dataclasses for all result types
│   ├── _errors.py           # Exception hierarchy
│   ├── _compat.py           # Python version shims
│   └── ext/
│       ├── __init__.py
│       ├── langchain.py     # MnemoChatMessageHistory
│       └── llamaindex.py    # MnemoChatStore
└── tests/
    ├── conftest.py          # Docker compose fixtures
    ├── test_client.py       # Sync client tests
    ├── test_async_client.py # Async client tests
    ├── test_langchain.py    # LangChain adapter tests
    └── test_llamaindex.py   # LlamaIndex adapter tests
```

### 5.3) Dependency Strategy

| Component | Dependencies | Rationale |
|-----------|-------------|-----------|
| Core client (`mnemo.client`) | **None** (stdlib only) | Zero-install friction for simple use cases |
| Async client (`mnemo.async_client`) | `aiohttp` (optional) | Only needed for async callers |
| LangChain adapter (`mnemo.ext.langchain`) | `langchain-core>=0.2` (optional) | Only imported when using the adapter |
| LlamaIndex adapter (`mnemo.ext.llamaindex`) | `llama-index-core>=0.10` (optional) | Only imported when using the adapter |
| Tests | `pytest`, `pytest-asyncio`, `docker` | Dev-only |

All optional deps are declared as extras in `pyproject.toml`:

```toml
[project.optional-dependencies]
async = ["aiohttp>=3.9"]
langchain = ["langchain-core>=0.2"]
llamaindex = ["llama-index-core>=0.10"]
dev = ["pytest>=8", "pytest-asyncio>=0.23", "docker>=7"]
all = ["mnemo-client[async,langchain,llamaindex]"]
```

### 5.4) Client Interface Design

#### Sync Client (`Mnemo`)

```python
class Mnemo:
    def __init__(
        self,
        base_url: str = "http://localhost:8080",
        api_key: str | None = None,
        *,
        timeout_s: float = 20.0,
        max_retries: int = 2,
        retry_backoff_s: float = 0.4,
        request_id: str | None = None,  # default correlation ID for all calls
    ): ...

    # ── High-level memory ──────────────────────────────────────────
    def add(self, user, text, *, session=None, role="user") -> RememberResult: ...
    def context(self, user, query, *, session=None, max_tokens=None,
                min_relevance=None, mode=None, contract=None,
                retrieval_policy=None, time_intent=None, as_of=None,
                temporal_weight=None, filters=None) -> ContextResult: ...
    def changes_since(self, user, *, from_dt, to_dt, session=None) -> ChangesSinceResult: ...
    def conflict_radar(self, user) -> ConflictRadarResult: ...
    def causal_recall(self, user, query) -> CausalRecallResult: ...
    def time_travel_trace(self, user, query, *, from_dt, to_dt, session=None,
                          contract=None, retrieval_policy=None,
                          max_tokens=None, min_relevance=None) -> TimeTravelTraceResult: ...
    def time_travel_summary(self, user, query, *, from_dt, to_dt,
                            session=None) -> TimeTravelSummaryResult: ...

    # ── Governance ─────────────────────────────────────────────────
    def get_policy(self, user) -> PolicyResult: ...
    def set_policy(self, user, **kwargs) -> PolicyResult: ...
    def preview_policy(self, user, **kwargs) -> PolicyPreviewResult: ...
    def get_policy_audit(self, user, *, limit=50) -> list[AuditRecord]: ...
    def get_policy_violations(self, user, *, from_dt, to_dt, limit=50) -> list[AuditRecord]: ...

    # ── Webhooks ───────────────────────────────────────────────────
    def create_webhook(self, user, target_url, events, *,
                       signing_secret=None) -> WebhookResult: ...
    def get_webhook(self, webhook_id) -> WebhookResult: ...
    def delete_webhook(self, webhook_id) -> DeleteResult: ...
    def get_webhook_events(self, webhook_id, *, limit=20) -> list[WebhookEvent]: ...
    def get_dead_letter_events(self, webhook_id, *, limit=20) -> list[WebhookEvent]: ...
    def replay_events(self, webhook_id, *, after_event_id=None, limit=100,
                      include_delivered=True, include_dead_letter=True) -> ReplayResult: ...
    def retry_event(self, webhook_id, event_id, *, force=False) -> RetryResult: ...
    def get_webhook_stats(self, webhook_id, *, window_seconds=300) -> WebhookStats: ...
    def get_webhook_audit(self, webhook_id, *, limit=20) -> list[AuditRecord]: ...

    # ── Operator ───────────────────────────────────────────────────
    def ops_summary(self, *, window_seconds=300) -> OpsSummaryResult: ...
    def trace_lookup(self, request_id, *, from_dt=None, to_dt=None,
                     limit=100) -> TraceLookupResult: ...

    # ── Import ─────────────────────────────────────────────────────
    def import_chat_history(self, user, source, payload, *,
                            idempotency_key=None, dry_run=False,
                            default_session=None) -> ImportJobResult: ...
    def get_import_job(self, job_id) -> ImportJobResult: ...

    # ── Session messages (for framework adapters) ──────────────────
    def get_messages(self, session_id, *, limit=100, after=None) -> list[Message]: ...
    def clear_messages(self, session_id) -> DeleteResult: ...
    def delete_message(self, session_id, idx: int) -> DeleteResult: ...

    # ── Health ─────────────────────────────────────────────────────
    def health(self) -> HealthResult: ...
```

#### Async Client (`AsyncMnemo`)

Mirrors the sync client exactly with `async def` on every method. Uses `aiohttp` for transport.

### 5.5) LangChain Adapter

```python
from langchain_core.chat_history import BaseChatMessageHistory
from langchain_core.messages import BaseMessage, HumanMessage, AIMessage, SystemMessage

class MnemoChatMessageHistory(BaseChatMessageHistory):
    """Mnemo-backed chat message history for LangChain.

    Usage:
        from mnemo import Mnemo
        from mnemo.ext.langchain import MnemoChatMessageHistory

        client = Mnemo("http://localhost:8080")
        history = MnemoChatMessageHistory(
            session_id="session-001",
            user_id="kendra",
            client=client,
        )
    """

    def __init__(self, session_id: str, user_id: str, client: Mnemo):
        self.session_id = session_id
        self.user_id = user_id
        self._client = client

    @property
    def messages(self) -> list[BaseMessage]:
        """Fetch all messages for this session from Mnemo."""
        raw = self._client.get_messages(self.session_id)
        return [_mnemo_to_langchain(m) for m in raw]

    def add_messages(self, messages: Sequence[BaseMessage]) -> None:
        """Store messages in Mnemo via the remember API."""
        for msg in messages:
            role = _langchain_role(msg)
            self._client.add(
                self.user_id,
                msg.content,
                session=self.session_id,
                role=role,
            )

    def clear(self) -> None:
        """Clear all messages for this session."""
        self._client.clear_messages(self.session_id)
```

### 5.6) LlamaIndex Adapter

```python
from llama_index.core.storage.chat_store.base import BaseChatStore
from llama_index.core.llms import ChatMessage, MessageRole

class MnemoChatStore(BaseChatStore):
    """Mnemo-backed chat store for LlamaIndex.

    Usage:
        from mnemo import Mnemo
        from mnemo.ext.llamaindex import MnemoChatStore

        client = Mnemo("http://localhost:8080")
        store = MnemoChatStore(client=client, user_id="kendra")
    """

    def __init__(self, client: Mnemo, user_id: str):
        self._client = client
        self.user_id = user_id

    def set_messages(self, key: str, messages: list[ChatMessage]) -> None:
        self._client.clear_messages(key)
        for msg in messages:
            self._client.add(self.user_id, msg.content, session=key,
                            role=msg.role.value)

    def get_messages(self, key: str) -> list[ChatMessage]:
        raw = self._client.get_messages(key)
        return [_mnemo_to_llamaindex(m) for m in raw]

    def add_message(self, key: str, message: ChatMessage) -> None:
        self._client.add(self.user_id, message.content, session=key,
                        role=message.role.value)

    def delete_messages(self, key: str) -> list[ChatMessage] | None:
        existing = self.get_messages(key)
        self._client.clear_messages(key)
        return existing if existing else None

    def delete_message(self, key: str, idx: int) -> ChatMessage | None:
        existing = self.get_messages(key)
        if idx < 0 or idx >= len(existing):
            return None
        removed = existing[idx]
        self._client.delete_message(key, idx)
        return removed

    def delete_last_message(self, key: str) -> ChatMessage | None:
        existing = self.get_messages(key)
        if not existing:
            return None
        return self.delete_message(key, len(existing) - 1)

    def get_keys(self) -> list[str]:
        # Uses list-sessions-for-user; returns session IDs as keys.
        sessions = self._client.list_sessions(self.user_id)
        return [s.id for s in sessions]
```

## 6) Execution Plan

### Milestone S1: Server-Side Message Endpoints (1 day)

| Ticket | Description | Falsification |
|--------|-------------|---------------|
| S1-1 | `GET /api/v1/sessions/:id/messages` — returns `[{role, content, created_at, episode_id}]` ordered by `created_at` ASC. Paginated with `?limit=&after=`. | Integration test: create 3 episodes, verify messages returns them in order with correct roles. |
| S1-2 | `DELETE /api/v1/sessions/:id/messages` — deletes all episodes for the session, emits `session_messages_cleared` governance audit. | Integration test: add messages, clear, verify get_messages returns empty. Verify audit record. |
| S1-3 | `DELETE /api/v1/sessions/:id/messages/:idx` — deletes the episode at ordinal index `idx` (0-based). Returns 404 if out of bounds. | Integration test: add 3 messages, delete index 1, verify remaining 2 messages are correct. |

### Milestone S2: Python SDK Core Rebuild (2 days)

| Ticket | Description | Falsification |
|--------|-------------|---------------|
| S2-1 | Extract error hierarchy and models to `_errors.py` and `_models.py`. | Unit tests for error construction and model serialization. |
| S2-2 | Build sync transport (`_transport.py`) with request-id propagation, query parameter support, response header capture. Fix rate-limit backoff to honor `retry_after_ms`. | Test: mock 429 with retry_after_ms=2000, verify SDK sleeps >= 2s. |
| S2-3 | Implement full sync `Mnemo` client with all methods from §5.4. | Docker integration test: every method called against live server. |
| S2-4 | Implement `AsyncMnemo` client using `aiohttp`. | Docker integration test: same test suite but async. |
| S2-5 | Add `contract`, `retrieval_policy`, `filters` parameters to `context()`. | Test: call context with contract="support_safe", verify `contract_applied` in response. |
| S2-6 | Add `x-mnemo-request-id` send/receive on every call. | Test: send custom request-id, verify it echoes in response header and appears in trace lookup. |

### Milestone S3: LangChain Adapter (1 day)

| Ticket | Description | Falsification |
|--------|-------------|---------------|
| S3-1 | Implement `MnemoChatMessageHistory` per §5.5. | Docker test: add 5 messages via adapter, verify `.messages` returns them. Call `.clear()`, verify empty. |
| S3-2 | Message type conversion utilities (`_mnemo_to_langchain`, `_langchain_role`). | Unit tests for all message type mappings (HumanMessage, AIMessage, SystemMessage, ToolMessage). |
| S3-3 | Integration test: wire `MnemoChatMessageHistory` into a `ConversationChain` and verify round-trip. | End-to-end: create chain, send 3 messages, verify Mnemo stored them, verify chain retrieves them. |

### Milestone S4: LlamaIndex Adapter (1 day)

| Ticket | Description | Falsification |
|--------|-------------|---------------|
| S4-1 | Implement `MnemoChatStore` per §5.6. | Docker test: all 7 abstract methods exercised against live server. |
| S4-2 | Message type conversion utilities (`_mnemo_to_llamaindex`). | Unit tests for ChatMessage role mapping. |
| S4-3 | `get_keys()` implementation via list-sessions. | Docker test: create 3 sessions, verify `get_keys()` returns all 3. |
| S4-4 | Integration test: wire `MnemoChatStore` into a LlamaIndex `SimpleChatEngine` and verify round-trip. | End-to-end: create engine, send messages, verify storage and retrieval. |

### Milestone S5: Docker Test Infrastructure (0.5 day)

| Ticket | Description | Falsification |
|--------|-------------|---------------|
| S5-1 | Create `sdk/python/docker-compose.test.yml` with Redis + Qdrant + Mnemo server. | `docker compose up -d` starts all services; health check passes. |
| S5-2 | Create `sdk/python/tests/conftest.py` with pytest fixtures that start/stop Docker and provide a connected `Mnemo` client. | `pytest sdk/python/tests/` runs and connects to live server. |
| S5-3 | Add `sdk/python/Makefile` or script: `make test` → build server, start compose, run pytest, stop compose. | Single command runs full suite. |

### Milestone S6: Documentation and README (0.5 day)

| Ticket | Description |
|--------|-------------|
| S6-1 | Update `sdk/python/README.md` with full client API reference, LangChain example, LlamaIndex example. |
| S6-2 | Add `docs/SDK_QUICKSTART.md` with install + 5-minute integration examples. |
| S6-3 | Update root `README.md` to mention SDK integrations and link to quickstart. |
| S6-4 | Update `CHANGELOG.md` with SDK entries. |

## 7) Falsification Matrix

| Claim | Falsification Test | Pass Criteria |
|-------|-------------------|---------------|
| SDK wraps all high-level endpoints | `test_client.py` calls every `Mnemo` method against live server | All return typed results, no raw dicts |
| Async client works | `test_async_client.py` mirrors sync tests | Same assertions pass under `pytest-asyncio` |
| LangChain adapter is drop-in | Wire into `ConversationChain`, round-trip 5 messages | Messages persist and retrieve correctly |
| LlamaIndex adapter is drop-in | Wire into `SimpleChatEngine`, exercise all 7 methods | All methods succeed, `get_keys()` returns correct sessions |
| Request-id propagation works | Send custom `x-mnemo-request-id`, then call `trace_lookup` | Trace contains the episode and correlation matches |
| Rate-limit backoff honors server | Mock 429 with `retry_after_ms=2000` | SDK sleeps >= 2s before retry |
| Contract/policy params work via SDK | Call `context(contract="support_safe", retrieval_policy="precision")` | Response `contract_applied` and `retrieval_policy_applied` match |
| Docker test suite is reproducible | `make test` from clean checkout | All tests pass, no manual setup required |
| Message endpoints work | Create session, add 5 episodes, get_messages, delete index 2, get_messages again | Correct message count and ordering |

## 8) Competitive Impact

| Capability | Zep SDK | Mnemo SDK (after this PRD) |
|-----------|---------|---------------------------|
| LangChain adapter | `ZepChatMessageHistory` | `MnemoChatMessageHistory` (drop-in) |
| LlamaIndex adapter | None (Zep has no official LlamaIndex adapter) | `MnemoChatStore` (drop-in) |
| Async support | `AsyncZep` via httpx | `AsyncMnemo` via aiohttp |
| Contract/policy tuning | N/A (Zep has no contracts) | Full contract + retrieval policy params on `context()` |
| Request-id tracing | N/A | End-to-end `x-mnemo-request-id` |
| Temporal queries via SDK | N/A | `time_travel_trace()`, `changes_since()`, `causal_recall()` |
| Governance via SDK | N/A | `get_policy()`, `set_policy()`, `preview_policy()`, `get_policy_violations()` |
| Webhook management via SDK | N/A | Full webhook lifecycle + replay + retry |
| Zero-dependency core | No (requires httpx) | Yes (stdlib only for sync client) |

The LlamaIndex adapter alone is a category gap — Zep has no official LlamaIndex integration.

## 9) Rollout Criteria

### Gate 1: SDK Core (S2 complete)
- All `Mnemo` and `AsyncMnemo` methods tested against Docker
- Request-id propagation verified
- Rate-limit backoff verified

### Gate 2: Framework Adapters (S3 + S4 complete)
- LangChain `ConversationChain` round-trip passes
- LlamaIndex `SimpleChatEngine` round-trip passes
- All 7 `BaseChatStore` abstract methods verified

### Gate 3: Ship (S5 + S6 complete)
- `make test` from clean checkout passes
- README includes working examples
- Root README updated
- CHANGELOG updated

## 10) Risk Register

| Risk | Mitigation |
|------|------------|
| LangChain/LlamaIndex API breaks between versions | Pin minimum versions in extras; test against specific versions in CI |
| Message endpoint adds complexity to server | Thin projection over existing `list_episodes`; no new storage |
| `aiohttp` adds a dependency | Optional extra; sync client remains zero-dep |
| Message index-based delete is fragile | Document that indices shift after delete; LlamaIndex adapter handles this |
| Docker test infra is slow | Cache Cargo build artifacts; use pre-built server binary |
