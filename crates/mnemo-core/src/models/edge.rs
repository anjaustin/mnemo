//! Knowledge graph edges (facts) with bi-temporal reasoning.
//!
//! An [`Edge`] represents a typed relationship between two entities with
//! temporal validity, confidence decay, corroboration boosting, Fisher
//! importance scoring, and invalidation tracking. The bi-temporal model
//! distinguishes "when the fact was true" from "when we learned it."

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::classification::Classification;

// ─── Fact Temporal Scope (Spec 03 D2) ─────────────────────────────

/// How the temporal validity of a fact should be interpreted during retrieval.
///
/// Set during LLM extraction. Defaults to `Mutable` when the LLM does not
/// classify the fact or when the field is absent (backward-compatible `None`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FactTemporalScope {
    /// Fact is expected to change over time: preferences, status, targets.
    /// Retrieval strongly prefers the most recent valid value.
    Mutable,
    /// Fact is generally stable: birthdate, company founding year, nationality.
    /// Always included in retrieval regardless of age; decay has no effect.
    Stable,
    /// Fact is explicitly time-bounded: quarterly target, event date, deadline.
    /// Excluded from `current` retrieval after `expires_at`; available in `historical`.
    TimeBounded {
        #[serde(skip_serializing_if = "Option::is_none")]
        expires_at: Option<DateTime<Utc>>,
    },
}

impl FactTemporalScope {
    /// Returns `true` if this fact should be included in a `current` retrieval
    /// at the given reference time.
    pub fn is_current_at(&self, now: DateTime<Utc>) -> bool {
        match self {
            FactTemporalScope::Mutable => true,
            FactTemporalScope::Stable => true,
            FactTemporalScope::TimeBounded { expires_at } => {
                expires_at.map(|exp| now < exp).unwrap_or(true)
            }
        }
    }

    /// Returns `true` if this fact should be immune to temporal decay in scoring.
    pub fn resists_decay(&self) -> bool {
        matches!(self, FactTemporalScope::Stable)
    }
}

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
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
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

    /// The agent that produced the source episode, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_agent_id: Option<String>,

    /// If this edge invalidated a previous edge, record which one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalidated_by_episode_id: Option<Uuid>,

    /// Confidence score from the extraction model (0.0–1.0).
    pub confidence: f32,

    /// Number of episodes that corroborate this fact.
    pub corroboration_count: u32,

    /// Arbitrary metadata.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub metadata: serde_json::Value,

    /// Sensitivity classification.  Defaults to `Internal` for secure-by-default
    /// posture and backward compatibility with data created before v0.6.0.
    #[serde(default)]
    pub classification: Classification,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    // ── Spec 03 additions ──────────────────────────────────────────
    /// Temporal scope of this fact (Spec 03 D2).
    /// `None` is treated as `Mutable` for backward compatibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_scope: Option<FactTemporalScope>,

    /// Number of times this fact has been returned in a context retrieval (Spec 03 D3).
    /// Used for reinforcement scoring. Only distinct sessions are counted.
    #[serde(default)]
    pub access_count: u32,

    /// When this fact was last returned in a context retrieval (Spec 03 D3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_accessed_at: Option<DateTime<Utc>>,
}

/// Represents a relationship extracted by the LLM/extraction pipeline
/// before it's been resolved against the existing graph.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
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
    /// LLM-suggested classification.  Defaults to `Internal` if not provided.
    #[serde(default)]
    pub classification: Classification,

    /// LLM-suggested temporal scope. `None` → treated as `Mutable` at ingest time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_scope: Option<FactTemporalScope>,
}

// ─── Belief Change (Spec 03 D1) ───────────────────────────────────

/// A detected belief change: a new fact with the same subject+predicate but
/// a different object than an existing valid fact.
///
/// Stored in Redis per-user sorted by `detected_at` for the
/// `GET /api/v1/memory/{user}/belief_changes` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BeliefChange {
    pub id: Uuid,
    pub user_id: Uuid,
    /// The source entity name.
    pub subject: String,
    /// The relationship label (predicate).
    pub predicate: String,
    /// The old fact text.
    pub old_value: String,
    /// The new (potentially superseding) fact text.
    pub new_value: String,
    /// The edge ID of the old (now potentially invalidated) fact.
    pub old_edge_id: Uuid,
    /// The edge ID of the new fact.
    pub new_edge_id: Uuid,
    /// When this change was detected (ingest time of new episode).
    pub detected_at: DateTime<Utc>,
    /// Whether Mnemo automatically superseded the old fact.
    /// `true` = old edge was invalidated; `false` = flagged for review.
    pub auto_superseded: bool,
}

/// Query parameters for the belief changes endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct BeliefChangesQuery {
    #[serde(default = "default_belief_limit")]
    pub limit: u32,
    /// Only return changes since this timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<DateTime<Utc>>,
}

fn default_belief_limit() -> u32 {
    50
}

impl Default for BeliefChangesQuery {
    fn default() -> Self {
        Self {
            limit: 50,
            since: None,
        }
    }
}

/// Query parameters for filtering edges.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
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

    /// Maximum classification level to include.  Edges with classification
    /// above this level are excluded.  `None` means no filtering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_classification: Option<Classification>,

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
            max_classification: None,
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
        source_agent_id: Option<String>,
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
            source_agent_id,
            invalidated_by_episode_id: None,
            confidence: rel.confidence,
            corroboration_count: 1,
            metadata: serde_json::json!({}),
            classification: rel.classification,
            created_at: now,
            updated_at: now,
            temporal_scope: rel.temporal_scope.clone(),
            access_count: 0,
            last_accessed_at: None,
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

// ─── Confidence Decay + Revalidation ───────────────────────────────

/// Default half-life for edge confidence decay (in days).
pub const DEFAULT_EDGE_DECAY_HALF_LIFE_DAYS: u32 = 90;

/// Default revalidation threshold. When `effective_confidence` drops below
/// this value, the fact is considered stale and needs revalidation.
pub const DEFAULT_REVALIDATION_THRESHOLD: f32 = 0.3;

/// EWC lambda for edge importance protection — same principle as agent
/// experience consolidation: structurally important edges decay slower.
const EDGE_EWC_LAMBDA: f32 = 2.0;

/// Compute the effective confidence of an edge after temporal decay.
///
/// Formula: `confidence * corroboration_boost * decay_factor * importance_protection`
///
/// - `corroboration_boost`: `min(1.0 + 0.1 * (corroboration_count - 1), 2.0)`
///   Each corroboration adds 10% boost, capped at 2x.
/// - `decay_factor`: `2^(-age_days / half_life)` — exponential decay.
/// - `importance_protection`: `1 + clamp(fisher_importance, 0, 1) * EDGE_EWC_LAMBDA`
///   Structurally important edges resist decay.
///
/// The result is clamped to `[0.0, 1.0]`.
pub fn effective_edge_confidence(edge: &Edge, fisher_importance: f32, half_life_days: u32) -> f32 {
    if !edge.is_valid() {
        return 0.0; // invalidated edges have zero effective confidence
    }

    let age_days = (Utc::now() - edge.valid_at).num_days().max(0) as f32;
    let half_life = half_life_days.max(1) as f32;
    let decay_factor = 2f32.powf(-age_days / half_life);

    // Corroboration boost: each additional corroboration adds 10%, capped at 2x
    let corroboration_boost =
        (1.0 + 0.1 * (edge.corroboration_count.saturating_sub(1)) as f32).min(2.0);

    // EWC++ protection: high-importance edges resist decay
    let importance_protection = 1.0 + fisher_importance.clamp(0.0, 1.0) * EDGE_EWC_LAMBDA;

    (edge.confidence * corroboration_boost * decay_factor * importance_protection).clamp(0.0, 1.0)
}

/// Compute the Fisher importance of an edge based on structural centrality
/// and retrieval frequency signals.
///
/// Inputs:
/// - `corroboration_count`: how many episodes confirm this fact
/// - `total_edges_in_label`: total edges with the same label for this user
/// - `outgoing_count`: number of outgoing edges from the source entity
/// - `incoming_count`: number of incoming edges to the target entity
///
/// Higher importance for:
/// - Highly corroborated facts (many episodes agree)
/// - Rare relationship types (few edges with this label)
/// - Edges connecting well-connected entities (hub nodes)
pub fn compute_edge_fisher_importance(
    corroboration_count: u32,
    total_edges_in_label: u32,
    outgoing_count: u32,
    incoming_count: u32,
) -> f32 {
    // Corroboration signal: log-scaled count
    let corroboration_signal = (1.0 + corroboration_count as f32).ln() / (1.0 + 10.0_f32).ln();

    // Rarity signal: inverse of label frequency
    let rarity_signal = 1.0 / (1.0 + total_edges_in_label as f32);

    // Connectivity signal: how central are the connected entities
    let connectivity = (outgoing_count as f32 + incoming_count as f32).sqrt() / 10.0;
    let connectivity_signal = connectivity.min(1.0);

    // Composite: weighted average
    let importance = 0.4 * corroboration_signal + 0.3 * rarity_signal + 0.3 * connectivity_signal;
    importance.clamp(0.0, 1.0)
}

/// A fact that has decayed below the revalidation threshold.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct StaleFact {
    /// The edge representing the stale fact.
    pub edge: Edge,
    /// Current effective confidence after decay.
    pub effective_confidence: f32,
    /// The Fisher importance of this edge.
    pub fisher_importance: f32,
    /// Days since the fact was last corroborated or created.
    pub age_days: u64,
    /// Suggested clarification question for revalidation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_question: Option<String>,
}

/// Request body for `POST /api/v1/memory/:user/revalidate`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RevalidateFactRequest {
    /// The edge ID to revalidate.
    pub edge_id: Uuid,
    /// New confidence after revalidation (0.0-1.0).
    pub new_confidence: f32,
    /// Optional: episode that provides evidence for revalidation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_episode_id: Option<Uuid>,
}

/// Response from a revalidation action.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RevalidateFactResult {
    /// The updated edge.
    pub edge: Edge,
    /// Previous confidence before revalidation.
    pub previous_confidence: f32,
    /// New effective confidence after revalidation.
    pub new_effective_confidence: f32,
}

/// Query parameters for the stale facts endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct StaleFactsQuery {
    /// Revalidation threshold (default: 0.3). Facts below this effective
    /// confidence are considered stale.
    #[serde(default = "default_revalidation_threshold")]
    pub threshold: f32,
    /// Maximum number of stale facts to return.
    #[serde(default = "default_stale_limit")]
    pub limit: u32,
    /// Decay half-life in days (default: 90).
    #[serde(default = "default_decay_half_life")]
    pub half_life_days: u32,
}

fn default_revalidation_threshold() -> f32 {
    DEFAULT_REVALIDATION_THRESHOLD
}
fn default_stale_limit() -> u32 {
    50
}
fn default_decay_half_life() -> u32 {
    DEFAULT_EDGE_DECAY_HALF_LIFE_DAYS
}

impl Default for StaleFactsQuery {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_REVALIDATION_THRESHOLD,
            limit: 50,
            half_life_days: DEFAULT_EDGE_DECAY_HALF_LIFE_DAYS,
        }
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
        if let Some(max) = self.max_classification {
            if edge.classification > max {
                return false;
            }
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
            classification: Classification::default(),
            temporal_scope: None,
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
        );
        let json = serde_json::to_string(&edge).unwrap();
        let de: Edge = serde_json::from_str(&json).unwrap();
        assert_eq!(de.id, edge.id);
        assert_eq!(de.label, edge.label);
        assert_eq!(de.fact, edge.fact);
    }

    // ─── Confidence Decay + Revalidation Tests ────────────────────

    fn make_edge_with_age(age_days: i64, confidence: f32, corroboration_count: u32) -> Edge {
        let valid_at = Utc::now() - chrono::Duration::days(age_days);
        let mut edge = Edge::from_extraction(
            &sample_relationship(),
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            Uuid::from_u128(4),
            valid_at,
            None,
        );
        edge.confidence = confidence;
        edge.corroboration_count = corroboration_count;
        edge
    }

    #[test]
    fn test_effective_confidence_fresh_edge() {
        let edge = make_edge_with_age(0, 0.9, 1);
        let eff = effective_edge_confidence(&edge, 0.0, 90);
        // Fresh edge: decay_factor ~= 1.0, no corroboration boost, no importance
        // eff = 0.9 * 1.0 * 1.0 * 1.0 = 0.9
        assert!(
            (eff - 0.9).abs() < 0.05,
            "Fresh edge should have ~0.9 effective confidence, got {}",
            eff
        );
    }

    #[test]
    fn test_effective_confidence_decays_over_time() {
        let edge_new = make_edge_with_age(0, 0.8, 1);
        let edge_old = make_edge_with_age(180, 0.8, 1);
        let eff_new = effective_edge_confidence(&edge_new, 0.0, 90);
        let eff_old = effective_edge_confidence(&edge_old, 0.0, 90);
        assert!(
            eff_new > eff_old,
            "Older edge should have lower effective confidence: new={} old={}",
            eff_new,
            eff_old
        );
    }

    #[test]
    fn test_effective_confidence_at_half_life() {
        let edge = make_edge_with_age(90, 1.0, 1);
        let eff = effective_edge_confidence(&edge, 0.0, 90);
        // At exactly half-life: decay_factor = 0.5, no boosts
        // eff = 1.0 * 1.0 * 0.5 * 1.0 = 0.5
        assert!(
            (eff - 0.5).abs() < 0.05,
            "At half-life, confidence should be ~0.5, got {}",
            eff
        );
    }

    #[test]
    fn test_effective_confidence_corroboration_boost() {
        let edge_1 = make_edge_with_age(45, 0.8, 1);
        let edge_5 = make_edge_with_age(45, 0.8, 5);
        let eff_1 = effective_edge_confidence(&edge_1, 0.0, 90);
        let eff_5 = effective_edge_confidence(&edge_5, 0.0, 90);
        assert!(
            eff_5 > eff_1,
            "More corroboration should increase effective confidence: 1={} 5={}",
            eff_1,
            eff_5
        );
    }

    #[test]
    fn test_effective_confidence_corroboration_capped_at_2x() {
        let edge_100 = make_edge_with_age(0, 0.5, 100);
        let eff = effective_edge_confidence(&edge_100, 0.0, 90);
        // corroboration_boost = min(1.0 + 0.1 * 99, 2.0) = 2.0
        // eff = 0.5 * 2.0 * 1.0 * 1.0 = 1.0 (clamped)
        assert!(
            (eff - 1.0).abs() < 0.01,
            "Corroboration should cap at 2x, eff={}",
            eff
        );
    }

    #[test]
    fn test_effective_confidence_fisher_protection() {
        let edge = make_edge_with_age(90, 0.8, 1);
        let eff_no_importance = effective_edge_confidence(&edge, 0.0, 90);
        let eff_high_importance = effective_edge_confidence(&edge, 1.0, 90);
        assert!(
            eff_high_importance > eff_no_importance,
            "High importance should resist decay: low={} high={}",
            eff_no_importance,
            eff_high_importance
        );
    }

    #[test]
    fn test_effective_confidence_invalidated_edge_is_zero() {
        let mut edge = make_edge_with_age(0, 0.9, 5);
        edge.invalidate(Uuid::from_u128(99));
        let eff = effective_edge_confidence(&edge, 1.0, 90);
        assert_eq!(
            eff, 0.0,
            "Invalidated edge must have zero effective confidence"
        );
    }

    #[test]
    fn test_effective_confidence_clamped_to_one() {
        // High confidence + high corroboration + high importance + fresh
        let edge = make_edge_with_age(0, 1.0, 20);
        let eff = effective_edge_confidence(&edge, 1.0, 90);
        assert!(
            eff <= 1.0,
            "Effective confidence must be clamped to 1.0, got {}",
            eff
        );
    }

    #[test]
    fn test_compute_edge_fisher_importance_first_in_category() {
        // Only edge with this label — high rarity signal
        let fi = compute_edge_fisher_importance(1, 1, 3, 3);
        assert!(
            fi > 0.0 && fi <= 1.0,
            "Importance should be in (0, 1], got {}",
            fi
        );
    }

    #[test]
    fn test_compute_edge_fisher_importance_high_corroboration() {
        let fi_low = compute_edge_fisher_importance(1, 10, 5, 5);
        let fi_high = compute_edge_fisher_importance(10, 10, 5, 5);
        assert!(
            fi_high > fi_low,
            "Higher corroboration should increase importance: low={} high={}",
            fi_low,
            fi_high
        );
    }

    #[test]
    fn test_compute_edge_fisher_importance_rare_label() {
        let fi_common = compute_edge_fisher_importance(3, 100, 5, 5);
        let fi_rare = compute_edge_fisher_importance(3, 1, 5, 5);
        assert!(
            fi_rare > fi_common,
            "Rarer label should increase importance: common={} rare={}",
            fi_common,
            fi_rare
        );
    }

    #[test]
    fn test_compute_edge_fisher_importance_high_connectivity() {
        let fi_isolated = compute_edge_fisher_importance(3, 10, 1, 1);
        let fi_hub = compute_edge_fisher_importance(3, 10, 50, 50);
        assert!(
            fi_hub > fi_isolated,
            "Hub nodes should increase edge importance: isolated={} hub={}",
            fi_isolated,
            fi_hub
        );
    }

    #[test]
    fn test_compute_edge_fisher_importance_clamped() {
        let fi = compute_edge_fisher_importance(1000, 1, 1000, 1000);
        assert!(
            (0.0..=1.0).contains(&fi),
            "Importance must be clamped to [0, 1], got {}",
            fi
        );
    }

    #[test]
    fn test_stale_fact_serialization() {
        let edge = make_edge_with_age(100, 0.5, 1);
        let stale = StaleFact {
            edge,
            effective_confidence: 0.15,
            fisher_importance: 0.7,
            age_days: 100,
            suggested_question: Some("Is it still true that Kendra loves Adidas shoes?".into()),
        };
        let json = serde_json::to_string(&stale).unwrap();
        let back: StaleFact = serde_json::from_str(&json).unwrap();
        assert_eq!(back.effective_confidence, 0.15);
        assert_eq!(back.fisher_importance, 0.7);
        assert_eq!(back.age_days, 100);
        assert!(back.suggested_question.is_some());
    }

    #[test]
    fn test_revalidate_request_serialization() {
        let req = RevalidateFactRequest {
            edge_id: Uuid::from_u128(42),
            new_confidence: 0.85,
            evidence_episode_id: Some(Uuid::from_u128(99)),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: RevalidateFactRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.new_confidence, 0.85);
        assert!(back.evidence_episode_id.is_some());
    }

    #[test]
    fn test_stale_facts_query_defaults() {
        let query = StaleFactsQuery::default();
        assert_eq!(query.threshold, DEFAULT_REVALIDATION_THRESHOLD);
        assert_eq!(query.limit, 50);
        assert_eq!(query.half_life_days, DEFAULT_EDGE_DECAY_HALF_LIFE_DAYS);
    }

    #[test]
    fn test_stale_facts_query_deserialization_with_defaults() {
        let json = "{}";
        let query: StaleFactsQuery = serde_json::from_str(json).unwrap();
        assert_eq!(query.threshold, DEFAULT_REVALIDATION_THRESHOLD);
        assert_eq!(query.half_life_days, DEFAULT_EDGE_DECAY_HALF_LIFE_DAYS);
    }

    // ─── Confidence Decay Falsification ───────────────────────────

    #[test]
    fn test_falsify_zero_half_life_no_panic() {
        // half_life_days=0 is clamped to 1 internally — should not divide by zero
        let edge = make_edge_with_age(30, 0.8, 1);
        let eff = effective_edge_confidence(&edge, 0.5, 0);
        assert!(
            eff.is_finite(),
            "Zero half-life must not produce NaN/Inf, got {}",
            eff
        );
        assert!((0.0..=1.0).contains(&eff));
    }

    #[test]
    fn test_falsify_future_valid_at_no_negative_age() {
        // Edge valid_at in the future — age should be clamped to 0
        let future_edge = {
            let valid_at = Utc::now() + chrono::Duration::days(30);
            let mut edge = Edge::from_extraction(
                &sample_relationship(),
                Uuid::from_u128(1),
                Uuid::from_u128(2),
                Uuid::from_u128(3),
                Uuid::from_u128(4),
                valid_at,
                None,
            );
            edge.confidence = 0.8;
            edge
        };
        let eff = effective_edge_confidence(&future_edge, 0.0, 90);
        // age_days is clamped to 0, so decay_factor = 1.0
        assert!(
            (eff - 0.8).abs() < 0.05,
            "Future edge should have no decay applied, got {}",
            eff
        );
    }

    #[test]
    fn test_falsify_fisher_importance_above_one_clamped() {
        let edge = make_edge_with_age(90, 0.8, 1);
        let eff_1 = effective_edge_confidence(&edge, 1.0, 90);
        let eff_5 = effective_edge_confidence(&edge, 5.0, 90); // way above 1.0
        assert_eq!(
            eff_1, eff_5,
            "Fisher importance > 1.0 should be clamped to 1.0"
        );
    }

    #[test]
    fn test_falsify_fisher_importance_negative_clamped() {
        let edge = make_edge_with_age(90, 0.8, 1);
        let eff_0 = effective_edge_confidence(&edge, 0.0, 90);
        let eff_neg = effective_edge_confidence(&edge, -5.0, 90);
        assert_eq!(
            eff_0, eff_neg,
            "Negative Fisher importance should be clamped to 0.0"
        );
    }

    #[test]
    fn test_falsify_corroboration_count_max_no_overflow() {
        let edge = make_edge_with_age(0, 0.5, u32::MAX);
        let eff = effective_edge_confidence(&edge, 0.0, 90);
        assert!(eff.is_finite(), "u32::MAX corroboration must not overflow");
        assert!(
            (0.0..=1.0).contains(&eff),
            "Result must be clamped, got {}",
            eff
        );
    }

    #[test]
    fn test_falsify_zero_confidence_stays_zero() {
        let edge = make_edge_with_age(0, 0.0, 10);
        let eff = effective_edge_confidence(&edge, 1.0, 90);
        assert_eq!(
            eff, 0.0,
            "Zero confidence should stay zero regardless of boosts"
        );
    }

    #[test]
    fn test_falsify_decay_monotonically_decreasing() {
        // Effective confidence should decrease as age increases (same edge otherwise)
        let ages = [0, 30, 60, 90, 180, 365, 730];
        let mut prev_eff = f32::MAX;
        for age in ages {
            let edge = make_edge_with_age(age, 0.8, 1);
            let eff = effective_edge_confidence(&edge, 0.3, 90);
            assert!(
                eff <= prev_eff,
                "Decay must be monotonically decreasing: age={} eff={} prev={}",
                age,
                eff,
                prev_eff
            );
            prev_eff = eff;
        }
    }

    #[test]
    fn test_falsify_stale_query_zero_threshold() {
        // threshold=0.0 means nothing is stale (all effective_confidence >= 0.0)
        let query = StaleFactsQuery {
            threshold: 0.0,
            ..Default::default()
        };
        // An edge with any positive confidence should not be stale at threshold 0.0
        let edge = make_edge_with_age(365, 0.1, 1);
        let eff = effective_edge_confidence(&edge, 0.0, query.half_life_days);
        // At 365 days with half_life=90: decay = 2^(-365/90) ≈ 0.057
        // eff = 0.1 * 1.0 * 0.057 * 1.0 ≈ 0.006
        // Still > 0.0, so not stale at threshold 0.0
        assert!(
            eff >= query.threshold,
            "At threshold 0.0, positive-confidence edges should not be stale"
        );
    }

    #[test]
    fn test_falsify_stale_query_threshold_above_one() {
        // threshold > 1.0 means everything is stale
        let query = StaleFactsQuery {
            threshold: 1.5,
            ..Default::default()
        };
        let edge = make_edge_with_age(0, 1.0, 20);
        let eff = effective_edge_confidence(&edge, 1.0, query.half_life_days);
        assert!(
            eff < query.threshold,
            "At threshold 1.5, even max-boosted edges should be stale (eff={})",
            eff
        );
    }

    #[test]
    fn test_falsify_revalidate_boundary_confidence() {
        // Both 0.0 and 1.0 should be valid confidence values
        let req_zero = RevalidateFactRequest {
            edge_id: Uuid::from_u128(1),
            new_confidence: 0.0,
            evidence_episode_id: None,
        };
        let req_one = RevalidateFactRequest {
            edge_id: Uuid::from_u128(1),
            new_confidence: 1.0,
            evidence_episode_id: None,
        };
        // These should serialize/deserialize without error
        let json_zero = serde_json::to_string(&req_zero).unwrap();
        let json_one = serde_json::to_string(&req_one).unwrap();
        let back_zero: RevalidateFactRequest = serde_json::from_str(&json_zero).unwrap();
        let back_one: RevalidateFactRequest = serde_json::from_str(&json_one).unwrap();
        assert_eq!(back_zero.new_confidence, 0.0);
        assert_eq!(back_one.new_confidence, 1.0);
    }

    #[test]
    fn test_falsify_edge_fisher_importance_all_zeros() {
        // All zeros should not panic and should return a valid importance
        let fi = compute_edge_fisher_importance(0, 0, 0, 0);
        assert!(fi.is_finite(), "All-zero inputs must not produce NaN");
        assert!(
            (0.0..=1.0).contains(&fi),
            "Importance must be clamped, got {}",
            fi
        );
    }

    #[test]
    fn test_falsify_edge_fisher_importance_u32_max_inputs() {
        let fi = compute_edge_fisher_importance(u32::MAX, u32::MAX, u32::MAX, u32::MAX);
        assert!(fi.is_finite(), "u32::MAX inputs must not overflow to Inf");
        assert!(
            (0.0..=1.0).contains(&fi),
            "Importance must be clamped, got {}",
            fi
        );
    }

    // ─── Spec 03 D2: FactTemporalScope tests ───────────────────────

    #[test]
    fn test_fact_temporal_scope_mutable_is_always_current() {
        let scope = FactTemporalScope::Mutable;
        let past = Utc::now() - chrono::Duration::days(3650);
        let future = Utc::now() + chrono::Duration::days(3650);
        assert!(scope.is_current_at(past));
        assert!(scope.is_current_at(future));
    }

    #[test]
    fn test_fact_temporal_scope_stable_is_always_current() {
        let scope = FactTemporalScope::Stable;
        let past = Utc::now() - chrono::Duration::days(3650);
        let future = Utc::now() + chrono::Duration::days(3650);
        assert!(scope.is_current_at(past));
        assert!(scope.is_current_at(future));
    }

    #[test]
    fn test_fact_temporal_scope_stable_resists_decay() {
        assert!(FactTemporalScope::Stable.resists_decay());
        assert!(!FactTemporalScope::Mutable.resists_decay());
        assert!(!FactTemporalScope::TimeBounded { expires_at: None }.resists_decay());
    }

    #[test]
    fn test_fact_temporal_scope_time_bounded_expired() {
        let expired = FactTemporalScope::TimeBounded {
            expires_at: Some(Utc::now() - chrono::Duration::days(1)),
        };
        assert!(
            !expired.is_current_at(Utc::now()),
            "past expiry should not be current"
        );
    }

    #[test]
    fn test_fact_temporal_scope_time_bounded_not_yet_expired() {
        let future_scope = FactTemporalScope::TimeBounded {
            expires_at: Some(Utc::now() + chrono::Duration::days(30)),
        };
        assert!(
            future_scope.is_current_at(Utc::now()),
            "future expiry should be current"
        );
    }

    #[test]
    fn test_fact_temporal_scope_time_bounded_no_expiry_is_current() {
        let no_expiry = FactTemporalScope::TimeBounded { expires_at: None };
        assert!(
            no_expiry.is_current_at(Utc::now()),
            "no expiry means indefinitely current"
        );
    }

    #[test]
    fn test_fact_temporal_scope_serde_roundtrip_mutable() {
        let scope = FactTemporalScope::Mutable;
        let json = serde_json::to_string(&scope).unwrap();
        let back: FactTemporalScope = serde_json::from_str(&json).unwrap();
        assert_eq!(scope, back);
        assert!(json.contains("mutable"));
    }

    #[test]
    fn test_fact_temporal_scope_serde_roundtrip_stable() {
        let scope = FactTemporalScope::Stable;
        let json = serde_json::to_string(&scope).unwrap();
        let back: FactTemporalScope = serde_json::from_str(&json).unwrap();
        assert_eq!(scope, back);
        assert!(json.contains("stable"));
    }

    #[test]
    fn test_fact_temporal_scope_serde_roundtrip_time_bounded_with_expiry() {
        let exp = Utc::now() + chrono::Duration::days(10);
        let scope = FactTemporalScope::TimeBounded {
            expires_at: Some(exp),
        };
        let json = serde_json::to_string(&scope).unwrap();
        let back: FactTemporalScope = serde_json::from_str(&json).unwrap();
        assert_eq!(scope, back);
        assert!(json.contains("time_bounded"));
    }

    #[test]
    fn test_fact_temporal_scope_serde_tagged_format() {
        // Verify the tagged JSON uses `"type"` field as discriminant
        let json = serde_json::to_string(&FactTemporalScope::Stable).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "stable");
    }

    // ─── Spec 03 D1: BeliefChange serde ────────────────────────────

    #[test]
    fn test_belief_change_serde_roundtrip() {
        let change = BeliefChange {
            id: Uuid::from_u128(42),
            user_id: Uuid::from_u128(1),
            subject: "Kendra".into(),
            predicate: "prefers".into(),
            old_value: "Adidas shoes".into(),
            new_value: "Nike shoes".into(),
            old_edge_id: Uuid::from_u128(10),
            new_edge_id: Uuid::from_u128(11),
            detected_at: Utc::now(),
            auto_superseded: true,
        };
        let json = serde_json::to_string(&change).unwrap();
        let back: BeliefChange = serde_json::from_str(&json).unwrap();
        assert_eq!(change.id, back.id);
        assert_eq!(change.subject, back.subject);
        assert_eq!(change.old_value, back.old_value);
        assert_eq!(change.new_value, back.new_value);
        assert!(back.auto_superseded);
    }

    #[test]
    fn test_belief_change_auto_superseded_false() {
        let change = BeliefChange {
            id: Uuid::from_u128(99),
            user_id: Uuid::from_u128(2),
            subject: "Alice".into(),
            predicate: "works_at".into(),
            old_value: "Acme Corp".into(),
            new_value: "Globex Corp".into(),
            old_edge_id: Uuid::from_u128(20),
            new_edge_id: Uuid::nil(),
            detected_at: Utc::now(),
            auto_superseded: false,
        };
        let json = serde_json::to_string(&change).unwrap();
        let back: BeliefChange = serde_json::from_str(&json).unwrap();
        assert!(!back.auto_superseded);
        assert_eq!(back.new_edge_id, Uuid::nil());
    }

    #[test]
    fn test_belief_changes_query_default_limit() {
        let q = BeliefChangesQuery::default();
        assert_eq!(q.limit, 50);
        assert!(q.since.is_none());
    }

    #[test]
    fn test_belief_changes_query_serde_roundtrip() {
        let q = BeliefChangesQuery {
            limit: 25,
            since: None,
        };
        let json = serde_json::to_string(&q).unwrap();
        let back: BeliefChangesQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(back.limit, 25);
        assert!(back.since.is_none());
    }

    #[test]
    fn test_belief_changes_query_missing_limit_defaults() {
        // When limit is absent from JSON, it should default to 50
        let back: BeliefChangesQuery = serde_json::from_str("{}").unwrap();
        assert_eq!(back.limit, 50);
    }
}
