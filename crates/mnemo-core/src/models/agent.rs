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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentityProfile {
    /// Unique identifier for the agent (arbitrary string, e.g. `"support-bot-v2"`).
    pub agent_id: String,
    /// Monotonically increasing version counter. Starts at 1.
    pub version: u64,
    /// Free-form JSON representing the agent's identity. Schema is determined
    /// by the application; Mnemo treats it as an opaque value and never
    /// introspects its contents.
    #[serde(default)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateAgentIdentityRequest {
    /// Replacement `core` value. The entire blob is replaced (not merged).
    #[serde(default)]
    pub core: serde_json::Value,
}

/// A single observed experience signal from an agent–user interaction.
///
/// Experience events are the raw inputs that feed the promotion proposal
/// pipeline. They accumulate over time and decay according to
/// `decay_half_life_days`; the effective weight of an event at query time is
/// `weight * 0.5^(age_days / decay_half_life_days)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentIdentityAuditAction {
    /// Identity profile created for the first time.
    Created,
    /// Identity profile updated (direct PUT or approved promotion).
    Updated,
    /// Identity rolled back to a prior version.
    RolledBack,
}

/// Append-only record of every identity mutation for a given agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// Request body for `POST /api/v1/agents/:agent_id/identity/rollback`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityRollbackRequest {
    /// The historical `version` number to restore.
    pub target_version: u64,
    /// Human-readable rationale for the rollback (recommended for audit trails).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Lifecycle state of a promotion proposal.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromotionStatus {
    /// Awaiting human review.
    Pending,
    /// Approved and applied to the live identity profile.
    Approved,
    /// Rejected; the candidate `core` was not applied.
    Rejected,
}

/// A candidate identity update proposed from accumulated experience signals.
///
/// Proposals are created by the agent (or operator tooling) and require
/// explicit approval via `POST /api/v1/agents/:agent_id/promotions/:id/approve`
/// before they are applied to the live `AgentIdentityProfile`. This approval
/// gate prevents unreviewed identity drift.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionProposal {
    pub id: Uuid,
    pub agent_id: String,
    /// Natural-language description of what this proposal changes and why.
    pub proposal: String,
    /// The proposed replacement `core` blob. Applied verbatim if approved.
    #[serde(default)]
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
    /// UTC timestamp when the proposal was approved (absent until approved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<DateTime<Utc>>,
    /// UTC timestamp when the proposal was rejected (absent until rejected).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Request body for `POST /api/v1/agents/:agent_id/promotions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePromotionProposalRequest {
    /// Optional client-supplied ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Uuid>,
    pub proposal: String,
    #[serde(default)]
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
            approved_at: None,
            rejected_at: None,
            created_at: Utc::now(),
        }
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
            fisher >= 0.0 && fisher <= 1.0,
            "fisher must be in [0,1], got {}",
            fisher
        );

        let existing: Vec<ExperienceEvent> = (0..100)
            .map(|i| make_event("x", 0.1, 0.1, 0.0, i))
            .collect();
        let fisher2 = compute_fisher_importance(&event, &existing);
        assert!(
            fisher2 >= 0.0 && fisher2 <= 1.0,
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
}
