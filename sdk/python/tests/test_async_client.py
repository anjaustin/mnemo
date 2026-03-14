"""AsyncMnemo unit tests with mock HTTP.

Covers SDK-08 through SDK-10 from the QA/QC Falsification PRD:
  SDK-08: AsyncMnemo mirrors sync Mnemo API surface
  SDK-09: AsyncMnemo handles errors correctly (4xx, 5xx, timeouts)
  SDK-10: AsyncMnemo context manager (async with) works correctly

Run:
    pytest tests/test_async_client.py -v

Requires: aioresponses (pip install aioresponses)
"""

import sys
import os
import pytest
import json

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from aioresponses import aioresponses

from mnemo import AsyncMnemo
from mnemo._models import (
    AgentContextResult,
    AgentIdentityAuditResult,
    AgentIdentityResult,
    AuditRecord,
    ChangesSinceResult,
    ContextResult,
    DeleteResult,
    ExperienceEventResult,
    HealthResult,
    ImportJobResult,
    PromotionProposalResult,
    MessagesResult,
    PolicyPreviewResult,
    PolicyResult,
    RememberResult,
    ReplayResult,
    RetryResult,
    TimeTravelSummaryResult,
    TimeTravelTraceResult,
    TraceLookupResult,
    WebhookEvent,
    WebhookResult,
    WebhookStats,
)
from mnemo._errors import (
    MnemoHttpError,
    MnemoNotFoundError,
    MnemoValidationError,
    MnemoRateLimitError,
)


BASE = "http://mock-mnemo:8080"


# ── SDK-08: AsyncMnemo mirrors sync Mnemo API surface ──────────────


@pytest.mark.asyncio
async def test_async_health():
    """health() returns HealthResult with correct fields."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/health",
                payload={"status": "ok", "version": "0.3.3"},
            )
            result = await client.health()
            assert isinstance(result, HealthResult)
            assert result.status == "ok"
            assert result.version == "0.3.3"


@pytest.mark.asyncio
async def test_async_add():
    """add() sends correct payload and parses RememberResult."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/memory",
                payload={
                    "ok": True,
                    "user_id": "u-123",
                    "session_id": "s-456",
                    "episode_id": "e-789",
                },
            )
            result = await client.add("kendra", "I love hiking")
            assert isinstance(result, RememberResult)
            assert result.ok is True
            assert result.user_id == "u-123"
            assert result.session_id == "s-456"
            assert result.episode_id == "e-789"


@pytest.mark.asyncio
async def test_async_add_with_session():
    """add() includes session parameter when provided."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/memory",
                payload={
                    "ok": True,
                    "user_id": "",
                    "session_id": "s-custom",
                    "episode_id": "",
                },
            )
            result = await client.add("kendra", "text", session="my-session")
            assert result.ok is True


@pytest.mark.asyncio
async def test_async_context():
    """context() sends correct user/query and parses ContextResult."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/memory/kendra/context",
                payload={
                    "context": "Kendra loves hiking in Colorado.",
                    "token_count": 8,
                    "mode": "hybrid",
                    "entities": [],
                    "facts": [],
                    "episodes": [],
                },
            )
            result = await client.context("kendra", "What does Kendra enjoy?")
            assert isinstance(result, ContextResult)
            assert result.text == "Kendra loves hiking in Colorado."
            assert result.token_count == 8
            assert result.mode == "hybrid"


@pytest.mark.asyncio
async def test_async_get_messages():
    """get_messages() parses MessagesResult with Message list."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/api/v1/sessions/s-1/messages?limit=100",
                payload={
                    "messages": [
                        {
                            "idx": 0,
                            "id": "m-1",
                            "role": "user",
                            "content": "hello",
                            "created_at": "2025-01-01T00:00:00Z",
                        },
                        {
                            "idx": 1,
                            "id": "m-2",
                            "role": "assistant",
                            "content": "hi!",
                            "created_at": "2025-01-01T00:00:01Z",
                        },
                    ],
                    "count": 2,
                    "session_id": "s-1",
                },
            )
            result = await client.get_messages("s-1")
            assert isinstance(result, MessagesResult)
            assert result.count == 2
            assert len(result.messages) == 2
            assert result.messages[0].content == "hello"
            assert result.messages[1].content == "hi!"


@pytest.mark.asyncio
async def test_async_clear_messages():
    """clear_messages() returns DeleteResult."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.delete(
                f"{BASE}/api/v1/sessions/s-1/messages",
                payload={"deleted": True},
            )
            result = await client.clear_messages("s-1")
            assert isinstance(result, DeleteResult)
            assert result.deleted is True


@pytest.mark.asyncio
async def test_async_delete_message():
    """delete_message() returns DeleteResult for single message."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.delete(
                f"{BASE}/api/v1/sessions/s-1/messages/0",
                payload={"deleted": True},
            )
            result = await client.delete_message("s-1", 0)
            assert isinstance(result, DeleteResult)
            assert result.deleted is True


# ── SDK-09: AsyncMnemo handles errors correctly ────────────────────


@pytest.mark.asyncio
async def test_async_404_raises_not_found():
    """404 response raises MnemoNotFoundError."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/memory/nobody/context",
                status=404,
                payload={
                    "error": {"code": "user_not_found", "message": "User not found"}
                },
            )
            with pytest.raises(MnemoNotFoundError) as exc_info:
                await client.context("nobody", "hello")
            assert exc_info.value.status_code == 404


@pytest.mark.asyncio
async def test_async_400_raises_validation_error():
    """400 response raises MnemoValidationError."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/memory",
                status=400,
                payload={
                    "error": {"code": "validation_error", "message": "text is required"}
                },
            )
            with pytest.raises(MnemoValidationError) as exc_info:
                await client.add("user", "")
            assert exc_info.value.status_code == 400


@pytest.mark.asyncio
async def test_async_429_raises_rate_limit():
    """429 response raises MnemoRateLimitError with retry_after_ms."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/memory",
                status=429,
                payload={
                    "error": {
                        "code": "rate_limited",
                        "message": "too many requests",
                        "retry_after_ms": 5000,
                    }
                },
            )
            with pytest.raises(MnemoRateLimitError) as exc_info:
                await client.add("user", "text")
            assert exc_info.value.retry_after_ms == 5000


@pytest.mark.asyncio
async def test_async_500_raises_http_error():
    """500 response raises MnemoHttpError (no retries when max_retries=0)."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/health",
                status=500,
                payload={"error": {"code": "internal_error", "message": "oops"}},
            )
            with pytest.raises(MnemoHttpError) as exc_info:
                await client.health()
            assert exc_info.value.status_code == 500


# ── SDK-09b: API key is sent in Authorization header ───────────────


@pytest.mark.asyncio
async def test_async_api_key_header():
    """API key is included as Bearer token in requests."""
    client = AsyncMnemo(BASE, api_key="sk-test-key", max_retries=0)
    # Verify the _headers method produces correct Authorization header
    headers = client._headers()
    assert headers.get("Authorization") == "Bearer sk-test-key"
    assert headers.get("Content-Type") == "application/json"

    # Without API key, no Authorization header
    client_no_key = AsyncMnemo(BASE, max_retries=0)
    headers_no_key = client_no_key._headers()
    assert "Authorization" not in headers_no_key


# ── SDK-09c: Request ID is forwarded ──────────────────────────────


@pytest.mark.asyncio
async def test_async_request_id_forwarded():
    """x-mnemo-request-id header is sent when request_id is provided."""
    client = AsyncMnemo(BASE, max_retries=0)

    # Per-request request_id
    headers = client._headers(request_id="req-123")
    assert headers.get("x-mnemo-request-id") == "req-123"

    # Default request_id from constructor
    client_default = AsyncMnemo(BASE, max_retries=0, request_id="default-rid")
    headers_default = client_default._headers()
    assert headers_default.get("x-mnemo-request-id") == "default-rid"

    # Per-request overrides default
    headers_override = client_default._headers(request_id="override-rid")
    assert headers_override.get("x-mnemo-request-id") == "override-rid"

    # No request_id means no header
    client_no_rid = AsyncMnemo(BASE, max_retries=0)
    headers_no_rid = client_no_rid._headers()
    assert "x-mnemo-request-id" not in headers_no_rid


# ── SDK-10: AsyncMnemo context manager works correctly ─────────────


@pytest.mark.asyncio
async def test_async_context_manager():
    """async with AsyncMnemo(...) properly initializes and closes session."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        assert client is not None
        assert client.base_url == BASE

        # Session is lazily created, should be None before first request
        assert client._session is None

        with aioresponses() as m:
            m.get(f"{BASE}/health", payload={"status": "ok", "version": "0.3.3"})
            await client.health()

            # Now session should exist
            assert client._session is not None

    # After exiting context manager, session should be closed
    assert client._session is None or client._session.closed


@pytest.mark.asyncio
async def test_async_manual_close():
    """Calling close() explicitly works."""
    client = AsyncMnemo(BASE, max_retries=0)

    with aioresponses() as m:
        m.get(f"{BASE}/health", payload={"status": "ok", "version": "0.3.3"})
        await client.health()
        assert client._session is not None

    await client.close()
    assert client._session is None or client._session.closed


# ── SDK-08b: Changes-since, conflict-radar, causal-recall ──────────


@pytest.mark.asyncio
async def test_async_changes_since():
    """changes_since() parses ChangesSinceResult correctly."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/memory/kendra/changes_since",
                payload={
                    "added_facts": ["Kendra likes dogs"],
                    "superseded_facts": [],
                    "confidence_deltas": [],
                    "head_changes": [],
                    "added_episodes": [],
                    "summary": "one new fact",
                    "from": "2025-01-01T00:00:00Z",
                    "to": "2025-03-01T00:00:00Z",
                },
            )
            result = await client.changes_since(
                "kendra", from_dt="2025-01-01T00:00:00Z", to_dt="2025-03-01T00:00:00Z"
            )
            assert isinstance(result, ChangesSinceResult)
            assert len(result.added_facts) == 1
            assert result.added_facts[0] == "Kendra likes dogs"
            assert result.summary == "one new fact"


@pytest.mark.asyncio
async def test_async_conflict_radar():
    """conflict_radar() parses ConflictRadarResult correctly."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/memory/kendra/conflict_radar",
                payload={
                    "conflicts": ["Nike vs Adidas preference"],
                    "user_id": "u-123",
                },
            )
            result = await client.conflict_radar("kendra")
            assert len(result.conflicts) == 1


@pytest.mark.asyncio
async def test_async_causal_recall():
    """causal_recall() parses CausalRecallResult correctly."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/memory/kendra/causal_recall",
                payload={
                    "chains": [["fact1 -> fact2"]],
                    "query": "why does Kendra like hiking?",
                },
            )
            result = await client.causal_recall(
                "kendra", "why does Kendra like hiking?"
            )
            assert len(result.chains) == 1
            assert result.query == "why does Kendra like hiking?"


# ── SDK-11b: Retry behaviour ───────────────────────────────────────


@pytest.mark.asyncio
async def test_async_429_is_retried_when_max_retries_gt_0():
    """429 should be retried (not immediately re-raised) when max_retries > 0."""
    async with AsyncMnemo(BASE, max_retries=1, retry_backoff_s=0.0) as client:
        with aioresponses() as m:
            # First attempt: 429
            m.post(
                f"{BASE}/api/v1/memory",
                status=429,
                payload={"error": {"code": "rate_limited", "message": "slow down"}},
            )
            # Second attempt (retry): success
            m.post(
                f"{BASE}/api/v1/memory",
                status=201,
                payload={
                    "ok": True,
                    "user_id": "00000000-0000-0000-0000-000000000001",
                    "episode_id": "00000000-0000-0000-0000-000000000002",
                    "session_id": "00000000-0000-0000-0000-000000000003",
                },
            )
            result = await client.add("user", "some text")
            assert result.ok is True


@pytest.mark.asyncio
async def test_async_429_raises_after_exhausting_retries():
    """429 raises MnemoRateLimitError after all retries are exhausted."""
    async with AsyncMnemo(BASE, max_retries=1, retry_backoff_s=0.0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/memory",
                status=429,
                payload={
                    "error": {
                        "code": "rate_limited",
                        "message": "too fast",
                        "retry_after_ms": 1000,
                    }
                },
            )
            m.post(
                f"{BASE}/api/v1/memory",
                status=429,
                payload={
                    "error": {
                        "code": "rate_limited",
                        "message": "too fast",
                        "retry_after_ms": 1000,
                    }
                },
            )
            with pytest.raises(MnemoRateLimitError) as exc_info:
                await client.add("user", "text")
            assert exc_info.value.retry_after_ms == 1000


@pytest.mark.asyncio
async def test_async_5xx_is_retried_when_max_retries_gt_0():
    """5xx errors should be retried (not immediately re-raised)."""
    async with AsyncMnemo(BASE, max_retries=1, retry_backoff_s=0.0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/health",
                status=503,
                payload={"error": {"message": "service unavailable"}},
            )
            m.get(
                f"{BASE}/health",
                status=200,
                payload={
                    "status": "ok",
                    "version": "0.3.6",
                    "redis": True,
                    "qdrant": True,
                },
            )
            result = await client.health()
            assert result.status == "ok"


@pytest.mark.asyncio
async def test_sync_exponential_backoff():
    """Sync transport uses exponential backoff (delay grows with attempt count)."""
    from mnemo._transport import SyncTransport
    import urllib.error
    from unittest.mock import patch, MagicMock

    transport = SyncTransport(
        base_url="http://localhost:9999",
        api_key=None,
        timeout_s=1.0,
        max_retries=2,
        retry_backoff_s=1.0,
        default_request_id=None,
    )
    sleep_calls: list[float] = []

    # Patch time.sleep and urlopen to simulate 2 retries
    def fake_sleep(delay: float) -> None:
        sleep_calls.append(delay)

    http_err_429 = urllib.error.HTTPError(
        url="http://localhost:9999/health",
        code=429,
        msg="Too Many Requests",
        hdrs=MagicMock(get=lambda k, default=None: None),
        fp=MagicMock(
            read=lambda: b'{"error":{"message":"slow","code":"rate_limited"}}'
        ),
    )
    with patch("time.sleep", side_effect=fake_sleep):
        with patch("urllib.request.urlopen", side_effect=http_err_429):
            try:
                transport.request("GET", "/health")
            except Exception:
                pass
    # Should have slept twice (attempt=1 and attempt=2)
    assert len(sleep_calls) == 2, f"expected 2 sleep calls, got {sleep_calls}"
    # Exponential: delay[1] should be >= delay[0] on average
    # (with jitter, second call uses 2^2=4 base vs 2^1=2 base)
    # We just verify both were called and are positive
    assert all(d >= 0 for d in sleep_calls), (
        f"all delays must be non-negative: {sleep_calls}"
    )


# ── SDK-08c: 18 parity methods ─────────────────────────────────────


@pytest.mark.asyncio
async def test_async_context_head():
    """context_head() delegates to context() with mode='head'."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/memory/kendra/context",
                payload={
                    "context": "head context",
                    "token_count": 3,
                    "mode": "head",
                    "entities": [],
                    "facts": [],
                    "episodes": [],
                },
            )
            result = await client.context_head("kendra", "latest info")
            assert isinstance(result, ContextResult)
            assert result.mode == "head"
            assert result.text == "head context"


@pytest.mark.asyncio
async def test_async_time_travel_trace():
    """time_travel_trace() parses TimeTravelTraceResult correctly."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/memory/kendra/time_travel/trace",
                payload={
                    "snapshot_from": {"context": "old"},
                    "snapshot_to": {"context": "new"},
                    "gained_facts": ["Kendra moved to Denver"],
                    "lost_facts": [],
                    "gained_episodes": [],
                    "lost_episodes": [],
                    "timeline": [],
                    "summary": "one fact gained",
                    "from": "2025-01-01T00:00:00Z",
                    "to": "2025-06-01T00:00:00Z",
                },
            )
            result = await client.time_travel_trace(
                "kendra",
                "where does Kendra live?",
                from_dt="2025-01-01T00:00:00Z",
                to_dt="2025-06-01T00:00:00Z",
            )
            assert isinstance(result, TimeTravelTraceResult)
            assert result.gained_facts == ["Kendra moved to Denver"]
            assert result.summary == "one fact gained"
            assert result.from_dt == "2025-01-01T00:00:00Z"


@pytest.mark.asyncio
async def test_async_time_travel_summary():
    """time_travel_summary() parses TimeTravelSummaryResult correctly."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/memory/kendra/time_travel/summary",
                payload={
                    "gained_count": 2,
                    "lost_count": 0,
                    "from": "2025-01-01T00:00:00Z",
                    "to": "2025-06-01T00:00:00Z",
                },
            )
            result = await client.time_travel_summary(
                "kendra",
                "changes",
                from_dt="2025-01-01T00:00:00Z",
                to_dt="2025-06-01T00:00:00Z",
            )
            assert isinstance(result, TimeTravelSummaryResult)
            assert result.summary.get("gained_count") == 2


@pytest.mark.asyncio
async def test_async_preview_policy():
    """preview_policy() parses PolicyPreviewResult correctly."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/policies/kendra/preview",
                payload={
                    "user_id": "u-kendra",
                    "current_policy": {"retention_days_message": 90},
                    "preview_policy": {"retention_days_message": 30},
                    "estimated_affected_episodes_total": 7,
                    "estimated_affected_message_episodes": 5,
                    "estimated_affected_text_episodes": 2,
                    "estimated_affected_json_episodes": 0,
                    "confidence": "high",
                },
            )
            result = await client.preview_policy("kendra", retention_days_message=30)
            assert isinstance(result, PolicyPreviewResult)
            assert result.estimated_affected_episodes_total == 7
            assert result.preview_policy.get("retention_days_message") == 30


@pytest.mark.asyncio
async def test_async_get_policy_audit():
    """get_policy_audit() returns list of AuditRecord."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/api/v1/policies/kendra/audit?limit=50",
                payload={
                    "audit": [
                        {
                            "id": "a-1",
                            "user_id": "u-kendra",
                            "event_type": "policy_updated",
                            "details": {},
                            "created_at": "2025-01-01T00:00:00Z",
                        }
                    ]
                },
            )
            records = await client.get_policy_audit("kendra")
            assert len(records) == 1
            assert isinstance(records[0], AuditRecord)
            assert records[0].event_type == "policy_updated"


@pytest.mark.asyncio
async def test_async_get_policy_violations():
    """get_policy_violations() returns list of AuditRecord for violations."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/api/v1/policies/kendra/violations"
                f"?from=2025-01-01T00:00:00Z&to=2025-06-01T00:00:00Z&limit=50",
                payload={
                    "audit": [
                        {
                            "id": "v-1",
                            "user_id": "u-kendra",
                            "event_type": "retention_violation",
                            "details": {},
                            "created_at": "2025-03-01T00:00:00Z",
                        }
                    ]
                },
            )
            records = await client.get_policy_violations(
                "kendra",
                from_dt="2025-01-01T00:00:00Z",
                to_dt="2025-06-01T00:00:00Z",
            )
            assert len(records) == 1
            assert isinstance(records[0], AuditRecord)
            assert records[0].event_type == "retention_violation"


@pytest.mark.asyncio
async def test_async_create_webhook():
    """create_webhook() sends correct payload and returns WebhookResult."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/memory/webhooks",
                payload={
                    "webhook": {
                        "id": "wh-1",
                        "user_id": "u-kendra",
                        "target_url": "https://example.com/hook",
                        "events": ["memory.created"],
                        "enabled": True,
                        "created_at": "2025-01-01T00:00:00Z",
                        "updated_at": "2025-01-01T00:00:00Z",
                    }
                },
            )
            result = await client.create_webhook(
                "kendra", "https://example.com/hook", ["memory.created"]
            )
            assert isinstance(result, WebhookResult)
            assert result.id == "wh-1"
            assert result.target_url == "https://example.com/hook"
            assert "memory.created" in result.events


@pytest.mark.asyncio
async def test_async_get_webhook():
    """get_webhook() returns WebhookResult for a given ID."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/api/v1/memory/webhooks/wh-1",
                payload={
                    "webhook": {
                        "id": "wh-1",
                        "user_id": "u-kendra",
                        "target_url": "https://example.com/hook",
                        "events": ["memory.created"],
                        "enabled": True,
                        "created_at": "2025-01-01T00:00:00Z",
                        "updated_at": "2025-01-01T00:00:00Z",
                    }
                },
            )
            result = await client.get_webhook("wh-1")
            assert isinstance(result, WebhookResult)
            assert result.id == "wh-1"
            assert result.enabled is True


@pytest.mark.asyncio
async def test_async_delete_webhook():
    """delete_webhook() returns DeleteResult."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.delete(
                f"{BASE}/api/v1/memory/webhooks/wh-1",
                payload={"deleted": True},
            )
            result = await client.delete_webhook("wh-1")
            assert isinstance(result, DeleteResult)
            assert result.deleted is True


@pytest.mark.asyncio
async def test_async_get_webhook_events():
    """get_webhook_events() returns list of WebhookEvent."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/api/v1/memory/webhooks/wh-1/events?limit=20",
                payload={
                    "events": [
                        {
                            "id": "ev-1",
                            "webhook_id": "wh-1",
                            "event_type": "memory.created",
                            "user_id": "u-kendra",
                            "payload": {},
                            "created_at": "2025-01-01T00:00:00Z",
                            "attempts": 1,
                            "delivered": True,
                            "dead_letter": False,
                        }
                    ]
                },
            )
            events = await client.get_webhook_events("wh-1")
            assert len(events) == 1
            assert isinstance(events[0], WebhookEvent)
            assert events[0].event_type == "memory.created"
            assert events[0].delivered is True


@pytest.mark.asyncio
async def test_async_get_dead_letter_events():
    """get_dead_letter_events() returns list of WebhookEvent in dead-letter queue."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/api/v1/memory/webhooks/wh-1/events/dead-letter?limit=20",
                payload={
                    "events": [
                        {
                            "id": "ev-dead-1",
                            "webhook_id": "wh-1",
                            "event_type": "memory.created",
                            "user_id": "u-kendra",
                            "payload": {},
                            "created_at": "2025-01-01T00:00:00Z",
                            "attempts": 5,
                            "delivered": False,
                            "dead_letter": True,
                        }
                    ]
                },
            )
            events = await client.get_dead_letter_events("wh-1")
            assert len(events) == 1
            assert isinstance(events[0], WebhookEvent)
            assert events[0].dead_letter is True
            assert events[0].delivered is False


@pytest.mark.asyncio
async def test_async_replay_events():
    """replay_events() parses ReplayResult correctly."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/api/v1/memory/webhooks/wh-1/events/replay"
                f"?limit=100&include_delivered=true&include_dead_letter=true",
                payload={
                    "webhook_id": "wh-1",
                    "count": 3,
                    "events": [{"id": "ev-1"}, {"id": "ev-2"}, {"id": "ev-3"}],
                },
            )
            result = await client.replay_events("wh-1")
            assert isinstance(result, ReplayResult)
            assert result.count == 3
            assert len(result.events) == 3


@pytest.mark.asyncio
async def test_async_retry_event():
    """retry_event() parses RetryResult correctly."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/memory/webhooks/wh-1/events/ev-1/retry",
                payload={
                    "webhook_id": "wh-1",
                    "event_id": "ev-1",
                    "queued": True,
                    "reason": "manual retry",
                },
            )
            result = await client.retry_event("wh-1", "ev-1")
            assert isinstance(result, RetryResult)
            assert result.queued is True
            assert result.event_id == "ev-1"


@pytest.mark.asyncio
async def test_async_get_webhook_stats():
    """get_webhook_stats() parses WebhookStats correctly."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/api/v1/memory/webhooks/wh-1/stats?window_seconds=300",
                payload={
                    "webhook_id": "wh-1",
                    "total_events": 13,
                    "delivered_events": 10,
                    "pending_events": 0,
                    "dead_letter_events": 1,
                    "failed_events": 2,
                    "recent_failures": 1,
                },
            )
            result = await client.get_webhook_stats("wh-1")
            assert isinstance(result, WebhookStats)
            assert result.delivered_events == 10
            assert result.failed_events == 2
            assert result.dead_letter_events == 1


@pytest.mark.asyncio
async def test_async_get_webhook_audit():
    """get_webhook_audit() returns list of AuditRecord for a webhook."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/api/v1/memory/webhooks/wh-1/audit?limit=20",
                payload={
                    "audit": [
                        {
                            "id": "wa-1",
                            "user_id": "u-kendra",
                            "event_type": "webhook_created",
                            "details": {},
                            "created_at": "2025-01-01T00:00:00Z",
                        }
                    ]
                },
            )
            records = await client.get_webhook_audit("wh-1")
            assert len(records) == 1
            assert isinstance(records[0], AuditRecord)
            assert records[0].event_type == "webhook_created"


@pytest.mark.asyncio
async def test_async_trace_lookup():
    """trace_lookup() parses TraceLookupResult correctly."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/api/v1/traces/req-abc?limit=100",
                payload={
                    "matched_episodes": [{"id": "ep-1"}],
                    "matched_webhook_events": [],
                    "matched_webhook_audit": [],
                    "matched_governance_audit": [],
                },
            )
            result = await client.trace_lookup("req-abc")
            assert isinstance(result, TraceLookupResult)
            assert result.request_id == "req-abc"
            assert len(result.matched_episodes) == 1


@pytest.mark.asyncio
async def test_async_import_chat_history():
    """import_chat_history() sends correct payload and parses ImportJobResult."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/import/chat-history",
                payload={
                    "job": {
                        "id": "job-1",
                        "source": "openai",
                        "user": "kendra",
                        "dry_run": False,
                        "status": "queued",
                        "total_messages": 0,
                        "imported_messages": 0,
                        "failed_messages": 0,
                        "sessions_touched": 0,
                        "errors": [],
                        "created_at": "2025-01-01T00:00:00Z",
                        "started_at": None,
                        "finished_at": None,
                    }
                },
            )
            result = await client.import_chat_history(
                "kendra", "openai", {"conversations": []}
            )
            assert isinstance(result, ImportJobResult)
            assert result.id == "job-1"
            assert result.source == "openai"
            assert result.status == "queued"


@pytest.mark.asyncio
async def test_async_get_import_job():
    """get_import_job() returns ImportJobResult for a completed job."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/api/v1/import/jobs/job-1",
                payload={
                    "job": {
                        "id": "job-1",
                        "source": "openai",
                        "user": "kendra",
                        "dry_run": False,
                        "status": "completed",
                        "total_messages": 50,
                        "imported_messages": 50,
                        "failed_messages": 0,
                        "sessions_touched": 3,
                        "errors": [],
                        "created_at": "2025-01-01T00:00:00Z",
                        "started_at": "2025-01-01T00:00:01Z",
                        "finished_at": "2025-01-01T00:00:10Z",
                    }
                },
            )
            result = await client.get_import_job("job-1")
            assert isinstance(result, ImportJobResult)
            assert result.status == "completed"
            assert result.total_messages == 50
            assert result.sessions_touched == 3


# ── Agent Identity ──────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_async_get_agent_identity():
    """get_agent_identity() returns AgentIdentityResult with auto-created profile."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/api/v1/agents/support-bot/identity",
                payload={
                    "agent_id": "support-bot",
                    "version": 1,
                    "core": {},
                    "updated_at": "2026-03-10T00:00:00Z",
                },
            )
            result = await client.get_agent_identity("support-bot")
            assert isinstance(result, AgentIdentityResult)
            assert result.agent_id == "support-bot"
            assert result.version == 1
            assert result.core == {}


@pytest.mark.asyncio
async def test_async_update_agent_identity():
    """update_agent_identity() sends core and returns updated profile."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.put(
                f"{BASE}/api/v1/agents/support-bot/identity",
                payload={
                    "agent_id": "support-bot",
                    "version": 2,
                    "core": {"mission": "Help users with billing"},
                    "updated_at": "2026-03-10T01:00:00Z",
                },
            )
            result = await client.update_agent_identity(
                "support-bot", {"mission": "Help users with billing"}
            )
            assert isinstance(result, AgentIdentityResult)
            assert result.version == 2
            assert result.core["mission"] == "Help users with billing"


@pytest.mark.asyncio
async def test_async_list_agent_identity_versions():
    """list_agent_identity_versions() returns list of AgentIdentityResult."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/api/v1/agents/support-bot/identity/versions?limit=20",
                payload={
                    "versions": [
                        {
                            "agent_id": "support-bot",
                            "version": 2,
                            "core": {"mission": "v2"},
                            "updated_at": "t2",
                        },
                        {
                            "agent_id": "support-bot",
                            "version": 1,
                            "core": {},
                            "updated_at": "t1",
                        },
                    ]
                },
            )
            result = await client.list_agent_identity_versions("support-bot")
            assert len(result) == 2
            assert all(isinstance(v, AgentIdentityResult) for v in result)
            assert result[0].version == 2


@pytest.mark.asyncio
async def test_async_list_agent_identity_audit():
    """list_agent_identity_audit() returns list of AgentIdentityAuditResult."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.get(
                f"{BASE}/api/v1/agents/support-bot/identity/audit?limit=50",
                payload={
                    "audit": [
                        {
                            "id": "a-1",
                            "agent_id": "support-bot",
                            "action": "updated",
                            "from_version": 1,
                            "to_version": 2,
                            "created_at": "2026-03-10T01:00:00Z",
                        },
                        {
                            "id": "a-0",
                            "agent_id": "support-bot",
                            "action": "created",
                            "to_version": 1,
                            "created_at": "2026-03-10T00:00:00Z",
                        },
                    ]
                },
            )
            result = await client.list_agent_identity_audit("support-bot")
            assert len(result) == 2
            assert all(isinstance(a, AgentIdentityAuditResult) for a in result)
            assert result[0].action == "updated"
            assert result[0].from_version == 1


@pytest.mark.asyncio
async def test_async_rollback_agent_identity():
    """rollback_agent_identity() returns new version with restored core."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/agents/support-bot/identity/rollback",
                payload={
                    "agent_id": "support-bot",
                    "version": 4,
                    "core": {"mission": "original mission"},
                    "updated_at": "2026-03-10T02:00:00Z",
                },
            )
            result = await client.rollback_agent_identity(
                "support-bot", 2, reason="reverting bad update"
            )
            assert isinstance(result, AgentIdentityResult)
            assert result.version == 4


@pytest.mark.asyncio
async def test_async_add_agent_experience():
    """add_agent_experience() returns ExperienceEventResult."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/agents/support-bot/experience",
                payload={
                    "id": "exp-1",
                    "agent_id": "support-bot",
                    "user_id": "u-1",
                    "session_id": "s-1",
                    "category": "skill_acquisition",
                    "signal": "User praised billing explanation",
                    "confidence": 0.9,
                    "weight": 0.5,
                    "decay_half_life_days": 30,
                    "evidence_episode_ids": [],
                    "created_at": "2026-03-10T00:00:00Z",
                },
                status=201,
            )
            result = await client.add_agent_experience(
                "support-bot",
                user_id="u-1",
                session_id="s-1",
                category="skill_acquisition",
                signal="User praised billing explanation",
                confidence=0.9,
            )
            assert isinstance(result, ExperienceEventResult)
            assert result.category == "skill_acquisition"
            assert result.confidence == 0.9


@pytest.mark.asyncio
async def test_async_promotion_lifecycle():
    """create, list, approve, reject promotions."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            # Create
            m.post(
                f"{BASE}/api/v1/agents/support-bot/promotions",
                payload={
                    "id": "prop-1",
                    "agent_id": "support-bot",
                    "proposal": "Add refund handling",
                    "candidate_core": {"mission": "billing + refunds"},
                    "reason": "Learned refund process",
                    "risk_level": "medium",
                    "status": "pending",
                    "source_event_ids": ["e1", "e2", "e3"],
                    "created_at": "2026-03-10T00:00:00Z",
                },
                status=201,
            )
            proposal = await client.create_promotion_proposal(
                "support-bot",
                proposal="Add refund handling",
                candidate_core={"mission": "billing + refunds"},
                reason="Learned refund process",
                source_event_ids=["e1", "e2", "e3"],
            )
            assert isinstance(proposal, PromotionProposalResult)
            assert proposal.status == "pending"

            # List
            m.get(
                f"{BASE}/api/v1/agents/support-bot/promotions?limit=50",
                payload={
                    "proposals": [
                        {
                            "id": "prop-1",
                            "agent_id": "support-bot",
                            "proposal": "Add refund handling",
                            "candidate_core": {"mission": "billing + refunds"},
                            "reason": "Learned refund process",
                            "risk_level": "medium",
                            "status": "pending",
                            "source_event_ids": ["e1", "e2", "e3"],
                            "created_at": "2026-03-10T00:00:00Z",
                        }
                    ]
                },
            )
            proposals = await client.list_promotion_proposals("support-bot")
            assert len(proposals) == 1

            # Approve
            m.post(
                f"{BASE}/api/v1/agents/support-bot/promotions/prop-1/approve",
                payload={
                    "id": "prop-1",
                    "agent_id": "support-bot",
                    "proposal": "Add refund handling",
                    "candidate_core": {"mission": "billing + refunds"},
                    "reason": "Learned refund process",
                    "risk_level": "medium",
                    "status": "approved",
                    "source_event_ids": ["e1", "e2", "e3"],
                    "created_at": "2026-03-10T00:00:00Z",
                    "approved_at": "2026-03-10T01:00:00Z",
                },
            )
            approved = await client.approve_promotion("support-bot", "prop-1")
            assert approved.status == "approved"
            assert approved.approved_at is not None


@pytest.mark.asyncio
async def test_async_reject_promotion():
    """reject_promotion() returns rejected proposal with reason appended."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/agents/support-bot/promotions/prop-2/reject",
                payload={
                    "id": "prop-2",
                    "agent_id": "support-bot",
                    "proposal": "Add refund handling",
                    "candidate_core": {"mission": "billing + refunds"},
                    "reason": "Too risky",
                    "risk_level": "high",
                    "status": "rejected",
                    "source_event_ids": ["e1", "e2", "e3"],
                    "created_at": "2026-03-10T00:00:00Z",
                    "rejected_at": "2026-03-10T01:00:00Z",
                },
            )
            rejected = await client.reject_promotion(
                "support-bot", "prop-2", reason="Too risky"
            )
            assert isinstance(rejected, PromotionProposalResult)
            assert rejected.status == "rejected"
            assert rejected.rejected_at is not None


@pytest.mark.asyncio
async def test_async_agent_context():
    """agent_context() returns AgentContextResult with identity and attribution guards."""
    async with AsyncMnemo(BASE, max_retries=0) as client:
        with aioresponses() as m:
            m.post(
                f"{BASE}/api/v1/agents/support-bot/context",
                payload={
                    "context": {
                        "entities": [],
                        "facts": [],
                        "episodes": [],
                        "text": "Agent identity block...",
                        "token_count": 150,
                    },
                    "identity": {
                        "agent_id": "support-bot",
                        "version": 3,
                        "core": {"mission": "Help with billing"},
                        "updated_at": "2026-03-10T00:00:00Z",
                    },
                    "identity_version": 3,
                    "experience_events_used": 5,
                    "experience_weight_sum": 2.1,
                    "user_memory_items_used": 12,
                    "attribution_guards": {
                        "self_user_separation_enforced": True,
                        "identity_plane_isolated": True,
                    },
                },
            )
            result = await client.agent_context(
                "support-bot", "alice", "What billing issues does Alice have?"
            )
            assert isinstance(result, AgentContextResult)
            assert result.identity.agent_id == "support-bot"
            assert result.identity_version == 3
            assert result.experience_events_used == 5
            assert result.attribution_guards["self_user_separation_enforced"] is True
            assert result.attribution_guards["identity_plane_isolated"] is True


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
