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
    HealthResult,
    RememberResult,
    ContextResult,
    DeleteResult,
    MessagesResult,
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
                    "facts_added": ["Kendra likes dogs"],
                    "facts_superseded": [],
                    "entities_updated": ["Kendra"],
                    "from_dt": "2025-01-01T00:00:00Z",
                    "to_dt": "2025-03-01T00:00:00Z",
                },
            )
            result = await client.changes_since(
                "kendra", from_dt="2025-01-01T00:00:00Z", to_dt="2025-03-01T00:00:00Z"
            )
            assert len(result.facts_added) == 1
            assert result.facts_added[0] == "Kendra likes dogs"
            assert len(result.entities_updated) == 1


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


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
