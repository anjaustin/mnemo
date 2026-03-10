"""Mnemo synchronous Python client.

Full coverage of the Mnemo API including memory, governance, webhooks,
operator endpoints, import, and session message primitives required by
framework adapters.

Usage:
    from mnemo import Mnemo

    client = Mnemo("http://localhost:8080")
    result = client.add("kendra", "I love hiking in Colorado")
    ctx = client.context("kendra", "What does Kendra love doing outdoors?")
    print(ctx.text)
"""

from __future__ import annotations

from typing import Any

from mnemo._errors import (  # noqa: F401 — re-exported for convenience
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
    LlmSpan,
    MemoryDigestResult,
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
from mnemo._transport import SyncTransport, opt


class Mnemo:
    """Synchronous Mnemo client.

    All methods are blocking. For async usage see :class:`mnemo.AsyncMnemo`.
    """

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
        self._transport = SyncTransport(
            base_url=base_url,
            api_key=api_key,
            timeout_s=timeout_s,
            max_retries=max_retries,
            retry_backoff_s=retry_backoff_s,
            default_request_id=request_id,
        )

    # ── Internal request helper ─────────────────────────────────────

    def _req(
        self,
        method: str,
        path: str,
        payload: dict[str, Any] | None = None,
        *,
        request_id: str | None = None,
    ) -> tuple[dict[str, Any], str | None]:
        return self._transport.request(method, path, payload, request_id=request_id)

    # ── Health ──────────────────────────────────────────────────────

    def health(self, *, request_id: str | None = None) -> HealthResult:
        """Check server liveness and version."""
        body, rid = self._req("GET", "/health", request_id=request_id)
        return HealthResult(
            status=str(body.get("status", "")),
            version=str(body.get("version", "")),
            request_id=rid,
        )

    # ── High-level memory ───────────────────────────────────────────

    def add(
        self,
        user: str,
        text: str,
        *,
        session: str | None = None,
        role: str = "user",
        request_id: str | None = None,
    ) -> RememberResult:
        """Store a memory for a user. Creates user/session if they don't exist."""
        payload: dict[str, Any] = {"user": user, "text": text, "role": role}
        opt(payload, "session", session)
        body, rid = self._req("POST", "/api/v1/memory", payload, request_id=request_id)
        return RememberResult(
            ok=bool(body.get("ok")),
            user_id=str(body.get("user_id", "")),
            session_id=str(body.get("session_id", "")),
            episode_id=str(body.get("episode_id", "")),
            request_id=rid,
        )

    def context(
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
        """Retrieve memory context for a user (high-level)."""
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
        body, rid = self._req(
            "POST", f"/api/v1/memory/{user}/context", payload, request_id=request_id
        )
        return _parse_context(body, rid)

    def context_head(
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
        return self.context(
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

    def changes_since(
        self,
        user: str,
        *,
        from_dt: str,
        to_dt: str,
        session: str | None = None,
        request_id: str | None = None,
    ) -> ChangesSinceResult:
        """Get memory changes (added/superseded facts) between two timestamps."""
        payload: dict[str, Any] = {"from": from_dt, "to": to_dt}
        opt(payload, "session", session)
        body, rid = self._req(
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

    def conflict_radar(
        self,
        user: str,
        *,
        request_id: str | None = None,
    ) -> ConflictRadarResult:
        """Detect conflicting facts in a user's memory."""
        body, rid = self._req(
            "POST", f"/api/v1/memory/{user}/conflict_radar", {}, request_id=request_id
        )
        return ConflictRadarResult(
            conflicts=list(body.get("conflicts", [])),
            user_id=str(body.get("user_id", "")),
            request_id=rid,
        )

    def causal_recall(
        self,
        user: str,
        query: str,
        *,
        request_id: str | None = None,
    ) -> CausalRecallResult:
        """Retrieve causal reasoning chains for a query."""
        body, rid = self._req(
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

    def time_travel_trace(
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
        body, rid = self._req(
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

    def time_travel_summary(
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
        body, rid = self._req(
            "POST",
            f"/api/v1/memory/{user}/time_travel/summary",
            payload,
            request_id=request_id,
        )
        return TimeTravelSummaryResult(summary=body, request_id=rid)

    # ── Governance / policies ───────────────────────────────────────

    def get_policy(
        self,
        user: str,
        *,
        request_id: str | None = None,
    ) -> PolicyResult:
        """Get the governance policy for a user."""
        body, rid = self._req("GET", f"/api/v1/policies/{user}", request_id=request_id)
        return _parse_policy(body, rid)

    def set_policy(
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
        """Create or update the governance policy for a user."""
        payload: dict[str, Any] = {}
        opt(payload, "retention_days_message", retention_days_message)
        opt(payload, "retention_days_text", retention_days_text)
        opt(payload, "retention_days_json", retention_days_json)
        opt(payload, "webhook_domain_allowlist", webhook_domain_allowlist)
        opt(payload, "default_memory_contract", default_memory_contract)
        opt(payload, "default_retrieval_policy", default_retrieval_policy)
        body, rid = self._req(
            "PUT", f"/api/v1/policies/{user}", payload, request_id=request_id
        )
        return _parse_policy(body, rid)

    def preview_policy(
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
        body, rid = self._req(
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

    def get_policy_audit(
        self,
        user: str,
        *,
        limit: int = 50,
        request_id: str | None = None,
    ) -> list[AuditRecord]:
        """List governance audit events for a user's policy."""
        body, rid = self._req(
            "GET", f"/api/v1/policies/{user}/audit?limit={limit}", request_id=request_id
        )
        return [_parse_audit(r, rid) for r in body.get("audit", [])]

    def get_policy_violations(
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
        body, rid = self._req("GET", path, request_id=request_id)
        return [_parse_audit(r, rid) for r in body.get("audit", [])]

    # ── Webhooks ────────────────────────────────────────────────────

    def create_webhook(
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
        body, rid = self._req(
            "POST", "/api/v1/memory/webhooks", payload, request_id=request_id
        )
        return _parse_webhook(body, rid)

    def get_webhook(
        self,
        webhook_id: str,
        *,
        request_id: str | None = None,
    ) -> WebhookResult:
        """Get a webhook by ID."""
        body, rid = self._req(
            "GET", f"/api/v1/memory/webhooks/{webhook_id}", request_id=request_id
        )
        return _parse_webhook(body, rid)

    def delete_webhook(
        self,
        webhook_id: str,
        *,
        request_id: str | None = None,
    ) -> DeleteResult:
        """Delete a webhook."""
        body, rid = self._req(
            "DELETE", f"/api/v1/memory/webhooks/{webhook_id}", request_id=request_id
        )
        return DeleteResult(deleted=bool(body.get("deleted")), request_id=rid)

    def get_webhook_events(
        self,
        webhook_id: str,
        *,
        limit: int = 20,
        request_id: str | None = None,
    ) -> list[WebhookEvent]:
        """List events for a webhook."""
        body, rid = self._req(
            "GET",
            f"/api/v1/memory/webhooks/{webhook_id}/events?limit={limit}",
            request_id=request_id,
        )
        return [_parse_webhook_event(e, rid) for e in body.get("events", [])]

    def get_dead_letter_events(
        self,
        webhook_id: str,
        *,
        limit: int = 20,
        request_id: str | None = None,
    ) -> list[WebhookEvent]:
        """List dead-letter events for a webhook."""
        body, rid = self._req(
            "GET",
            f"/api/v1/memory/webhooks/{webhook_id}/events/dead-letter?limit={limit}",
            request_id=request_id,
        )
        return [_parse_webhook_event(e, rid) for e in body.get("events", [])]

    def replay_events(
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
        body, rid = self._req("GET", path, request_id=request_id)
        return ReplayResult(
            webhook_id=str(body.get("webhook_id", webhook_id)),
            count=int(body.get("count", 0)),
            events=list(body.get("events", [])),
            next_after_event_id=body.get("next_after_event_id"),
            request_id=rid,
        )

    def retry_event(
        self,
        webhook_id: str,
        event_id: str,
        *,
        force: bool = False,
        request_id: str | None = None,
    ) -> RetryResult:
        """Manually retry a failed webhook event."""
        path = f"/api/v1/memory/webhooks/{webhook_id}/events/{event_id}/retry"
        body, rid = self._req("POST", path, {"force": force}, request_id=request_id)
        return RetryResult(
            webhook_id=str(body.get("webhook_id", webhook_id)),
            event_id=str(body.get("event_id", event_id)),
            queued=bool(body.get("queued", False)),
            reason=str(body.get("reason", "")),
            event=body.get("event"),
            request_id=rid,
        )

    def get_webhook_stats(
        self,
        webhook_id: str,
        *,
        window_seconds: int = 300,
        request_id: str | None = None,
    ) -> WebhookStats:
        """Get delivery stats for a webhook."""
        body, rid = self._req(
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

    def get_webhook_audit(
        self,
        webhook_id: str,
        *,
        limit: int = 20,
        request_id: str | None = None,
    ) -> list[AuditRecord]:
        """List audit events for a webhook."""
        body, rid = self._req(
            "GET",
            f"/api/v1/memory/webhooks/{webhook_id}/audit?limit={limit}",
            request_id=request_id,
        )
        return [_parse_audit(r, rid) for r in body.get("audit", [])]

    # ── Operator ────────────────────────────────────────────────────

    def ops_summary(
        self,
        *,
        window_seconds: int = 300,
        request_id: str | None = None,
    ) -> OpsSummaryResult:
        """Get operator dashboard metrics summary."""
        body, rid = self._req(
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

    def trace_lookup(
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
        body, rid = self._req("GET", path, request_id=request_id)
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

    def import_chat_history(
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
        body, rid = self._req(
            "POST", "/api/v1/import/chat-history", payload, request_id=request_id
        )
        return _parse_import_job(body, rid)

    def get_import_job(
        self,
        job_id: str,
        *,
        request_id: str | None = None,
    ) -> ImportJobResult:
        """Get status of an import job."""
        body, rid = self._req(
            "GET", f"/api/v1/import/jobs/{job_id}", request_id=request_id
        )
        return _parse_import_job(body, rid)

    # ── Knowledge Graph API ─────────────────────────────────────────

    def graph_entities(
        self,
        user: str,
        *,
        limit: int = 20,
        request_id: str | None = None,
    ) -> GraphEntitiesResult:
        """List all entities in the knowledge graph for a user."""
        body, rid = self._req(
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

    def graph_entity(
        self,
        user: str,
        entity_id: str,
        *,
        request_id: str | None = None,
    ) -> GraphEntityDetail:
        """Get a single entity with its adjacency information."""
        body, rid = self._req(
            "GET",
            f"/api/v1/graph/{user}/entities/{entity_id}",
            request_id=request_id,
        )
        return _parse_graph_entity_detail(body, rid)

    def graph_edges(
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
        body, rid = self._req("GET", path, request_id=request_id)
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

    def graph_neighbors(
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
        body, rid = self._req("GET", path, request_id=request_id)
        return GraphNeighborsResult(
            seed_entity_id=str(body.get("seed_entity_id", entity_id)),
            depth=int(body.get("depth", depth)),
            nodes=list(body.get("nodes", [])),
            edges=list(body.get("edges", [])),
            entities_visited=int(body.get("entities_visited", 0)),
            request_id=rid,
        )

    def graph_community(
        self,
        user: str,
        *,
        max_iterations: int = 20,
        request_id: str | None = None,
    ) -> GraphCommunityResult:
        """Detect communities in the user's knowledge graph."""
        body, rid = self._req(
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

    def spans_by_request(
        self,
        request_id_to_lookup: str,
        *,
        request_id: str | None = None,
    ) -> SpansResult:
        """Return all LLM call spans for a given request ID."""
        body, rid = self._req(
            "GET",
            f"/api/v1/spans/request/{request_id_to_lookup}",
            request_id=request_id,
        )
        return _parse_spans_result(body, rid)

    def spans_by_user(
        self,
        user_id: str,
        *,
        limit: int = 100,
        request_id: str | None = None,
    ) -> SpansResult:
        """Return recent LLM spans for a user (by UUID)."""
        body, rid = self._req(
            "GET",
            f"/api/v1/spans/user/{user_id}?limit={limit}",
            request_id=request_id,
        )
        return _parse_spans_result(body, rid)

    # ── Memory Digest (sleep-time compute) ──────────────────────────

    def memory_digest(
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
                     If False, return cached digest or generate if not cached (GET then POST).
        """
        if refresh:
            body, rid = self._req(
                "POST", f"/api/v1/memory/{user}/digest", request_id=request_id
            )
        else:
            try:
                body, rid = self._req(
                    "GET", f"/api/v1/memory/{user}/digest", request_id=request_id
                )
            except MnemoNotFoundError:
                body, rid = self._req(
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

    # ── Session messages (framework adapter primitives) ─────────────

    def get_messages(
        self,
        session_id: str,
        *,
        limit: int = 100,
        after: str | None = None,
        request_id: str | None = None,
    ) -> MessagesResult:
        """Get messages for a session in chronological order.

        Required by :class:`mnemo.ext.langchain.MnemoChatMessageHistory`
        and :class:`mnemo.ext.llamaindex.MnemoChatStore`.
        """
        path = f"/api/v1/sessions/{session_id}/messages?limit={limit}"
        if after:
            path += f"&after={after}"
        body, rid = self._req("GET", path, request_id=request_id)
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

    def clear_messages(
        self,
        session_id: str,
        *,
        request_id: str | None = None,
    ) -> DeleteResult:
        """Clear all messages in a session without deleting the session.

        Required by :class:`mnemo.ext.langchain.MnemoChatMessageHistory`.
        """
        body, rid = self._req(
            "DELETE", f"/api/v1/sessions/{session_id}/messages", request_id=request_id
        )
        return DeleteResult(deleted=True, request_id=rid)

    def delete_message(
        self,
        session_id: str,
        idx: int,
        *,
        request_id: str | None = None,
    ) -> DeleteResult:
        """Delete a specific message by ordinal index.

        Required by :class:`mnemo.ext.llamaindex.MnemoChatStore`.
        """
        body, rid = self._req(
            "DELETE",
            f"/api/v1/sessions/{session_id}/messages/{idx}",
            request_id=request_id,
        )
        return DeleteResult(deleted=bool(body.get("deleted")), request_id=rid)

    def list_sessions(
        self,
        user_id: str,
        *,
        limit: int = 100,
        request_id: str | None = None,
    ) -> SessionsResult:
        """List all sessions for a user (by UUID).

        Required by :class:`mnemo.ext.llamaindex.MnemoChatStore` for
        server-side ``get_keys()``.
        """
        body, rid = self._req(
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


# ─── Parsing helpers ───────────────────────────────────────────────


def _parse_context(body: dict[str, Any], rid: str | None) -> ContextResult:
    return ContextResult(
        text=str(body.get("context", "")),
        token_count=int(body.get("token_count", 0)),
        entities=list(body.get("entities", [])),
        facts=list(body.get("facts", [])),
        episodes=list(body.get("episodes", [])),
        latency_ms=int(body.get("latency_ms", 0)),
        sources=list(body.get("sources", [])),
        mode=str(body.get("mode", "hybrid")),
        head=body.get("head") if isinstance(body.get("head"), dict) else None,
        contract_applied=body.get("contract_applied"),
        retrieval_policy_applied=body.get("retrieval_policy_applied"),
        temporal_diagnostics=body.get("temporal_diagnostics"),
        retrieval_policy_diagnostics=body.get("retrieval_policy_diagnostics"),
        request_id=rid,
    )


def _parse_policy(body: dict[str, Any], rid: str | None) -> PolicyResult:
    p = body.get("policy", body)
    return PolicyResult(
        user_id=str(p.get("user_id", "")),
        retention_days_message=int(p.get("retention_days_message", 0)),
        retention_days_text=int(p.get("retention_days_text", 0)),
        retention_days_json=int(p.get("retention_days_json", 0)),
        webhook_domain_allowlist=list(p.get("webhook_domain_allowlist", [])),
        default_memory_contract=str(p.get("default_memory_contract", "default")),
        default_retrieval_policy=str(p.get("default_retrieval_policy", "balanced")),
        created_at=str(p.get("created_at", "")),
        updated_at=str(p.get("updated_at", "")),
        request_id=rid,
    )


def _parse_audit(r: dict[str, Any], rid: str | None) -> AuditRecord:
    return AuditRecord(
        id=str(r.get("id", "")),
        user_id=str(r.get("user_id", "")),
        event_type=str(r.get("event_type", "")),
        details=dict(r.get("details", {})),
        created_at=str(r.get("created_at", "")),
        request_id=rid,
    )


def _parse_webhook(body: dict[str, Any], rid: str | None) -> WebhookResult:
    w = body.get("webhook", body)
    return WebhookResult(
        id=str(w.get("id", "")),
        user_id=str(w.get("user_id", "")),
        target_url=str(w.get("target_url", "")),
        events=list(w.get("events", [])),
        enabled=bool(w.get("enabled", True)),
        created_at=str(w.get("created_at", "")),
        updated_at=str(w.get("updated_at", "")),
        request_id=rid,
    )


def _parse_webhook_event(e: dict[str, Any], rid: str | None) -> WebhookEvent:
    return WebhookEvent(
        id=str(e.get("id", "")),
        webhook_id=str(e.get("webhook_id", "")),
        event_type=str(e.get("event_type", "")),
        user_id=str(e.get("user_id", "")),
        payload=dict(e.get("payload", {})),
        created_at=str(e.get("created_at", "")),
        attempts=int(e.get("attempts", 0)),
        delivered=bool(e.get("delivered", False)),
        dead_letter=bool(e.get("dead_letter", False)),
        request_id=rid,
    )


def _parse_spans_result(body: dict[str, Any], rid: str | None) -> SpansResult:
    spans = [
        LlmSpan(
            id=str(s.get("id", "")),
            provider=str(s.get("provider", "")),
            model=str(s.get("model", "")),
            operation=str(s.get("operation", "")),
            prompt_tokens=int(s.get("prompt_tokens", 0)),
            completion_tokens=int(s.get("completion_tokens", 0)),
            total_tokens=int(s.get("total_tokens", 0)),
            latency_ms=int(s.get("latency_ms", 0)),
            success=bool(s.get("success", True)),
            started_at=str(s.get("started_at", "")),
            finished_at=str(s.get("finished_at", "")),
            request_id=s.get("request_id"),
            user_id=s.get("user_id"),
            error=s.get("error"),
        )
        for s in body.get("spans", [])
    ]
    return SpansResult(
        spans=spans,
        count=int(body.get("count", len(spans))),
        total_tokens=int(body.get("total_tokens", 0)),
        total_latency_ms=body.get("total_latency_ms"),
        request_id=rid,
    )


def _parse_import_job(body: dict[str, Any], rid: str | None) -> ImportJobResult:
    j = body.get("job", body)
    return ImportJobResult(
        id=str(j.get("id", "")),
        source=str(j.get("source", "")),
        user=str(j.get("user", "")),
        dry_run=bool(j.get("dry_run", False)),
        status=str(j.get("status", "")),
        total_messages=int(j.get("total_messages", 0)),
        imported_messages=int(j.get("imported_messages", 0)),
        failed_messages=int(j.get("failed_messages", 0)),
        sessions_touched=int(j.get("sessions_touched", 0)),
        errors=list(j.get("errors", [])),
        created_at=str(j.get("created_at", "")),
        started_at=j.get("started_at"),
        finished_at=j.get("finished_at"),
        request_id=rid,
    )


def _parse_adjacency_edge(e: dict[str, Any]) -> AdjacencyEdge:
    return AdjacencyEdge(
        id=str(e.get("id", "")),
        label=str(e.get("label", "")),
        fact=str(e.get("fact", "")),
        valid=bool(e.get("valid", True)),
        source_entity_id=e.get("source_entity_id"),
        target_entity_id=e.get("target_entity_id"),
    )


def _parse_graph_entity_detail(
    body: dict[str, Any], rid: str | None
) -> GraphEntityDetail:
    return GraphEntityDetail(
        id=str(body.get("id", "")),
        name=str(body.get("name", "")),
        entity_type=str(body.get("entity_type", "unknown")),
        summary=body.get("summary"),
        mention_count=int(body.get("mention_count", 0)),
        community_id=body.get("community_id"),
        created_at=str(body.get("created_at", "")),
        updated_at=str(body.get("updated_at", "")),
        outgoing_edges=[
            _parse_adjacency_edge(e) for e in body.get("outgoing_edges", [])
        ],
        incoming_edges=[
            _parse_adjacency_edge(e) for e in body.get("incoming_edges", [])
        ],
        request_id=rid,
    )
