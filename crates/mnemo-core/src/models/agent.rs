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
}
