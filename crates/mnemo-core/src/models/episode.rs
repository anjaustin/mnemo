use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The type of an episode, determining how it's processed during graph construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeType {
    /// A chat message with role and optional speaker name.
    Message,
    /// Structured JSON data (CRM events, app events, etc.).
    Json,
    /// Unstructured text (documents, notes, transcripts).
    Text,
}

/// The role of a message episode sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

/// An episode is the atomic unit of data ingestion in Mnemo.
///
/// Episodes flow through the ingestion pipeline:
/// 1. Received via API → stored immediately (sync)
/// 2. Queued for async processing → entity/relationship extraction
/// 3. Graph construction → entities and edges created/updated
///
/// The bi-temporal model tracks:
/// - `created_at`: when the event occurred in the real world
/// - `ingested_at`: when Mnemo received and processed it
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub id: Uuid,
    pub session_id: Uuid,
    pub user_id: Uuid,

    /// What kind of data this episode contains.
    #[serde(rename = "type")]
    pub episode_type: EpisodeType,

    /// The raw content of the episode.
    pub content: String,

    /// For message episodes: the role of the sender.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<MessageRole>,

    /// For message episodes: the display name of the sender.
    /// Critical for entity resolution (e.g., linking "Kendra" in the message to the user entity).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Arbitrary metadata attached to this episode.
    #[serde(default)]
    pub metadata: serde_json::Value,

    /// When this event occurred in the real world (provided by the caller).
    pub created_at: DateTime<Utc>,

    /// When Mnemo received this episode.
    pub ingested_at: DateTime<Utc>,

    /// Processing state for the async graph construction pipeline.
    pub processing_status: ProcessingStatus,

    /// If processing failed, the error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing_error: Option<String>,

    /// IDs of entities extracted from this episode.
    #[serde(default)]
    pub entity_ids: Vec<Uuid>,

    /// IDs of edges extracted from this episode.
    #[serde(default)]
    pub edge_ids: Vec<Uuid>,

    /// Number of times this episode has been retried after processing failure.
    #[serde(default)]
    pub retry_count: u32,
}

/// Tracks the async processing state of an episode through the ingestion pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingStatus {
    /// Received and stored, awaiting processing.
    Pending,
    /// Currently being processed (entity extraction, graph construction).
    Processing,
    /// Successfully processed — entities and edges extracted.
    Completed,
    /// Processing failed — see `processing_error`.
    Failed,
    /// Skipped — e.g., system messages or empty content.
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEpisodeRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Uuid>,

    #[serde(rename = "type")]
    pub episode_type: EpisodeType,

    pub content: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<MessageRole>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(default)]
    pub metadata: serde_json::Value,

    /// When this event occurred. If omitted, defaults to now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
}

/// Batch ingestion request for backfilling conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchCreateEpisodesRequest {
    pub episodes: Vec<CreateEpisodeRequest>,
}

/// Pagination for listing episodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListEpisodesParams {
    #[serde(default = "default_limit")]
    pub limit: u32,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Uuid>,

    /// Filter by processing status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ProcessingStatus>,
}

fn default_limit() -> u32 {
    50
}

impl Episode {
    pub fn from_request(req: CreateEpisodeRequest, session_id: Uuid, user_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: req.id.unwrap_or_else(Uuid::now_v7),
            session_id,
            user_id,
            episode_type: req.episode_type,
            content: req.content,
            role: req.role,
            name: req.name,
            metadata: req.metadata,
            created_at: req.created_at.unwrap_or(now),
            ingested_at: now,
            processing_status: ProcessingStatus::Pending,
            processing_error: None,
            entity_ids: Vec::new(),
            edge_ids: Vec::new(),
            retry_count: 0,
        }
    }

    /// Check if this episode should be processed for entity extraction.
    pub fn should_process(&self) -> bool {
        // Skip system messages and empty content
        if self.content.trim().is_empty() {
            return false;
        }
        if self.role == Some(MessageRole::System) {
            return false;
        }
        true
    }

    /// Mark this episode as currently being processed.
    pub fn mark_processing(&mut self) {
        self.processing_status = ProcessingStatus::Processing;
    }

    /// Mark this episode as successfully processed with extracted entity/edge IDs.
    pub fn mark_completed(&mut self, entity_ids: Vec<Uuid>, edge_ids: Vec<Uuid>) {
        self.processing_status = ProcessingStatus::Completed;
        self.entity_ids = entity_ids;
        self.edge_ids = edge_ids;
        self.processing_error = None;
    }

    /// Mark this episode as failed with an error message.
    pub fn mark_failed(&mut self, error: String) {
        self.processing_status = ProcessingStatus::Failed;
        self.processing_error = Some(error);
    }

    /// Mark this episode as skipped (won't be processed).
    pub fn mark_skipped(&mut self) {
        self.processing_status = ProcessingStatus::Skipped;
    }

    /// Prepare this episode for retry after a transient failure.
    /// Returns the backoff delay in milliseconds, or None if max retries exceeded.
    pub fn requeue_for_retry(&mut self, error: String, max_retries: u32) -> Option<u64> {
        if self.retry_count >= max_retries {
            self.mark_failed(error);
            return None;
        }
        self.retry_count += 1;
        self.processing_status = ProcessingStatus::Pending;
        self.processing_error = Some(error);
        // Exponential backoff: 500ms * 2^(retry_count - 1) → 500ms, 1s, 2s, 4s...
        let delay_ms = 500u64 * 2u64.pow(self.retry_count - 1);
        Some(delay_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_message_request() -> CreateEpisodeRequest {
        CreateEpisodeRequest {
            id: None,
            episode_type: EpisodeType::Message,
            content: "I just switched from Adidas to Nike".to_string(),
            role: Some(MessageRole::User),
            name: Some("Kendra".to_string()),
            metadata: serde_json::json!({}),
            created_at: None,
        }
    }

    #[test]
    fn test_episode_from_request() {
        let session_id = Uuid::now_v7();
        let user_id = Uuid::now_v7();
        let episode = Episode::from_request(sample_message_request(), session_id, user_id);

        assert_eq!(episode.session_id, session_id);
        assert_eq!(episode.user_id, user_id);
        assert_eq!(episode.episode_type, EpisodeType::Message);
        assert_eq!(episode.processing_status, ProcessingStatus::Pending);
        assert!(episode.entity_ids.is_empty());
    }

    #[test]
    fn test_episode_processing_lifecycle() {
        let mut episode =
            Episode::from_request(sample_message_request(), Uuid::now_v7(), Uuid::now_v7());

        assert_eq!(episode.processing_status, ProcessingStatus::Pending);
        assert!(episode.should_process());

        episode.mark_processing();
        assert_eq!(episode.processing_status, ProcessingStatus::Processing);

        let entity_ids = vec![Uuid::now_v7(), Uuid::now_v7()];
        let edge_ids = vec![Uuid::now_v7()];
        episode.mark_completed(entity_ids.clone(), edge_ids.clone());

        assert_eq!(episode.processing_status, ProcessingStatus::Completed);
        assert_eq!(episode.entity_ids.len(), 2);
        assert_eq!(episode.edge_ids.len(), 1);
    }

    #[test]
    fn test_episode_failure_lifecycle() {
        let mut episode =
            Episode::from_request(sample_message_request(), Uuid::now_v7(), Uuid::now_v7());

        episode.mark_processing();
        episode.mark_failed("LLM provider timeout".to_string());

        assert_eq!(episode.processing_status, ProcessingStatus::Failed);
        assert_eq!(
            episode.processing_error.as_deref(),
            Some("LLM provider timeout")
        );
    }

    #[test]
    fn test_should_process_skips_empty_content() {
        let mut req = sample_message_request();
        req.content = "   ".to_string();
        let episode = Episode::from_request(req, Uuid::now_v7(), Uuid::now_v7());
        assert!(!episode.should_process());
    }

    #[test]
    fn test_should_process_skips_system_messages() {
        let mut req = sample_message_request();
        req.role = Some(MessageRole::System);
        let episode = Episode::from_request(req, Uuid::now_v7(), Uuid::now_v7());
        assert!(!episode.should_process());
    }

    #[test]
    fn test_json_episode_type() {
        let req = CreateEpisodeRequest {
            id: None,
            episode_type: EpisodeType::Json,
            content: r#"{"event":"purchase","item":"Nike Air Max","price":129.99}"#.to_string(),
            role: None,
            name: None,
            metadata: serde_json::json!({"source": "crm"}),
            created_at: None,
        };
        let episode = Episode::from_request(req, Uuid::now_v7(), Uuid::now_v7());
        assert_eq!(episode.episode_type, EpisodeType::Json);
        assert!(episode.role.is_none());
        assert!(episode.should_process());
    }

    #[test]
    fn test_episode_serialization_roundtrip() {
        let episode =
            Episode::from_request(sample_message_request(), Uuid::now_v7(), Uuid::now_v7());
        let json = serde_json::to_string(&episode).unwrap();
        let de: Episode = serde_json::from_str(&json).unwrap();
        assert_eq!(de.id, episode.id);
        assert_eq!(de.episode_type, episode.episode_type);
        assert_eq!(de.content, episode.content);
    }
}
