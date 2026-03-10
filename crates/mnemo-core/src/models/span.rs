use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single LLM call span captured for tracing/observability.
///
/// Shared between the server (route-time spans) and the ingest worker
/// (background extraction/embedding/summarization spans).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSpan {
    pub id: Uuid,
    /// The `x-mnemo-request-id` that triggered this call (if available).
    pub request_id: Option<String>,
    pub user_id: Option<Uuid>,
    pub provider: String,
    pub model: String,
    /// Operation type: "extract", "summarize", "digest", "embed_episode",
    /// "session_summarize", "detect_contradictions", "chat_completion".
    pub operation: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub latency_ms: u64,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}
