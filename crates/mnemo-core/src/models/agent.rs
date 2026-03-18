//! Agent identity, experience memory, and governance.
//!
//! Defines [`AgentIdentityProfile`] (versioned personality core with experience
//! events and EWC++ consolidation), COW branching for A/B testing, domain fork
//! with selective experience transfer, Merkle-proof-carrying identity updates,
//! SHA-256 witness chain audit trail, and promotion proposals with configurable
//! approval policies and conflict analysis.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Versioned identity profile for an AI agent.
///
/// Stores a free-form `core` JSON blob that represents the agent's learned
/// personality, preferences, and behavioural traits. Every successful write
/// increments `version`; all prior versions are retained in a Redis sorted
/// set and can be restored via `POST /api/v1/agents/:agent_id/identity/rollback`.
///
/// The identity layer is **agent-scoped** (keyed by `agent_id` string), not
/// user-scoped. A single agent serves many users; its identity evolves across
/// all of them.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AgentIdentityProfile {
    /// Unique identifier for the agent (arbitrary string, e.g. `"support-bot-v2"`).
    pub agent_id: String,
    /// Monotonically increasing version counter. Starts at 1.
    pub version: u64,
    /// Free-form JSON representing the agent's identity. Schema is determined
    /// by the application; Mnemo treats it as an opaque value and never
    /// introspects its contents.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub core: serde_json::Value,
    /// UTC timestamp of the most recent write to this profile.
    pub updated_at: DateTime<Utc>,
}

impl AgentIdentityProfile {
    pub fn new(agent_id: String) -> Self {
        Self {
            agent_id,
            version: 1,
            core: serde_json::json!({}),
            updated_at: Utc::now(),
        }
    }

    pub fn apply_update(&mut self, req: UpdateAgentIdentityRequest) {
        self.core = req.core;
        self.version += 1;
        self.updated_at = Utc::now();
    }
}

/// Request body for `PUT /api/v1/agents/:agent_id/identity`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UpdateAgentIdentityRequest {
    /// Replacement `core` value. The entire blob is replaced (not merged).
    #[serde(default)]
    #[schema(value_type = Object)]
    pub core: serde_json::Value,
}

/// A single observed experience signal from an agent–user interaction.
///
/// Experience events are the raw inputs that feed the promotion proposal
/// pipeline. They accumulate over time and decay according to
/// `decay_half_life_days`; the effective weight of an event at query time is
/// `weight * 0.5^(age_days / decay_half_life_days)`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ExperienceEvent {
    /// Unique event ID (UUIDv7, lexicographically sortable by creation time).
    pub id: Uuid,
    /// The agent this experience belongs to.
    pub agent_id: String,
    /// Optional: user whose session produced this experience.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Uuid>,
    /// Optional: session in which the experience was observed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    /// Category label (e.g. `"tone"`, `"domain"`, `"preference"`). Free-form;
    /// conventions are application-defined.
    pub category: String,
    /// Natural-language description of the observed signal
    /// (e.g. `"user responded positively to formal tone"`).
    pub signal: String,
    /// How certain the agent is that this signal is meaningful. Range 0.0–1.0.
    pub confidence: f32,
    /// Relative importance of this event compared to others in the same
    /// category. Range 0.0–1.0. Default: 0.5.
    pub weight: f32,
    /// Number of days after which this event's effective weight halves.
    /// Shorter = faster decay. Default: 30 days.
    pub decay_half_life_days: u32,
    /// Episode IDs that serve as evidence for this event, for traceability.
    #[serde(default)]
    pub evidence_episode_ids: Vec<Uuid>,
    /// Fisher importance score (EWC++). Measures how structurally important this
    /// experience is to the agent's current identity. High-importance events
    /// resist decay even when old. Range 0.0–1.0. Computed server-side.
    #[serde(default)]
    pub fisher_importance: f32,
    /// UTC timestamp when this event was recorded.
    pub created_at: DateTime<Utc>,
}

/// Request body for `POST /api/v1/agents/:agent_id/experience`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateExperienceRequest {
    /// Optional client-supplied ID (UUIDv7). Server generates one if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    pub category: String,
    pub signal: String,
    pub confidence: f32,
    /// Defaults to 0.5 if omitted.
    #[serde(default = "default_weight")]
    pub weight: f32,
    /// Defaults to 30 days if omitted.
    #[serde(default = "default_half_life")]
    pub decay_half_life_days: u32,
    #[serde(default)]
    pub evidence_episode_ids: Vec<Uuid>,
    /// Backdated timestamp. Server uses `Utc::now()` if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
}

fn default_weight() -> f32 {
    0.5
}

fn default_half_life() -> u32 {
    30
}

impl ExperienceEvent {
    /// Compute the effective weight of this experience event at the current time.
    ///
    /// Uses EWC++ (Elastic Weight Consolidation) to protect structurally important
    /// experiences from decay. The formula:
    ///
    /// ```text
    /// decay = 2^(-age_days / half_life)
    /// protection = 1.0 + fisher_importance * EWC_LAMBDA
    /// effective = weight * confidence * decay * protection
    /// ```
    ///
    /// Events with high `fisher_importance` resist decay — they remain influential
    /// even when old because they're load-bearing for the agent's current identity.
    pub fn effective_weight(&self) -> f32 {
        effective_experience_weight_ewc(self)
    }

    pub fn from_request(agent_id: &str, req: CreateExperienceRequest) -> Self {
        Self {
            id: req.id.unwrap_or_else(Uuid::now_v7),
            agent_id: agent_id.to_string(),
            user_id: req.user_id,
            session_id: req.session_id,
            category: req.category,
            signal: req.signal,
            confidence: req.confidence,
            weight: req.weight,
            decay_half_life_days: req.decay_half_life_days,
            evidence_episode_ids: req.evidence_episode_ids,
            fisher_importance: 0.0, // computed server-side after creation
            created_at: req.created_at.unwrap_or_else(Utc::now),
        }
    }
}

/// Audit action recorded whenever an agent identity is mutated.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentIdentityAuditAction {
    /// Identity profile created for the first time.
    Created,
    /// Identity profile updated (direct PUT or approved promotion).
    Updated,
    /// Identity rolled back to a prior version.
    RolledBack,
    /// P2-5: Branch created from the parent agent identity.
    BranchCreated,
    /// P2-5: Branch merged back into the parent agent identity.
    BranchMerged,
    /// P2-5: Branch deleted without merging.
    BranchDeleted,
    /// P2-5: Agent identity forked into a new independent agent.
    Forked,
}

/// Append-only record of every identity mutation for a given agent.
///
/// Forms a witness chain: each event includes `prev_hash` (the `event_hash` of
/// the preceding event) and `event_hash` (SHA-256 of canonical fields). This
/// makes the audit log tamper-evident — any deletion, reordering, or field
/// modification breaks the hash chain and is detectable by walking the chain.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AgentIdentityAuditEvent {
    pub id: Uuid,
    pub agent_id: String,
    pub action: AgentIdentityAuditAction,
    /// Version before the mutation (absent for `Created`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_version: Option<u64>,
    /// Version after the mutation.
    pub to_version: u64,
    /// For `RolledBack`: the historical version that was restored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollback_to_version: Option<u64>,
    /// Optional human-readable rationale (required for rollback requests).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,

    // ─── Witness chain fields ─────────────────────────────────
    /// SHA-256 hash of the preceding audit event. `None` for the genesis (first) event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    /// SHA-256(action || from_version || to_version || prev_hash || timestamp_ms).
    /// Deterministic and self-verifiable.
    #[serde(default)]
    pub event_hash: String,
}

/// Result of walking and verifying the audit witness chain.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AuditChainVerification {
    /// `true` if every event's `event_hash` matches recomputation and the
    /// `prev_hash` chain is unbroken.
    pub valid: bool,
    /// Total number of events in the chain.
    pub chain_length: usize,
    /// Indices (0-based, oldest-first) where the chain broke.
    pub breaks: Vec<AuditChainBreak>,
}

/// Description of a single chain break.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AuditChainBreak {
    /// 0-based index in the oldest-first event list.
    pub index: usize,
    /// Event ID at the break point.
    pub event_id: Uuid,
    /// What went wrong.
    pub reason: String,
}

impl AgentIdentityAuditEvent {
    /// Compute the canonical SHA-256 hash for this event.
    ///
    /// Hash input (pipe-delimited):
    /// `action|from_version|to_version|prev_hash|created_at_millis`
    pub fn compute_hash(&self) -> String {
        use sha2::{Digest, Sha256};

        let action_str = serde_json::to_string(&self.action)
            .unwrap_or_else(|_| "unknown".to_string())
            .replace('"', "");
        let from_v = self.from_version.map(|v| v.to_string()).unwrap_or_default();
        let to_v = self.to_version.to_string();
        let prev = self.prev_hash.as_deref().unwrap_or("");
        let ts = self.created_at.timestamp_millis().to_string();

        let input = format!("{}|{}|{}|{}|{}", action_str, from_v, to_v, prev, ts);

        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Verify that this event's `event_hash` matches recomputation.
    pub fn verify_hash(&self) -> bool {
        self.event_hash == self.compute_hash()
    }
}

/// Verify the full audit chain (events must be in oldest-first order).
///
/// Legacy events (created before witness chain was added) have an empty
/// `event_hash` and are treated as the "pre-chain" prefix: they are counted
/// but not hash-verified. The chain verification starts from the first event
/// that has a non-empty `event_hash`.
pub fn verify_audit_chain(events: &[AgentIdentityAuditEvent]) -> AuditChainVerification {
    let mut breaks = Vec::new();

    // Find the first event with a non-empty event_hash (start of the witness chain).
    let chain_start = events
        .iter()
        .position(|e| !e.event_hash.is_empty())
        .unwrap_or(events.len());

    for (i, event) in events.iter().enumerate().skip(chain_start) {
        // 1. Check that event_hash matches recomputation
        if !event.verify_hash() {
            breaks.push(AuditChainBreak {
                index: i,
                event_id: event.id,
                reason: format!(
                    "event_hash mismatch: stored={}, computed={}",
                    event.event_hash,
                    event.compute_hash()
                ),
            });
            continue;
        }

        // 2. Check prev_hash linkage
        if i == chain_start {
            // First witness-chain event: genesis of the chain.
            // If it's also the first event overall, prev_hash must be None.
            // If it follows legacy events, prev_hash should be None (no chain to link to).
            if event.prev_hash.is_some() && i == 0 {
                breaks.push(AuditChainBreak {
                    index: i,
                    event_id: event.id,
                    reason: "Genesis event should have prev_hash = None".to_string(),
                });
            }
        } else {
            // Non-genesis chain event: prev_hash must equal the preceding event's event_hash
            let expected_prev = &events[i - 1].event_hash;
            match &event.prev_hash {
                None => {
                    breaks.push(AuditChainBreak {
                        index: i,
                        event_id: event.id,
                        reason: "Non-genesis event has prev_hash = None".to_string(),
                    });
                }
                Some(ph) if ph != expected_prev => {
                    breaks.push(AuditChainBreak {
                        index: i,
                        event_id: event.id,
                        reason: format!(
                            "prev_hash mismatch: stored={}, expected={}",
                            ph, expected_prev
                        ),
                    });
                }
                _ => {} // OK
            }
        }
    }

    AuditChainVerification {
        valid: breaks.is_empty(),
        chain_length: events.len(),
        breaks,
    }
}

/// Request body for `POST /api/v1/agents/:agent_id/identity/rollback`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct IdentityRollbackRequest {
    /// The historical `version` number to restore.
    pub target_version: u64,
    /// Human-readable rationale for the rollback (recommended for audit trails).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Lifecycle state of a promotion proposal.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromotionStatus {
    /// Awaiting human review.
    Pending,
    /// Approved and applied to the live identity profile.
    Approved,
    /// Rejected; the candidate `core` was not applied.
    Rejected,
    /// Auto-rejected because the approval window expired.
    Expired,
}

/// A candidate identity update proposed from accumulated experience signals.
///
/// Proposals are created by the agent (or operator tooling) and require
/// explicit approval via `POST /api/v1/agents/:agent_id/promotions/:id/approve`
/// before they are applied to the live `AgentIdentityProfile`. This approval
/// gate prevents unreviewed identity drift.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PromotionProposal {
    pub id: Uuid,
    pub agent_id: String,
    /// Natural-language description of what this proposal changes and why.
    pub proposal: String,
    /// The proposed replacement `core` blob. Applied verbatim if approved.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub candidate_core: serde_json::Value,
    /// Human-readable rationale for making this proposal.
    pub reason: String,
    /// Operator-facing risk classification. Valid values: `"low"`, `"medium"`, `"high"`.
    /// Default: `"medium"`.
    pub risk_level: String,
    pub status: PromotionStatus,
    /// Experience event IDs that motivated this proposal. Used for traceability.
    /// Minimum 3 source events is recommended (enforced by client convention, not
    /// server validation).
    #[serde(default)]
    pub source_event_ids: Vec<Uuid>,
    /// Identities (key names) of approvers who have signed off on this proposal.
    #[serde(default)]
    pub approvers: Vec<String>,
    /// UTC timestamp when the proposal was approved (absent until approved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<DateTime<Utc>>,
    /// UTC timestamp when the proposal was rejected (absent until rejected).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected_at: Option<DateTime<Utc>>,
    /// UTC timestamp when the proposal expired (absent until expired).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expired_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Request body for `POST /api/v1/agents/:agent_id/promotions`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreatePromotionProposalRequest {
    /// Optional client-supplied ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Uuid>,
    pub proposal: String,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub candidate_core: serde_json::Value,
    pub reason: String,
    /// Defaults to `"medium"` if omitted.
    #[serde(default = "default_risk_level")]
    pub risk_level: String,
    #[serde(default)]
    pub source_event_ids: Vec<Uuid>,
}

fn default_risk_level() -> String {
    "medium".to_string()
}

impl PromotionProposal {
    pub fn from_request(agent_id: &str, req: CreatePromotionProposalRequest) -> Self {
        Self {
            id: req.id.unwrap_or_else(Uuid::now_v7),
            agent_id: agent_id.to_string(),
            proposal: req.proposal,
            candidate_core: req.candidate_core,
            reason: req.reason,
            risk_level: req.risk_level,
            status: PromotionStatus::Pending,
            source_event_ids: req.source_event_ids,
            approvers: Vec::new(),
            approved_at: None,
            rejected_at: None,
            expired_at: None,
            created_at: Utc::now(),
        }
    }

    /// Check whether this proposal has expired based on an approval policy.
    pub fn is_expired(&self, policy: &ApprovalPolicy) -> bool {
        if self.status != PromotionStatus::Pending {
            return false;
        }
        let requirement = policy.requirement_for_risk(&self.risk_level);
        if let Some(hours) = requirement.auto_reject_after_hours {
            let deadline = self.created_at + chrono::Duration::hours(hours as i64);
            if Utc::now() >= deadline {
                return true;
            }
        }
        false
    }

    /// Check whether the cooling period has elapsed (if required).
    pub fn cooling_period_elapsed(&self, policy: &ApprovalPolicy) -> bool {
        let requirement = policy.requirement_for_risk(&self.risk_level);
        match requirement.cooling_period_hours {
            None => true,
            Some(hours) => {
                let deadline = self.created_at + chrono::Duration::hours(hours as i64);
                Utc::now() >= deadline
            }
        }
    }

    /// Check whether this proposal has met the approval quorum.
    pub fn has_quorum(&self, policy: &ApprovalPolicy) -> bool {
        let requirement = policy.requirement_for_risk(&self.risk_level);
        self.approvers.len() >= requirement.min_approvers as usize
    }
}

// ─── Approval Policy ───────────────────────────────────────────────

/// Defines how many approvers are needed per risk level for an agent's
/// promotion proposals.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ApprovalPolicy {
    pub agent_id: String,
    pub low_risk: ApprovalRequirement,
    pub medium_risk: ApprovalRequirement,
    pub high_risk: ApprovalRequirement,
    pub updated_at: DateTime<Utc>,
}

/// Requirements for a single risk level.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ApprovalRequirement {
    /// Minimum number of distinct approvers required.
    pub min_approvers: u32,
    /// Mandatory wait (hours) after proposal creation before it can auto-apply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooling_period_hours: Option<u32>,
    /// Auto-reject if not approved within this many hours.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_reject_after_hours: Option<u32>,
}

impl Default for ApprovalRequirement {
    fn default() -> Self {
        Self {
            min_approvers: 1,
            cooling_period_hours: None,
            auto_reject_after_hours: None,
        }
    }
}

impl ApprovalPolicy {
    /// Create a default policy: 1 approver for all risk levels.
    pub fn default_for_agent(agent_id: &str) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            low_risk: ApprovalRequirement {
                min_approvers: 1,
                ..Default::default()
            },
            medium_risk: ApprovalRequirement {
                min_approvers: 1,
                ..Default::default()
            },
            high_risk: ApprovalRequirement {
                min_approvers: 1,
                ..Default::default()
            },
            updated_at: Utc::now(),
        }
    }

    /// Get the requirement for the given risk level string.
    pub fn requirement_for_risk(&self, risk_level: &str) -> &ApprovalRequirement {
        match risk_level.to_lowercase().as_str() {
            "low" => &self.low_risk,
            "high" => &self.high_risk,
            _ => &self.medium_risk, // default to medium for unknown
        }
    }
}

/// Request body for `PUT /api/v1/agents/:agent_id/approval-policy`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SetApprovalPolicyRequest {
    pub low_risk: ApprovalRequirement,
    pub medium_risk: ApprovalRequirement,
    pub high_risk: ApprovalRequirement,
}

// ─── Conflict Analysis ─────────────────────────────────────────────

/// Result of analyzing experience events for conflicts with a promotion proposal.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ConflictAnalysis {
    pub proposal_id: Uuid,
    pub agent_id: String,
    /// Experience event IDs whose signals support the proposed change.
    pub supporting_signals: Vec<Uuid>,
    /// Experience event IDs whose signals oppose the proposed change.
    pub conflicting_signals: Vec<Uuid>,
    /// 0.0 (no conflict) to 1.0 (strong conflict).
    pub conflict_score: f32,
    pub recommendation: ConflictRecommendation,
}

/// Recommendation based on conflict analysis.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConflictRecommendation {
    /// Low conflict — safe to approve.
    Proceed,
    /// Moderate conflict — needs human review.
    ReviewConflicts,
    /// High conflict — experience evidence opposes this change.
    Reject,
}

impl ConflictRecommendation {
    /// Derive recommendation from a conflict score.
    pub fn from_score(score: f32) -> Self {
        if score < 0.3 {
            Self::Proceed
        } else if score < 0.7 {
            Self::ReviewConflicts
        } else {
            Self::Reject
        }
    }
}

/// Analyze experience events for conflicts with a proposed identity change.
///
/// The analysis compares the text of the proposal and candidate_core against
/// each experience event's signal text. Events whose signals are semantically
/// aligned (same category, similar language) are counted as supporting.
/// Events whose signals contradict (opposite sentiment, conflicting preferences)
/// are counted as conflicting.
///
/// This is a text-heuristic approach (no LLM call). The conflict score is:
/// `conflicting_count / (supporting_count + conflicting_count + 1)`.
pub fn analyze_conflicts(
    proposal: &PromotionProposal,
    experience_events: &[ExperienceEvent],
) -> ConflictAnalysis {
    let candidate_str = proposal.candidate_core.to_string().to_lowercase();
    let proposal_str = proposal.proposal.to_lowercase();

    let mut supporting: Vec<Uuid> = Vec::new();
    let mut conflicting: Vec<Uuid> = Vec::new();

    for event in experience_events {
        let signal_lower = event.signal.to_lowercase();

        // Check if the event's signal appears in the candidate core or proposal text.
        // If the signal aligns with the proposed change, it's supporting.
        // If it opposes (contains negation words near matching terms), it's conflicting.
        let signal_words: Vec<&str> = signal_lower.split_whitespace().collect();
        let candidate_words: Vec<&str> = candidate_str.split_whitespace().collect();
        let proposal_words: Vec<&str> = proposal_str.split_whitespace().collect();

        // Simple keyword overlap heuristic
        let overlap_candidate = signal_words
            .iter()
            .filter(|w| w.len() > 3) // skip short words
            .filter(|w| candidate_words.iter().any(|cw| cw.contains(**w)))
            .count();

        let overlap_proposal = signal_words
            .iter()
            .filter(|w| w.len() > 3)
            .filter(|w| proposal_words.iter().any(|pw| pw.contains(**w)))
            .count();

        let total_overlap = overlap_candidate + overlap_proposal;
        if total_overlap == 0 {
            continue; // no relevance
        }

        // Check for negation/opposition signals
        let has_negation = signal_lower.contains("not ")
            || signal_lower.contains("don't")
            || signal_lower.contains("avoid")
            || signal_lower.contains("dislike")
            || signal_lower.contains("stop")
            || signal_lower.contains("reduce")
            || signal_lower.contains("less ")
            || signal_lower.contains("fewer");

        let proposal_has_negation = proposal_str.contains("not ")
            || proposal_str.contains("don't")
            || proposal_str.contains("avoid")
            || proposal_str.contains("remove")
            || proposal_str.contains("reduce")
            || proposal_str.contains("less ");

        // If both or neither have negation → supporting (aligned sentiment).
        // If only one has negation → conflicting (opposing sentiment).
        if has_negation != proposal_has_negation {
            conflicting.push(event.id);
        } else {
            supporting.push(event.id);
        }
    }

    let conflict_score = if supporting.is_empty() && conflicting.is_empty() {
        0.0
    } else {
        conflicting.len() as f32 / (supporting.len() + conflicting.len()) as f32
    };

    let recommendation = ConflictRecommendation::from_score(conflict_score);

    ConflictAnalysis {
        proposal_id: proposal.id,
        agent_id: proposal.agent_id.clone(),
        supporting_signals: supporting,
        conflicting_signals: conflicting,
        conflict_score,
        recommendation,
    }
}

// ─── EWC++ (Elastic Weight Consolidation) ──────────────────────────

/// EWC regularization strength. Controls how much Fisher importance
/// protects an event from decay. Higher = more protection.
/// Value of 2.0 means an event with fisher_importance=1.0 decays 3x slower.
const EWC_LAMBDA: f32 = 2.0;

/// Compute EWC++-enhanced effective weight for an experience event.
///
/// The standard formula `weight * confidence * decay` is augmented with a
/// Fisher importance protection factor:
///
/// `effective = weight * confidence * decay * (1 + fisher_importance * EWC_LAMBDA)`
///
/// This ensures structurally important events (high Fisher importance) resist
/// decay even when old, while incidental events decay normally.
pub fn effective_experience_weight_ewc(event: &ExperienceEvent) -> f32 {
    let age_days = (Utc::now() - event.created_at).num_days().max(0) as f32;
    let half_life = event.decay_half_life_days.max(1) as f32;
    let decay_factor = 2f32.powf(-age_days / half_life);

    // EWC++ protection: high-importance events resist decay
    let protection = 1.0 + event.fisher_importance.clamp(0.0, 1.0) * EWC_LAMBDA;

    (event.weight * event.confidence * decay_factor * protection).max(0.0)
}

/// Compute Fisher importance for a new experience event relative to existing
/// events in the same category.
///
/// The importance is based on two signals:
/// 1. **Novelty**: How different is this event from existing signals in its category?
///    Fewer existing events in the same category = higher novelty.
/// 2. **Corroboration**: Does this event reinforce a consistent pattern?
///    Higher average confidence in the category = higher corroboration.
///
/// `category_events`: all existing events in the same category as the new event.
///
/// Returns a value in [0.0, 1.0].
pub fn compute_fisher_importance(
    new_event: &ExperienceEvent,
    category_events: &[ExperienceEvent],
) -> f32 {
    if category_events.is_empty() {
        // First event in a category is always important (novelty = 1.0)
        return 1.0;
    }

    let n = category_events.len() as f32;

    // Novelty: inverse of category saturation. Diminishing returns as category grows.
    // At 1 existing event: 0.5, at 5: ~0.17, at 20: ~0.05
    let novelty = 1.0 / (1.0 + n);

    // Corroboration: does this event's confidence align with the category mean?
    // If the new event has similar confidence to the mean, it's corroborating.
    let mean_confidence = category_events.iter().map(|e| e.confidence).sum::<f32>() / n;
    let confidence_alignment = 1.0
        - (new_event.confidence - mean_confidence)
            .abs()
            .clamp(0.0, 1.0);

    // Weight signal: events with high explicit weight are more important
    let weight_signal = new_event.weight.clamp(0.0, 1.0);

    // Composite: weighted average of the three signals
    let importance = 0.4 * novelty + 0.35 * confidence_alignment + 0.25 * weight_signal;
    importance.clamp(0.0, 1.0)
}

// ─── COW Branching ────────────────────────────────────────────────

/// Metadata for a copy-on-write branch of an agent identity.
///
/// Branches allow controlled experimentation: create a branch, run it for N
/// conversations, compare metrics against main, then merge or discard.
/// The branch stores a full copy of the identity core at fork time and
/// evolves independently from the main identity.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BranchMetadata {
    /// Name of the branch (e.g. `"experiment-1"`, `"tone-warmer"`).
    pub branch_name: String,
    /// The agent_id that this branch was forked from.
    pub parent_agent_id: String,
    /// The version of the parent identity at the time of forking.
    pub fork_version: u64,
    /// When the branch was created.
    pub created_at: DateTime<Utc>,
    /// Optional description of the experiment.
    #[serde(default)]
    pub description: Option<String>,
    /// Whether the branch has been merged back into the parent.
    #[serde(default)]
    pub merged: bool,
}

/// Request body for creating a new branch.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateBranchRequest {
    /// Name of the branch (alphanumeric + hyphens, max 64 chars).
    pub branch_name: String,
    /// Optional description of the experiment.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional initial core override. If omitted, the branch starts with
    /// a copy of the parent's current core.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub core_override: Option<serde_json::Value>,
}

/// Response for branch operations.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BranchInfo {
    /// Branch metadata.
    pub metadata: BranchMetadata,
    /// Current identity profile on the branch.
    pub identity: AgentIdentityProfile,
}

/// Result of merging a branch back into the parent.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MergeResult {
    /// The branch that was merged.
    pub branch_name: String,
    /// The parent's identity after merge (new version).
    pub merged_identity: AgentIdentityProfile,
    /// The parent version before merge.
    pub parent_version_before: u64,
    /// The branch's core at merge time.
    #[schema(value_type = Object)]
    pub branch_core_applied: serde_json::Value,
}

/// Validate a branch name: must be 1-64 chars, alphanumeric + hyphens only.
pub fn validate_branch_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("branch name cannot be empty".into());
    }
    if name.len() > 64 {
        return Err("branch name must be <= 64 characters".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "branch name must contain only alphanumeric characters, hyphens, or underscores".into(),
        );
    }
    Ok(())
}

// ─── Domain Expansion / Transfer Learning ─────────────────────────

/// Filter criteria for selecting which experience events to transfer
/// when forking an agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ExperienceFilter {
    /// Only transfer events in these categories. Empty = all categories.
    #[serde(default)]
    pub categories: Vec<String>,
    /// Minimum confidence threshold. Events below this are excluded.
    #[serde(default)]
    pub min_confidence: Option<f32>,
    /// Minimum effective weight (after EWC decay). Events below this are excluded.
    #[serde(default)]
    pub min_weight: Option<f32>,
    /// Maximum number of events to transfer.
    #[serde(default)]
    pub max_events: Option<u32>,
}

impl ExperienceFilter {
    /// Check if an experience event passes this filter.
    pub fn matches(&self, event: &ExperienceEvent) -> bool {
        // Category filter
        if !self.categories.is_empty() && !self.categories.iter().any(|c| c == &event.category) {
            return false;
        }
        // Confidence threshold
        if let Some(min_conf) = self.min_confidence {
            if event.confidence < min_conf {
                return false;
            }
        }
        // Weight threshold
        if let Some(min_w) = self.min_weight {
            if event.weight < min_w {
                return false;
            }
        }
        true
    }
}

/// Request body for `POST /api/v1/agents/:agent_id/fork`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ForkAgentRequest {
    /// The agent ID for the new forked agent. Must be unique.
    pub new_agent_id: String,
    /// Optional identity core override for the new agent.
    /// If None, the parent's core is copied verbatim.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub core_override: Option<serde_json::Value>,
    /// Filter for selecting which experience events to transfer.
    /// If None, all experience events are transferred.
    #[serde(default)]
    pub experience_filter: Option<ExperienceFilter>,
    /// Optional description of why the fork was created.
    #[serde(default)]
    pub description: Option<String>,
}

/// Validate a new agent ID for forking (same rules as branch names, but allow dots).
pub fn validate_fork_agent_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("new_agent_id must not be empty".into());
    }
    if id.len() > 128 {
        return Err("new_agent_id must be <= 128 characters".into());
    }
    if id.contains(':') || id.contains('/') || id.contains("..") {
        return Err("new_agent_id must not contain ':', '/', or '..'".into());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(
            "new_agent_id must contain only alphanumeric characters, hyphens, underscores, or dots"
                .into(),
        );
    }
    Ok(())
}

/// Lineage metadata tracking the fork relationship.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ForkLineage {
    /// The parent agent this was forked from.
    pub parent_agent_id: String,
    /// The parent's version at fork time.
    pub parent_version: u64,
    /// When the fork happened.
    pub forked_at: DateTime<Utc>,
    /// Description of the fork.
    pub description: Option<String>,
    /// Number of experience events transferred.
    pub experience_events_transferred: u32,
    /// The filter used during transfer (if any).
    pub experience_filter: Option<ExperienceFilter>,
}

/// Result of a fork operation.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ForkResult {
    /// The newly created agent identity.
    pub new_agent: AgentIdentityProfile,
    /// Fork lineage metadata.
    pub lineage: ForkLineage,
}

// ─── Verified Identity Updates (Proof-Carrying Writes) ─────────────

/// The canonical allowlist of top-level identity core keys.
pub const IDENTITY_ALLOWLIST: &[&str] = &[
    "boundaries",
    "capabilities",
    "mission",
    "persona",
    "style",
    "values",
];

/// Forbidden substrings that must not appear in any key at any depth.
pub const IDENTITY_FORBIDDEN_SUBSTRINGS: &[&str] = &[
    "address",
    "email",
    "episode",
    "external_id",
    "phone",
    "session",
    "user",
];

/// SHA-256 hash of a leaf or internal node in the allowlist Merkle tree.
pub type MerkleHash = [u8; 32];

/// Compute SHA-256 of arbitrary bytes.
fn sha256(data: &[u8]) -> MerkleHash {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Compute the hash of a leaf: `SHA256("leaf:" || key_bytes)`.
fn leaf_hash(key: &str) -> MerkleHash {
    let mut buf = Vec::with_capacity(5 + key.len());
    buf.extend_from_slice(b"leaf:");
    buf.extend_from_slice(key.as_bytes());
    sha256(&buf)
}

/// Compute the hash of an internal node: `SHA256("node:" || left || right)`.
fn node_hash(left: &MerkleHash, right: &MerkleHash) -> MerkleHash {
    let mut buf = Vec::with_capacity(5 + 64);
    buf.extend_from_slice(b"node:");
    buf.extend_from_slice(left);
    buf.extend_from_slice(right);
    sha256(&buf)
}

/// A Merkle tree built from the identity allowlist.
///
/// Leaves are sorted alphabetically and hashed with a domain separator.
/// If the number of leaves is odd, the last leaf is duplicated.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AllowlistMerkleTree {
    /// The Merkle root hash (hex-encoded for JSON).
    pub root: String,
    /// Sorted leaf keys used to build the tree.
    pub leaves: Vec<String>,
    /// All layers of the tree (layer 0 = leaves, last = root).
    #[serde(skip)]
    layers: Vec<Vec<MerkleHash>>,
}

impl AllowlistMerkleTree {
    /// Build a Merkle tree from the canonical allowlist.
    pub fn from_allowlist() -> Self {
        Self::from_keys(IDENTITY_ALLOWLIST.iter().map(|s| s.to_string()).collect())
    }

    /// Build a Merkle tree from an arbitrary sorted key list.
    pub fn from_keys(mut keys: Vec<String>) -> Self {
        keys.sort();
        let mut leaf_hashes: Vec<MerkleHash> = keys.iter().map(|k| leaf_hash(k)).collect();
        // Pad to even length
        if !leaf_hashes.len().is_multiple_of(2) {
            leaf_hashes.push(*leaf_hashes.last().unwrap());
        }

        let mut layers = vec![leaf_hashes.clone()];
        let mut current = leaf_hashes;

        while current.len() > 1 {
            let mut next = Vec::with_capacity(current.len().div_ceil(2));
            for pair in current.chunks(2) {
                if pair.len() == 2 {
                    next.push(node_hash(&pair[0], &pair[1]));
                } else {
                    next.push(node_hash(&pair[0], &pair[0]));
                }
            }
            layers.push(next.clone());
            current = next;
        }

        let root_hash = current.first().copied().unwrap_or([0u8; 32]);
        Self {
            root: hex::encode(root_hash),
            leaves: keys,
            layers,
        }
    }

    /// Generate a membership proof for a given key.
    /// Returns `None` if the key is not in the allowlist.
    pub fn prove(&self, key: &str) -> Option<AllowlistMembershipProof> {
        let idx = self.leaves.iter().position(|k| k == key)?;
        let mut siblings = Vec::new();
        let mut current_idx = idx;

        // Account for padding: if odd number of original leaves, the leaf layer
        // was padded, so use the padded layer length
        for layer in &self.layers[..self.layers.len().saturating_sub(1)] {
            let sibling_idx = if current_idx % 2 == 0 {
                current_idx + 1
            } else {
                current_idx - 1
            };
            let sibling_hash = if sibling_idx < layer.len() {
                layer[sibling_idx]
            } else {
                layer[current_idx] // duplicate for odd
            };
            siblings.push(ProofSibling {
                hash: hex::encode(sibling_hash),
                position: if current_idx % 2 == 0 {
                    SiblingPosition::Right
                } else {
                    SiblingPosition::Left
                },
            });
            current_idx /= 2;
        }

        Some(AllowlistMembershipProof {
            key: key.to_string(),
            leaf_index: idx as u32,
            siblings,
            root: self.root.clone(),
        })
    }

    /// Verify a membership proof against this tree's root.
    pub fn verify(&self, proof: &AllowlistMembershipProof) -> bool {
        Self::verify_against_root(&self.root, proof)
    }

    /// Verify a proof against a given root hash (static verification).
    pub fn verify_against_root(root: &str, proof: &AllowlistMembershipProof) -> bool {
        let mut current = leaf_hash(&proof.key);
        for sibling in &proof.siblings {
            let sibling_hash = match hex::decode(&sibling.hash) {
                Ok(h) if h.len() == 32 => {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&h);
                    arr
                }
                _ => return false,
            };
            current = match sibling.position {
                SiblingPosition::Right => node_hash(&current, &sibling_hash),
                SiblingPosition::Left => node_hash(&sibling_hash, &current),
            };
        }
        hex::encode(current) == root
    }
}

/// Which side the sibling is on in the Merkle proof path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SiblingPosition {
    Left,
    Right,
}

/// A single sibling in a Merkle proof path.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProofSibling {
    /// Hex-encoded SHA-256 hash of the sibling node.
    pub hash: String,
    /// Whether this sibling is on the left or right.
    pub position: SiblingPosition,
}

/// Proof that a single key is a member of the identity allowlist.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AllowlistMembershipProof {
    /// The key being proved.
    pub key: String,
    /// Leaf index in the sorted allowlist.
    pub leaf_index: u32,
    /// Sibling hashes along the path from leaf to root.
    pub siblings: Vec<ProofSibling>,
    /// The Merkle root this proof targets.
    pub root: String,
}

/// A complete proof covering all top-level keys in a candidate identity core.
/// Each key must have a valid membership proof.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct IdentityUpdateProof {
    /// Merkle root hash (hex) the proofs are verified against.
    pub merkle_root: String,
    /// One membership proof per top-level key in the candidate core.
    pub key_proofs: Vec<AllowlistMembershipProof>,
}

/// Request body for `POST /api/v1/agents/:agent_id/identity/verified`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct VerifiedIdentityUpdateRequest {
    /// The candidate identity core (same as `UpdateAgentIdentityRequest.core`).
    #[schema(value_type = Object)]
    pub core: serde_json::Value,
    /// Cryptographic proof that all keys satisfy the contamination guard.
    pub proof: IdentityUpdateProof,
}

/// Result of proof verification.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProofVerificationResult {
    /// Whether all proofs verified successfully.
    pub verified: bool,
    /// Details per key.
    pub key_results: Vec<KeyVerificationResult>,
    /// The Merkle root used for verification.
    pub merkle_root: String,
}

/// Per-key verification result.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct KeyVerificationResult {
    /// The key being verified.
    pub key: String,
    /// Whether the proof is valid.
    pub valid: bool,
    /// Error message if invalid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Verify an `IdentityUpdateProof` against the canonical allowlist Merkle root.
///
/// Checks:
/// 1. The proof's merkle_root matches the canonical root.
/// 2. Every top-level key in `core` has a valid membership proof.
/// 3. No extra proofs for keys not in `core`.
/// 4. Forbidden-substring check on all keys at all depths.
pub fn verify_identity_update_proof(
    core: &serde_json::Value,
    proof: &IdentityUpdateProof,
) -> ProofVerificationResult {
    let tree = AllowlistMerkleTree::from_allowlist();
    let canonical_root = &tree.root;

    let mut key_results = Vec::new();

    // Check root matches canonical
    if proof.merkle_root != *canonical_root {
        return ProofVerificationResult {
            verified: false,
            key_results: vec![KeyVerificationResult {
                key: "<merkle_root>".into(),
                valid: false,
                error: Some(format!(
                    "proof merkle_root {} does not match canonical root {}",
                    proof.merkle_root, canonical_root
                )),
            }],
            merkle_root: canonical_root.clone(),
        };
    }

    // Extract top-level keys from core
    let core_keys: Vec<String> = match core.as_object() {
        Some(map) => map.keys().cloned().collect(),
        None => {
            return ProofVerificationResult {
                verified: false,
                key_results: vec![KeyVerificationResult {
                    key: "<core>".into(),
                    valid: false,
                    error: Some("core must be a JSON object".into()),
                }],
                merkle_root: canonical_root.clone(),
            };
        }
    };

    // Check each core key has a valid proof
    for core_key in &core_keys {
        let matching_proof = proof.key_proofs.iter().find(|p| p.key == *core_key);
        match matching_proof {
            None => {
                key_results.push(KeyVerificationResult {
                    key: core_key.clone(),
                    valid: false,
                    error: Some("no proof provided for this key".into()),
                });
            }
            Some(kp) => {
                if AllowlistMerkleTree::verify_against_root(canonical_root, kp) {
                    key_results.push(KeyVerificationResult {
                        key: core_key.clone(),
                        valid: true,
                        error: None,
                    });
                } else {
                    key_results.push(KeyVerificationResult {
                        key: core_key.clone(),
                        valid: false,
                        error: Some("Merkle proof verification failed".into()),
                    });
                }
            }
        }
    }

    // Check for extra proofs (proofs for keys not in core)
    for kp in &proof.key_proofs {
        if !core_keys.contains(&kp.key) {
            key_results.push(KeyVerificationResult {
                key: kp.key.clone(),
                valid: false,
                error: Some("proof provided for key not present in core".into()),
            });
        }
    }

    // Forbidden-substring deep scan
    if let Err(forbidden_key) = check_forbidden_substrings(core, "core/") {
        key_results.push(KeyVerificationResult {
            key: forbidden_key.clone(),
            valid: false,
            error: Some(format!("forbidden substring detected at {}", forbidden_key)),
        });
    }

    let verified = key_results.iter().all(|r| r.valid);
    ProofVerificationResult {
        verified,
        key_results,
        merkle_root: canonical_root.clone(),
    }
}

/// Recursively check for forbidden substrings in all keys.
fn check_forbidden_substrings(value: &serde_json::Value, path: &str) -> Result<(), String> {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let normalized = k.to_ascii_lowercase();
                if IDENTITY_FORBIDDEN_SUBSTRINGS
                    .iter()
                    .any(|token| normalized.contains(token))
                {
                    return Err(format!("{}{}", path, k));
                }
                let next = format!("{}{}/", path, k);
                check_forbidden_substrings(v, &next)?;
            }
        }
        serde_json::Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                let next = format!("{}[{}]/", path, idx);
                check_forbidden_substrings(item, &next)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_agent_identity_new_defaults() {
        let profile = AgentIdentityProfile::new("support-bot".into());
        assert_eq!(profile.agent_id, "support-bot");
        assert_eq!(profile.version, 1);
        assert_eq!(profile.core, json!({}));
    }

    #[test]
    fn test_agent_identity_apply_update_increments_version() {
        let mut profile = AgentIdentityProfile::new("bot".into());
        assert_eq!(profile.version, 1);
        let req = UpdateAgentIdentityRequest {
            core: json!({"mission": "help with billing"}),
        };
        profile.apply_update(req);
        assert_eq!(profile.version, 2);
        assert_eq!(profile.core["mission"], "help with billing");
    }

    #[test]
    fn test_agent_identity_apply_update_replaces_core_entirely() {
        let mut profile = AgentIdentityProfile::new("bot".into());
        profile.core = json!({"mission": "old", "style": "formal"});
        let req = UpdateAgentIdentityRequest {
            core: json!({"mission": "new"}),
        };
        profile.apply_update(req);
        // "style" should be gone — full replacement, not merge
        assert!(profile.core.get("style").is_none());
        assert_eq!(profile.core["mission"], "new");
    }

    #[test]
    fn test_agent_identity_serialization_roundtrip() {
        let profile = AgentIdentityProfile::new("bot-v2".into());
        let json_str = serde_json::to_string(&profile).unwrap();
        let restored: AgentIdentityProfile = serde_json::from_str(&json_str).unwrap();
        assert_eq!(restored.agent_id, "bot-v2");
        assert_eq!(restored.version, 1);
    }

    #[test]
    fn test_experience_event_from_request_defaults() {
        let req = CreateExperienceRequest {
            id: None,
            user_id: None,
            session_id: None,
            category: "tone".into(),
            signal: "user prefers formal".into(),
            confidence: 0.9,
            weight: 0.5,
            decay_half_life_days: 30,
            evidence_episode_ids: vec![],
            created_at: None,
        };
        let event = ExperienceEvent::from_request("bot", req);
        assert_eq!(event.agent_id, "bot");
        assert_eq!(event.category, "tone");
        assert_eq!(event.confidence, 0.9);
        assert_eq!(event.weight, 0.5);
        assert_eq!(event.decay_half_life_days, 30);
    }

    #[test]
    fn test_experience_event_from_request_preserves_client_id() {
        let client_id = Uuid::now_v7();
        let req = CreateExperienceRequest {
            id: Some(client_id),
            user_id: None,
            session_id: None,
            category: "domain".into(),
            signal: "knows billing".into(),
            confidence: 0.8,
            weight: 0.7,
            decay_half_life_days: 60,
            evidence_episode_ids: vec![],
            created_at: None,
        };
        let event = ExperienceEvent::from_request("bot", req);
        assert_eq!(event.id, client_id);
    }

    #[test]
    fn test_promotion_proposal_from_request_defaults_to_pending() {
        let req = CreatePromotionProposalRequest {
            id: None,
            proposal: "Add refund handling".into(),
            candidate_core: json!({"mission": "billing + refunds"}),
            reason: "Learned from 3 sessions".into(),
            risk_level: "low".into(),
            source_event_ids: vec![Uuid::now_v7(); 3],
        };
        let proposal = PromotionProposal::from_request("bot", req);
        assert_eq!(proposal.status, PromotionStatus::Pending);
        assert_eq!(proposal.agent_id, "bot");
        assert_eq!(proposal.risk_level, "low");
        assert!(proposal.approved_at.is_none());
        assert!(proposal.rejected_at.is_none());
    }

    #[test]
    fn test_promotion_status_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&PromotionStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&PromotionStatus::Approved).unwrap(),
            "\"approved\""
        );
        assert_eq!(
            serde_json::to_string(&PromotionStatus::Rejected).unwrap(),
            "\"rejected\""
        );
        let round: PromotionStatus = serde_json::from_str("\"rejected\"").unwrap();
        assert_eq!(round, PromotionStatus::Rejected);
    }

    #[test]
    fn test_audit_action_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&AgentIdentityAuditAction::Created).unwrap(),
            "\"created\""
        );
        assert_eq!(
            serde_json::to_string(&AgentIdentityAuditAction::Updated).unwrap(),
            "\"updated\""
        );
        assert_eq!(
            serde_json::to_string(&AgentIdentityAuditAction::RolledBack).unwrap(),
            "\"rolled_back\""
        );
    }

    #[test]
    fn test_default_weight_and_half_life() {
        let json_str = r#"{"category":"x","signal":"y","confidence":0.5}"#;
        let req: CreateExperienceRequest = serde_json::from_str(json_str).unwrap();
        assert_eq!(req.weight, 0.5);
        assert_eq!(req.decay_half_life_days, 30);
    }

    #[test]
    fn test_default_risk_level() {
        let json_str = r#"{"proposal":"p","reason":"r","source_event_ids":[]}"#;
        let req: CreatePromotionProposalRequest = serde_json::from_str(json_str).unwrap();
        assert_eq!(req.risk_level, "medium");
    }

    // ─── EWC++ unit tests ──────────────────────────────────────────

    fn make_event(
        category: &str,
        weight: f32,
        confidence: f32,
        fisher: f32,
        age_days: i64,
    ) -> ExperienceEvent {
        ExperienceEvent {
            id: Uuid::now_v7(),
            agent_id: "test-agent".into(),
            user_id: None,
            session_id: None,
            category: category.into(),
            signal: format!("test signal {}", category),
            confidence,
            weight,
            decay_half_life_days: 30,
            evidence_episode_ids: vec![],
            fisher_importance: fisher,
            created_at: Utc::now() - chrono::Duration::days(age_days),
        }
    }

    #[test]
    fn test_ewc_fresh_event_no_fisher() {
        // Brand-new event, no Fisher importance → same as old formula
        let event = make_event("tone", 0.8, 0.9, 0.0, 0);
        let w = effective_experience_weight_ewc(&event);
        // decay=1.0, protection=1.0, expected = 0.8 * 0.9 * 1.0 * 1.0 = 0.72
        assert!((w - 0.72).abs() < 0.01, "expected ~0.72, got {}", w);
    }

    #[test]
    fn test_ewc_fresh_event_with_fisher() {
        // Brand-new event, fisher=1.0 → protection = 1 + 1.0 * 2.0 = 3.0
        let event = make_event("tone", 0.8, 0.9, 1.0, 0);
        let w = effective_experience_weight_ewc(&event);
        // expected = 0.8 * 0.9 * 1.0 * 3.0 = 2.16
        assert!((w - 2.16).abs() < 0.01, "expected ~2.16, got {}", w);
    }

    #[test]
    fn test_ewc_old_event_without_fisher_decays() {
        // 30-day-old event, half_life=30 → decay=0.5, fisher=0 → protection=1
        let event = make_event("tone", 1.0, 1.0, 0.0, 30);
        let w = effective_experience_weight_ewc(&event);
        // expected = 1.0 * 1.0 * 0.5 * 1.0 = 0.5
        assert!((w - 0.5).abs() < 0.05, "expected ~0.5, got {}", w);
    }

    #[test]
    fn test_ewc_old_event_with_fisher_resists_decay() {
        // 30-day-old event, half_life=30, fisher=1.0 → protection=3.0
        let event = make_event("tone", 1.0, 1.0, 1.0, 30);
        let w = effective_experience_weight_ewc(&event);
        // expected = 1.0 * 1.0 * 0.5 * 3.0 = 1.5
        assert!((w - 1.5).abs() < 0.05, "expected ~1.5, got {}", w);
    }

    #[test]
    fn test_ewc_high_fisher_beats_zero_fisher_over_time() {
        // Both 60 days old → decay ≈ 0.25
        let important = make_event("domain", 1.0, 1.0, 1.0, 60);
        let incidental = make_event("domain", 1.0, 1.0, 0.0, 60);
        let w_important = effective_experience_weight_ewc(&important);
        let w_incidental = effective_experience_weight_ewc(&incidental);
        // important should be ~3x incidental
        assert!(
            w_important > w_incidental * 2.5,
            "important={} should be >>  incidental={}",
            w_important,
            w_incidental
        );
    }

    #[test]
    fn test_ewc_effective_weight_method_matches_function() {
        let event = make_event("tone", 0.7, 0.85, 0.5, 15);
        let from_method = event.effective_weight();
        let from_fn = effective_experience_weight_ewc(&event);
        assert!(
            (from_method - from_fn).abs() < f32::EPSILON,
            "method and function must agree"
        );
    }

    #[test]
    fn test_ewc_fisher_clamped_to_0_1() {
        // Fisher > 1.0 should be clamped to 1.0
        let event = make_event("tone", 1.0, 1.0, 5.0, 0);
        let w = effective_experience_weight_ewc(&event);
        // protection = 1 + 1.0 * 2.0 = 3.0 (clamped fisher to 1.0)
        assert!((w - 3.0).abs() < 0.01, "expected ~3.0, got {}", w);

        // Fisher < 0.0 should be clamped to 0.0
        let event2 = make_event("tone", 1.0, 1.0, -1.0, 0);
        let w2 = effective_experience_weight_ewc(&event2);
        assert!((w2 - 1.0).abs() < 0.01, "expected ~1.0, got {}", w2);
    }

    #[test]
    fn test_ewc_zero_weight_stays_zero() {
        let event = make_event("tone", 0.0, 1.0, 1.0, 0);
        let w = effective_experience_weight_ewc(&event);
        assert!(w.abs() < f32::EPSILON, "zero weight should stay zero");
    }

    #[test]
    fn test_fisher_importance_first_event_in_category() {
        let event = make_event("new_category", 0.8, 0.9, 0.0, 0);
        let fisher = compute_fisher_importance(&event, &[]);
        assert!(
            (fisher - 1.0).abs() < f32::EPSILON,
            "first event in category should have fisher=1.0, got {}",
            fisher
        );
    }

    #[test]
    fn test_fisher_importance_second_event_lower_than_first() {
        let first = make_event("tone", 0.8, 0.9, 0.0, 5);
        let second = make_event("tone", 0.8, 0.9, 0.0, 0);
        let fisher = compute_fisher_importance(&second, &[first]);
        assert!(
            fisher < 1.0,
            "second event should have lower fisher than first: {}",
            fisher
        );
        assert!(
            fisher > 0.0,
            "second event should still have positive fisher: {}",
            fisher
        );
    }

    #[test]
    fn test_fisher_importance_decreases_with_category_saturation() {
        let base = make_event("tone", 0.8, 0.9, 0.0, 0);
        let existing_1: Vec<ExperienceEvent> = (0..1)
            .map(|i| make_event("tone", 0.8, 0.9, 0.0, i + 1))
            .collect();
        let existing_5: Vec<ExperienceEvent> = (0..5)
            .map(|i| make_event("tone", 0.8, 0.9, 0.0, i + 1))
            .collect();
        let existing_20: Vec<ExperienceEvent> = (0..20)
            .map(|i| make_event("tone", 0.8, 0.9, 0.0, i + 1))
            .collect();

        let f1 = compute_fisher_importance(&base, &existing_1);
        let f5 = compute_fisher_importance(&base, &existing_5);
        let f20 = compute_fisher_importance(&base, &existing_20);

        assert!(
            f1 > f5,
            "1 existing ({}) should yield higher fisher than 5 existing ({})",
            f1,
            f5
        );
        assert!(
            f5 > f20,
            "5 existing ({}) should yield higher fisher than 20 existing ({})",
            f5,
            f20
        );
    }

    #[test]
    fn test_fisher_importance_confidence_alignment_matters() {
        // Event whose confidence matches category mean should score higher than outlier
        let existing = vec![
            make_event("tone", 0.5, 0.8, 0.0, 1),
            make_event("tone", 0.5, 0.82, 0.0, 2),
            make_event("tone", 0.5, 0.78, 0.0, 3),
        ];
        let aligned = make_event("tone", 0.5, 0.80, 0.0, 0); // matches mean
        let outlier = make_event("tone", 0.5, 0.20, 0.0, 0); // far from mean

        let f_aligned = compute_fisher_importance(&aligned, &existing);
        let f_outlier = compute_fisher_importance(&outlier, &existing);

        assert!(
            f_aligned > f_outlier,
            "aligned ({}) should have higher fisher than outlier ({})",
            f_aligned,
            f_outlier
        );
    }

    #[test]
    fn test_fisher_importance_high_weight_increases_score() {
        let existing = vec![make_event("domain", 0.5, 0.8, 0.0, 1)];
        let high_weight = make_event("domain", 1.0, 0.8, 0.0, 0);
        let low_weight = make_event("domain", 0.1, 0.8, 0.0, 0);

        let f_high = compute_fisher_importance(&high_weight, &existing);
        let f_low = compute_fisher_importance(&low_weight, &existing);

        assert!(
            f_high > f_low,
            "high weight ({}) should have higher fisher than low weight ({})",
            f_high,
            f_low
        );
    }

    #[test]
    fn test_fisher_importance_always_in_0_1() {
        // Extreme values
        let event = make_event("x", 100.0, 100.0, 0.0, 0);
        let fisher = compute_fisher_importance(&event, &[]);
        assert!(
            (0.0..=1.0).contains(&fisher),
            "fisher must be in [0,1], got {}",
            fisher
        );

        let existing: Vec<ExperienceEvent> = (0..100)
            .map(|i| make_event("x", 0.1, 0.1, 0.0, i))
            .collect();
        let fisher2 = compute_fisher_importance(&event, &existing);
        assert!(
            (0.0..=1.0).contains(&fisher2),
            "fisher must be in [0,1], got {}",
            fisher2
        );
    }

    #[test]
    fn test_experience_event_fisher_importance_default_serde() {
        // Events serialized before fisher_importance was added should deserialize with 0.0
        let json_str = r#"{
            "id": "01926a1c-7c4e-7000-8000-000000000000",
            "agent_id": "bot",
            "category": "tone",
            "signal": "formal",
            "confidence": 0.9,
            "weight": 0.5,
            "decay_half_life_days": 30,
            "created_at": "2024-01-01T00:00:00Z"
        }"#;
        let event: ExperienceEvent = serde_json::from_str(json_str).unwrap();
        assert!(
            event.fisher_importance.abs() < f32::EPSILON,
            "missing fisher_importance should default to 0.0"
        );
    }

    // ─── Witness Chain Tests ──────────────────────────────────────

    fn make_audit_event(
        action: AgentIdentityAuditAction,
        from_v: Option<u64>,
        to_v: u64,
        prev_hash: Option<String>,
    ) -> AgentIdentityAuditEvent {
        let mut event = AgentIdentityAuditEvent {
            id: Uuid::now_v7(),
            agent_id: "test-agent".to_string(),
            action,
            from_version: from_v,
            to_version: to_v,
            rollback_to_version: None,
            reason: None,
            created_at: Utc::now(),
            prev_hash,
            event_hash: String::new(),
        };
        event.event_hash = event.compute_hash();
        event
    }

    fn build_chain(n: usize) -> Vec<AgentIdentityAuditEvent> {
        let mut chain: Vec<AgentIdentityAuditEvent> = Vec::new();
        for i in 0..n {
            let prev = if i == 0 {
                None
            } else {
                Some(chain[i - 1].event_hash.clone())
            };
            let action = if i == 0 {
                AgentIdentityAuditAction::Created
            } else {
                AgentIdentityAuditAction::Updated
            };
            chain.push(make_audit_event(
                action,
                if i == 0 { None } else { Some(i as u64) },
                (i + 1) as u64,
                prev,
            ));
        }
        chain
    }

    #[test]
    fn test_witness_hash_is_deterministic() {
        let event = make_audit_event(AgentIdentityAuditAction::Created, None, 1, None);
        let h1 = event.compute_hash();
        let h2 = event.compute_hash();
        assert_eq!(h1, h2, "Hash must be deterministic");
        assert_eq!(h1.len(), 64, "SHA-256 hex should be 64 chars");
    }

    #[test]
    fn test_witness_hash_changes_with_different_action() {
        let e1 = make_audit_event(AgentIdentityAuditAction::Created, None, 1, None);
        let mut e2 = e1.clone();
        e2.action = AgentIdentityAuditAction::Updated;
        e2.event_hash = e2.compute_hash();
        assert_ne!(
            e1.event_hash, e2.event_hash,
            "Different actions should produce different hashes"
        );
    }

    #[test]
    fn test_witness_hash_changes_with_different_prev_hash() {
        let e1 = make_audit_event(AgentIdentityAuditAction::Updated, Some(1), 2, None);
        let e2 = make_audit_event(
            AgentIdentityAuditAction::Updated,
            Some(1),
            2,
            Some("abc123".to_string()),
        );
        assert_ne!(
            e1.event_hash, e2.event_hash,
            "Different prev_hash should produce different hashes"
        );
    }

    #[test]
    fn test_witness_verify_hash_passes_for_correct_event() {
        let event = make_audit_event(AgentIdentityAuditAction::Created, None, 1, None);
        assert!(event.verify_hash(), "Fresh event should verify");
    }

    #[test]
    fn test_witness_verify_hash_fails_for_tampered_event() {
        let mut event = make_audit_event(AgentIdentityAuditAction::Created, None, 1, None);
        event.to_version = 999; // tamper
        assert!(
            !event.verify_hash(),
            "Tampered event should fail verification"
        );
    }

    #[test]
    fn test_witness_chain_empty_is_valid() {
        let result = verify_audit_chain(&[]);
        assert!(result.valid);
        assert_eq!(result.chain_length, 0);
        assert!(result.breaks.is_empty());
    }

    #[test]
    fn test_witness_chain_single_genesis_is_valid() {
        let chain = build_chain(1);
        let result = verify_audit_chain(&chain);
        assert!(
            result.valid,
            "Single genesis event should be valid: {:?}",
            result.breaks
        );
        assert_eq!(result.chain_length, 1);
    }

    #[test]
    fn test_witness_chain_multi_event_valid() {
        let chain = build_chain(5);
        let result = verify_audit_chain(&chain);
        assert!(
            result.valid,
            "5-event chain should be valid: {:?}",
            result.breaks
        );
        assert_eq!(result.chain_length, 5);
    }

    #[test]
    fn test_witness_chain_detects_tampered_event_hash() {
        let mut chain = build_chain(3);
        // Tamper with the middle event's to_version
        chain[1].to_version = 999;
        let result = verify_audit_chain(&chain);
        assert!(!result.valid);
        assert!(!result.breaks.is_empty());
        // At minimum, event 1 should be flagged (hash mismatch)
        assert!(
            result.breaks.iter().any(|b| b.index == 1),
            "Middle event should be flagged"
        );
    }

    #[test]
    fn test_witness_chain_detects_deleted_event() {
        let mut chain = build_chain(4);
        // Remove event at index 2 — event 3 now points to event 2's hash,
        // but event 2 is gone, so event 3's prev_hash won't match event 1's hash
        chain.remove(2);
        let result = verify_audit_chain(&chain);
        assert!(!result.valid);
        assert!(result
            .breaks
            .iter()
            .any(|b| b.reason.contains("prev_hash mismatch")));
    }

    #[test]
    fn test_witness_chain_detects_reordered_events() {
        let mut chain = build_chain(3);
        // Swap events 1 and 2
        chain.swap(1, 2);
        let result = verify_audit_chain(&chain);
        assert!(!result.valid);
    }

    #[test]
    fn test_witness_chain_genesis_with_prev_hash_flagged() {
        let mut chain = build_chain(1);
        chain[0].prev_hash = Some("rogue_hash".to_string());
        chain[0].event_hash = chain[0].compute_hash(); // recompute so hash itself is valid
        let result = verify_audit_chain(&chain);
        assert!(!result.valid);
        assert!(result.breaks[0].reason.contains("Genesis event"));
    }

    #[test]
    fn test_witness_chain_non_genesis_missing_prev_hash() {
        let mut chain = build_chain(2);
        chain[1].prev_hash = None;
        chain[1].event_hash = chain[1].compute_hash();
        let result = verify_audit_chain(&chain);
        assert!(!result.valid);
        assert!(result.breaks[0].reason.contains("Non-genesis"));
    }

    #[test]
    fn test_witness_audit_event_serde_backward_compat() {
        // Events created before witness chain fields should deserialize with defaults
        let json_str = r#"{
            "id": "01926a1c-7c4e-7000-8000-000000000001",
            "agent_id": "test-bot",
            "action": "created",
            "to_version": 1,
            "created_at": "2024-06-01T00:00:00Z"
        }"#;
        let event: AgentIdentityAuditEvent = serde_json::from_str(json_str).unwrap();
        assert!(
            event.prev_hash.is_none(),
            "Missing prev_hash should be None"
        );
        assert!(
            event.event_hash.is_empty(),
            "Missing event_hash should be empty string"
        );
    }

    #[test]
    fn test_witness_chain_verification_serializes() {
        let v = AuditChainVerification {
            valid: true,
            chain_length: 3,
            breaks: vec![],
        };
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["valid"], true);
        assert_eq!(json["chain_length"], 3);
        assert!(json["breaks"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_witness_chain_break_includes_event_id() {
        let brk = AuditChainBreak {
            index: 2,
            event_id: Uuid::now_v7(),
            reason: "hash mismatch".to_string(),
        };
        let json = serde_json::to_value(&brk).unwrap();
        assert_eq!(json["index"], 2);
        assert!(json["event_id"].is_string());
        assert_eq!(json["reason"], "hash mismatch");
    }

    #[test]
    fn test_witness_large_chain_performance() {
        // Ensure 1000-event chain verifies quickly
        let chain = build_chain(1000);
        let start = std::time::Instant::now();
        let result = verify_audit_chain(&chain);
        let elapsed = start.elapsed();
        assert!(result.valid, "1000-event chain should be valid");
        assert!(
            elapsed.as_millis() < 100,
            "1000-event chain should verify in <100ms, took {}ms",
            elapsed.as_millis()
        );
    }

    // ─── Falsification: adversarial witness chain tests ───────────

    #[test]
    fn test_falsify_legacy_events_without_hashes_are_valid() {
        // Pre-witness-chain events have empty event_hash and None prev_hash.
        // The chain verifier should skip these gracefully.
        let legacy = AgentIdentityAuditEvent {
            id: Uuid::now_v7(),
            agent_id: "legacy-bot".to_string(),
            action: AgentIdentityAuditAction::Created,
            from_version: None,
            to_version: 1,
            rollback_to_version: None,
            reason: None,
            created_at: Utc::now(),
            prev_hash: None,
            event_hash: String::new(), // legacy: no hash
        };
        let result = verify_audit_chain(&[legacy]);
        assert!(
            result.valid,
            "Legacy events should pass: {:?}",
            result.breaks
        );
        assert_eq!(result.chain_length, 1);
    }

    #[test]
    fn test_falsify_legacy_then_witnessed_chain_valid() {
        // 2 legacy events followed by 3 witnessed events
        let legacy1 = AgentIdentityAuditEvent {
            id: Uuid::now_v7(),
            agent_id: "bot".to_string(),
            action: AgentIdentityAuditAction::Created,
            from_version: None,
            to_version: 1,
            rollback_to_version: None,
            reason: None,
            created_at: Utc::now(),
            prev_hash: None,
            event_hash: String::new(),
        };
        let legacy2 = AgentIdentityAuditEvent {
            id: Uuid::now_v7(),
            agent_id: "bot".to_string(),
            action: AgentIdentityAuditAction::Updated,
            from_version: Some(1),
            to_version: 2,
            rollback_to_version: None,
            reason: None,
            created_at: Utc::now(),
            prev_hash: None,
            event_hash: String::new(),
        };

        // First witnessed event has no predecessor in the chain (prev_hash = None)
        let w1 = make_audit_event(AgentIdentityAuditAction::Updated, Some(2), 3, None);
        let w2 = make_audit_event(
            AgentIdentityAuditAction::Updated,
            Some(3),
            4,
            Some(w1.event_hash.clone()),
        );
        let w3 = make_audit_event(
            AgentIdentityAuditAction::Updated,
            Some(4),
            5,
            Some(w2.event_hash.clone()),
        );

        let chain = vec![legacy1, legacy2, w1, w2, w3];
        let result = verify_audit_chain(&chain);
        assert!(
            result.valid,
            "Legacy + witnessed chain should be valid: {:?}",
            result.breaks
        );
        assert_eq!(result.chain_length, 5);
    }

    #[test]
    fn test_falsify_hash_input_contains_all_fields() {
        // Changing any field should change the hash
        let base = make_audit_event(AgentIdentityAuditAction::Created, None, 1, None);
        let base_hash = base.event_hash.clone();

        // Change from_version
        let mut v = base.clone();
        v.from_version = Some(99);
        v.event_hash = v.compute_hash();
        assert_ne!(v.event_hash, base_hash, "from_version must affect hash");

        // Change to_version
        let mut v = base.clone();
        v.to_version = 42;
        v.event_hash = v.compute_hash();
        assert_ne!(v.event_hash, base_hash, "to_version must affect hash");

        // Change timestamp
        let mut v = base.clone();
        v.created_at = Utc::now() + chrono::Duration::hours(1);
        v.event_hash = v.compute_hash();
        assert_ne!(v.event_hash, base_hash, "created_at must affect hash");
    }

    #[test]
    fn test_falsify_reason_field_not_in_hash() {
        // The `reason` field is NOT included in the hash (it's supplementary metadata).
        // This is by design: operators may annotate events without breaking the chain.
        let e1 = make_audit_event(AgentIdentityAuditAction::Updated, Some(1), 2, None);
        let mut e2 = e1.clone();
        e2.reason = Some("I changed my mind".to_string());
        e2.event_hash = e2.compute_hash();
        assert_eq!(
            e1.event_hash, e2.event_hash,
            "reason should not affect event_hash"
        );
    }

    #[test]
    fn test_falsify_rollback_to_version_not_in_hash() {
        // Similarly, rollback_to_version is metadata — changing it shouldn't break hash
        let e1 = make_audit_event(AgentIdentityAuditAction::RolledBack, Some(3), 4, None);
        let mut e2 = e1.clone();
        e2.rollback_to_version = Some(1);
        e2.event_hash = e2.compute_hash();
        assert_eq!(
            e1.event_hash, e2.event_hash,
            "rollback_to_version should not affect event_hash"
        );
    }

    #[test]
    fn test_falsify_concurrent_fork_detected() {
        // Simulate two events with the same prev_hash (fork)
        let genesis = make_audit_event(AgentIdentityAuditAction::Created, None, 1, None);
        let fork_a = make_audit_event(
            AgentIdentityAuditAction::Updated,
            Some(1),
            2,
            Some(genesis.event_hash.clone()),
        );
        let fork_b = make_audit_event(
            AgentIdentityAuditAction::Updated,
            Some(1),
            3,
            Some(genesis.event_hash.clone()),
        );

        // If both appear in the chain, event 2 (fork_b) has prev_hash pointing to genesis,
        // but the verifier expects it to point to fork_a (the event at index 1)
        let chain = vec![genesis, fork_a.clone(), fork_b];
        let result = verify_audit_chain(&chain);
        assert!(!result.valid, "Forked chain should be detected as invalid");
        assert!(result.breaks.iter().any(|b| b.index == 2));
    }

    // ─── COW Branching model tests ────────────────────────────────

    #[test]
    fn test_validate_branch_name_valid() {
        assert!(validate_branch_name("experiment-1").is_ok());
        assert!(validate_branch_name("tone_warmer").is_ok());
        assert!(validate_branch_name("a").is_ok());
        assert!(validate_branch_name("abc-123_xyz").is_ok());
    }

    #[test]
    fn test_validate_branch_name_empty() {
        assert!(validate_branch_name("").is_err());
    }

    #[test]
    fn test_validate_branch_name_too_long() {
        let long = "a".repeat(65);
        assert!(validate_branch_name(&long).is_err());
        let exact = "a".repeat(64);
        assert!(validate_branch_name(&exact).is_ok());
    }

    #[test]
    fn test_validate_branch_name_invalid_chars() {
        assert!(validate_branch_name("my branch").is_err()); // space
        assert!(validate_branch_name("foo/bar").is_err()); // slash
        assert!(validate_branch_name("foo..bar").is_err()); // dot
        assert!(validate_branch_name("foo@bar").is_err()); // at
    }

    #[test]
    fn test_branch_metadata_serialization() {
        let meta = BranchMetadata {
            branch_name: "experiment-1".to_string(),
            parent_agent_id: "support-bot".to_string(),
            fork_version: 5,
            created_at: chrono::Utc::now(),
            description: Some("Test warmer tone".to_string()),
            merged: false,
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["branch_name"], "experiment-1");
        assert_eq!(json["parent_agent_id"], "support-bot");
        assert_eq!(json["fork_version"], 5);
        assert_eq!(json["merged"], false);

        let roundtrip: BranchMetadata = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip.branch_name, "experiment-1");
    }

    #[test]
    fn test_create_branch_request_defaults() {
        let req: CreateBranchRequest = serde_json::from_value(json!({
            "branch_name": "test"
        }))
        .unwrap();
        assert!(req.description.is_none());
        assert!(req.core_override.is_none());
    }

    #[test]
    fn test_create_branch_request_with_override() {
        let req: CreateBranchRequest = serde_json::from_value(json!({
            "branch_name": "test",
            "description": "warmer tone",
            "core_override": {"tone": "friendly"}
        }))
        .unwrap();
        assert_eq!(req.description.as_deref(), Some("warmer tone"));
        assert_eq!(req.core_override.unwrap()["tone"], "friendly");
    }

    #[test]
    fn test_merge_result_serialization() {
        let result = MergeResult {
            branch_name: "exp-1".to_string(),
            merged_identity: AgentIdentityProfile::new("test".to_string()),
            parent_version_before: 3,
            branch_core_applied: json!({"tone": "warm"}),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["branch_name"], "exp-1");
        assert_eq!(json["parent_version_before"], 3);
    }

    // ─── COW Branching falsification ─────────────────────────────

    #[test]
    fn test_falsify_branch_name_path_traversal() {
        assert!(validate_branch_name("../main").is_err());
        assert!(validate_branch_name("..%2f..").is_err()); // contains %
        assert!(validate_branch_name("foo/../bar").is_err()); // contains /
        assert!(validate_branch_name("..").is_err()); // contains .
    }

    #[test]
    fn test_falsify_branch_name_colon_rejected() {
        // Colons would collide with the agent_id:branch:name key scheme
        assert!(validate_branch_name("foo:bar").is_err());
    }

    #[test]
    fn test_falsify_branch_name_unicode_rejected() {
        assert!(validate_branch_name("日本語").is_err());
        assert!(validate_branch_name("café").is_err());
        assert!(validate_branch_name("test\u{0000}null").is_err()); // null byte
    }

    #[test]
    fn test_falsify_branch_metadata_missing_merged_defaults_false() {
        // Backward compat: if merged field is missing from JSON, should default to false
        let json = json!({
            "branch_name": "test",
            "parent_agent_id": "bot",
            "fork_version": 1,
            "created_at": "2025-01-01T00:00:00Z"
        });
        let meta: BranchMetadata = serde_json::from_value(json).unwrap();
        assert!(!meta.merged, "merged should default to false");
    }

    #[test]
    fn test_falsify_branch_metadata_missing_description_defaults_none() {
        let json = json!({
            "branch_name": "test",
            "parent_agent_id": "bot",
            "fork_version": 1,
            "created_at": "2025-01-01T00:00:00Z"
        });
        let meta: BranchMetadata = serde_json::from_value(json).unwrap();
        assert!(meta.description.is_none());
    }

    #[test]
    fn test_falsify_create_branch_empty_name_in_request() {
        let req: CreateBranchRequest = serde_json::from_value(json!({
            "branch_name": ""
        }))
        .unwrap();
        // validate_branch_name should catch this
        assert!(validate_branch_name(&req.branch_name).is_err());
    }

    #[test]
    fn test_falsify_branch_name_only_hyphens_valid() {
        // Edge case: branch name of just hyphens should be valid
        assert!(validate_branch_name("---").is_ok());
        assert!(validate_branch_name("-").is_ok());
    }

    #[test]
    fn test_falsify_merge_result_contains_branch_core() {
        let result = MergeResult {
            branch_name: "test".to_string(),
            merged_identity: AgentIdentityProfile::new("bot".to_string()),
            parent_version_before: 1,
            branch_core_applied: json!({"tone": "warm", "style": "casual"}),
        };
        // Verify the branch_core_applied is captured for audit trail
        assert_eq!(result.branch_core_applied["tone"], "warm");
        assert_eq!(result.branch_core_applied["style"], "casual");
    }

    #[test]
    fn test_branch_info_includes_both_metadata_and_identity() {
        let info = BranchInfo {
            metadata: BranchMetadata {
                branch_name: "test".to_string(),
                parent_agent_id: "bot".to_string(),
                fork_version: 1,
                created_at: chrono::Utc::now(),
                description: None,
                merged: false,
            },
            identity: AgentIdentityProfile::new("bot:branch:test".to_string()),
        };
        let json = serde_json::to_value(&info).unwrap();
        assert!(json.get("metadata").is_some());
        assert!(json.get("identity").is_some());
        assert_eq!(json["metadata"]["branch_name"], "test");
        assert_eq!(json["identity"]["agent_id"], "bot:branch:test");
    }

    // ─── Domain Expansion / Fork tests ────────────────────────────

    #[test]
    fn test_experience_filter_default_matches_all() {
        let filter = ExperienceFilter::default();
        let event = ExperienceEvent {
            id: Uuid::from_u128(1),
            agent_id: "bot".into(),
            user_id: None,
            session_id: None,
            category: "greeting".into(),
            signal: "user said hello".into(),
            confidence: 0.8,
            weight: 0.5,
            decay_half_life_days: 30,
            evidence_episode_ids: vec![],
            fisher_importance: 0.3,
            created_at: Utc::now(),
        };
        assert!(filter.matches(&event));
    }

    #[test]
    fn test_experience_filter_category_match() {
        let filter = ExperienceFilter {
            categories: vec!["billing".to_string(), "support".to_string()],
            ..Default::default()
        };
        let mut event = ExperienceEvent {
            id: Uuid::from_u128(1),
            agent_id: "bot".into(),
            user_id: None,
            session_id: None,
            category: "billing".into(),
            signal: "test".into(),
            confidence: 0.9,
            weight: 0.5,
            decay_half_life_days: 30,
            evidence_episode_ids: vec![],
            fisher_importance: 0.3,
            created_at: Utc::now(),
        };
        assert!(filter.matches(&event));

        event.category = "greeting".into();
        assert!(!filter.matches(&event));
    }

    #[test]
    fn test_experience_filter_confidence_threshold() {
        let filter = ExperienceFilter {
            min_confidence: Some(0.7),
            ..Default::default()
        };
        let mut event = ExperienceEvent {
            id: Uuid::from_u128(1),
            agent_id: "bot".into(),
            user_id: None,
            session_id: None,
            category: "test".into(),
            signal: "test".into(),
            confidence: 0.9,
            weight: 0.5,
            decay_half_life_days: 30,
            evidence_episode_ids: vec![],
            fisher_importance: 0.3,
            created_at: Utc::now(),
        };
        assert!(filter.matches(&event));

        event.confidence = 0.3;
        assert!(!filter.matches(&event));
    }

    #[test]
    fn test_experience_filter_weight_threshold() {
        let filter = ExperienceFilter {
            min_weight: Some(0.4),
            ..Default::default()
        };
        let mut event = ExperienceEvent {
            id: Uuid::from_u128(1),
            agent_id: "bot".into(),
            user_id: None,
            session_id: None,
            category: "test".into(),
            signal: "test".into(),
            confidence: 0.9,
            weight: 0.5,
            decay_half_life_days: 30,
            evidence_episode_ids: vec![],
            fisher_importance: 0.3,
            created_at: Utc::now(),
        };
        assert!(filter.matches(&event));

        event.weight = 0.1;
        assert!(!filter.matches(&event));
    }

    #[test]
    fn test_experience_filter_combined() {
        let filter = ExperienceFilter {
            categories: vec!["billing".to_string()],
            min_confidence: Some(0.5),
            min_weight: Some(0.3),
            max_events: Some(100),
        };
        let event = ExperienceEvent {
            id: Uuid::from_u128(1),
            agent_id: "bot".into(),
            user_id: None,
            session_id: None,
            category: "billing".into(),
            signal: "test".into(),
            confidence: 0.8,
            weight: 0.5,
            decay_half_life_days: 30,
            evidence_episode_ids: vec![],
            fisher_importance: 0.3,
            created_at: Utc::now(),
        };
        assert!(filter.matches(&event));
    }

    #[test]
    fn test_validate_fork_agent_id_valid() {
        assert!(validate_fork_agent_id("new-bot-v2").is_ok());
        assert!(validate_fork_agent_id("support.billing.v3").is_ok());
        assert!(validate_fork_agent_id("a").is_ok());
    }

    #[test]
    fn test_validate_fork_agent_id_empty() {
        assert!(validate_fork_agent_id("").is_err());
    }

    #[test]
    fn test_validate_fork_agent_id_too_long() {
        let long_id = "a".repeat(129);
        assert!(validate_fork_agent_id(&long_id).is_err());
    }

    #[test]
    fn test_validate_fork_agent_id_colon_rejected() {
        assert!(validate_fork_agent_id("bot:fork").is_err());
    }

    #[test]
    fn test_validate_fork_agent_id_slash_rejected() {
        assert!(validate_fork_agent_id("bot/fork").is_err());
    }

    #[test]
    fn test_validate_fork_agent_id_path_traversal_rejected() {
        assert!(validate_fork_agent_id("..bot").is_err());
    }

    #[test]
    fn test_fork_agent_request_serializes() {
        let req = ForkAgentRequest {
            new_agent_id: "support-v2".into(),
            core_override: Some(json!({"mission": "help with billing"})),
            experience_filter: Some(ExperienceFilter {
                categories: vec!["billing".into()],
                min_confidence: Some(0.7),
                ..Default::default()
            }),
            description: Some("Fork for billing domain".into()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["new_agent_id"], "support-v2");
        assert!(json["core_override"].is_object());
        assert!(json["experience_filter"].is_object());
    }

    #[test]
    fn test_fork_result_serializes() {
        let result = ForkResult {
            new_agent: AgentIdentityProfile::new("support-v2".into()),
            lineage: ForkLineage {
                parent_agent_id: "support-v1".into(),
                parent_version: 5,
                forked_at: Utc::now(),
                description: Some("billing fork".into()),
                experience_events_transferred: 42,
                experience_filter: None,
            },
        };
        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("new_agent").is_some());
        assert!(json.get("lineage").is_some());
        assert_eq!(json["lineage"]["parent_agent_id"], "support-v1");
        assert_eq!(json["lineage"]["parent_version"], 5);
        assert_eq!(json["lineage"]["experience_events_transferred"], 42);
    }

    #[test]
    fn test_fork_lineage_serde_roundtrip() {
        let lineage = ForkLineage {
            parent_agent_id: "bot-a".into(),
            parent_version: 3,
            forked_at: Utc::now(),
            description: None,
            experience_events_transferred: 10,
            experience_filter: Some(ExperienceFilter {
                categories: vec!["tech".into()],
                min_confidence: Some(0.5),
                min_weight: None,
                max_events: Some(50),
            }),
        };
        let json = serde_json::to_string(&lineage).unwrap();
        let back: ForkLineage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.parent_agent_id, "bot-a");
        assert_eq!(back.parent_version, 3);
        assert_eq!(back.experience_events_transferred, 10);
    }

    // ─── Domain Expansion Falsification ────────────────────────────

    #[test]
    fn test_falsify_fork_id_with_null_bytes() {
        // Null bytes are non-alphanumeric; must be rejected
        assert!(validate_fork_agent_id("agent\0hidden").is_err());
        assert!(validate_fork_agent_id("\0").is_err());
    }

    #[test]
    fn test_falsify_fork_id_with_spaces_and_tabs() {
        assert!(validate_fork_agent_id("agent bot").is_err());
        assert!(validate_fork_agent_id("agent\tbot").is_err());
        assert!(validate_fork_agent_id(" ").is_err());
    }

    #[test]
    fn test_falsify_fork_id_with_unicode() {
        // Non-ASCII characters must be rejected (only ascii_alphanumeric + -_.)
        assert!(validate_fork_agent_id("böt").is_err());
        assert!(validate_fork_agent_id("エージェント").is_err());
        assert!(validate_fork_agent_id("agent-🤖").is_err());
    }

    #[test]
    fn test_falsify_fork_id_boundary_length() {
        // Exactly 128 chars should pass
        let at_limit = "a".repeat(128);
        assert!(validate_fork_agent_id(&at_limit).is_ok());
        // 129 should fail
        let over_limit = "a".repeat(129);
        assert!(validate_fork_agent_id(&over_limit).is_err());
    }

    #[test]
    fn test_falsify_fork_id_double_dot_anywhere() {
        assert!(validate_fork_agent_id("a..b").is_err());
        assert!(validate_fork_agent_id("..start").is_err());
        assert!(validate_fork_agent_id("end..").is_err());
        // Single dots are fine
        assert!(validate_fork_agent_id("a.b.c").is_ok());
    }

    #[test]
    fn test_falsify_experience_filter_absurd_confidence() {
        // min_confidence > 1.0 — filter should reject everything since confidence is 0..=1
        let filter = ExperienceFilter {
            categories: vec![],
            min_confidence: Some(1.5),
            min_weight: None,
            max_events: None,
        };
        let event = ExperienceEvent {
            id: uuid::Uuid::from_u128(99),
            agent_id: "test".into(),
            user_id: None,
            session_id: None,
            category: "general".into(),
            signal: "test signal".into(),
            confidence: 1.0, // max possible
            weight: 1.0,
            decay_half_life_days: 30,
            evidence_episode_ids: vec![],
            fisher_importance: 0.0,
            created_at: Utc::now(),
        };
        // Even max confidence (1.0) doesn't pass a 1.5 threshold
        assert!(!filter.matches(&event));
    }

    #[test]
    fn test_falsify_experience_filter_negative_thresholds() {
        // Negative thresholds should let everything through (all confidences >= 0)
        let filter = ExperienceFilter {
            categories: vec![],
            min_confidence: Some(-0.5),
            min_weight: Some(-1.0),
            max_events: None,
        };
        let event = ExperienceEvent {
            id: uuid::Uuid::from_u128(100),
            agent_id: "test".into(),
            user_id: None,
            session_id: None,
            category: "any".into(),
            signal: "anything".into(),
            confidence: 0.0,
            weight: 0.0,
            decay_half_life_days: 1,
            evidence_episode_ids: vec![],
            fisher_importance: 0.0,
            created_at: Utc::now(),
        };
        assert!(
            filter.matches(&event),
            "Negative thresholds should let zero-value events through"
        );
    }

    #[test]
    fn test_falsify_experience_filter_max_events_not_enforced_by_matches() {
        // max_events is a storage-layer concern, not enforced by matches()
        let filter = ExperienceFilter {
            categories: vec![],
            min_confidence: None,
            min_weight: None,
            max_events: Some(0), // zero max_events
        };
        let event = ExperienceEvent {
            id: uuid::Uuid::from_u128(101),
            agent_id: "test".into(),
            user_id: None,
            session_id: None,
            category: "any".into(),
            signal: "test".into(),
            confidence: 0.9,
            weight: 0.9,
            decay_half_life_days: 30,
            evidence_episode_ids: vec![],
            fisher_importance: 0.0,
            created_at: Utc::now(),
        };
        // matches() should still return true — max_events is applied at the collection layer
        assert!(
            filter.matches(&event),
            "max_events should not be enforced by matches()"
        );
    }

    #[test]
    fn test_falsify_fork_request_minimal_deserialization() {
        // Only required field is new_agent_id — all optional fields should default
        let json = r#"{"new_agent_id": "minimal-bot"}"#;
        let req: ForkAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.new_agent_id, "minimal-bot");
        assert!(req.core_override.is_none());
        assert!(req.experience_filter.is_none());
        assert!(req.description.is_none());
    }

    #[test]
    fn test_falsify_fork_lineage_all_optionals_none_roundtrip() {
        let lineage = ForkLineage {
            parent_agent_id: "parent".into(),
            parent_version: 1,
            forked_at: Utc::now(),
            description: None,
            experience_events_transferred: 0,
            experience_filter: None,
        };
        let json = serde_json::to_string(&lineage).unwrap();
        let back: ForkLineage = serde_json::from_str(&json).unwrap();
        assert!(back.description.is_none());
        assert!(back.experience_filter.is_none());
        assert_eq!(back.experience_events_transferred, 0);
    }

    #[test]
    fn test_falsify_fork_result_preserves_new_agent_version_one() {
        // Fork should always produce version 1 in the result
        let result = ForkResult {
            new_agent: AgentIdentityProfile::new("child".into()),
            lineage: ForkLineage {
                parent_agent_id: "parent".into(),
                parent_version: 99,
                forked_at: Utc::now(),
                description: Some("test".into()),
                experience_events_transferred: 0,
                experience_filter: None,
            },
        };
        assert_eq!(
            result.new_agent.version, 1,
            "Forked agent must start at version 1"
        );
        assert_eq!(
            result.lineage.parent_version, 99,
            "Parent version must be preserved"
        );
    }

    #[test]
    fn test_falsify_experience_filter_empty_vs_wildcard_categories() {
        let event = ExperienceEvent {
            id: uuid::Uuid::from_u128(102),
            agent_id: "test".into(),
            user_id: None,
            session_id: None,
            category: "niche_category".into(),
            signal: "test".into(),
            confidence: 0.5,
            weight: 0.5,
            decay_half_life_days: 30,
            evidence_episode_ids: vec![],
            fisher_importance: 0.0,
            created_at: Utc::now(),
        };
        // Empty categories = wildcard (match all)
        let wildcard = ExperienceFilter::default();
        assert!(
            wildcard.matches(&event),
            "Default filter should match any event"
        );

        // Non-matching category should exclude
        let strict = ExperienceFilter {
            categories: vec!["only_this".into()],
            ..Default::default()
        };
        assert!(
            !strict.matches(&event),
            "Specific category should exclude non-matching events"
        );
    }

    // ─── Verified Identity Updates (Merkle Proof) Tests ───────────

    #[test]
    fn test_merkle_tree_from_allowlist_deterministic() {
        let tree1 = AllowlistMerkleTree::from_allowlist();
        let tree2 = AllowlistMerkleTree::from_allowlist();
        assert_eq!(tree1.root, tree2.root, "Merkle root must be deterministic");
        assert_eq!(tree1.leaves.len(), IDENTITY_ALLOWLIST.len());
    }

    #[test]
    fn test_merkle_tree_leaves_sorted() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let mut sorted = tree.leaves.clone();
        sorted.sort();
        assert_eq!(tree.leaves, sorted, "Leaves must be sorted alphabetically");
    }

    #[test]
    fn test_merkle_tree_root_is_64_hex_chars() {
        let tree = AllowlistMerkleTree::from_allowlist();
        assert_eq!(tree.root.len(), 64, "SHA-256 hex = 64 chars");
        assert!(
            tree.root.chars().all(|c| c.is_ascii_hexdigit()),
            "Root must be valid hex"
        );
    }

    #[test]
    fn test_merkle_proof_for_each_allowlist_key() {
        let tree = AllowlistMerkleTree::from_allowlist();
        for key in IDENTITY_ALLOWLIST {
            let proof = tree
                .prove(key)
                .unwrap_or_else(|| panic!("proof for '{}' should exist", key));
            assert_eq!(proof.key, *key);
            assert_eq!(proof.root, tree.root);
            assert!(
                tree.verify(&proof),
                "Proof for '{}' must verify against the tree root",
                key
            );
        }
    }

    #[test]
    fn test_merkle_proof_nonexistent_key_returns_none() {
        let tree = AllowlistMerkleTree::from_allowlist();
        assert!(tree.prove("nonexistent_key").is_none());
        assert!(tree.prove("").is_none());
    }

    #[test]
    fn test_merkle_static_verification() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let proof = tree.prove("mission").unwrap();
        // Verify using only the root string (not the tree object)
        assert!(AllowlistMerkleTree::verify_against_root(&tree.root, &proof));
    }

    #[test]
    fn test_merkle_proof_wrong_root_fails() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let proof = tree.prove("mission").unwrap();
        let bad_root = "0".repeat(64);
        assert!(!AllowlistMerkleTree::verify_against_root(&bad_root, &proof));
    }

    #[test]
    fn test_merkle_proof_tampered_key_fails() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let mut proof = tree.prove("mission").unwrap();
        proof.key = "hacked".into(); // tamper with the key
        assert!(!tree.verify(&proof), "Tampered key must fail verification");
    }

    #[test]
    fn test_merkle_proof_serde_roundtrip() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let proof = tree.prove("style").unwrap();
        let json = serde_json::to_string(&proof).unwrap();
        let back: AllowlistMembershipProof = serde_json::from_str(&json).unwrap();
        assert_eq!(back.key, "style");
        assert_eq!(back.root, tree.root);
        assert!(tree.verify(&back), "Deserialized proof must still verify");
    }

    #[test]
    fn test_verify_identity_update_proof_valid() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let core = json!({
            "mission": "help users",
            "style": "friendly"
        });
        let proof = IdentityUpdateProof {
            merkle_root: tree.root.clone(),
            key_proofs: vec![tree.prove("mission").unwrap(), tree.prove("style").unwrap()],
        };
        let result = verify_identity_update_proof(&core, &proof);
        assert!(
            result.verified,
            "Valid proof should pass: {:?}",
            result.key_results
        );
    }

    #[test]
    fn test_verify_identity_update_proof_missing_key_proof() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let core = json!({
            "mission": "help users",
            "style": "friendly"
        });
        // Only provide proof for "mission", not "style"
        let proof = IdentityUpdateProof {
            merkle_root: tree.root.clone(),
            key_proofs: vec![tree.prove("mission").unwrap()],
        };
        let result = verify_identity_update_proof(&core, &proof);
        assert!(!result.verified);
        assert!(result
            .key_results
            .iter()
            .any(|r| r.key == "style" && !r.valid));
    }

    #[test]
    fn test_verify_identity_update_proof_wrong_root() {
        let core = json!({"mission": "test"});
        let tree = AllowlistMerkleTree::from_allowlist();
        let proof = IdentityUpdateProof {
            merkle_root: "bad".repeat(16), // wrong root
            key_proofs: vec![tree.prove("mission").unwrap()],
        };
        let result = verify_identity_update_proof(&core, &proof);
        assert!(!result.verified);
    }

    #[test]
    fn test_verify_identity_update_proof_forbidden_nested_key() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let core = json!({
            "mission": {
                "user_data": "should fail"
            }
        });
        let proof = IdentityUpdateProof {
            merkle_root: tree.root.clone(),
            key_proofs: vec![tree.prove("mission").unwrap()],
        };
        let result = verify_identity_update_proof(&core, &proof);
        assert!(!result.verified, "Forbidden nested key should fail");
    }

    #[test]
    fn test_verify_identity_update_proof_extra_proof_rejected() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let core = json!({"mission": "test"});
        let proof = IdentityUpdateProof {
            merkle_root: tree.root.clone(),
            key_proofs: vec![
                tree.prove("mission").unwrap(),
                tree.prove("style").unwrap(), // extra, not in core
            ],
        };
        let result = verify_identity_update_proof(&core, &proof);
        assert!(!result.verified, "Extra proofs for absent keys should fail");
    }

    #[test]
    fn test_verify_identity_update_proof_non_object_core() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let core = json!("not an object");
        let proof = IdentityUpdateProof {
            merkle_root: tree.root.clone(),
            key_proofs: vec![],
        };
        let result = verify_identity_update_proof(&core, &proof);
        assert!(!result.verified, "Non-object core should fail");
    }

    #[test]
    fn test_verify_identity_update_proof_empty_core() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let core = json!({});
        let proof = IdentityUpdateProof {
            merkle_root: tree.root.clone(),
            key_proofs: vec![],
        };
        let result = verify_identity_update_proof(&core, &proof);
        assert!(result.verified, "Empty core with no proofs should pass");
    }

    #[test]
    fn test_verified_identity_update_request_serde() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let req = VerifiedIdentityUpdateRequest {
            core: json!({"persona": "helpful"}),
            proof: IdentityUpdateProof {
                merkle_root: tree.root.clone(),
                key_proofs: vec![tree.prove("persona").unwrap()],
            },
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: VerifiedIdentityUpdateRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.core, json!({"persona": "helpful"}));
        assert_eq!(back.proof.merkle_root, tree.root);
    }

    #[test]
    fn test_forbidden_substrings_constant_matches_server() {
        // Ensure our constants match what the server uses
        assert!(IDENTITY_FORBIDDEN_SUBSTRINGS.contains(&"user"));
        assert!(IDENTITY_FORBIDDEN_SUBSTRINGS.contains(&"session"));
        assert!(IDENTITY_FORBIDDEN_SUBSTRINGS.contains(&"episode"));
        assert!(IDENTITY_FORBIDDEN_SUBSTRINGS.contains(&"email"));
        assert!(IDENTITY_FORBIDDEN_SUBSTRINGS.contains(&"phone"));
        assert!(IDENTITY_FORBIDDEN_SUBSTRINGS.contains(&"address"));
        assert!(IDENTITY_FORBIDDEN_SUBSTRINGS.contains(&"external_id"));
    }

    #[test]
    fn test_allowlist_constant_matches_server() {
        assert!(IDENTITY_ALLOWLIST.contains(&"mission"));
        assert!(IDENTITY_ALLOWLIST.contains(&"style"));
        assert!(IDENTITY_ALLOWLIST.contains(&"boundaries"));
        assert!(IDENTITY_ALLOWLIST.contains(&"capabilities"));
        assert!(IDENTITY_ALLOWLIST.contains(&"values"));
        assert!(IDENTITY_ALLOWLIST.contains(&"persona"));
        assert_eq!(IDENTITY_ALLOWLIST.len(), 6);
    }

    #[test]
    fn test_merkle_tree_from_custom_keys() {
        let tree = AllowlistMerkleTree::from_keys(vec!["alpha".into(), "beta".into()]);
        assert_eq!(tree.leaves, vec!["alpha", "beta"]);
        let proof_a = tree.prove("alpha").unwrap();
        let proof_b = tree.prove("beta").unwrap();
        assert!(tree.verify(&proof_a));
        assert!(tree.verify(&proof_b));
    }

    #[test]
    fn test_merkle_tree_single_leaf() {
        let tree = AllowlistMerkleTree::from_keys(vec!["only".into()]);
        assert_eq!(tree.leaves.len(), 1);
        let proof = tree.prove("only").unwrap();
        assert!(tree.verify(&proof));
    }

    // ─── Verified Identity Updates Falsification ──────────────────

    #[test]
    fn test_falsify_proof_tampered_sibling_hash() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let mut proof = tree.prove("mission").unwrap();
        // Flip a bit in the first sibling hash
        if let Some(sibling) = proof.siblings.first_mut() {
            let mut bytes: Vec<u8> = hex::decode(&sibling.hash).unwrap();
            bytes[0] ^= 0x01; // bit flip
            sibling.hash = hex::encode(bytes);
        }
        assert!(
            !tree.verify(&proof),
            "Tampered sibling hash must fail verification"
        );
    }

    #[test]
    fn test_falsify_proof_swapped_sibling_positions() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let mut proof = tree.prove("mission").unwrap();
        // Swap all sibling positions
        for sibling in &mut proof.siblings {
            sibling.position = match sibling.position {
                SiblingPosition::Left => SiblingPosition::Right,
                SiblingPosition::Right => SiblingPosition::Left,
            };
        }
        // This should fail because the hash computation depends on order
        assert!(
            !tree.verify(&proof),
            "Swapped sibling positions must fail verification"
        );
    }

    #[test]
    fn test_falsify_proof_truncated_siblings() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let mut proof = tree.prove("mission").unwrap();
        // Remove the last sibling (path incomplete)
        if !proof.siblings.is_empty() {
            proof.siblings.pop();
        }
        assert!(
            !tree.verify(&proof),
            "Truncated proof path must fail verification"
        );
    }

    #[test]
    fn test_falsify_proof_key_substitution() {
        // Take a valid proof for "mission" but claim it proves "boundaries"
        let tree = AllowlistMerkleTree::from_allowlist();
        let mut proof = tree.prove("mission").unwrap();
        proof.key = "boundaries".into(); // key substitution attack
        assert!(
            !tree.verify(&proof),
            "Key substitution must fail — leaf hash changes"
        );
    }

    #[test]
    fn test_falsify_proof_for_unlisted_key() {
        let tree = AllowlistMerkleTree::from_allowlist();
        // Craft a core with an unlisted key and provide no valid proof
        let core = json!({"hacked_key": "evil"});
        let proof = IdentityUpdateProof {
            merkle_root: tree.root.clone(),
            key_proofs: vec![], // no proof for hacked_key
        };
        let result = verify_identity_update_proof(&core, &proof);
        assert!(!result.verified);
        assert!(result
            .key_results
            .iter()
            .any(|r| r.key == "hacked_key" && !r.valid));
    }

    #[test]
    fn test_falsify_cross_tree_root_attack() {
        // Build a custom tree with "hacked_key" and try to use its root
        let custom_tree =
            AllowlistMerkleTree::from_keys(vec!["hacked_key".into(), "mission".into()]);
        let canonical_tree = AllowlistMerkleTree::from_allowlist();

        assert_ne!(custom_tree.root, canonical_tree.root);

        let core = json!({"mission": "test"});
        let custom_proof = custom_tree.prove("mission").unwrap();
        let proof = IdentityUpdateProof {
            merkle_root: custom_tree.root.clone(), // wrong root
            key_proofs: vec![custom_proof],
        };
        let result = verify_identity_update_proof(&core, &proof);
        assert!(!result.verified, "Cross-tree root must be rejected");
    }

    #[test]
    fn test_falsify_deeply_nested_forbidden_substring() {
        let tree = AllowlistMerkleTree::from_allowlist();
        // Forbidden substring buried at depth 5
        let core = json!({
            "mission": {
                "level1": {
                    "level2": {
                        "level3": {
                            "level4": {
                                "user_secret": "should be caught"
                            }
                        }
                    }
                }
            }
        });
        let proof = IdentityUpdateProof {
            merkle_root: tree.root.clone(),
            key_proofs: vec![tree.prove("mission").unwrap()],
        };
        let result = verify_identity_update_proof(&core, &proof);
        assert!(
            !result.verified,
            "Deeply nested forbidden key must be caught"
        );
    }

    #[test]
    fn test_falsify_forbidden_substring_in_array_element() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let core = json!({
            "values": [
                {"email_contact": "bad@example.com"}
            ]
        });
        let proof = IdentityUpdateProof {
            merkle_root: tree.root.clone(),
            key_proofs: vec![tree.prove("values").unwrap()],
        };
        let result = verify_identity_update_proof(&core, &proof);
        assert!(
            !result.verified,
            "Forbidden key inside array element must be caught"
        );
    }

    #[test]
    fn test_falsify_case_sensitivity_attack() {
        let tree = AllowlistMerkleTree::from_allowlist();
        // "Mission" (capitalized) is NOT in the allowlist (only "mission")
        let core = json!({"Mission": "sneaky"});
        let proof = IdentityUpdateProof {
            merkle_root: tree.root.clone(),
            key_proofs: vec![], // no valid proof possible
        };
        let result = verify_identity_update_proof(&core, &proof);
        assert!(!result.verified, "Case-differing key must be rejected");
    }

    #[test]
    fn test_falsify_duplicate_proofs_for_same_key() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let core = json!({"mission": "test"});
        let mission_proof = tree.prove("mission").unwrap();
        // Provide duplicate proofs — should still pass (no explicit rejection of duplicates,
        // but the extra "mission" proof won't match a core key that doesn't exist twice)
        let proof = IdentityUpdateProof {
            merkle_root: tree.root.clone(),
            key_proofs: vec![mission_proof.clone(), mission_proof],
        };
        let result = verify_identity_update_proof(&core, &proof);
        // Duplicate proofs produce an "extra proof" error for the second occurrence
        // because core_keys only has "mission" once but key_proofs has it twice.
        // Actually the check iterates core keys and finds a match, then checks for
        // extra proofs — duplicates map to the same key, so no extra. This should pass.
        assert!(
            result.verified,
            "Duplicate proofs for same key should not cause failure"
        );
    }

    #[test]
    fn test_falsify_proof_with_invalid_hex_sibling() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let mut proof = tree.prove("mission").unwrap();
        if let Some(sibling) = proof.siblings.first_mut() {
            sibling.hash = "not_valid_hex_zzzz".into();
        }
        assert!(
            !tree.verify(&proof),
            "Invalid hex in sibling must fail verification"
        );
    }

    #[test]
    fn test_falsify_proof_with_short_hex_sibling() {
        let tree = AllowlistMerkleTree::from_allowlist();
        let mut proof = tree.prove("mission").unwrap();
        if let Some(sibling) = proof.siblings.first_mut() {
            sibling.hash = "abcd".into(); // valid hex but wrong length
        }
        assert!(
            !tree.verify(&proof),
            "Short hex sibling must fail (not 32 bytes)"
        );
    }

    // ─── Approval Policy & Governance Tests (Feature 5) ───────────

    #[test]
    fn test_promotion_status_expired_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&PromotionStatus::Expired).unwrap(),
            "\"expired\""
        );
        let round: PromotionStatus = serde_json::from_str("\"expired\"").unwrap();
        assert_eq!(round, PromotionStatus::Expired);
    }

    #[test]
    fn test_approval_policy_default_for_agent() {
        let policy = ApprovalPolicy::default_for_agent("test-bot");
        assert_eq!(policy.agent_id, "test-bot");
        assert_eq!(policy.low_risk.min_approvers, 1);
        assert_eq!(policy.medium_risk.min_approvers, 1);
        assert_eq!(policy.high_risk.min_approvers, 1);
        assert!(policy.low_risk.cooling_period_hours.is_none());
        assert!(policy.low_risk.auto_reject_after_hours.is_none());
    }

    #[test]
    fn test_approval_policy_requirement_for_risk() {
        let policy = ApprovalPolicy {
            agent_id: "bot".into(),
            low_risk: ApprovalRequirement {
                min_approvers: 1,
                ..Default::default()
            },
            medium_risk: ApprovalRequirement {
                min_approvers: 2,
                cooling_period_hours: Some(12),
                ..Default::default()
            },
            high_risk: ApprovalRequirement {
                min_approvers: 3,
                cooling_period_hours: Some(24),
                auto_reject_after_hours: Some(72),
            },
            updated_at: Utc::now(),
        };
        assert_eq!(policy.requirement_for_risk("low").min_approvers, 1);
        assert_eq!(policy.requirement_for_risk("medium").min_approvers, 2);
        assert_eq!(policy.requirement_for_risk("high").min_approvers, 3);
        // Unknown risk levels default to medium
        assert_eq!(policy.requirement_for_risk("unknown").min_approvers, 2);
        assert_eq!(policy.requirement_for_risk("").min_approvers, 2);
    }

    #[test]
    fn test_approval_policy_requirement_for_risk_case_insensitive() {
        let policy = ApprovalPolicy::default_for_agent("bot");
        // "HIGH", "High", "high" should all map to high_risk
        assert_eq!(
            policy.requirement_for_risk("HIGH").min_approvers,
            policy.high_risk.min_approvers
        );
        assert_eq!(
            policy.requirement_for_risk("Low").min_approvers,
            policy.low_risk.min_approvers
        );
    }

    #[test]
    fn test_approval_requirement_default() {
        let req = ApprovalRequirement::default();
        assert_eq!(req.min_approvers, 1);
        assert!(req.cooling_period_hours.is_none());
        assert!(req.auto_reject_after_hours.is_none());
    }

    #[test]
    fn test_approval_policy_serde_roundtrip() {
        let policy = ApprovalPolicy {
            agent_id: "test-bot".into(),
            low_risk: ApprovalRequirement {
                min_approvers: 1,
                cooling_period_hours: None,
                auto_reject_after_hours: Some(168),
            },
            medium_risk: ApprovalRequirement {
                min_approvers: 2,
                cooling_period_hours: Some(12),
                auto_reject_after_hours: Some(72),
            },
            high_risk: ApprovalRequirement {
                min_approvers: 3,
                cooling_period_hours: Some(24),
                auto_reject_after_hours: Some(48),
            },
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&policy).unwrap();
        let back: ApprovalPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(back.agent_id, "test-bot");
        assert_eq!(back.high_risk.min_approvers, 3);
        assert_eq!(back.high_risk.cooling_period_hours, Some(24));
        assert_eq!(back.high_risk.auto_reject_after_hours, Some(48));
    }

    #[test]
    fn test_set_approval_policy_request_serde() {
        let req: SetApprovalPolicyRequest = serde_json::from_value(json!({
            "low_risk": {"min_approvers": 1},
            "medium_risk": {"min_approvers": 2, "cooling_period_hours": 6},
            "high_risk": {"min_approvers": 3, "cooling_period_hours": 24, "auto_reject_after_hours": 48}
        }))
        .unwrap();
        assert_eq!(req.low_risk.min_approvers, 1);
        assert_eq!(req.medium_risk.min_approvers, 2);
        assert_eq!(req.medium_risk.cooling_period_hours, Some(6));
        assert_eq!(req.high_risk.auto_reject_after_hours, Some(48));
    }

    #[test]
    fn test_proposal_from_request_initializes_new_fields() {
        let req = CreatePromotionProposalRequest {
            id: None,
            proposal: "test".into(),
            candidate_core: json!({"mission": "new"}),
            reason: "reason".into(),
            risk_level: "high".into(),
            source_event_ids: vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
        };
        let proposal = PromotionProposal::from_request("bot", req);
        assert!(proposal.approvers.is_empty());
        assert!(proposal.expired_at.is_none());
        assert_eq!(proposal.status, PromotionStatus::Pending);
    }

    #[test]
    fn test_proposal_has_quorum_default_policy() {
        let policy = ApprovalPolicy::default_for_agent("bot");
        let mut proposal = PromotionProposal::from_request(
            "bot",
            CreatePromotionProposalRequest {
                id: None,
                proposal: "test".into(),
                candidate_core: json!({}),
                reason: "r".into(),
                risk_level: "low".into(),
                source_event_ids: vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
            },
        );
        // No approvers yet → no quorum
        assert!(!proposal.has_quorum(&policy));
        // Add one approver → quorum met (default requires 1)
        proposal.approvers.push("admin-key".into());
        assert!(proposal.has_quorum(&policy));
    }

    #[test]
    fn test_proposal_has_quorum_multi_approver() {
        let policy = ApprovalPolicy {
            agent_id: "bot".into(),
            low_risk: ApprovalRequirement::default(),
            medium_risk: ApprovalRequirement::default(),
            high_risk: ApprovalRequirement {
                min_approvers: 3,
                cooling_period_hours: None,
                auto_reject_after_hours: None,
            },
            updated_at: Utc::now(),
        };
        let mut proposal = PromotionProposal::from_request(
            "bot",
            CreatePromotionProposalRequest {
                id: None,
                proposal: "test".into(),
                candidate_core: json!({}),
                reason: "r".into(),
                risk_level: "high".into(),
                source_event_ids: vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
            },
        );
        proposal.approvers.push("admin-1".into());
        assert!(!proposal.has_quorum(&policy));
        proposal.approvers.push("admin-2".into());
        assert!(!proposal.has_quorum(&policy));
        proposal.approvers.push("admin-3".into());
        assert!(proposal.has_quorum(&policy));
    }

    #[test]
    fn test_proposal_is_expired_no_auto_reject() {
        let policy = ApprovalPolicy::default_for_agent("bot");
        let proposal = PromotionProposal::from_request(
            "bot",
            CreatePromotionProposalRequest {
                id: None,
                proposal: "test".into(),
                candidate_core: json!({}),
                reason: "r".into(),
                risk_level: "low".into(),
                source_event_ids: vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
            },
        );
        // Default policy has no auto_reject_after_hours → never expires
        assert!(!proposal.is_expired(&policy));
    }

    #[test]
    fn test_proposal_is_expired_with_deadline() {
        let policy = ApprovalPolicy {
            agent_id: "bot".into(),
            low_risk: ApprovalRequirement::default(),
            medium_risk: ApprovalRequirement {
                min_approvers: 1,
                cooling_period_hours: None,
                auto_reject_after_hours: Some(1), // 1 hour
            },
            high_risk: ApprovalRequirement::default(),
            updated_at: Utc::now(),
        };
        // Create a proposal backdated to 2 hours ago
        let mut proposal = PromotionProposal::from_request(
            "bot",
            CreatePromotionProposalRequest {
                id: None,
                proposal: "test".into(),
                candidate_core: json!({}),
                reason: "r".into(),
                risk_level: "medium".into(),
                source_event_ids: vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
            },
        );
        proposal.created_at = Utc::now() - chrono::Duration::hours(2);
        assert!(proposal.is_expired(&policy));
    }

    #[test]
    fn test_proposal_is_expired_only_when_pending() {
        let policy = ApprovalPolicy {
            agent_id: "bot".into(),
            low_risk: ApprovalRequirement::default(),
            medium_risk: ApprovalRequirement {
                min_approvers: 1,
                cooling_period_hours: None,
                auto_reject_after_hours: Some(1),
            },
            high_risk: ApprovalRequirement::default(),
            updated_at: Utc::now(),
        };
        let mut proposal = PromotionProposal::from_request(
            "bot",
            CreatePromotionProposalRequest {
                id: None,
                proposal: "test".into(),
                candidate_core: json!({}),
                reason: "r".into(),
                risk_level: "medium".into(),
                source_event_ids: vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
            },
        );
        proposal.created_at = Utc::now() - chrono::Duration::hours(2);
        // Already approved → is_expired should return false
        proposal.status = PromotionStatus::Approved;
        assert!(!proposal.is_expired(&policy));
        // Already rejected → is_expired should return false
        proposal.status = PromotionStatus::Rejected;
        assert!(!proposal.is_expired(&policy));
    }

    #[test]
    fn test_proposal_cooling_period_elapsed_no_cooling() {
        let policy = ApprovalPolicy::default_for_agent("bot");
        let proposal = PromotionProposal::from_request(
            "bot",
            CreatePromotionProposalRequest {
                id: None,
                proposal: "test".into(),
                candidate_core: json!({}),
                reason: "r".into(),
                risk_level: "low".into(),
                source_event_ids: vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
            },
        );
        // No cooling period → always elapsed
        assert!(proposal.cooling_period_elapsed(&policy));
    }

    #[test]
    fn test_proposal_cooling_period_not_elapsed() {
        let policy = ApprovalPolicy {
            agent_id: "bot".into(),
            low_risk: ApprovalRequirement::default(),
            medium_risk: ApprovalRequirement::default(),
            high_risk: ApprovalRequirement {
                min_approvers: 3,
                cooling_period_hours: Some(24),
                auto_reject_after_hours: None,
            },
            updated_at: Utc::now(),
        };
        // Just created → cooling period not elapsed
        let proposal = PromotionProposal::from_request(
            "bot",
            CreatePromotionProposalRequest {
                id: None,
                proposal: "test".into(),
                candidate_core: json!({}),
                reason: "r".into(),
                risk_level: "high".into(),
                source_event_ids: vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
            },
        );
        assert!(!proposal.cooling_period_elapsed(&policy));
    }

    #[test]
    fn test_proposal_cooling_period_elapsed_after_wait() {
        let policy = ApprovalPolicy {
            agent_id: "bot".into(),
            low_risk: ApprovalRequirement::default(),
            medium_risk: ApprovalRequirement::default(),
            high_risk: ApprovalRequirement {
                min_approvers: 3,
                cooling_period_hours: Some(24),
                auto_reject_after_hours: None,
            },
            updated_at: Utc::now(),
        };
        let mut proposal = PromotionProposal::from_request(
            "bot",
            CreatePromotionProposalRequest {
                id: None,
                proposal: "test".into(),
                candidate_core: json!({}),
                reason: "r".into(),
                risk_level: "high".into(),
                source_event_ids: vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
            },
        );
        proposal.created_at = Utc::now() - chrono::Duration::hours(25);
        assert!(proposal.cooling_period_elapsed(&policy));
    }

    #[test]
    fn test_proposal_approvers_backward_compat() {
        // Proposals created before approvers field should deserialize with empty vec
        let json_str = r#"{
            "id": "01926a1c-7c4e-7000-8000-000000000001",
            "agent_id": "bot",
            "proposal": "test",
            "candidate_core": {},
            "reason": "test",
            "risk_level": "low",
            "status": "pending",
            "source_event_ids": [],
            "created_at": "2024-01-01T00:00:00Z"
        }"#;
        let proposal: PromotionProposal = serde_json::from_str(json_str).unwrap();
        assert!(proposal.approvers.is_empty());
        assert!(proposal.expired_at.is_none());
    }

    // ─── Conflict Analysis Tests ──────────────────────────────────

    #[test]
    fn test_conflict_recommendation_from_score() {
        assert_eq!(
            ConflictRecommendation::from_score(0.0),
            ConflictRecommendation::Proceed
        );
        assert_eq!(
            ConflictRecommendation::from_score(0.2),
            ConflictRecommendation::Proceed
        );
        assert_eq!(
            ConflictRecommendation::from_score(0.29),
            ConflictRecommendation::Proceed
        );
        assert_eq!(
            ConflictRecommendation::from_score(0.3),
            ConflictRecommendation::ReviewConflicts
        );
        assert_eq!(
            ConflictRecommendation::from_score(0.5),
            ConflictRecommendation::ReviewConflicts
        );
        assert_eq!(
            ConflictRecommendation::from_score(0.69),
            ConflictRecommendation::ReviewConflicts
        );
        assert_eq!(
            ConflictRecommendation::from_score(0.7),
            ConflictRecommendation::Reject
        );
        assert_eq!(
            ConflictRecommendation::from_score(1.0),
            ConflictRecommendation::Reject
        );
    }

    #[test]
    fn test_conflict_recommendation_serde_roundtrip() {
        for variant in [
            ConflictRecommendation::Proceed,
            ConflictRecommendation::ReviewConflicts,
            ConflictRecommendation::Reject,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let back: ConflictRecommendation = serde_json::from_str(&json).unwrap();
            assert_eq!(back, variant);
        }
    }

    #[test]
    fn test_conflict_analysis_serde_roundtrip() {
        let analysis = ConflictAnalysis {
            proposal_id: Uuid::from_u128(1),
            agent_id: "bot".into(),
            supporting_signals: vec![Uuid::from_u128(2), Uuid::from_u128(3)],
            conflicting_signals: vec![Uuid::from_u128(4)],
            conflict_score: 0.33,
            recommendation: ConflictRecommendation::ReviewConflicts,
        };
        let json = serde_json::to_string(&analysis).unwrap();
        let back: ConflictAnalysis = serde_json::from_str(&json).unwrap();
        assert_eq!(back.proposal_id, Uuid::from_u128(1));
        assert_eq!(back.supporting_signals.len(), 2);
        assert_eq!(back.conflicting_signals.len(), 1);
    }

    #[test]
    fn test_analyze_conflicts_no_events() {
        let proposal = PromotionProposal::from_request(
            "bot",
            CreatePromotionProposalRequest {
                id: Some(Uuid::from_u128(1)),
                proposal: "Add formal tone preference".into(),
                candidate_core: json!({"style": "formal"}),
                reason: "test".into(),
                risk_level: "low".into(),
                source_event_ids: vec![
                    Uuid::from_u128(10),
                    Uuid::from_u128(11),
                    Uuid::from_u128(12),
                ],
            },
        );
        let result = analyze_conflicts(&proposal, &[]);
        assert!(result.supporting_signals.is_empty());
        assert!(result.conflicting_signals.is_empty());
        assert_eq!(result.conflict_score, 0.0);
        assert_eq!(result.recommendation, ConflictRecommendation::Proceed);
    }

    #[test]
    fn test_analyze_conflicts_all_supporting() {
        let proposal = PromotionProposal::from_request(
            "bot",
            CreatePromotionProposalRequest {
                id: Some(Uuid::from_u128(1)),
                proposal: "Add formal tone preference".into(),
                candidate_core: json!({"style": "formal"}),
                reason: "test".into(),
                risk_level: "low".into(),
                source_event_ids: vec![
                    Uuid::from_u128(10),
                    Uuid::from_u128(11),
                    Uuid::from_u128(12),
                ],
            },
        );
        let events = vec![
            ExperienceEvent {
                id: Uuid::from_u128(100),
                agent_id: "bot".into(),
                user_id: None,
                session_id: None,
                category: "tone".into(),
                signal: "user prefers formal language".into(),
                confidence: 0.9,
                weight: 0.8,
                decay_half_life_days: 30,
                evidence_episode_ids: vec![],
                fisher_importance: 0.5,
                created_at: Utc::now(),
            },
            ExperienceEvent {
                id: Uuid::from_u128(101),
                agent_id: "bot".into(),
                user_id: None,
                session_id: None,
                category: "tone".into(),
                signal: "formal style appreciated by users".into(),
                confidence: 0.85,
                weight: 0.7,
                decay_half_life_days: 30,
                evidence_episode_ids: vec![],
                fisher_importance: 0.4,
                created_at: Utc::now(),
            },
        ];
        let result = analyze_conflicts(&proposal, &events);
        assert!(!result.supporting_signals.is_empty());
        assert!(result.conflicting_signals.is_empty());
        assert_eq!(result.conflict_score, 0.0);
        assert_eq!(result.recommendation, ConflictRecommendation::Proceed);
    }

    #[test]
    fn test_analyze_conflicts_detects_opposition() {
        let proposal = PromotionProposal::from_request(
            "bot",
            CreatePromotionProposalRequest {
                id: Some(Uuid::from_u128(1)),
                proposal: "Add formal tone preference".into(),
                candidate_core: json!({"style": "formal"}),
                reason: "test".into(),
                risk_level: "low".into(),
                source_event_ids: vec![
                    Uuid::from_u128(10),
                    Uuid::from_u128(11),
                    Uuid::from_u128(12),
                ],
            },
        );
        let events = vec![ExperienceEvent {
            id: Uuid::from_u128(200),
            agent_id: "bot".into(),
            user_id: None,
            session_id: None,
            category: "tone".into(),
            signal: "user does not prefer formal tone at all".into(),
            confidence: 0.9,
            weight: 0.8,
            decay_half_life_days: 30,
            evidence_episode_ids: vec![],
            fisher_importance: 0.5,
            created_at: Utc::now(),
        }];
        let result = analyze_conflicts(&proposal, &events);
        // The signal contains "not" + overlapping keywords → conflicting
        assert!(
            !result.conflicting_signals.is_empty(),
            "Should detect opposition signal"
        );
        assert!(result.conflict_score > 0.0);
    }

    #[test]
    fn test_analyze_conflicts_irrelevant_events_ignored() {
        let proposal = PromotionProposal::from_request(
            "bot",
            CreatePromotionProposalRequest {
                id: Some(Uuid::from_u128(1)),
                proposal: "Add billing expertise".into(),
                candidate_core: json!({"capabilities": ["billing"]}),
                reason: "test".into(),
                risk_level: "low".into(),
                source_event_ids: vec![
                    Uuid::from_u128(10),
                    Uuid::from_u128(11),
                    Uuid::from_u128(12),
                ],
            },
        );
        let events = vec![ExperienceEvent {
            id: Uuid::from_u128(300),
            agent_id: "bot".into(),
            user_id: None,
            session_id: None,
            category: "greeting".into(),
            signal: "user says hi a lot".into(),
            confidence: 0.9,
            weight: 0.5,
            decay_half_life_days: 30,
            evidence_episode_ids: vec![],
            fisher_importance: 0.1,
            created_at: Utc::now(),
        }];
        let result = analyze_conflicts(&proposal, &events);
        // No keyword overlap → not relevant
        assert!(result.supporting_signals.is_empty());
        assert!(result.conflicting_signals.is_empty());
        assert_eq!(result.conflict_score, 0.0);
    }

    #[test]
    fn test_analyze_conflicts_mixed_signals() {
        let proposal = PromotionProposal::from_request(
            "bot",
            CreatePromotionProposalRequest {
                id: Some(Uuid::from_u128(1)),
                proposal: "Adopt formal communication style".into(),
                candidate_core: json!({"style": "formal communication"}),
                reason: "test".into(),
                risk_level: "medium".into(),
                source_event_ids: vec![
                    Uuid::from_u128(10),
                    Uuid::from_u128(11),
                    Uuid::from_u128(12),
                ],
            },
        );
        let events = vec![
            ExperienceEvent {
                id: Uuid::from_u128(400),
                agent_id: "bot".into(),
                user_id: None,
                session_id: None,
                category: "style".into(),
                signal: "formal communication works well".into(),
                confidence: 0.9,
                weight: 0.8,
                decay_half_life_days: 30,
                evidence_episode_ids: vec![],
                fisher_importance: 0.5,
                created_at: Utc::now(),
            },
            ExperienceEvent {
                id: Uuid::from_u128(401),
                agent_id: "bot".into(),
                user_id: None,
                session_id: None,
                category: "style".into(),
                signal: "avoid formal communication style".into(),
                confidence: 0.8,
                weight: 0.7,
                decay_half_life_days: 30,
                evidence_episode_ids: vec![],
                fisher_importance: 0.4,
                created_at: Utc::now(),
            },
        ];
        let result = analyze_conflicts(&proposal, &events);
        // Should have both supporting and conflicting
        assert!(
            !result.supporting_signals.is_empty() || !result.conflicting_signals.is_empty(),
            "Mixed signals should produce some categorization"
        );
        assert!(
            result.conflict_score > 0.0 && result.conflict_score < 1.0,
            "Mixed signals should produce moderate conflict score, got {}",
            result.conflict_score
        );
    }
}
