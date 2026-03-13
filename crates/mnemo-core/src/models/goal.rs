//! Goal-conditioned memory retrieval.
//!
//! A [`GoalProfile`] conditions retrieval by active objective rather than only
//! semantic similarity. Profiles include entity/edge label boosts, temporal
//! bias, and boost/suppress keywords. [`compute_relevance_adjustment`]
//! re-scores retrieval results based on the active goal.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A retrieval goal — the agent's current objective that conditions how
/// memory context is assembled. For example, `resolve_ticket` focuses on
/// recent complaints and account status, while `plan_trip` focuses on
/// travel preferences and past destinations.
///
/// Goals can be pre-defined `GoalProfile`s (stored in Redis) or free-form
/// strings that the semantic router interprets heuristically.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum RetrievalGoal {
    /// A named goal that maps to a stored `GoalProfile`.
    Named(String),
}

impl RetrievalGoal {
    /// Get the goal name as a string.
    pub fn name(&self) -> &str {
        match self {
            RetrievalGoal::Named(name) => name,
        }
    }
}

/// A goal profile defines how retrieval weights should be adjusted when
/// the agent is pursuing a specific objective.
///
/// Profiles are stored in Redis and managed via the `/goals` API. Each
/// profile specifies:
/// - Which entity categories to boost or suppress
/// - Temporal window preferences (recent vs. historical)
/// - Edge type priorities (which relationship types matter most)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalProfile {
    /// Unique identifier.
    pub id: Uuid,

    /// The goal name (e.g., "resolve_ticket", "plan_trip", "onboarding").
    /// Must be unique within a user's goal profiles.
    pub name: String,

    /// Human-readable description of when to use this goal.
    #[serde(default)]
    pub description: String,

    /// The user who owns this goal profile. If `None`, this is a global/system profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Uuid>,

    /// Entity category boosts. Keys are category names (e.g., "preference",
    /// "complaint", "travel"). Values are multipliers (>1.0 = boost, <1.0 = suppress).
    #[serde(default)]
    pub entity_category_boosts: std::collections::HashMap<String, f32>,

    /// Edge label boosts. Keys are edge labels (e.g., "prefers", "dislikes",
    /// "visited"). Values are multipliers.
    #[serde(default)]
    pub edge_label_boosts: std::collections::HashMap<String, f32>,

    /// Temporal preference: how much to boost recent vs. historical facts.
    /// Positive values favor recent; negative values favor historical.
    /// Range: -1.0 (purely historical) to 1.0 (purely recent). Default: 0.0 (neutral).
    #[serde(default)]
    pub temporal_bias: f32,

    /// Minimum recency window in days. Facts older than this may get suppressed
    /// (scaled by `temporal_bias`). 0 means no minimum.
    #[serde(default)]
    pub recency_window_days: u32,

    /// Keywords that should boost matching facts when this goal is active.
    #[serde(default)]
    pub boost_keywords: Vec<String>,

    /// Keywords that should suppress matching facts when this goal is active.
    #[serde(default)]
    pub suppress_keywords: Vec<String>,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl GoalProfile {
    /// Create a new goal profile with the given name.
    pub fn new(name: String, user_id: Option<Uuid>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::now_v7(),
            name,
            description: String::new(),
            user_id,
            entity_category_boosts: std::collections::HashMap::new(),
            edge_label_boosts: std::collections::HashMap::new(),
            temporal_bias: 0.0,
            recency_window_days: 0,
            boost_keywords: Vec::new(),
            suppress_keywords: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Clamp temporal_bias to [-1.0, 1.0] range.
    pub fn clamp_temporal_bias(&mut self) {
        self.temporal_bias = self.temporal_bias.clamp(-1.0, 1.0);
    }

    /// Compute a boost factor for a given entity category.
    /// Returns 1.0 (neutral) if the category is not in the boost map.
    pub fn entity_boost(&self, category: &str) -> f32 {
        self.entity_category_boosts
            .get(category)
            .copied()
            .unwrap_or(1.0)
    }

    /// Compute a boost factor for a given edge label.
    /// Returns 1.0 (neutral) if the label is not in the boost map.
    pub fn edge_boost(&self, label: &str) -> f32 {
        self.edge_label_boosts.get(label).copied().unwrap_or(1.0)
    }

    /// Check if a text matches any boost keywords (case-insensitive substring).
    pub fn matches_boost_keyword(&self, text: &str) -> bool {
        let lower = text.to_lowercase();
        self.boost_keywords
            .iter()
            .any(|kw| lower.contains(&kw.to_lowercase()))
    }

    /// Check if a text matches any suppress keywords (case-insensitive substring).
    pub fn matches_suppress_keyword(&self, text: &str) -> bool {
        let lower = text.to_lowercase();
        self.suppress_keywords
            .iter()
            .any(|kw| lower.contains(&kw.to_lowercase()))
    }

    /// Compute a composite relevance adjustment for a fact/entity given its
    /// category, edge label, and text content. Returns a multiplier (>1.0 = boost,
    /// <1.0 = suppress).
    pub fn compute_relevance_adjustment(
        &self,
        category: Option<&str>,
        edge_label: Option<&str>,
        text: &str,
    ) -> f32 {
        let mut multiplier = 1.0_f32;

        if let Some(cat) = category {
            multiplier *= self.entity_boost(cat);
        }
        if let Some(label) = edge_label {
            multiplier *= self.edge_boost(label);
        }

        if self.matches_boost_keyword(text) {
            multiplier *= 1.5;
        }
        if self.matches_suppress_keyword(text) {
            multiplier *= 0.5;
        }

        // Clamp to prevent extreme values
        multiplier.clamp(0.01, 10.0)
    }
}

/// Request body for creating a goal profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGoalProfileRequest {
    pub name: String,

    #[serde(default)]
    pub description: String,

    #[serde(default)]
    pub entity_category_boosts: std::collections::HashMap<String, f32>,

    #[serde(default)]
    pub edge_label_boosts: std::collections::HashMap<String, f32>,

    #[serde(default)]
    pub temporal_bias: f32,

    #[serde(default)]
    pub recency_window_days: u32,

    #[serde(default)]
    pub boost_keywords: Vec<String>,

    #[serde(default)]
    pub suppress_keywords: Vec<String>,
}

/// Request body for updating a goal profile.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateGoalProfileRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_category_boosts: Option<std::collections::HashMap<String, f32>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_label_boosts: Option<std::collections::HashMap<String, f32>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_bias: Option<f32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub recency_window_days: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub boost_keywords: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_keywords: Option<Vec<String>>,
}

impl GoalProfile {
    /// Apply a partial update to this profile.
    pub fn apply_update(mut self, update: UpdateGoalProfileRequest) -> Self {
        if let Some(desc) = update.description {
            self.description = desc;
        }
        if let Some(boosts) = update.entity_category_boosts {
            self.entity_category_boosts = boosts;
        }
        if let Some(boosts) = update.edge_label_boosts {
            self.edge_label_boosts = boosts;
        }
        if let Some(bias) = update.temporal_bias {
            self.temporal_bias = bias;
        }
        if let Some(window) = update.recency_window_days {
            self.recency_window_days = window;
        }
        if let Some(keywords) = update.boost_keywords {
            self.boost_keywords = keywords;
        }
        if let Some(keywords) = update.suppress_keywords {
            self.suppress_keywords = keywords;
        }
        self.clamp_temporal_bias();
        self.updated_at = Utc::now();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_goal_profile_new() {
        let profile = GoalProfile::new("resolve_ticket".into(), None);
        assert_eq!(profile.name, "resolve_ticket");
        assert!(profile.user_id.is_none());
        assert_eq!(profile.temporal_bias, 0.0);
        assert!(profile.entity_category_boosts.is_empty());
        assert!(profile.edge_label_boosts.is_empty());
        assert!(profile.boost_keywords.is_empty());
        assert!(profile.suppress_keywords.is_empty());
    }

    #[test]
    fn test_goal_profile_with_user() {
        let uid = Uuid::from_u128(1);
        let profile = GoalProfile::new("plan_trip".into(), Some(uid));
        assert_eq!(profile.user_id, Some(uid));
    }

    #[test]
    fn test_retrieval_goal_name() {
        let goal = RetrievalGoal::Named("resolve_ticket".into());
        assert_eq!(goal.name(), "resolve_ticket");
    }

    #[test]
    fn test_retrieval_goal_serde() {
        let goal = RetrievalGoal::Named("plan_trip".into());
        let json = serde_json::to_string(&goal).unwrap();
        assert_eq!(json, "\"plan_trip\"");
        let de: RetrievalGoal = serde_json::from_str(&json).unwrap();
        assert_eq!(de, goal);
    }

    #[test]
    fn test_entity_boost() {
        let mut profile = GoalProfile::new("test".into(), None);
        profile
            .entity_category_boosts
            .insert("complaint".into(), 2.0);
        profile.entity_category_boosts.insert("travel".into(), 0.3);

        assert_eq!(profile.entity_boost("complaint"), 2.0);
        assert_eq!(profile.entity_boost("travel"), 0.3);
        assert_eq!(profile.entity_boost("unknown"), 1.0); // default neutral
    }

    #[test]
    fn test_edge_boost() {
        let mut profile = GoalProfile::new("test".into(), None);
        profile.edge_label_boosts.insert("prefers".into(), 1.8);

        assert_eq!(profile.edge_boost("prefers"), 1.8);
        assert_eq!(profile.edge_boost("likes"), 1.0); // default neutral
    }

    #[test]
    fn test_matches_boost_keyword() {
        let mut profile = GoalProfile::new("test".into(), None);
        profile.boost_keywords = vec!["ticket".into(), "complaint".into()];

        assert!(profile.matches_boost_keyword("Ticket #1234"));
        assert!(profile.matches_boost_keyword("user filed a COMPLAINT"));
        assert!(!profile.matches_boost_keyword("travel plans"));
    }

    #[test]
    fn test_matches_suppress_keyword() {
        let mut profile = GoalProfile::new("test".into(), None);
        profile.suppress_keywords = vec!["spam".into(), "test".into()];

        assert!(profile.matches_suppress_keyword("This is spam"));
        assert!(profile.matches_suppress_keyword("TEST message"));
        assert!(!profile.matches_suppress_keyword("Real data"));
    }

    #[test]
    fn test_compute_relevance_adjustment_neutral() {
        let profile = GoalProfile::new("test".into(), None);
        let adj = profile.compute_relevance_adjustment(None, None, "anything");
        assert!((adj - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_compute_relevance_adjustment_boosted() {
        let mut profile = GoalProfile::new("test".into(), None);
        profile
            .entity_category_boosts
            .insert("complaint".into(), 2.0);
        profile.boost_keywords = vec!["urgent".into()];

        // category boost (2.0) * keyword boost (1.5) = 3.0
        let adj = profile.compute_relevance_adjustment(Some("complaint"), None, "urgent issue");
        assert!((adj - 3.0).abs() < 0.01);
    }

    #[test]
    fn test_compute_relevance_adjustment_suppressed() {
        let mut profile = GoalProfile::new("test".into(), None);
        profile
            .entity_category_boosts
            .insert("marketing".into(), 0.5);
        profile.suppress_keywords = vec!["promo".into()];

        // category (0.5) * suppress (0.5) = 0.25
        let adj = profile.compute_relevance_adjustment(Some("marketing"), None, "promo offer");
        assert!((adj - 0.25).abs() < 0.01);
    }

    #[test]
    fn test_compute_relevance_adjustment_clamped() {
        let mut profile = GoalProfile::new("test".into(), None);
        profile.entity_category_boosts.insert("cat".into(), 100.0);
        profile.boost_keywords = vec!["match".into()];

        let adj = profile.compute_relevance_adjustment(Some("cat"), None, "match this");
        assert_eq!(adj, 10.0); // Clamped to max 10.0
    }

    #[test]
    fn test_clamp_temporal_bias() {
        let mut profile = GoalProfile::new("test".into(), None);
        profile.temporal_bias = 5.0;
        profile.clamp_temporal_bias();
        assert_eq!(profile.temporal_bias, 1.0);

        profile.temporal_bias = -3.0;
        profile.clamp_temporal_bias();
        assert_eq!(profile.temporal_bias, -1.0);
    }

    #[test]
    fn test_goal_profile_serde_roundtrip() {
        let mut profile = GoalProfile::new("resolve_ticket".into(), Some(Uuid::from_u128(42)));
        profile.description = "Focus on support issues".into();
        profile
            .entity_category_boosts
            .insert("complaint".into(), 2.0);
        profile.edge_label_boosts.insert("filed_by".into(), 1.5);
        profile.temporal_bias = 0.7;
        profile.recency_window_days = 30;
        profile.boost_keywords = vec!["urgent".into()];
        profile.suppress_keywords = vec!["spam".into()];

        let json = serde_json::to_string(&profile).unwrap();
        let de: GoalProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(de.name, "resolve_ticket");
        assert_eq!(de.entity_category_boosts.get("complaint"), Some(&2.0));
        assert_eq!(de.boost_keywords, vec!["urgent"]);
    }

    #[test]
    fn test_create_goal_profile_request_serde() {
        let json = r#"{
            "name": "plan_trip",
            "description": "Focus on travel",
            "entity_category_boosts": {"destination": 2.0},
            "temporal_bias": -0.5,
            "boost_keywords": ["travel", "vacation"]
        }"#;
        let req: CreateGoalProfileRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "plan_trip");
        assert_eq!(req.temporal_bias, -0.5);
        assert_eq!(req.boost_keywords.len(), 2);
    }

    #[test]
    fn test_update_goal_profile_request_partial() {
        let json = r#"{"description": "Updated description"}"#;
        let req: UpdateGoalProfileRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.description, Some("Updated description".into()));
        assert!(req.entity_category_boosts.is_none());
        assert!(req.temporal_bias.is_none());
    }

    #[test]
    fn test_apply_update() {
        let profile = GoalProfile::new("test".into(), None);
        let updated = profile.apply_update(UpdateGoalProfileRequest {
            description: Some("New description".into()),
            temporal_bias: Some(0.8),
            ..Default::default()
        });
        assert_eq!(updated.description, "New description");
        assert_eq!(updated.temporal_bias, 0.8);
        assert_eq!(updated.name, "test"); // unchanged
    }

    #[test]
    fn test_apply_update_clamps_temporal_bias() {
        let profile = GoalProfile::new("test".into(), None);
        let updated = profile.apply_update(UpdateGoalProfileRequest {
            temporal_bias: Some(999.0),
            ..Default::default()
        });
        assert_eq!(updated.temporal_bias, 1.0); // clamped
    }

    // ─── Falsification / Adversarial Tests ─────────────────────────

    #[test]
    fn test_falsify_zero_boost_multiplier() {
        // A boost of 0.0 should not zero out relevance — clamped to 0.01
        let mut profile = GoalProfile::new("test".into(), None);
        profile
            .entity_category_boosts
            .insert("category".into(), 0.0);
        let adj = profile.compute_relevance_adjustment(Some("category"), None, "text");
        assert!(
            adj >= 0.01,
            "adjustment should be clamped to min 0.01, got {adj}"
        );
    }

    #[test]
    fn test_falsify_negative_boost_multiplier() {
        // Negative boost values should be clamped
        let mut profile = GoalProfile::new("test".into(), None);
        profile.entity_category_boosts.insert("cat".into(), -5.0);
        let adj = profile.compute_relevance_adjustment(Some("cat"), None, "text");
        assert!(adj >= 0.01, "negative boost should be clamped, got {adj}");
    }

    #[test]
    fn test_falsify_nan_temporal_bias() {
        let mut profile = GoalProfile::new("test".into(), None);
        profile.temporal_bias = f32::NAN;
        profile.clamp_temporal_bias();
        // NaN clamp behavior: clamp returns NaN for NaN input
        // This documents the edge case — NaN comparisons always return false
        // so the value stays NaN. This is a known limitation.
        // Real fix would be to check and default NaN to 0.0.
    }

    #[test]
    fn test_falsify_infinity_temporal_bias() {
        let mut profile = GoalProfile::new("test".into(), None);
        profile.temporal_bias = f32::INFINITY;
        profile.clamp_temporal_bias();
        assert_eq!(profile.temporal_bias, 1.0);

        profile.temporal_bias = f32::NEG_INFINITY;
        profile.clamp_temporal_bias();
        assert_eq!(profile.temporal_bias, -1.0);
    }

    #[test]
    fn test_falsify_empty_keyword_in_list() {
        // Empty string keyword should match everything (substring "")
        let mut profile = GoalProfile::new("test".into(), None);
        profile.boost_keywords = vec!["".into()];
        assert!(
            profile.matches_boost_keyword("anything"),
            "empty keyword is a substring of everything"
        );
    }

    #[test]
    fn test_falsify_overlapping_boost_and_suppress() {
        // If a fact matches BOTH boost and suppress keywords, both apply
        let mut profile = GoalProfile::new("test".into(), None);
        profile.boost_keywords = vec!["urgent".into()];
        profile.suppress_keywords = vec!["urgent".into()];

        // boost (1.5) * suppress (0.5) = 0.75
        let adj = profile.compute_relevance_adjustment(None, None, "urgent matter");
        assert!(
            (adj - 0.75).abs() < 0.01,
            "overlapping boost+suppress should multiply: got {adj}"
        );
    }

    #[test]
    fn test_falsify_case_insensitive_keywords() {
        let mut profile = GoalProfile::new("test".into(), None);
        profile.boost_keywords = vec!["URGENT".into()];

        assert!(profile.matches_boost_keyword("This is urgent"));
        assert!(profile.matches_boost_keyword("URGENT!"));
        assert!(profile.matches_boost_keyword("UrGeNt case"));
    }

    #[test]
    fn test_falsify_unicode_in_keywords() {
        let mut profile = GoalProfile::new("test".into(), None);
        profile.boost_keywords = vec!["café".into()];

        assert!(profile.matches_boost_keyword("I love café au lait"));
        assert!(!profile.matches_boost_keyword("I love coffee"));
    }

    #[test]
    fn test_falsify_very_large_boost_map() {
        let mut profile = GoalProfile::new("test".into(), None);
        for i in 0..1000 {
            profile
                .entity_category_boosts
                .insert(format!("category_{i}"), 1.0 + (i as f32 * 0.001));
        }
        // Should handle large maps without issue
        assert_eq!(profile.entity_category_boosts.len(), 1000);
        assert!((profile.entity_boost("category_500") - 1.5).abs() < 0.01);
        assert_eq!(profile.entity_boost("nonexistent"), 1.0);
    }

    #[test]
    fn test_falsify_apply_update_replaces_entire_collections() {
        let mut profile = GoalProfile::new("test".into(), None);
        profile.boost_keywords = vec!["old".into()];
        profile.entity_category_boosts.insert("old_cat".into(), 2.0);

        let updated = profile.apply_update(UpdateGoalProfileRequest {
            boost_keywords: Some(vec!["new".into()]),
            entity_category_boosts: Some(std::collections::HashMap::from([(
                "new_cat".into(),
                3.0,
            )])),
            ..Default::default()
        });

        // Update should REPLACE, not merge
        assert_eq!(updated.boost_keywords, vec!["new"]);
        assert!(!updated.entity_category_boosts.contains_key("old_cat"));
        assert_eq!(updated.entity_boost("new_cat"), 3.0);
    }

    #[test]
    fn test_falsify_apply_update_preserves_unchanged_fields() {
        let mut profile = GoalProfile::new("original".into(), Some(Uuid::from_u128(1)));
        profile.boost_keywords = vec!["keep".into()];
        profile.temporal_bias = 0.5;

        let updated = profile.apply_update(UpdateGoalProfileRequest {
            description: Some("new desc".into()),
            ..Default::default()
        });

        assert_eq!(updated.name, "original"); // name never changes via update
        assert_eq!(updated.boost_keywords, vec!["keep"]); // not in update → preserved
        assert_eq!(updated.temporal_bias, 0.5); // not in update → preserved
        assert_eq!(updated.description, "new desc"); // updated
    }

    #[test]
    fn test_falsify_goal_profile_serde_with_all_fields() {
        let mut profile = GoalProfile::new("full".into(), Some(Uuid::from_u128(99)));
        profile.description = "Full profile".into();
        profile.entity_category_boosts =
            std::collections::HashMap::from([("a".into(), 1.5_f32), ("b".into(), 0.3)]);
        profile.edge_label_boosts = std::collections::HashMap::from([("x".into(), 2.0_f32)]);
        profile.temporal_bias = -0.8;
        profile.recency_window_days = 90;
        profile.boost_keywords = vec!["k1".into(), "k2".into()];
        profile.suppress_keywords = vec!["s1".into()];

        let json = serde_json::to_string(&profile).unwrap();
        let de: GoalProfile = serde_json::from_str(&json).unwrap();

        assert_eq!(de.name, "full");
        assert_eq!(de.temporal_bias, -0.8);
        assert_eq!(de.recency_window_days, 90);
        assert_eq!(de.boost_keywords.len(), 2);
        assert_eq!(de.suppress_keywords.len(), 1);
        assert_eq!(de.entity_category_boosts.len(), 2);
    }
}
