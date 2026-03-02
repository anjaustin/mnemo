use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An edge represents a fact/relationship between two entities in the temporal knowledge graph.
///
/// Edges are the core of Mnemo's temporal reasoning. Unlike traditional knowledge graphs
/// where relationships are static, Mnemo edges track when a fact became true (`valid_at`)
/// and when it was superseded (`invalid_at`). This bi-temporal model enables:
///
/// - Point-in-time queries: "What did we know as of March 2025?"
/// - Change tracking: "What changed in this user's preferences?"
/// - Conflict resolution: New facts invalidate old ones rather than deleting them.
///
/// Example lifecycle:
///   Edge: "Kendra" --loves--> "Adidas shoes" (valid_at: 2024-08-10)
///   New info: "My Adidas shoes fell apart! Nike is my new favorite!"
///   Result: Original edge gets invalid_at: 2025-02-28
///           New edge: "Kendra" --loves--> "Nike shoes" (valid_at: 2025-02-28)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: Uuid,
    pub user_id: Uuid,

    /// The source entity ID.
    pub source_entity_id: Uuid,

    /// The target entity ID.
    pub target_entity_id: Uuid,

    /// The relationship label (e.g., "loves", "works_at", "purchased").
    pub label: String,

    /// A natural language description of the fact.
    /// E.g., "Kendra loves Adidas shoes and wears them exclusively."
    pub fact: String,

    /// When this fact became true in the real world.
    pub valid_at: DateTime<Utc>,

    /// When this fact was superseded or invalidated. None = still valid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_at: Option<DateTime<Utc>>,

    /// When this edge was created in Mnemo (ingestion time).
    pub ingested_at: DateTime<Utc>,

    /// The episode that caused this edge to be created.
    pub source_episode_id: Uuid,

    /// If this edge invalidated a previous edge, record which one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalidated_by_episode_id: Option<Uuid>,

    /// Confidence score from the extraction model (0.0–1.0).
    pub confidence: f32,

    /// Number of episodes that corroborate this fact.
    pub corroboration_count: u32,

    /// Arbitrary metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Represents a relationship extracted by the LLM/extraction pipeline
/// before it's been resolved against the existing graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedRelationship {
    /// The source entity name (will be resolved to an entity ID).
    pub source_name: String,
    /// The target entity name.
    pub target_name: String,
    /// The relationship label.
    pub label: String,
    /// Natural language fact description.
    pub fact: String,
    /// Extraction confidence.
    pub confidence: f32,
    /// When the fact is stated to be valid (extracted from temporal cues in the text).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_at: Option<DateTime<Utc>>,
}

/// Query parameters for filtering edges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeFilter {
    /// Filter by source entity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_entity_id: Option<Uuid>,

    /// Filter by target entity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_entity_id: Option<Uuid>,

    /// Filter by relationship label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// Only return edges valid at this point in time.
    /// An edge is valid at time T if: valid_at <= T AND (invalid_at IS NULL OR invalid_at > T)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_at_time: Option<DateTime<Utc>>,

    /// Include invalidated edges (default: false — only current facts).
    #[serde(default)]
    pub include_invalidated: bool,

    #[serde(default = "default_limit")]
    pub limit: u32,
}

impl Default for EdgeFilter {
    fn default() -> Self {
        Self {
            source_entity_id: None,
            target_entity_id: None,
            label: None,
            valid_at_time: None,
            include_invalidated: false,
            limit: default_limit(),
        }
    }
}

fn default_limit() -> u32 {
    100
}

impl Edge {
    /// Create a new edge from an extraction result with resolved entity IDs.
    pub fn from_extraction(
        rel: &ExtractedRelationship,
        user_id: Uuid,
        source_entity_id: Uuid,
        target_entity_id: Uuid,
        episode_id: Uuid,
        event_time: DateTime<Utc>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::now_v7(),
            user_id,
            source_entity_id,
            target_entity_id,
            label: rel.label.clone(),
            fact: rel.fact.clone(),
            valid_at: rel.valid_at.unwrap_or(event_time),
            invalid_at: None,
            ingested_at: now,
            source_episode_id: episode_id,
            invalidated_by_episode_id: None,
            confidence: rel.confidence,
            corroboration_count: 1,
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }

    /// Is this edge currently valid (not invalidated)?
    pub fn is_valid(&self) -> bool {
        self.invalid_at.is_none()
    }

    /// Was this edge valid at a specific point in time?
    pub fn is_valid_at(&self, time: DateTime<Utc>) -> bool {
        if self.valid_at > time {
            return false;
        }
        match self.invalid_at {
            Some(invalid) => time < invalid,
            None => true,
        }
    }

    /// Invalidate this edge, recording when and why.
    pub fn invalidate(&mut self, invalidated_by: Uuid) {
        let now = Utc::now();
        self.invalid_at = Some(now);
        self.invalidated_by_episode_id = Some(invalidated_by);
        self.updated_at = now;
    }

    /// Record additional corroboration (another episode confirms this fact).
    pub fn corroborate(&mut self) {
        self.corroboration_count += 1;
        self.updated_at = Utc::now();
    }
}

impl EdgeFilter {
    /// Check if a given edge matches this filter.
    pub fn matches(&self, edge: &Edge) -> bool {
        if let Some(src) = self.source_entity_id {
            if edge.source_entity_id != src {
                return false;
            }
        }
        if let Some(tgt) = self.target_entity_id {
            if edge.target_entity_id != tgt {
                return false;
            }
        }
        if let Some(ref label) = self.label {
            if &edge.label != label {
                return false;
            }
        }
        if let Some(time) = self.valid_at_time {
            if !edge.is_valid_at(time) {
                return false;
            }
        }
        if !self.include_invalidated && !edge.is_valid() {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_relationship() -> ExtractedRelationship {
        ExtractedRelationship {
            source_name: "Kendra".to_string(),
            target_name: "Adidas shoes".to_string(),
            label: "loves".to_string(),
            fact: "Kendra loves Adidas shoes and wears them exclusively".to_string(),
            confidence: 0.95,
            valid_at: None,
        }
    }

    #[test]
    fn test_edge_from_extraction() {
        let user_id = Uuid::now_v7();
        let src = Uuid::now_v7();
        let tgt = Uuid::now_v7();
        let episode_id = Uuid::now_v7();
        let event_time = Utc::now();

        let edge = Edge::from_extraction(
            &sample_relationship(),
            user_id,
            src,
            tgt,
            episode_id,
            event_time,
        );

        assert_eq!(edge.label, "loves");
        assert!(edge.is_valid());
        assert_eq!(edge.confidence, 0.95);
        assert_eq!(edge.corroboration_count, 1);
        assert_eq!(edge.source_entity_id, src);
        assert_eq!(edge.target_entity_id, tgt);
    }

    #[test]
    fn test_edge_temporal_validity() {
        let now = Utc::now();
        let past = now - chrono::Duration::days(30);
        let future = now + chrono::Duration::days(30);

        let mut edge = Edge::from_extraction(
            &sample_relationship(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            past, // valid since 30 days ago
        );

        // Edge is valid at the current time
        assert!(edge.is_valid_at(now));
        // Edge was not valid before it was created
        assert!(!edge.is_valid_at(past - chrono::Duration::days(1)));
        // Edge is valid in the future (not yet invalidated)
        assert!(edge.is_valid_at(future));

        // Now invalidate it
        edge.invalidate(Uuid::now_v7());

        // After invalidation, it's no longer valid at future times
        assert!(!edge.is_valid());
        // But it was valid during its active period
        assert!(edge.is_valid_at(past + chrono::Duration::days(1)));
    }

    #[test]
    fn test_edge_invalidation_records_episode() {
        let mut edge = Edge::from_extraction(
            &sample_relationship(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Utc::now(),
        );

        let invalidating_episode = Uuid::now_v7();
        edge.invalidate(invalidating_episode);

        assert!(!edge.is_valid());
        assert!(edge.invalid_at.is_some());
        assert_eq!(edge.invalidated_by_episode_id, Some(invalidating_episode));
    }

    #[test]
    fn test_edge_corroboration() {
        let mut edge = Edge::from_extraction(
            &sample_relationship(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Utc::now(),
        );

        assert_eq!(edge.corroboration_count, 1);
        edge.corroborate();
        edge.corroborate();
        assert_eq!(edge.corroboration_count, 3);
    }

    #[test]
    fn test_edge_filter_matching() {
        let src = Uuid::now_v7();
        let tgt = Uuid::now_v7();
        let edge = Edge::from_extraction(
            &sample_relationship(),
            Uuid::now_v7(),
            src,
            tgt,
            Uuid::now_v7(),
            Utc::now(),
        );

        // Empty filter matches everything
        assert!(EdgeFilter::default().matches(&edge));

        // Source filter
        let filter = EdgeFilter {
            source_entity_id: Some(src),
            ..Default::default()
        };
        assert!(filter.matches(&edge));

        let filter = EdgeFilter {
            source_entity_id: Some(Uuid::now_v7()), // wrong source
            ..Default::default()
        };
        assert!(!filter.matches(&edge));

        // Label filter
        let filter = EdgeFilter {
            label: Some("loves".to_string()),
            ..Default::default()
        };
        assert!(filter.matches(&edge));

        let filter = EdgeFilter {
            label: Some("hates".to_string()),
            ..Default::default()
        };
        assert!(!filter.matches(&edge));
    }

    #[test]
    fn test_edge_filter_excludes_invalidated_by_default() {
        let mut edge = Edge::from_extraction(
            &sample_relationship(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Utc::now(),
        );
        edge.invalidate(Uuid::now_v7());

        // Default filter excludes invalidated
        assert!(!EdgeFilter::default().matches(&edge));

        // Explicit include
        let filter = EdgeFilter {
            include_invalidated: true,
            ..Default::default()
        };
        assert!(filter.matches(&edge));
    }

    #[test]
    fn test_edge_serialization_roundtrip() {
        let edge = Edge::from_extraction(
            &sample_relationship(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Utc::now(),
        );
        let json = serde_json::to_string(&edge).unwrap();
        let de: Edge = serde_json::from_str(&json).unwrap();
        assert_eq!(de.id, edge.id);
        assert_eq!(de.label, edge.label);
        assert_eq!(de.fact, edge.fact);
    }
}
