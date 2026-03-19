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
    LlmSpan,
    MemoryDigestResult,
    Message,
    MessagesResult,
    OpsSummaryResult,
    PromotionProposalResult,
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
from mnemo._parsers import (
    parse_agent_audit as _parse_agent_audit,
    parse_agent_context as _parse_agent_context,
    parse_agent_identity as _parse_agent_identity,
    parse_audit as _parse_audit,
    parse_context as _parse_context,
    parse_experience_event as _parse_experience_event,
    parse_graph_edges_result as _parse_graph_edges_result,
    parse_graph_entities_result as _parse_graph_entities_result,
    parse_graph_entity_detail as _parse_graph_entity_detail,
    parse_import_job as _parse_import_job,
    parse_policy as _parse_policy,
    parse_promotion_proposal as _parse_promotion_proposal,
    parse_spans_result as _parse_spans_result,
    parse_webhook as _parse_webhook,
    parse_webhook_event as _parse_webhook_event,
)
from mnemo._transport import SyncTransport, opt
from typing import Iterator


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
        include_modalities: list[str] | None = None,
        request_id: str | None = None,
    ) -> ContextResult:
        """Retrieve memory context for a user (high-level).

        Args:
            include_modalities: Filter by content modality. Supported values:
                "text", "image", "audio", "document". Empty or None includes all.
        """
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
        opt(payload, "include_modalities", include_modalities)
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

    def update_webhook(
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
        body, rid = self._req(
            "PATCH",
            f"/api/v1/memory/webhooks/{webhook_id}",
            payload,
            request_id=request_id,
        )
        webhook = body.get("webhook", body)
        return _parse_webhook(webhook, rid)

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
        return _parse_graph_entities_result(body, rid)

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
        return _parse_graph_edges_result(body, rid)

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

    # ── Agent Identity ────────────────────────────────────────────────

    def get_agent_identity(
        self,
        agent_id: str,
        *,
        request_id: str | None = None,
    ) -> AgentIdentityResult:
        """Get the current identity profile for an agent.

        Auto-creates a default profile on first access.
        """
        body, rid = self._req(
            "GET", f"/api/v1/agents/{agent_id}/identity", request_id=request_id
        )
        return _parse_agent_identity(body, rid)

    def update_agent_identity(
        self,
        agent_id: str,
        core: dict[str, Any],
        *,
        request_id: str | None = None,
    ) -> AgentIdentityResult:
        """Replace the agent's identity core (versioned, audited).

        Allowed top-level keys: mission, style, boundaries, capabilities, values, persona.
        Keys containing user/session/episode data are rejected by the contamination guard.
        """
        body, rid = self._req(
            "PUT",
            f"/api/v1/agents/{agent_id}/identity",
            {"core": core},
            request_id=request_id,
        )
        return _parse_agent_identity(body, rid)

    def list_agent_identity_versions(
        self,
        agent_id: str,
        *,
        limit: int = 20,
        request_id: str | None = None,
    ) -> list[AgentIdentityResult]:
        """List historical identity versions (newest first)."""
        body, rid = self._req(
            "GET",
            f"/api/v1/agents/{agent_id}/identity/versions?limit={limit}",
            request_id=request_id,
        )
        return [_parse_agent_identity(v, rid) for v in body.get("versions", [])]

    def list_agent_identity_audit(
        self,
        agent_id: str,
        *,
        limit: int = 50,
        request_id: str | None = None,
    ) -> list[AgentIdentityAuditResult]:
        """List audit trail for agent identity changes (newest first)."""
        body, rid = self._req(
            "GET",
            f"/api/v1/agents/{agent_id}/identity/audit?limit={limit}",
            request_id=request_id,
        )
        return [_parse_agent_audit(a, rid) for a in body.get("audit", [])]

    def rollback_agent_identity(
        self,
        agent_id: str,
        target_version: int,
        *,
        reason: str | None = None,
        request_id: str | None = None,
    ) -> AgentIdentityResult:
        """Rollback agent identity to a previous version.

        Creates a new version with the target version's core.
        """
        payload: dict[str, Any] = {"target_version": target_version}
        if reason is not None:
            payload["reason"] = reason
        body, rid = self._req(
            "POST",
            f"/api/v1/agents/{agent_id}/identity/rollback",
            payload,
            request_id=request_id,
        )
        return _parse_agent_identity(body, rid)

    def add_agent_experience(
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
        body, rid = self._req(
            "POST",
            f"/api/v1/agents/{agent_id}/experience",
            payload,
            request_id=request_id,
        )
        return _parse_experience_event(body, rid)

    def create_promotion_proposal(
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
        """Create a promotion proposal for agent identity evolution.

        Requires >= 3 source_event_ids as evidence gating.
        """
        body, rid = self._req(
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
        return _parse_promotion_proposal(body, rid)

    def list_promotion_proposals(
        self,
        agent_id: str,
        *,
        limit: int = 50,
        request_id: str | None = None,
    ) -> list[PromotionProposalResult]:
        """List promotion proposals for an agent (newest first)."""
        body, rid = self._req(
            "GET",
            f"/api/v1/agents/{agent_id}/promotions?limit={limit}",
            request_id=request_id,
        )
        return [_parse_promotion_proposal(p, rid) for p in body.get("proposals", [])]

    def approve_promotion(
        self,
        agent_id: str,
        proposal_id: str,
        *,
        request_id: str | None = None,
    ) -> PromotionProposalResult:
        """Approve a pending promotion proposal.

        Applies candidate_core to the agent identity (creates new version).
        """
        body, rid = self._req(
            "POST",
            f"/api/v1/agents/{agent_id}/promotions/{proposal_id}/approve",
            {},
            request_id=request_id,
        )
        return _parse_promotion_proposal(body, rid)

    def reject_promotion(
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
        body, rid = self._req(
            "POST",
            f"/api/v1/agents/{agent_id}/promotions/{proposal_id}/reject",
            payload,
            request_id=request_id,
        )
        return _parse_promotion_proposal(body, rid)

    def agent_context(
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
        body, rid = self._req(
            "POST",
            f"/api/v1/agents/{agent_id}/context",
            payload,
            request_id=request_id,
        )
        return _parse_agent_context(body, rid)

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

    # ─── Resource deletion ─────────────────────────────────────────

    def delete_user(
        self,
        user_id: str,
        *,
        request_id: str | None = None,
    ) -> DeleteResult:
        """Delete a user and all associated data (sessions, episodes, vectors).

        .. warning::

           This is irreversible. All memory for the user will be permanently
           removed.
        """
        body, rid = self._req(
            "DELETE", f"/api/v1/users/{user_id}", request_id=request_id
        )
        return DeleteResult(deleted=True, request_id=rid)

    def delete_session(
        self,
        session_id: str,
        *,
        request_id: str | None = None,
    ) -> DeleteResult:
        """Delete a session and all its episodes."""
        body, rid = self._req(
            "DELETE", f"/api/v1/sessions/{session_id}", request_id=request_id
        )
        return DeleteResult(deleted=True, request_id=rid)

    def delete_entity(
        self,
        entity_id: str,
        *,
        request_id: str | None = None,
    ) -> DeleteResult:
        """Delete a graph entity by UUID."""
        body, rid = self._req(
            "DELETE", f"/api/v1/entities/{entity_id}", request_id=request_id
        )
        return DeleteResult(deleted=True, request_id=rid)

    def delete_edge(
        self,
        edge_id: str,
        *,
        request_id: str | None = None,
    ) -> DeleteResult:
        """Delete a graph edge (fact) by UUID."""
        body, rid = self._req(
            "DELETE", f"/api/v1/edges/{edge_id}", request_id=request_id
        )
        return DeleteResult(deleted=True, request_id=rid)

    # ── Multi-modal uploads ─────────────────────────────────────────

    def upload_attachment(
        self,
        episode_id: str,
        file_path: str,
        *,
        request_id: str | None = None,
    ) -> dict[str, Any]:
        """Upload an attachment (image, audio, document) to an episode.

        Args:
            episode_id: The episode to attach to.
            file_path: Path to the file to upload.

        Returns:
            Attachment metadata including id, type, and processing status.
        """
        from pathlib import Path
        import mimetypes

        path = Path(file_path)
        mime_type = mimetypes.guess_type(str(path))[0] or "application/octet-stream"

        with open(path, "rb") as f:
            files = {"file": (path.name, f, mime_type)}
            # Use transport's underlying session for multipart
            url = f"{self._transport.base_url}/api/v1/episodes/{episode_id}/attachments"
            headers = {}
            if self._transport.api_key:
                headers["Authorization"] = f"Bearer {self._transport.api_key}"
            if request_id:
                headers["x-request-id"] = request_id

            import httpx

            response = httpx.post(url, files=files, headers=headers, timeout=60.0)
            response.raise_for_status()
            return response.json()

    def get_attachment(
        self,
        attachment_id: str,
        *,
        request_id: str | None = None,
    ) -> dict[str, Any]:
        """Get attachment metadata and download URL.

        Args:
            attachment_id: The attachment ID.

        Returns:
            Attachment metadata including download_url (pre-signed, expires in 15 min).
        """
        body, rid = self._req(
            "GET", f"/api/v1/attachments/{attachment_id}", request_id=request_id
        )
        return body

    def list_attachments(
        self,
        episode_id: str,
        *,
        limit: int = 20,
        request_id: str | None = None,
    ) -> list[dict[str, Any]]:
        """List attachments for an episode.

        Args:
            episode_id: The episode ID.
            limit: Maximum number of results.

        Returns:
            List of attachment metadata objects.
        """
        body, rid = self._req(
            "GET",
            f"/api/v1/episodes/{episode_id}/attachments?limit={limit}",
            request_id=request_id,
        )
        return body.get("data", [])

    # ── Pagination helpers ──────────────────────────────────────────

    def iter_entities(
        self,
        user: str,
        *,
        page_size: int = 100,
        request_id: str | None = None,
    ) -> Iterator[GraphEntity]:
        """Auto-paginate through all entities for a user.

        Yields :class:`GraphEntity` instances one at a time, fetching
        pages of ``page_size`` behind the scenes.

        Example::

            for entity in client.iter_entities("kendra"):
                print(entity.name, entity.entity_type)
        """
        after: str | None = None
        while True:
            path = f"/api/v1/graph/{user}/entities?limit={page_size}"
            if after:
                path += f"&after={after}"
            body, rid = self._req("GET", path, request_id=request_id)
            from mnemo._parsers import parse_graph_entity

            items = body.get("data", [])
            if not items:
                break
            for raw in items:
                yield parse_graph_entity(raw)
            if len(items) < page_size:
                break
            after = str(items[-1].get("id", ""))

    def iter_sessions(
        self,
        user_id: str,
        *,
        page_size: int = 100,
        request_id: str | None = None,
    ) -> Iterator[SessionInfo]:
        """Auto-paginate through all sessions for a user (by UUID).

        Yields :class:`SessionInfo` instances one at a time.

        Example::

            for session in client.iter_sessions(user_uuid):
                print(session.name, session.episode_count)
        """
        after: str | None = None
        while True:
            path = f"/api/v1/users/{user_id}/sessions?limit={page_size}"
            if after:
                path += f"&after={after}"
            body, rid = self._req("GET", path, request_id=request_id)
            items = body.get("data", [])
            if not items:
                break
            for s in items:
                yield SessionInfo(
                    id=str(s.get("id", "")),
                    name=s.get("name"),
                    user_id=str(s.get("user_id", "")),
                    created_at=str(s.get("created_at", "")),
                    updated_at=str(s.get("updated_at", "")),
                    episode_count=int(s.get("episode_count", 0)),
                )
            if len(items) < page_size:
                break
            after = str(items[-1].get("id", ""))

    def iter_messages(
        self,
        session_id: str,
        *,
        page_size: int = 100,
        request_id: str | None = None,
    ) -> Iterator[Message]:
        """Auto-paginate through all messages in a session.

        Yields :class:`Message` instances one at a time.

        Example::

            for msg in client.iter_messages(session_uuid):
                print(msg.role, msg.content[:60])
        """
        after: str | None = None
        while True:
            path = f"/api/v1/sessions/{session_id}/messages?limit={page_size}"
            if after:
                path += f"&after={after}"
            body, rid = self._req("GET", path, request_id=request_id)
            items = body.get("messages", [])
            if not items:
                break
            for i, m in enumerate(items):
                yield Message(
                    idx=int(m.get("idx", i)),
                    id=str(m.get("id", "")),
                    role=m.get("role"),
                    content=str(m.get("content", "")),
                    created_at=str(m.get("created_at", "")),
                )
            if len(items) < page_size:
                break
            after = str(items[-1].get("id", ""))
