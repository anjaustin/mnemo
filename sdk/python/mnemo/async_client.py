"""Mnemo asynchronous Python client.

Mirror of :class:`mnemo.Mnemo` with async/await support.
Requires ``aiohttp`` (install with ``pip install mnemo-client[async]``).

Usage:
    from mnemo import AsyncMnemo

    async def main():
        client = AsyncMnemo("http://localhost:8080")
        result = await client.add("kendra", "I love hiking in Colorado")
        ctx = await client.context("kendra", "What does Kendra love doing outdoors?")
        print(ctx.text)
        await client.close()
"""

from __future__ import annotations

import json
from typing import Any

from mnemo._errors import (  # noqa: F401
    MnemoConnectionError,
    MnemoError,
    MnemoHttpError,
    MnemoNotFoundError,
    MnemoRateLimitError,
    MnemoTimeoutError,
    MnemoValidationError,
)
from mnemo._models import (
    AuditRecord,
    CausalRecallResult,
    ChangesSinceResult,
    ConflictRadarResult,
    ContextResult,
    DeleteResult,
    HealthResult,
    ImportJobResult,
    Message,
    MessagesResult,
    OpsSummaryResult,
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
from mnemo._transport import opt
from mnemo.client import (
    _parse_audit,
    _parse_context,
    _parse_import_job,
    _parse_policy,
    _parse_webhook,
    _parse_webhook_event,
)


class AsyncMnemo:
    """Asynchronous Mnemo client. Requires ``aiohttp``."""

    def __init__(
        self,
        base_url: str = "http://localhost:8080",
        api_key: str | None = None,
        *,
        timeout_s: float = 20.0,
        max_retries: int = 2,
        retry_backoff_s: float = 0.4,
        request_id: str | None = None,
    ) -> None:
        try:
            import aiohttp  # noqa: F401
        except ImportError as e:
            raise ImportError(
                "AsyncMnemo requires aiohttp. Install it with: "
                "pip install mnemo-client[async]"
            ) from e

        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.timeout_s = timeout_s
        self.max_retries = max_retries
        self.retry_backoff_s = retry_backoff_s
        self.default_request_id = request_id
        self._session: Any = None

    async def _get_session(self) -> Any:
        import aiohttp

        if self._session is None or self._session.closed:
            timeout = aiohttp.ClientTimeout(total=self.timeout_s)
            self._session = aiohttp.ClientSession(timeout=timeout)
        return self._session

    async def close(self) -> None:
        """Close the underlying aiohttp session."""
        if self._session and not self._session.closed:
            await self._session.close()

    async def __aenter__(self) -> "AsyncMnemo":
        return self

    async def __aexit__(self, *_: Any) -> None:
        await self.close()

    def _headers(self, request_id: str | None = None) -> dict[str, str]:
        headers = {"Content-Type": "application/json"}
        if self.api_key:
            headers["Authorization"] = f"Bearer {self.api_key}"
        rid = request_id or self.default_request_id
        if rid:
            headers["x-mnemo-request-id"] = rid
        return headers

    async def _req(
        self,
        method: str,
        path: str,
        payload: dict[str, Any] | None = None,
        *,
        request_id: str | None = None,
    ) -> tuple[dict[str, Any], str | None]:
        import asyncio
        import aiohttp

        session = await self._get_session()
        url = f"{self.base_url}{path}"
        headers = self._headers(request_id)
        data = json.dumps(payload) if payload is not None else None

        for attempt in range(self.max_retries + 1):
            try:
                async with session.request(
                    method, url, data=data, headers=headers
                ) as resp:
                    raw = await resp.text()
                    rid = resp.headers.get("x-mnemo-request-id")
                    body: dict[str, Any] = {} if not raw else json.loads(raw)

                    if resp.status < 400:
                        return body, rid

                    message = _aio_extract_message(body)
                    error_code = _aio_extract_code(body)

                    if resp.status == 429:
                        retry_ms = _aio_extract_retry_ms(body)
                        raise MnemoRateLimitError(
                            resp.status,
                            message,
                            retry_after_ms=retry_ms,
                            error_code=error_code,
                            body=body,
                            request_id=rid,
                        )
                    elif resp.status == 404:
                        raise MnemoNotFoundError(
                            resp.status,
                            message,
                            error_code=error_code,
                            body=body,
                            request_id=rid,
                        )
                    elif resp.status == 400:
                        raise MnemoValidationError(
                            resp.status,
                            message,
                            error_code=error_code,
                            body=body,
                            request_id=rid,
                        )
                    else:
                        err = MnemoHttpError(
                            resp.status,
                            message,
                            error_code=error_code,
                            body=body,
                            request_id=rid,
                        )
                        if attempt >= self.max_retries or resp.status < 500:
                            raise err
            except (MnemoRateLimitError, MnemoNotFoundError, MnemoValidationError):
                raise
            except MnemoHttpError:
                if attempt >= self.max_retries:
                    raise
            except asyncio.TimeoutError as exc:
                if attempt >= self.max_retries:
                    raise MnemoTimeoutError("Request timed out") from exc
            except aiohttp.ClientError as exc:
                if attempt >= self.max_retries:
                    raise MnemoConnectionError(f"Connection failed: {exc}") from exc

            await asyncio.sleep(self.retry_backoff_s * (attempt + 1))

        raise MnemoError("Exhausted retries")  # unreachable but satisfies type checker

    # ── Health ──────────────────────────────────────────────────────

    async def health(self, *, request_id: str | None = None) -> HealthResult:
        body, rid = await self._req("GET", "/health", request_id=request_id)
        return HealthResult(
            status=str(body.get("status", "")),
            version=str(body.get("version", "")),
            request_id=rid,
        )

    # ── High-level memory ───────────────────────────────────────────

    async def add(
        self,
        user: str,
        text: str,
        *,
        session: str | None = None,
        role: str = "user",
        request_id: str | None = None,
    ) -> RememberResult:
        payload: dict[str, Any] = {"user": user, "text": text, "role": role}
        opt(payload, "session", session)
        body, rid = await self._req(
            "POST", "/api/v1/memory", payload, request_id=request_id
        )
        return RememberResult(
            ok=bool(body.get("ok")),
            user_id=str(body.get("user_id", "")),
            session_id=str(body.get("session_id", "")),
            episode_id=str(body.get("episode_id", "")),
            request_id=rid,
        )

    async def context(
        self,
        user: str,
        query: str,
        *,
        session: str | None = None,
        max_tokens: int | None = None,
        min_relevance: float | None = None,
        mode: str | None = None,
        contract: str | None = None,
        retrieval_policy: str | None = None,
        time_intent: str | None = None,
        as_of: str | None = None,
        temporal_weight: float | None = None,
        filters: dict[str, Any] | None = None,
        request_id: str | None = None,
    ) -> ContextResult:
        payload: dict[str, Any] = {"query": query}
        opt(payload, "session", session)
        opt(payload, "max_tokens", max_tokens)
        opt(payload, "min_relevance", min_relevance)
        opt(payload, "mode", mode)
        opt(payload, "contract", contract)
        opt(payload, "retrieval_policy", retrieval_policy)
        opt(payload, "time_intent", time_intent)
        opt(payload, "as_of", as_of)
        opt(payload, "temporal_weight", temporal_weight)
        opt(payload, "filters", filters)
        body, rid = await self._req(
            "POST", f"/api/v1/memory/{user}/context", payload, request_id=request_id
        )
        return _parse_context(body, rid)

    async def changes_since(
        self,
        user: str,
        *,
        from_dt: str,
        to_dt: str,
        session: str | None = None,
        request_id: str | None = None,
    ) -> ChangesSinceResult:
        payload: dict[str, Any] = {"from_dt": from_dt, "to_dt": to_dt}
        opt(payload, "session", session)
        body, rid = await self._req(
            "POST",
            f"/api/v1/memory/{user}/changes_since",
            payload,
            request_id=request_id,
        )
        return ChangesSinceResult(
            facts_added=list(body.get("facts_added", [])),
            facts_superseded=list(body.get("facts_superseded", [])),
            entities_updated=list(body.get("entities_updated", [])),
            from_dt=str(body.get("from_dt", from_dt)),
            to_dt=str(body.get("to_dt", to_dt)),
            request_id=rid,
        )

    async def conflict_radar(
        self, user: str, *, request_id: str | None = None
    ) -> ConflictRadarResult:
        body, rid = await self._req(
            "POST", f"/api/v1/memory/{user}/conflict_radar", {}, request_id=request_id
        )
        return ConflictRadarResult(
            conflicts=list(body.get("conflicts", [])),
            user_id=str(body.get("user_id", "")),
            request_id=rid,
        )

    async def causal_recall(
        self, user: str, query: str, *, request_id: str | None = None
    ) -> CausalRecallResult:
        body, rid = await self._req(
            "POST",
            f"/api/v1/memory/{user}/causal_recall",
            {"query": query},
            request_id=request_id,
        )
        return CausalRecallResult(
            chains=list(body.get("chains", [])),
            query=str(body.get("query", query)),
            request_id=rid,
        )

    async def get_policy(
        self, user: str, *, request_id: str | None = None
    ) -> PolicyResult:
        body, rid = await self._req(
            "GET", f"/api/v1/policies/{user}", request_id=request_id
        )
        return _parse_policy(body, rid)

    async def set_policy(
        self,
        user: str,
        *,
        retention_days_message: int | None = None,
        retention_days_text: int | None = None,
        retention_days_json: int | None = None,
        webhook_domain_allowlist: list[str] | None = None,
        default_memory_contract: str | None = None,
        default_retrieval_policy: str | None = None,
        request_id: str | None = None,
    ) -> PolicyResult:
        payload: dict[str, Any] = {}
        opt(payload, "retention_days_message", retention_days_message)
        opt(payload, "retention_days_text", retention_days_text)
        opt(payload, "retention_days_json", retention_days_json)
        opt(payload, "webhook_domain_allowlist", webhook_domain_allowlist)
        opt(payload, "default_memory_contract", default_memory_contract)
        opt(payload, "default_retrieval_policy", default_retrieval_policy)
        body, rid = await self._req(
            "PUT", f"/api/v1/policies/{user}", payload, request_id=request_id
        )
        return _parse_policy(body, rid)

    async def ops_summary(
        self, *, window_seconds: int = 300, request_id: str | None = None
    ) -> OpsSummaryResult:
        body, rid = await self._req(
            "GET",
            f"/api/v1/ops/summary?window_seconds={window_seconds}",
            request_id=request_id,
        )
        return OpsSummaryResult(
            http_requests_total=int(body.get("http_requests_total", 0)),
            http_responses_2xx=int(body.get("http_responses_2xx", 0)),
            http_responses_4xx=int(body.get("http_responses_4xx", 0)),
            http_responses_5xx=int(body.get("http_responses_5xx", 0)),
            policy_updates=int(body.get("policy_update_total", 0)),
            policy_violations=int(body.get("policy_violation_total", 0)),
            webhook_delivered=int(body.get("webhook_deliveries_success_total", 0)),
            webhook_failed=int(body.get("webhook_deliveries_failure_total", 0)),
            webhook_dead_letter=int(body.get("webhook_dead_letter_total", 0)),
            governance_events=int(body.get("governance_audit_total", 0)),
            request_id=rid,
        )

    async def get_messages(
        self,
        session_id: str,
        *,
        limit: int = 100,
        after: str | None = None,
        request_id: str | None = None,
    ) -> MessagesResult:
        path = f"/api/v1/sessions/{session_id}/messages?limit={limit}"
        if after:
            path += f"&after={after}"
        body, rid = await self._req("GET", path, request_id=request_id)
        messages = [
            Message(
                idx=int(m.get("idx", i)),
                id=str(m.get("id", "")),
                role=m.get("role"),
                content=str(m.get("content", "")),
                created_at=str(m.get("created_at", "")),
            )
            for i, m in enumerate(body.get("messages", []))
        ]
        return MessagesResult(
            messages=messages,
            count=int(body.get("count", len(messages))),
            session_id=str(body.get("session_id", session_id)),
            request_id=rid,
        )

    async def clear_messages(
        self, session_id: str, *, request_id: str | None = None
    ) -> DeleteResult:
        body, rid = await self._req(
            "DELETE", f"/api/v1/sessions/{session_id}/messages", request_id=request_id
        )
        return DeleteResult(deleted=True, request_id=rid)

    async def delete_message(
        self, session_id: str, idx: int, *, request_id: str | None = None
    ) -> DeleteResult:
        body, rid = await self._req(
            "DELETE",
            f"/api/v1/sessions/{session_id}/messages/{idx}",
            request_id=request_id,
        )
        return DeleteResult(deleted=bool(body.get("deleted")), request_id=rid)


def _aio_extract_message(body: dict[str, Any]) -> str:
    err = body.get("error", {})
    if isinstance(err, dict):
        return str(err.get("message", "request failed"))
    return "request failed"


def _aio_extract_code(body: dict[str, Any]) -> str | None:
    err = body.get("error", {})
    if isinstance(err, dict):
        return err.get("code")
    return None


def _aio_extract_retry_ms(body: dict[str, Any]) -> int | None:
    err = body.get("error", {})
    if isinstance(err, dict):
        return err.get("retry_after_ms")
    return None
