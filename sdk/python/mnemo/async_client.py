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
import math
import random
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
    AgentContextResult,
    AgentIdentityAuditResult,
    AgentIdentityResult,
    AuditRecord,
    CausalRecallResult,
    ChangesSinceResult,
    ConflictRadarResult,
    ContextResult,
    DeleteResult,
    ExperienceEventResult,
    GraphCommunityResult,
    GraphEdge,
    GraphEdgesResult,
    GraphEntitiesResult,
    GraphEntity,
    GraphEntityDetail,
    AdjacencyEdge,
    GraphNeighborsResult,
    HealthResult,
    ImportJobResult,
    MemoryDigestResult,
    PromotionProposalResult,
    Message,
    MessagesResult,
    OpsSummaryResult,
    PolicyPreviewResult,
    PolicyResult,
    RememberResult,
    ReplayResult,
    RetryResult,
    SessionInfo,
    SessionsResult,
    SpansResult,
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
    _parse_graph_entity_detail,
    _parse_import_job,
    _parse_policy,
    _parse_spans_result,
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
                        err_429 = MnemoRateLimitError(
                            resp.status,
                            message,
                            retry_after_ms=retry_ms,
                            error_code=error_code,
                            body=body,
                            request_id=rid,
                        )
                        if attempt >= self.max_retries:
                            raise err_429
                        # Fall through to retry with exponential backoff
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
            except (MnemoNotFoundError, MnemoValidationError):
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

            # Exponential backoff with full jitter: base * 2^attempt * U(0,1)
            delay = self.retry_backoff_s * math.pow(2, attempt) * random.random()
            await asyncio.sleep(delay)

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
        payload: dict[str, Any] = {"from": from_dt, "to": to_dt}
        opt(payload, "session", session)
        body, rid = await self._req(
            "POST",
            f"/api/v1/memory/{user}/changes_since",
            payload,
            request_id=request_id,
        )
        return ChangesSinceResult(
            added_facts=list(body.get("added_facts", [])),
            superseded_facts=list(body.get("superseded_facts", [])),
            confidence_deltas=list(body.get("confidence_deltas", [])),
            head_changes=list(body.get("head_changes", [])),
            added_episodes=list(body.get("added_episodes", [])),
            summary=str(body.get("summary", "")),
            from_dt=str(body.get("from", from_dt)),
            to_dt=str(body.get("to", to_dt)),
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
            window_seconds=int(body.get("window_seconds", window_seconds)),
            http_requests_total=int(body.get("http_requests_total", 0)),
            http_responses_2xx=int(body.get("http_responses_2xx", 0)),
            http_responses_4xx=int(body.get("http_responses_4xx", 0)),
            http_responses_5xx=int(body.get("http_responses_5xx", 0)),
            policy_update_total=int(body.get("policy_update_total", 0)),
            policy_violation_total=int(body.get("policy_violation_total", 0)),
            webhook_deliveries_success_total=int(
                body.get("webhook_deliveries_success_total", 0)
            ),
            webhook_deliveries_failure_total=int(
                body.get("webhook_deliveries_failure_total", 0)
            ),
            webhook_dead_letter_total=int(body.get("webhook_dead_letter_total", 0)),
            active_webhooks=int(body.get("active_webhooks", 0)),
            dead_letter_backlog=int(body.get("dead_letter_backlog", 0)),
            pending_webhook_events=int(body.get("pending_webhook_events", 0)),
            governance_audit_events_in_window=int(
                body.get("governance_audit_events_in_window", 0)
            ),
            webhook_audit_events_in_window=int(
                body.get("webhook_audit_events_in_window", 0)
            ),
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

    async def list_sessions(
        self,
        user_id: str,
        *,
        limit: int = 100,
        request_id: str | None = None,
    ) -> SessionsResult:
        """List all sessions for a user (by UUID)."""
        body, rid = await self._req(
            "GET",
            f"/api/v1/users/{user_id}/sessions?limit={limit}",
            request_id=request_id,
        )
        sessions = [
            SessionInfo(
                id=str(s.get("id", "")),
                name=s.get("name"),
                user_id=str(s.get("user_id", "")),
                created_at=str(s.get("created_at", "")),
                updated_at=str(s.get("updated_at", "")),
                episode_count=int(s.get("episode_count", 0)),
            )
            for s in body.get("data", [])
        ]
        return SessionsResult(
            sessions=sessions,
            count=int(body.get("count", len(sessions))),
            request_id=rid,
        )

    # ── Time-travel ─────────────────────────────────────────────────

    async def context_head(
        self,
        user: str,
        query: str,
        *,
        session: str | None = None,
        max_tokens: int | None = None,
        min_relevance: float | None = None,
        time_intent: str | None = None,
        temporal_weight: float | None = None,
        request_id: str | None = None,
    ) -> ContextResult:
        """Retrieve only the most recent session head (fast path)."""
        return await self.context(
            user,
            query,
            session=session,
            max_tokens=max_tokens,
            min_relevance=min_relevance,
            mode="head",
            time_intent=time_intent,
            temporal_weight=temporal_weight,
            request_id=request_id,
        )

    async def time_travel_trace(
        self,
        user: str,
        query: str,
        *,
        from_dt: str,
        to_dt: str,
        session: str | None = None,
        contract: str | None = None,
        retrieval_policy: str | None = None,
        max_tokens: int | None = None,
        min_relevance: float | None = None,
        request_id: str | None = None,
    ) -> TimeTravelTraceResult:
        """Diff memory snapshots over a time window."""
        payload: dict[str, Any] = {
            "query": query,
            "from": from_dt,
            "to": to_dt,
        }
        opt(payload, "session", session)
        opt(payload, "contract", contract)
        opt(payload, "retrieval_policy", retrieval_policy)
        opt(payload, "max_tokens", max_tokens)
        opt(payload, "min_relevance", min_relevance)
        body, rid = await self._req(
            "POST",
            f"/api/v1/memory/{user}/time_travel/trace",
            payload,
            request_id=request_id,
        )
        return TimeTravelTraceResult(
            snapshot_from=dict(body.get("snapshot_from", {})),
            snapshot_to=dict(body.get("snapshot_to", {})),
            gained_facts=list(body.get("gained_facts", [])),
            lost_facts=list(body.get("lost_facts", [])),
            gained_episodes=list(body.get("gained_episodes", [])),
            lost_episodes=list(body.get("lost_episodes", [])),
            timeline=list(body.get("timeline", [])),
            summary=str(body.get("summary", "")),
            from_dt=str(body.get("from", from_dt)),
            to_dt=str(body.get("to", to_dt)),
            request_id=rid,
        )

    async def time_travel_summary(
        self,
        user: str,
        query: str,
        *,
        from_dt: str,
        to_dt: str,
        session: str | None = None,
        request_id: str | None = None,
    ) -> TimeTravelSummaryResult:
        """Lightweight snapshot delta counts for fast rendering."""
        payload: dict[str, Any] = {
            "query": query,
            "from": from_dt,
            "to": to_dt,
        }
        opt(payload, "session", session)
        body, rid = await self._req(
            "POST",
            f"/api/v1/memory/{user}/time_travel/summary",
            payload,
            request_id=request_id,
        )
        return TimeTravelSummaryResult(summary=body, request_id=rid)

    # ── Governance / policies (extended) ────────────────────────────

    async def preview_policy(
        self,
        user: str,
        *,
        retention_days_message: int | None = None,
        retention_days_text: int | None = None,
        retention_days_json: int | None = None,
        request_id: str | None = None,
    ) -> PolicyPreviewResult:
        """Estimate impact of a policy change without applying it."""
        payload: dict[str, Any] = {}
        opt(payload, "retention_days_message", retention_days_message)
        opt(payload, "retention_days_text", retention_days_text)
        opt(payload, "retention_days_json", retention_days_json)
        body, rid = await self._req(
            "POST", f"/api/v1/policies/{user}/preview", payload, request_id=request_id
        )
        return PolicyPreviewResult(
            user_id=str(body.get("user_id", "")),
            current_policy=dict(body.get("current_policy", {})),
            preview_policy=dict(body.get("preview_policy", {})),
            estimated_affected_episodes_total=int(
                body.get("estimated_affected_episodes_total", 0)
            ),
            estimated_affected_message_episodes=int(
                body.get("estimated_affected_message_episodes", 0)
            ),
            estimated_affected_text_episodes=int(
                body.get("estimated_affected_text_episodes", 0)
            ),
            estimated_affected_json_episodes=int(
                body.get("estimated_affected_json_episodes", 0)
            ),
            confidence=str(body.get("confidence", "")),
            request_id=rid,
        )

    async def get_policy_audit(
        self,
        user: str,
        *,
        limit: int = 50,
        request_id: str | None = None,
    ) -> list[AuditRecord]:
        """List governance audit events for a user's policy."""
        body, rid = await self._req(
            "GET",
            f"/api/v1/policies/{user}/audit?limit={limit}",
            request_id=request_id,
        )
        return [_parse_audit(r, rid) for r in body.get("audit", [])]

    async def get_policy_violations(
        self,
        user: str,
        *,
        from_dt: str,
        to_dt: str,
        limit: int = 50,
        request_id: str | None = None,
    ) -> list[AuditRecord]:
        """List policy violations within a time window."""
        path = (
            f"/api/v1/policies/{user}/violations"
            f"?from={from_dt}&to={to_dt}&limit={limit}"
        )
        body, rid = await self._req("GET", path, request_id=request_id)
        return [_parse_audit(r, rid) for r in body.get("audit", [])]

    # ── Webhooks ────────────────────────────────────────────────────

    async def create_webhook(
        self,
        user: str,
        target_url: str,
        events: list[str],
        *,
        signing_secret: str | None = None,
        request_id: str | None = None,
    ) -> WebhookResult:
        """Register a webhook for memory events."""
        payload: dict[str, Any] = {
            "user": user,
            "target_url": target_url,
            "events": events,
        }
        opt(payload, "signing_secret", signing_secret)
        body, rid = await self._req(
            "POST", "/api/v1/memory/webhooks", payload, request_id=request_id
        )
        return _parse_webhook(body, rid)

    async def get_webhook(
        self,
        webhook_id: str,
        *,
        request_id: str | None = None,
    ) -> WebhookResult:
        """Get a webhook by ID."""
        body, rid = await self._req(
            "GET", f"/api/v1/memory/webhooks/{webhook_id}", request_id=request_id
        )
        return _parse_webhook(body, rid)

    async def update_webhook(
        self,
        webhook_id: str,
        *,
        target_url: str | None = None,
        events: list[str] | None = None,
        enabled: bool | None = None,
        signing_secret: str | None = None,
        request_id: str | None = None,
    ) -> WebhookResult:
        """Update a webhook subscription (partial update).

        Only the fields provided will be changed. TLS enforcement and
        domain allowlist policies are applied to ``target_url`` changes.
        """
        payload: dict[str, Any] = {}
        if target_url is not None:
            payload["target_url"] = target_url
        if events is not None:
            payload["events"] = events
        if enabled is not None:
            payload["enabled"] = enabled
        if signing_secret is not None:
            payload["signing_secret"] = signing_secret
        body, rid = await self._req(
            "PATCH",
            f"/api/v1/memory/webhooks/{webhook_id}",
            payload,
            request_id=request_id,
        )
        webhook = body.get("webhook", body)
        return _parse_webhook(webhook, rid)

    async def delete_webhook(
        self,
        webhook_id: str,
        *,
        request_id: str | None = None,
    ) -> DeleteResult:
        """Delete a webhook."""
        body, rid = await self._req(
            "DELETE", f"/api/v1/memory/webhooks/{webhook_id}", request_id=request_id
        )
        return DeleteResult(deleted=bool(body.get("deleted")), request_id=rid)

    async def get_webhook_events(
        self,
        webhook_id: str,
        *,
        limit: int = 20,
        request_id: str | None = None,
    ) -> list[WebhookEvent]:
        """List events for a webhook."""
        body, rid = await self._req(
            "GET",
            f"/api/v1/memory/webhooks/{webhook_id}/events?limit={limit}",
            request_id=request_id,
        )
        return [_parse_webhook_event(e, rid) for e in body.get("events", [])]

    async def get_dead_letter_events(
        self,
        webhook_id: str,
        *,
        limit: int = 20,
        request_id: str | None = None,
    ) -> list[WebhookEvent]:
        """List dead-letter events for a webhook."""
        body, rid = await self._req(
            "GET",
            f"/api/v1/memory/webhooks/{webhook_id}/events/dead-letter?limit={limit}",
            request_id=request_id,
        )
        return [_parse_webhook_event(e, rid) for e in body.get("events", [])]

    async def replay_events(
        self,
        webhook_id: str,
        *,
        after_event_id: str | None = None,
        limit: int = 100,
        include_delivered: bool = True,
        include_dead_letter: bool = True,
        request_id: str | None = None,
    ) -> ReplayResult:
        """Replay webhook events from a cursor."""
        path = (
            f"/api/v1/memory/webhooks/{webhook_id}/events/replay"
            f"?limit={limit}"
            f"&include_delivered={str(include_delivered).lower()}"
            f"&include_dead_letter={str(include_dead_letter).lower()}"
        )
        if after_event_id:
            path += f"&after_event_id={after_event_id}"
        body, rid = await self._req("GET", path, request_id=request_id)
        return ReplayResult(
            webhook_id=str(body.get("webhook_id", webhook_id)),
            count=int(body.get("count", 0)),
            events=list(body.get("events", [])),
            next_after_event_id=body.get("next_after_event_id"),
            request_id=rid,
        )

    async def retry_event(
        self,
        webhook_id: str,
        event_id: str,
        *,
        force: bool = False,
        request_id: str | None = None,
    ) -> RetryResult:
        """Manually retry a failed webhook event."""
        path = f"/api/v1/memory/webhooks/{webhook_id}/events/{event_id}/retry"
        body, rid = await self._req(
            "POST", path, {"force": force}, request_id=request_id
        )
        return RetryResult(
            webhook_id=str(body.get("webhook_id", webhook_id)),
            event_id=str(body.get("event_id", event_id)),
            queued=bool(body.get("queued", False)),
            reason=str(body.get("reason", "")),
            event=body.get("event"),
            request_id=rid,
        )

    async def get_webhook_stats(
        self,
        webhook_id: str,
        *,
        window_seconds: int = 300,
        request_id: str | None = None,
    ) -> WebhookStats:
        """Get delivery stats for a webhook."""
        body, rid = await self._req(
            "GET",
            f"/api/v1/memory/webhooks/{webhook_id}/stats?window_seconds={window_seconds}",
            request_id=request_id,
        )
        return WebhookStats(
            webhook_id=str(body.get("webhook_id", webhook_id)),
            total_events=int(body.get("total_events", 0)),
            delivered_events=int(body.get("delivered_events", 0)),
            pending_events=int(body.get("pending_events", 0)),
            dead_letter_events=int(body.get("dead_letter_events", 0)),
            failed_events=int(body.get("failed_events", 0)),
            recent_failures=int(body.get("recent_failures", 0)),
            circuit_open=bool(body.get("circuit_open", False)),
            circuit_open_until=body.get("circuit_open_until"),
            rate_limit_per_minute=int(body.get("rate_limit_per_minute", 0)),
            request_id=rid,
        )

    async def get_webhook_audit(
        self,
        webhook_id: str,
        *,
        limit: int = 20,
        request_id: str | None = None,
    ) -> list[AuditRecord]:
        """List audit events for a webhook."""
        body, rid = await self._req(
            "GET",
            f"/api/v1/memory/webhooks/{webhook_id}/audit?limit={limit}",
            request_id=request_id,
        )
        return [_parse_audit(r, rid) for r in body.get("audit", [])]

    # ── Operator ────────────────────────────────────────────────────

    async def trace_lookup(
        self,
        request_id_to_find: str,
        *,
        from_dt: str | None = None,
        to_dt: str | None = None,
        limit: int = 100,
        request_id: str | None = None,
    ) -> TraceLookupResult:
        """Look up cross-pipeline trace by request correlation ID."""
        path = f"/api/v1/traces/{request_id_to_find}?limit={limit}"
        if from_dt:
            path += f"&from={from_dt}"
        if to_dt:
            path += f"&to={to_dt}"
        body, rid = await self._req("GET", path, request_id=request_id)
        return TraceLookupResult(
            request_id=request_id_to_find,
            matched_episodes=list(body.get("matched_episodes", [])),
            matched_webhook_events=list(body.get("matched_webhook_events", [])),
            matched_webhook_audit=list(body.get("matched_webhook_audit", [])),
            matched_governance_audit=list(body.get("matched_governance_audit", [])),
            summary=dict(body.get("summary", {})),
            sdk_request_id=rid,
        )

    # ── Import ──────────────────────────────────────────────────────

    async def import_chat_history(
        self,
        user: str,
        source: str,
        payload_data: dict[str, Any],
        *,
        idempotency_key: str | None = None,
        dry_run: bool = False,
        default_session: str | None = None,
        request_id: str | None = None,
    ) -> ImportJobResult:
        """Start an async chat history import job."""
        payload: dict[str, Any] = {
            "user": user,
            "source": source,
            "payload": payload_data,
            "dry_run": dry_run,
        }
        opt(payload, "idempotency_key", idempotency_key)
        opt(payload, "default_session", default_session)
        body, rid = await self._req(
            "POST", "/api/v1/import/chat-history", payload, request_id=request_id
        )
        return _parse_import_job(body, rid)

    async def get_import_job(
        self,
        job_id: str,
        *,
        request_id: str | None = None,
    ) -> ImportJobResult:
        """Get status of an import job."""
        body, rid = await self._req(
            "GET", f"/api/v1/import/jobs/{job_id}", request_id=request_id
        )
        return _parse_import_job(body, rid)

    # ── Knowledge Graph API ─────────────────────────────────────────

    async def graph_entities(
        self,
        user: str,
        *,
        limit: int = 20,
        request_id: str | None = None,
    ) -> GraphEntitiesResult:
        """List all entities in the knowledge graph for a user."""
        body, rid = await self._req(
            "GET",
            f"/api/v1/graph/{user}/entities?limit={limit}",
            request_id=request_id,
        )
        entities = [
            GraphEntity(
                id=str(e.get("id", "")),
                name=str(e.get("name", "")),
                entity_type=str(e.get("entity_type", "unknown")),
                summary=e.get("summary"),
                mention_count=int(e.get("mention_count", 0)),
                community_id=e.get("community_id"),
                created_at=str(e.get("created_at", "")),
                updated_at=str(e.get("updated_at", "")),
            )
            for e in body.get("data", [])
        ]
        return GraphEntitiesResult(
            data=entities,
            count=int(body.get("count", len(entities))),
            user_id=str(body.get("user_id", "")),
            request_id=rid,
        )

    async def graph_entity(
        self,
        user: str,
        entity_id: str,
        *,
        request_id: str | None = None,
    ) -> GraphEntityDetail:
        """Get a single entity with adjacency information."""
        body, rid = await self._req(
            "GET",
            f"/api/v1/graph/{user}/entities/{entity_id}",
            request_id=request_id,
        )
        return _parse_graph_entity_detail(body, rid)

    async def graph_edges(
        self,
        user: str,
        *,
        limit: int = 20,
        label: str | None = None,
        valid_only: bool = True,
        request_id: str | None = None,
    ) -> GraphEdgesResult:
        """List edges in the knowledge graph for a user."""
        path = f"/api/v1/graph/{user}/edges?limit={limit}&valid_only={str(valid_only).lower()}"
        if label:
            path += f"&label={label}"
        body, rid = await self._req("GET", path, request_id=request_id)
        edges = [
            GraphEdge(
                id=str(e.get("id", "")),
                source_entity_id=str(e.get("source_entity_id", "")),
                target_entity_id=str(e.get("target_entity_id", "")),
                label=str(e.get("label", "")),
                fact=str(e.get("fact", "")),
                confidence=float(e.get("confidence", 1.0)),
                valid=bool(e.get("valid", True)),
                valid_at=str(e.get("valid_at", "")),
                invalid_at=e.get("invalid_at"),
                created_at=str(e.get("created_at", "")),
            )
            for e in body.get("data", [])
        ]
        return GraphEdgesResult(
            data=edges,
            count=int(body.get("count", len(edges))),
            user_id=str(body.get("user_id", "")),
            request_id=rid,
        )

    async def graph_neighbors(
        self,
        user: str,
        entity_id: str,
        *,
        depth: int = 1,
        max_nodes: int = 50,
        valid_only: bool = True,
        request_id: str | None = None,
    ) -> GraphNeighborsResult:
        """Return the neighborhood (BFS subgraph) around an entity."""
        path = (
            f"/api/v1/graph/{user}/neighbors/{entity_id}"
            f"?depth={depth}&max_nodes={max_nodes}&valid_only={str(valid_only).lower()}"
        )
        body, rid = await self._req("GET", path, request_id=request_id)
        return GraphNeighborsResult(
            seed_entity_id=str(body.get("seed_entity_id", entity_id)),
            depth=int(body.get("depth", depth)),
            nodes=list(body.get("nodes", [])),
            edges=list(body.get("edges", [])),
            entities_visited=int(body.get("entities_visited", 0)),
            request_id=rid,
        )

    async def graph_community(
        self,
        user: str,
        *,
        max_iterations: int = 20,
        request_id: str | None = None,
    ) -> GraphCommunityResult:
        """Detect communities in the user's knowledge graph."""
        body, rid = await self._req(
            "GET",
            f"/api/v1/graph/{user}/community?max_iterations={max_iterations}",
            request_id=request_id,
        )
        return GraphCommunityResult(
            user_id=str(body.get("user_id", "")),
            total_entities=int(body.get("total_entities", 0)),
            community_count=int(body.get("community_count", 0)),
            communities=list(body.get("communities", [])),
            request_id=rid,
        )

    # ── LLM Span Tracing ────────────────────────────────────────────

    async def spans_by_request(
        self,
        request_id_to_lookup: str,
        *,
        request_id: str | None = None,
    ) -> SpansResult:
        """Return all LLM call spans for a given request ID."""
        body, rid = await self._req(
            "GET",
            f"/api/v1/spans/request/{request_id_to_lookup}",
            request_id=request_id,
        )
        return _parse_spans_result(body, rid)

    async def spans_by_user(
        self,
        user_id: str,
        *,
        limit: int = 100,
        request_id: str | None = None,
    ) -> SpansResult:
        """Return recent LLM spans for a user (by UUID)."""
        body, rid = await self._req(
            "GET",
            f"/api/v1/spans/user/{user_id}?limit={limit}",
            request_id=request_id,
        )
        return _parse_spans_result(body, rid)

    # ── Memory Digest (sleep-time compute) ──────────────────────────

    async def memory_digest(
        self,
        user: str,
        *,
        refresh: bool = False,
        request_id: str | None = None,
    ) -> MemoryDigestResult:
        """Get the cached memory digest for a user.

        Args:
            user: Username or UUID.
            refresh: If True, generate a fresh digest via the LLM (POST).
        """
        if refresh:
            body, rid = await self._req(
                "POST", f"/api/v1/memory/{user}/digest", request_id=request_id
            )
        else:
            try:
                body, rid = await self._req(
                    "GET", f"/api/v1/memory/{user}/digest", request_id=request_id
                )
            except MnemoNotFoundError:
                body, rid = await self._req(
                    "POST", f"/api/v1/memory/{user}/digest", request_id=request_id
                )
        return MemoryDigestResult(
            user_id=str(body.get("user_id", "")),
            summary=str(body.get("summary", "")),
            entity_count=int(body.get("entity_count", 0)),
            edge_count=int(body.get("edge_count", 0)),
            dominant_topics=list(body.get("dominant_topics", [])),
            generated_at=str(body.get("generated_at", "")),
            model=str(body.get("model", "")),
            request_id=rid,
        )

    # ── Agent Identity ────────────────────────────────────────────────

    async def get_agent_identity(
        self,
        agent_id: str,
        *,
        request_id: str | None = None,
    ) -> AgentIdentityResult:
        """Get the current identity profile for an agent."""
        body, rid = await self._req(
            "GET", f"/api/v1/agents/{agent_id}/identity", request_id=request_id
        )
        return _aio_parse_agent_identity(body, rid)

    async def update_agent_identity(
        self,
        agent_id: str,
        core: dict[str, Any],
        *,
        request_id: str | None = None,
    ) -> AgentIdentityResult:
        """Replace the agent's identity core (versioned, audited)."""
        body, rid = await self._req(
            "PUT",
            f"/api/v1/agents/{agent_id}/identity",
            {"core": core},
            request_id=request_id,
        )
        return _aio_parse_agent_identity(body, rid)

    async def list_agent_identity_versions(
        self,
        agent_id: str,
        *,
        limit: int = 20,
        request_id: str | None = None,
    ) -> list[AgentIdentityResult]:
        """List historical identity versions (newest first)."""
        body, rid = await self._req(
            "GET",
            f"/api/v1/agents/{agent_id}/identity/versions?limit={limit}",
            request_id=request_id,
        )
        return [_aio_parse_agent_identity(v, rid) for v in body.get("versions", [])]

    async def list_agent_identity_audit(
        self,
        agent_id: str,
        *,
        limit: int = 50,
        request_id: str | None = None,
    ) -> list[AgentIdentityAuditResult]:
        """List audit trail for agent identity changes (newest first)."""
        body, rid = await self._req(
            "GET",
            f"/api/v1/agents/{agent_id}/identity/audit?limit={limit}",
            request_id=request_id,
        )
        return [_aio_parse_agent_audit(a, rid) for a in body.get("audit", [])]

    async def rollback_agent_identity(
        self,
        agent_id: str,
        target_version: int,
        *,
        reason: str | None = None,
        request_id: str | None = None,
    ) -> AgentIdentityResult:
        """Rollback agent identity to a previous version."""
        payload: dict[str, Any] = {"target_version": target_version}
        if reason is not None:
            payload["reason"] = reason
        body, rid = await self._req(
            "POST",
            f"/api/v1/agents/{agent_id}/identity/rollback",
            payload,
            request_id=request_id,
        )
        return _aio_parse_agent_identity(body, rid)

    async def add_agent_experience(
        self,
        agent_id: str,
        *,
        user_id: str,
        session_id: str,
        category: str,
        signal: str,
        confidence: float = 1.0,
        weight: float = 0.5,
        decay_half_life_days: int = 30,
        evidence_episode_ids: list[str] | None = None,
        request_id: str | None = None,
    ) -> ExperienceEventResult:
        """Record an experience event for an agent."""
        payload: dict[str, Any] = {
            "user_id": user_id,
            "session_id": session_id,
            "category": category,
            "signal": signal,
            "confidence": confidence,
            "weight": weight,
            "decay_half_life_days": decay_half_life_days,
        }
        if evidence_episode_ids is not None:
            payload["evidence_episode_ids"] = evidence_episode_ids
        body, rid = await self._req(
            "POST",
            f"/api/v1/agents/{agent_id}/experience",
            payload,
            request_id=request_id,
        )
        return _aio_parse_experience_event(body, rid)

    async def create_promotion_proposal(
        self,
        agent_id: str,
        *,
        proposal: str,
        candidate_core: dict[str, Any],
        reason: str,
        source_event_ids: list[str],
        risk_level: str = "medium",
        request_id: str | None = None,
    ) -> PromotionProposalResult:
        """Create a promotion proposal for agent identity evolution."""
        body, rid = await self._req(
            "POST",
            f"/api/v1/agents/{agent_id}/promotions",
            {
                "proposal": proposal,
                "candidate_core": candidate_core,
                "reason": reason,
                "source_event_ids": source_event_ids,
                "risk_level": risk_level,
            },
            request_id=request_id,
        )
        return _aio_parse_promotion_proposal(body, rid)

    async def list_promotion_proposals(
        self,
        agent_id: str,
        *,
        limit: int = 50,
        request_id: str | None = None,
    ) -> list[PromotionProposalResult]:
        """List promotion proposals for an agent (newest first)."""
        body, rid = await self._req(
            "GET",
            f"/api/v1/agents/{agent_id}/promotions?limit={limit}",
            request_id=request_id,
        )
        return [
            _aio_parse_promotion_proposal(p, rid) for p in body.get("proposals", [])
        ]

    async def approve_promotion(
        self,
        agent_id: str,
        proposal_id: str,
        *,
        request_id: str | None = None,
    ) -> PromotionProposalResult:
        """Approve a pending promotion proposal."""
        body, rid = await self._req(
            "POST",
            f"/api/v1/agents/{agent_id}/promotions/{proposal_id}/approve",
            {},
            request_id=request_id,
        )
        return _aio_parse_promotion_proposal(body, rid)

    async def reject_promotion(
        self,
        agent_id: str,
        proposal_id: str,
        *,
        reason: str | None = None,
        request_id: str | None = None,
    ) -> PromotionProposalResult:
        """Reject a pending promotion proposal."""
        payload: dict[str, Any] = {}
        if reason is not None:
            payload["reason"] = reason
        body, rid = await self._req(
            "POST",
            f"/api/v1/agents/{agent_id}/promotions/{proposal_id}/reject",
            payload,
            request_id=request_id,
        )
        return _aio_parse_promotion_proposal(body, rid)

    async def agent_context(
        self,
        agent_id: str,
        user: str,
        query: str,
        *,
        session: str | None = None,
        max_tokens: int | None = None,
        min_relevance: float | None = None,
        mode: str | None = None,
        request_id: str | None = None,
    ) -> AgentContextResult:
        """Get agent-scoped context combining identity, experience, and user memory."""
        payload: dict[str, Any] = {"user": user, "query": query}
        if session is not None:
            payload["session"] = session
        if max_tokens is not None:
            payload["max_tokens"] = max_tokens
        if min_relevance is not None:
            payload["min_relevance"] = min_relevance
        if mode is not None:
            payload["mode"] = mode
        body, rid = await self._req(
            "POST",
            f"/api/v1/agents/{agent_id}/context",
            payload,
            request_id=request_id,
        )
        return _aio_parse_agent_context(body, rid)


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


# ─── Agent Identity parsers ───────────────────────────────────────


def _aio_parse_agent_identity(
    body: dict[str, Any], rid: str | None
) -> AgentIdentityResult:
    return AgentIdentityResult(
        agent_id=str(body.get("agent_id", "")),
        version=int(body.get("version", 0)),
        core=dict(body.get("core", {})),
        updated_at=str(body.get("updated_at", "")),
        request_id=rid,
    )


def _aio_parse_experience_event(
    body: dict[str, Any], rid: str | None
) -> ExperienceEventResult:
    return ExperienceEventResult(
        id=str(body.get("id", "")),
        agent_id=str(body.get("agent_id", "")),
        user_id=str(body.get("user_id", "")),
        session_id=str(body.get("session_id", "")),
        category=str(body.get("category", "")),
        signal=str(body.get("signal", "")),
        confidence=float(body.get("confidence", 0.0)),
        weight=float(body.get("weight", 0.0)),
        decay_half_life_days=int(body.get("decay_half_life_days", 0)),
        evidence_episode_ids=[str(eid) for eid in body.get("evidence_episode_ids", [])],
        created_at=str(body.get("created_at", "")),
        request_id=rid,
    )


def _aio_parse_agent_audit(
    body: dict[str, Any], rid: str | None
) -> AgentIdentityAuditResult:
    return AgentIdentityAuditResult(
        id=str(body.get("id", "")),
        agent_id=str(body.get("agent_id", "")),
        action=str(body.get("action", "")),
        from_version=body.get("from_version"),
        to_version=body.get("to_version"),
        rollback_to_version=body.get("rollback_to_version"),
        reason=body.get("reason"),
        created_at=str(body.get("created_at", "")),
        request_id=rid,
    )


def _aio_parse_promotion_proposal(
    body: dict[str, Any], rid: str | None
) -> PromotionProposalResult:
    return PromotionProposalResult(
        id=str(body.get("id", "")),
        agent_id=str(body.get("agent_id", "")),
        proposal=str(body.get("proposal", "")),
        candidate_core=dict(body.get("candidate_core", {})),
        reason=str(body.get("reason", "")),
        risk_level=str(body.get("risk_level", "medium")),
        status=str(body.get("status", "")),
        source_event_ids=[str(eid) for eid in body.get("source_event_ids", [])],
        created_at=str(body.get("created_at", "")),
        approved_at=body.get("approved_at"),
        rejected_at=body.get("rejected_at"),
        request_id=rid,
    )


def _aio_parse_agent_context(
    body: dict[str, Any], rid: str | None
) -> AgentContextResult:
    identity_raw = body.get("identity", {})
    return AgentContextResult(
        context=dict(body.get("context", {})),
        identity=_aio_parse_agent_identity(identity_raw, None),
        identity_version=int(body.get("identity_version", 0)),
        experience_events_used=int(body.get("experience_events_used", 0)),
        experience_weight_sum=float(body.get("experience_weight_sum", 0.0)),
        user_memory_items_used=int(body.get("user_memory_items_used", 0)),
        attribution_guards=dict(body.get("attribution_guards", {})),
        request_id=rid,
    )
