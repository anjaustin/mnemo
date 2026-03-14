"""Shared response parsers for sync and async Mnemo clients.

These functions convert raw JSON dicts from the API into typed dataclasses.
Both :class:`mnemo.Mnemo` and :class:`mnemo.AsyncMnemo` import from here.
"""

from __future__ import annotations

from typing import Any

from mnemo._models import (
    AdjacencyEdge,
    AgentContextResult,
    AgentIdentityAuditResult,
    AgentIdentityResult,
    AuditRecord,
    ContextEntitySummary,
    ContextEpisodeSummary,
    ContextFactSummary,
    ContextResult,
    ExperienceEventResult,
    GraphEdge,
    GraphEdgesResult,
    GraphEntitiesResult,
    GraphEntity,
    GraphEntityDetail,
    ImportJobResult,
    LlmSpan,
    PolicyResult,
    PromotionProposalResult,
    SpansResult,
    WebhookEvent,
    WebhookResult,
)


# ─── Context ──────────────────────────────────────────────────────


def parse_context_entity(e: dict[str, Any]) -> ContextEntitySummary:
    return ContextEntitySummary(
        id=str(e.get("id", "")),
        name=str(e.get("name", "")),
        entity_type=str(e.get("entity_type", "")),
        summary=e.get("summary"),
        relevance=float(e.get("relevance", 0.0)),
    )


def parse_context_fact(f: dict[str, Any]) -> ContextFactSummary:
    return ContextFactSummary(
        id=str(f.get("id", "")),
        source_entity=str(f.get("source_entity", "")),
        target_entity=str(f.get("target_entity", "")),
        label=str(f.get("label", "")),
        fact=str(f.get("fact", "")),
        valid_at=str(f.get("valid_at", "")),
        invalid_at=f.get("invalid_at"),
        relevance=float(f.get("relevance", 0.0)),
    )


def parse_context_episode(ep: dict[str, Any]) -> ContextEpisodeSummary:
    return ContextEpisodeSummary(
        id=str(ep.get("id", "")),
        session_id=str(ep.get("session_id", "")),
        role=ep.get("role"),
        preview=str(ep.get("preview", "")),
        created_at=str(ep.get("created_at", "")),
        relevance=float(ep.get("relevance", 0.0)),
    )


def parse_context(body: dict[str, Any], rid: str | None) -> ContextResult:
    return ContextResult(
        text=str(body.get("context", "")),
        token_count=int(body.get("token_count", 0)),
        entities=[parse_context_entity(e) for e in body.get("entities", [])],
        facts=[parse_context_fact(f) for f in body.get("facts", [])],
        episodes=[parse_context_episode(ep) for ep in body.get("episodes", [])],
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


# ─── Policy ───────────────────────────────────────────────────────


def parse_policy(body: dict[str, Any], rid: str | None) -> PolicyResult:
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


# ─── Audit ────────────────────────────────────────────────────────


def parse_audit(r: dict[str, Any], rid: str | None) -> AuditRecord:
    return AuditRecord(
        id=str(r.get("id", "")),
        user_id=str(r.get("user_id", "")),
        event_type=str(r.get("event_type", "")),
        details=dict(r.get("details", {})),
        created_at=str(r.get("created_at", "")),
        request_id=rid,
    )


# ─── Webhooks ─────────────────────────────────────────────────────


def parse_webhook(body: dict[str, Any], rid: str | None) -> WebhookResult:
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


def parse_webhook_event(e: dict[str, Any], rid: str | None) -> WebhookEvent:
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


# ─── Spans ────────────────────────────────────────────────────────


def parse_spans_result(body: dict[str, Any], rid: str | None) -> SpansResult:
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


# ─── Import ───────────────────────────────────────────────────────


def parse_import_job(body: dict[str, Any], rid: str | None) -> ImportJobResult:
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


# ─── Graph ────────────────────────────────────────────────────────


def parse_adjacency_edge(e: dict[str, Any]) -> AdjacencyEdge:
    return AdjacencyEdge(
        id=str(e.get("id", "")),
        label=str(e.get("label", "")),
        fact=str(e.get("fact", "")),
        valid=bool(e.get("valid", True)),
        source_entity_id=e.get("source_entity_id"),
        target_entity_id=e.get("target_entity_id"),
    )


def parse_graph_entity_detail(
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
            parse_adjacency_edge(e) for e in body.get("outgoing_edges", [])
        ],
        incoming_edges=[
            parse_adjacency_edge(e) for e in body.get("incoming_edges", [])
        ],
        request_id=rid,
    )


def parse_graph_entity(e: dict[str, Any]) -> GraphEntity:
    return GraphEntity(
        id=str(e.get("id", "")),
        name=str(e.get("name", "")),
        entity_type=str(e.get("entity_type", "unknown")),
        summary=e.get("summary"),
        mention_count=int(e.get("mention_count", 0)),
        community_id=e.get("community_id"),
        created_at=str(e.get("created_at", "")),
        updated_at=str(e.get("updated_at", "")),
    )


def parse_graph_entities_result(
    body: dict[str, Any], rid: str | None
) -> GraphEntitiesResult:
    entities = [parse_graph_entity(e) for e in body.get("data", [])]
    return GraphEntitiesResult(
        data=entities,
        count=int(body.get("count", len(entities))),
        user_id=str(body.get("user_id", "")),
        request_id=rid,
    )


def parse_graph_edge(e: dict[str, Any]) -> GraphEdge:
    return GraphEdge(
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


def parse_graph_edges_result(body: dict[str, Any], rid: str | None) -> GraphEdgesResult:
    edges = [parse_graph_edge(e) for e in body.get("data", [])]
    return GraphEdgesResult(
        data=edges,
        count=int(body.get("count", len(edges))),
        user_id=str(body.get("user_id", "")),
        request_id=rid,
    )


# ─── Agent Identity ──────────────────────────────────────────────


def parse_agent_identity(body: dict[str, Any], rid: str | None) -> AgentIdentityResult:
    return AgentIdentityResult(
        agent_id=str(body.get("agent_id", "")),
        version=int(body.get("version", 0)),
        core=dict(body.get("core", {})),
        updated_at=str(body.get("updated_at", "")),
        request_id=rid,
    )


def parse_experience_event(
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


def parse_agent_audit(
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


def parse_promotion_proposal(
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


def parse_agent_context(body: dict[str, Any], rid: str | None) -> AgentContextResult:
    identity_raw = body.get("identity", {})
    return AgentContextResult(
        context=dict(body.get("context", {})),
        identity=parse_agent_identity(identity_raw, None),
        identity_version=int(body.get("identity_version", 0)),
        experience_events_used=int(body.get("experience_events_used", 0)),
        experience_weight_sum=float(body.get("experience_weight_sum", 0.0)),
        user_memory_items_used=int(body.get("user_memory_items_used", 0)),
        attribution_guards=dict(body.get("attribution_guards", {})),
        request_id=rid,
    )
