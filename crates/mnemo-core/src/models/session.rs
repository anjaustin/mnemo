//! Conversation sessions.
//!
//! A [`Session`] groups related episodes into a conversation. Sessions track
//! episode counts, start/end timestamps, and provide the unit of progressive
//! summarization and narrative chapter generation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A session represents a single conversation thread.
///
/// Sessions belong to a user and contain an ordered sequence of episodes.
/// They track conversation state and provide a natural grouping for
/// context retrieval ("what happened in this conversation?").
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Uuid,

    /// Optional human-readable label (e.g., "Support ticket #4521").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Arbitrary key-value metadata for application-specific data.
    #[serde(default)]
    pub metadata: serde_json::Value,

    /// Running count of episodes in this session.
    pub episode_count: u64,

    /// The auto-generated summary of this session, updated progressively.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,

    /// Token count of the summary (for budget tracking).
    pub summary_tokens: u32,

    /// Pointer to the latest episode in this session (Thread HEAD).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_episode_id: Option<Uuid>,

    /// When the session HEAD was last updated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_updated_at: Option<DateTime<Utc>>,

    /// Monotonic counter incremented whenever HEAD advances.
    #[serde(default)]
    pub head_version: u64,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    /// When the last episode was added.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateSessionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Uuid>,

    pub user_id: Uuid,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, utoipa::ToSchema)]
pub struct UpdateSessionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    /// Progressive summary written by the ingest pipeline after every N episodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,

    /// Token count of the written summary (for budget tracking).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_tokens: Option<u32>,
}

/// Pagination parameters for listing sessions.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ListSessionsParams {
    #[serde(default = "default_limit")]
    pub limit: u32,

    /// Cursor-based pagination: provide the last session's ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Uuid>,

    /// Filter by activity window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<DateTime<Utc>>,
}

fn default_limit() -> u32 {
    20
}

impl Session {
    pub fn from_request(req: CreateSessionRequest) -> Self {
        let now = Utc::now();
        Self {
            id: req.id.unwrap_or_else(Uuid::now_v7),
            user_id: req.user_id,
            name: req.name,
            metadata: req.metadata,
            episode_count: 0,
            summary: None,
            summary_tokens: 0,
            head_episode_id: None,
            head_updated_at: None,
            head_version: 0,
            created_at: now,
            updated_at: now,
            last_activity_at: None,
        }
    }

    pub fn apply_update(mut self, update: UpdateSessionRequest) -> Self {
        if let Some(name) = update.name {
            self.name = Some(name);
        }
        if let Some(metadata) = update.metadata {
            self.metadata = metadata;
        }
        if let Some(summary) = update.summary {
            self.summary = Some(summary);
        }
        if let Some(tokens) = update.summary_tokens {
            self.summary_tokens = tokens;
        }
        self.updated_at = Utc::now();
        self
    }

    /// Record that a new episode was added.
    pub fn record_episode(&mut self, episode_id: Uuid, event_time: DateTime<Utc>) {
        self.episode_count += 1;
        self.last_activity_at = Some(Utc::now());
        self.head_episode_id = Some(episode_id);
        self.head_updated_at = Some(event_time);
        self.head_version += 1;
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_creation() {
        let user_id = Uuid::now_v7();
        let session = Session::from_request(CreateSessionRequest {
            id: None,
            user_id,
            name: Some("Support Chat".to_string()),
            metadata: serde_json::json!({}),
        });
        assert_eq!(session.user_id, user_id);
        assert_eq!(session.episode_count, 0);
        assert!(session.summary.is_none());
        assert!(session.last_activity_at.is_none());
        assert!(session.head_episode_id.is_none());
        assert!(session.head_updated_at.is_none());
        assert_eq!(session.head_version, 0);
    }

    #[test]
    fn test_session_record_episode_increments() {
        let mut session = Session::from_request(CreateSessionRequest {
            id: None,
            user_id: Uuid::now_v7(),
            name: None,
            metadata: serde_json::json!({}),
        });
        session.record_episode(Uuid::now_v7(), Utc::now());
        session.record_episode(Uuid::now_v7(), Utc::now());
        session.record_episode(Uuid::now_v7(), Utc::now());
        assert_eq!(session.episode_count, 3);
        assert!(session.last_activity_at.is_some());
        assert!(session.head_episode_id.is_some());
        assert!(session.head_updated_at.is_some());
        assert_eq!(session.head_version, 3);
    }

    #[test]
    fn test_session_serialization_roundtrip() {
        let session = Session::from_request(CreateSessionRequest {
            id: None,
            user_id: Uuid::now_v7(),
            name: Some("Test".to_string()),
            metadata: serde_json::json!({"channel": "web"}),
        });
        let json = serde_json::to_string(&session).unwrap();
        let de: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(de.id, session.id);
        assert_eq!(de.name, session.name);
    }
}
