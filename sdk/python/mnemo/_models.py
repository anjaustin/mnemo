"""Typed result dataclasses for the Mnemo SDK."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


# ─── High-level memory ─────────────────────────────────────────────


@dataclass(slots=True)
class RememberResult:
    ok: bool
    user_id: str
    session_id: str
    episode_id: str
    request_id: str | None = None


@dataclass(slots=True)
class ContextResult:
    text: str
    token_count: int
    entities: list[dict[str, Any]]
    facts: list[dict[str, Any]]
    episodes: list[dict[str, Any]]
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
    facts_added: list[dict[str, Any]]
    facts_superseded: list[dict[str, Any]]
    entities_updated: list[dict[str, Any]]
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
    snapshots: list[dict[str, Any]]
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
    estimated_episodes_affected: int
    policy: dict[str, Any]
    request_id: str | None = None


@dataclass(slots=True)
class AuditRecord:
    id: str
    user_id: str
    action: str
    details: dict[str, Any]
    at: str
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
    replayed: int
    events: list[dict[str, Any]]
    request_id: str | None = None


@dataclass(slots=True)
class RetryResult:
    ok: bool
    event_id: str
    request_id: str | None = None


@dataclass(slots=True)
class WebhookStats:
    webhook_id: str
    window_seconds: int
    delivered: int
    failed: int
    dead_letter: int
    request_id: str | None = None


# ─── Operator ──────────────────────────────────────────────────────


@dataclass(slots=True)
class OpsSummaryResult:
    http_requests_total: int
    http_responses_2xx: int
    http_responses_4xx: int
    http_responses_5xx: int
    policy_updates: int
    policy_violations: int
    webhook_delivered: int
    webhook_failed: int
    webhook_dead_letter: int
    governance_events: int
    request_id: str | None = None


@dataclass(slots=True)
class TraceLookupResult:
    request_id: str
    episodes: list[dict[str, Any]]
    webhook_events: list[dict[str, Any]]
    webhook_audit: list[dict[str, Any]]
    governance_audit: list[dict[str, Any]]
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
