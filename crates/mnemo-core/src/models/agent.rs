use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentityProfile {
    pub agent_id: String,
    pub version: u64,
    #[serde(default)]
    pub core: serde_json::Value,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateAgentIdentityRequest {
    #[serde(default)]
    pub core: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceEvent {
    pub id: Uuid,
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    pub category: String,
    pub signal: String,
    pub confidence: f32,
    pub weight: f32,
    pub decay_half_life_days: u32,
    #[serde(default)]
    pub evidence_episode_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateExperienceRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    pub category: String,
    pub signal: String,
    pub confidence: f32,
    #[serde(default = "default_weight")]
    pub weight: f32,
    #[serde(default = "default_half_life")]
    pub decay_half_life_days: u32,
    #[serde(default)]
    pub evidence_episode_ids: Vec<Uuid>,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentIdentityAuditAction {
    Created,
    Updated,
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentityAuditEvent {
    pub id: Uuid,
    pub agent_id: String,
    pub action: AgentIdentityAuditAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_version: Option<u64>,
    pub to_version: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollback_to_version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityRollbackRequest {
    pub target_version: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
