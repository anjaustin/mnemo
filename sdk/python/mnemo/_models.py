"""Typed result dataclasses for the Mnemo SDK."""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any


# ─── Webhook event types ──────────────────────────────────────────


class WebhookEventType(str, Enum):
    """All supported webhook event types.

    Use these constants when creating or updating webhook subscriptions
    instead of raw strings to avoid typos::

        from mnemo import WebhookEventType
        client.create_webhook(
            user="alice",
            target_url="https://example.com/hook",
            events=[WebhookEventType.FACT_ADDED, WebhookEventType.FACT_SUPERSEDED],
        )
    """

    FACT_ADDED = "fact_added"
    FACT_SUPERSEDED = "fact_superseded"
    HEAD_ADVANCED = "head_advanced"
    CONFLICT_DETECTED = "conflict_detected"
    REVALIDATION_NEEDED = "revalidation_needed"
    CLARIFICATION_GENERATED = "clarification_generated"
    CLARIFICATION_RESOLVED = "clarification_resolved"
    NARRATIVE_REFRESHED = "narrative_refreshed"
    PROMOTION_PROPOSED = "promotion_proposed"
    PROMOTION_APPROVED = "promotion_approved"
    PROMOTION_REJECTED = "promotion_rejected"
    PROMOTION_EXPIRED = "promotion_expired"
    PROMOTION_CONFLICT_DETECTED = "promotion_conflict_detected"


# ─── High-level memory ─────────────────────────────────────────────


@dataclass(slots=True)
class RememberResult:
    ok: bool
    user_id: str
    session_id: str
    episode_id: str
    request_id: str | None = None


@dataclass(slots=True)
class ContextEntitySummary:
    """Entity included in a context response."""

    id: str
    name: str
    entity_type: str
    summary: str | None = None
    relevance: float = 0.0


@dataclass(slots=True)
class ContextFactSummary:
    """Fact/edge included in a context response."""

    id: str
    source_entity: str
    target_entity: str
    label: str
    fact: str
    valid_at: str = ""
    invalid_at: str | None = None
    relevance: float = 0.0


@dataclass(slots=True)
class ContextEpisodeSummary:
    """Episode included in a context response."""

    id: str
    session_id: str
    role: str | None = None
    preview: str = ""
    created_at: str = ""
    relevance: float = 0.0


@dataclass(slots=True)
class ContextResult:
    text: str
    token_count: int
    entities: list[ContextEntitySummary]
    facts: list[ContextFactSummary]
    episodes: list[ContextEpisodeSummary]
    latency_ms: int
    sources: list[str]
    mode: str
    head: dict[str, Any] | None = None
    contract_applied: str | None = None
    retrieval_policy_applied: str | None = None
    temporal_diagnostics: dict[str, Any] | None = None
    retrieval_policy_diagnostics: dict[str, Any] | None = None
    request_id: str | None = None


@dataclass(slots=True)
class ChangesSinceResult:
    """Fact delta between two timestamps for a user.

    Fields match the server response from ``POST /api/v1/memory/:user/changes_since``.
    """

    added_facts: list[dict[str, Any]]
    """Facts (edges) created within the time window."""
    superseded_facts: list[dict[str, Any]]
    """Facts that were invalidated (superseded) within the time window."""
    confidence_deltas: list[dict[str, Any]]
    """Facts whose confidence score changed within the window."""
    head_changes: list[dict[str, Any]]
    """Thread-HEAD changes (current-fact pointer updates) within the window."""
    added_episodes: list[dict[str, Any]]
    """Raw episodes ingested within the window."""
    summary: str
    """Human-readable natural language summary of the changes."""
    from_dt: str
    to_dt: str
    request_id: str | None = None


@dataclass(slots=True)
class ConflictRadarResult:
    conflicts: list[dict[str, Any]]
    user_id: str
    request_id: str | None = None


@dataclass(slots=True)
class CausalRecallResult:
    chains: list[dict[str, Any]]
    query: str
    request_id: str | None = None


@dataclass(slots=True)
class TimeTravelTraceResult:
    """Full time-travel trace: dual snapshots + fact diff + chronological timeline.

    Fields match the server response from ``POST /api/v1/memory/:user/time_travel/trace``.
    """

    snapshot_from: dict[str, Any]
    """Memory snapshot at ``from_dt``: fact/episode counts, top facts, top episodes."""
    snapshot_to: dict[str, Any]
    """Memory snapshot at ``to_dt``: fact/episode counts, top facts, top episodes."""
    gained_facts: list[dict[str, Any]]
    """Facts present in ``snapshot_to`` but not in ``snapshot_from``."""
    lost_facts: list[dict[str, Any]]
    """Facts present in ``snapshot_from`` but invalidated by ``to_dt``."""
    gained_episodes: list[dict[str, Any]]
    """Episodes ingested between ``from_dt`` and ``to_dt``."""
    lost_episodes: list[dict[str, Any]]
    """Episodes that were deleted or expired by ``to_dt``."""
    timeline: list[dict[str, Any]]
    """Chronological list of memory-change events between the two timestamps."""
    summary: str
    """Natural language summary of what changed and why."""
    from_dt: str
    to_dt: str
    request_id: str | None = None


@dataclass(slots=True)
class TimeTravelSummaryResult:
    summary: dict[str, Any]
    request_id: str | None = None


# ─── Governance / policies ─────────────────────────────────────────


@dataclass(slots=True)
class PolicyResult:
    user_id: str
    retention_days_message: int
    retention_days_text: int
    retention_days_json: int
    webhook_domain_allowlist: list[str]
    default_memory_contract: str
    default_retrieval_policy: str
    created_at: str
    updated_at: str
    request_id: str | None = None


@dataclass(slots=True)
class PolicyPreviewResult:
    user_id: str
    current_policy: dict[str, Any]
    preview_policy: dict[str, Any]
    estimated_affected_episodes_total: int
    estimated_affected_message_episodes: int = 0
    estimated_affected_text_episodes: int = 0
    estimated_affected_json_episodes: int = 0
    confidence: str = ""
    request_id: str | None = None


@dataclass(slots=True)
class AuditRecord:
    id: str
    user_id: str
    event_type: str
    details: dict[str, Any]
    created_at: str
    request_id: str | None = None


# ─── Webhooks ──────────────────────────────────────────────────────


@dataclass(slots=True)
class WebhookResult:
    id: str
    user_id: str
    target_url: str
    events: list[str]
    enabled: bool
    created_at: str
    updated_at: str
    request_id: str | None = None


@dataclass(slots=True)
class WebhookEvent:
    id: str
    webhook_id: str
    event_type: str
    user_id: str
    payload: dict[str, Any]
    created_at: str
    attempts: int
    delivered: bool
    dead_letter: bool
    request_id: str | None = None


@dataclass(slots=True)
class ReplayResult:
    webhook_id: str
    count: int
    events: list[dict[str, Any]]
    next_after_event_id: str | None = None
    request_id: str | None = None


@dataclass(slots=True)
class RetryResult:
    webhook_id: str
    event_id: str
    queued: bool
    reason: str = ""
    event: dict[str, Any] | None = None
    request_id: str | None = None


@dataclass(slots=True)
class WebhookStats:
    webhook_id: str
    total_events: int
    delivered_events: int
    pending_events: int
    dead_letter_events: int
    failed_events: int
    recent_failures: int
    circuit_open: bool = False
    circuit_open_until: str | None = None
    rate_limit_per_minute: int = 0
    request_id: str | None = None


# ─── Operator ──────────────────────────────────────────────────────


@dataclass(slots=True)
class OpsSummaryResult:
    window_seconds: int
    http_requests_total: int
    http_responses_2xx: int
    http_responses_4xx: int
    http_responses_5xx: int
    policy_update_total: int
    policy_violation_total: int
    webhook_deliveries_success_total: int
    webhook_deliveries_failure_total: int
    webhook_dead_letter_total: int
    active_webhooks: int
    dead_letter_backlog: int
    pending_webhook_events: int
    governance_audit_events_in_window: int
    webhook_audit_events_in_window: int
    request_id: str | None = None


@dataclass(slots=True)
class TraceLookupResult:
    request_id: str
    matched_episodes: list[dict[str, Any]]
    matched_webhook_events: list[dict[str, Any]]
    matched_webhook_audit: list[dict[str, Any]]
    matched_governance_audit: list[dict[str, Any]]
    summary: dict[str, Any] = field(default_factory=dict)
    sdk_request_id: str | None = None


# ─── Import ────────────────────────────────────────────────────────


@dataclass(slots=True)
class ImportJobResult:
    id: str
    source: str
    user: str
    dry_run: bool
    status: str
    total_messages: int
    imported_messages: int
    failed_messages: int
    sessions_touched: int
    errors: list[str]
    created_at: str
    started_at: str | None = None
    finished_at: str | None = None
    request_id: str | None = None


# ─── Sessions ──────────────────────────────────────────────────────


@dataclass(slots=True)
class SessionInfo:
    """Summary of a server-side session."""

    id: str
    name: str | None = None
    user_id: str = ""
    created_at: str = ""
    updated_at: str = ""
    episode_count: int = 0


@dataclass(slots=True)
class SessionsResult:
    """Result of listing sessions for a user."""

    sessions: list[SessionInfo]
    count: int
    request_id: str | None = None


# ─── Session messages ──────────────────────────────────────────────


@dataclass(slots=True)
class Message:
    idx: int
    id: str
    role: str | None
    content: str
    created_at: str


@dataclass(slots=True)
class MessagesResult:
    messages: list[Message]
    count: int
    session_id: str
    request_id: str | None = None


# ─── Health ────────────────────────────────────────────────────────


@dataclass(slots=True)
class HealthResult:
    status: str
    version: str
    request_id: str | None = None


# ─── Generic ───────────────────────────────────────────────────────


@dataclass(slots=True)
class DeleteResult:
    deleted: bool
    request_id: str | None = None


# ─── Knowledge Graph ───────────────────────────────────────────────


@dataclass(slots=True)
class GraphEntity:
    id: str
    name: str
    entity_type: str
    summary: str | None = None
    mention_count: int = 0
    community_id: str | None = None
    created_at: str = ""
    updated_at: str = ""


@dataclass(slots=True)
class AdjacencyEdge:
    """Simplified edge returned in entity detail (adjacency view).

    Note on field population asymmetry:
      - For **outgoing** edges (returned in ``GraphEntityDetail.outgoing_edges``),
        the server populates ``target_entity_id`` and leaves ``source_entity_id``
        as ``None`` (the source is implicitly the entity you queried).
      - For **incoming** edges (returned in ``GraphEntityDetail.incoming_edges``),
        the server populates ``source_entity_id`` and leaves ``target_entity_id``
        as ``None`` (the target is implicitly the entity you queried).

    This is by design: the queried entity is always the implicit "other side",
    so the server omits the redundant ID to keep payloads compact.
    """

    id: str
    label: str
    fact: str
    valid: bool = True
    source_entity_id: str | None = None
    target_entity_id: str | None = None


@dataclass(slots=True)
class GraphEntityDetail:
    """Full entity detail with adjacency (outgoing/incoming edges)."""

    id: str
    name: str
    entity_type: str
    summary: str | None = None
    mention_count: int = 0
    community_id: str | None = None
    created_at: str = ""
    updated_at: str = ""
    outgoing_edges: list[AdjacencyEdge] = field(default_factory=list)
    incoming_edges: list[AdjacencyEdge] = field(default_factory=list)
    request_id: str | None = None


@dataclass(slots=True)
class GraphEdge:
    id: str
    source_entity_id: str
    target_entity_id: str
    label: str
    fact: str
    confidence: float = 1.0
    valid: bool = True
    valid_at: str = ""
    invalid_at: str | None = None
    created_at: str = ""


@dataclass(slots=True)
class GraphEntitiesResult:
    data: list[GraphEntity]
    count: int
    user_id: str
    request_id: str | None = None


@dataclass(slots=True)
class GraphEdgesResult:
    data: list[GraphEdge]
    count: int
    user_id: str
    request_id: str | None = None


@dataclass(slots=True)
class GraphNeighborsResult:
    seed_entity_id: str
    depth: int
    nodes: list[dict[str, Any]]
    edges: list[dict[str, Any]]
    entities_visited: int
    request_id: str | None = None


@dataclass(slots=True)
class GraphCommunityResult:
    user_id: str
    total_entities: int
    community_count: int
    communities: list[dict[str, Any]]
    request_id: str | None = None


# ─── LLM Spans ─────────────────────────────────────────────────────


@dataclass(slots=True)
class LlmSpan:
    id: str
    provider: str
    model: str
    operation: str
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    latency_ms: int
    success: bool
    started_at: str
    finished_at: str
    request_id: str | None = None
    user_id: str | None = None
    error: str | None = None


@dataclass(slots=True)
class SpansResult:
    spans: list[LlmSpan]
    count: int
    total_tokens: int
    request_id: str | None = None
    total_latency_ms: int | None = None


# ─── Memory Digest ─────────────────────────────────────────────────


@dataclass(slots=True)
class MemoryDigestResult:
    user_id: str
    summary: str
    entity_count: int
    edge_count: int
    dominant_topics: list[str]
    generated_at: str
    model: str
    request_id: str | None = None


# ─── Agent Identity ────────────────────────────────────────────────


@dataclass(slots=True)
class AgentIdentityResult:
    """Versioned agent identity profile."""

    agent_id: str
    version: int
    core: dict[str, Any]
    """Opaque JSON identity blob (mission, style, boundaries, etc.)."""
    updated_at: str
    request_id: str | None = None


@dataclass(slots=True)
class ExperienceEventResult:
    """A single agent experience event."""

    id: str
    agent_id: str
    user_id: str
    session_id: str
    category: str
    signal: str
    confidence: float
    weight: float
    decay_half_life_days: int
    evidence_episode_ids: list[str]
    created_at: str
    request_id: str | None = None


@dataclass(slots=True)
class AgentIdentityAuditResult:
    """Audit event for agent identity changes."""

    id: str
    agent_id: str
    action: str
    """One of: created, updated, rolled_back."""
    from_version: int | None = None
    to_version: int | None = None
    rollback_to_version: int | None = None
    reason: str | None = None
    created_at: str = ""
    request_id: str | None = None


@dataclass(slots=True)
class PromotionProposalResult:
    """A promotion proposal for agent identity evolution."""

    id: str
    agent_id: str
    proposal: str
    candidate_core: dict[str, Any]
    reason: str
    risk_level: str
    status: str
    """One of: pending, approved, rejected."""
    source_event_ids: list[str]
    created_at: str
    approved_at: str | None = None
    rejected_at: str | None = None
    request_id: str | None = None


@dataclass(slots=True)
class AgentContextResult:
    """Agent-scoped context combining identity, experience, and user memory."""

    context: dict[str, Any]
    """Flattened context block (entities, facts, episodes, token_count)."""
    identity: AgentIdentityResult
    identity_version: int
    experience_events_used: int
    experience_weight_sum: float
    user_memory_items_used: int
    attribution_guards: dict[str, bool]
    request_id: str | None = None
