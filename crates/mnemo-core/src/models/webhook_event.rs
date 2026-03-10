//! Webhook event types emitted by the ingestion pipeline.
//!
//! Defined in `mnemo-core` so both `mnemo-ingest` (sender) and
//! `mnemo-server` (receiver) can reference them without circular deps.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An event emitted by the ingest worker when the knowledge graph changes.
///
/// Sent over a `tokio::mpsc` channel from the ingest worker to the server,
/// which translates it into a webhook delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IngestWebhookEvent {
    /// A new edge (fact) was created during episode processing.
    FactAdded {
        user_id: Uuid,
        edge_id: Uuid,
        source_entity: String,
        target_entity: String,
        label: String,
        fact: String,
        episode_id: Uuid,
        /// The request_id from the episode metadata, if present.
        request_id: Option<String>,
    },
    /// An existing edge was invalidated (superseded) because a newer
    /// episode introduced a conflicting fact with the same
    /// (source, target, label) triple.
    FactSuperseded {
        user_id: Uuid,
        /// The old edge that was invalidated.
        old_edge_id: Uuid,
        /// The episode that caused the invalidation.
        invalidated_by_episode_id: Uuid,
        source_entity: String,
        target_entity: String,
        label: String,
        old_fact: String,
        /// The request_id from the episode metadata, if present.
        request_id: Option<String>,
    },
}

impl IngestWebhookEvent {
    /// The user this event belongs to.
    pub fn user_id(&self) -> Uuid {
        match self {
            Self::FactAdded { user_id, .. } | Self::FactSuperseded { user_id, .. } => *user_id,
        }
    }

    /// The request_id associated with this event, if any.
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::FactAdded { request_id, .. } | Self::FactSuperseded { request_id, .. } => {
                request_id.as_deref()
            }
        }
    }
}
