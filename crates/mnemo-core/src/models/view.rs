//! Policy-scoped memory views.
//!
//! A [`MemoryView`] is a named, reusable access policy that filters memory
//! during context assembly. Views constrain by classification ceiling, entity
//! type whitelist, edge label blacklist, temporal scope, fact count limit,
//! and narrative inclusion. Applied via `?view=<name>` on context endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::classification::Classification;

// ─── Temporal Scope ────────────────────────────────────────────────

/// Constrains which facts are visible based on time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TemporalScope {
    /// Only facts created/valid within the last N days.
    LastNDays { days: u32 },
    /// Only facts created/valid since this timestamp.
    Since { since: DateTime<Utc> },
    /// Only currently-valid facts (not invalidated).
    CurrentOnly,
}

// ─── Memory View ───────────────────────────────────────────────────

/// A named, configurable lens over a user's memory.
///
/// Views enforce least-privilege context assembly: a support agent using the
/// `support_safe` view only sees `Public` + `Internal` facts, while an admin
/// using `internal_full` sees everything.
///
/// Views are stored in Redis and resolved by name during context requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryView {
    pub id: Uuid,

    /// Unique name for this view (e.g., "support_safe", "sales", "internal_full").
    pub name: String,

    /// Human-readable description.
    pub description: String,

    /// Maximum classification level this view can expose.
    /// Facts/entities with classification above this ceiling are excluded.
    pub max_classification: Classification,

    /// Whitelist of entity types to include.  `None` = all types allowed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_entity_types: Option<Vec<String>>,

    /// Blacklist of edge labels to exclude.  `None` = no labels blocked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_edge_labels: Option<Vec<String>>,

    /// Maximum number of facts in the context output.  `None` = no cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_facts: Option<u32>,

    /// Whether to include the narrative summary in context output.
    #[serde(default = "default_true")]
    pub include_narrative: bool,

    /// Temporal restriction on which facts are visible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_scope: Option<TemporalScope>,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn default_true() -> bool {
    true
}

/// Flattened constraints applied during retrieval. Built from a MemoryView
/// and optionally narrowed by the caller's own classification ceiling.
#[derive(Debug, Clone)]
pub struct ViewConstraints {
    pub max_classification: Classification,
    pub allowed_entity_types: Option<Vec<String>>,
    pub blocked_edge_labels: Option<Vec<String>>,
    pub max_facts: Option<u32>,
    pub include_narrative: bool,
    pub temporal_scope: Option<TemporalScope>,
    /// Name of the view that produced these constraints (for audit).
    pub view_name: Option<String>,
}

impl ViewConstraints {
    /// Build constraints from a MemoryView, capped by the caller's classification ceiling.
    pub fn from_view(view: &MemoryView, caller_max: Classification) -> Self {
        // Take the more restrictive of the view's ceiling and the caller's ceiling.
        let effective_max = if view.max_classification <= caller_max {
            view.max_classification
        } else {
            caller_max
        };
        Self {
            max_classification: effective_max,
            allowed_entity_types: view.allowed_entity_types.clone(),
            blocked_edge_labels: view.blocked_edge_labels.clone(),
            max_facts: view.max_facts,
            include_narrative: view.include_narrative,
            temporal_scope: view.temporal_scope.clone(),
            view_name: Some(view.name.clone()),
        }
    }

    /// Default constraints when no view is specified.
    /// Uses the caller's own classification ceiling with no other restrictions.
    pub fn default_for_caller(caller_max: Classification) -> Self {
        Self {
            max_classification: caller_max,
            allowed_entity_types: None,
            blocked_edge_labels: None,
            max_facts: None,
            include_narrative: true,
            temporal_scope: None,
            view_name: None,
        }
    }

    /// Should this entity be included based on classification and type constraints?
    pub fn allows_entity(&self, classification: Classification, entity_type: &str) -> bool {
        if classification > self.max_classification {
            return false;
        }
        if let Some(ref allowed) = self.allowed_entity_types {
            if !allowed.iter().any(|t| t.eq_ignore_ascii_case(entity_type)) {
                return false;
            }
        }
        true
    }

    /// Should this edge/fact be included based on classification and label constraints?
    pub fn allows_edge(&self, classification: Classification, label: &str) -> bool {
        if classification > self.max_classification {
            return false;
        }
        if let Some(ref blocked) = self.blocked_edge_labels {
            if blocked.iter().any(|l| l.eq_ignore_ascii_case(label)) {
                return false;
            }
        }
        true
    }

    /// Should a fact at this timestamp be included based on temporal constraints?
    pub fn allows_time(&self, valid_at: DateTime<Utc>) -> bool {
        match &self.temporal_scope {
            None => true,
            Some(TemporalScope::CurrentOnly) => true, // checked via edge validity elsewhere
            Some(TemporalScope::Since { since }) => valid_at >= *since,
            Some(TemporalScope::LastNDays { days }) => {
                let cutoff = Utc::now() - chrono::Duration::days(*days as i64);
                valid_at >= cutoff
            }
        }
    }
}

/// Request body for creating or updating a MemoryView.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateViewRequest {
    pub name: String,
    pub description: String,
    pub max_classification: Classification,
    #[serde(default)]
    pub allowed_entity_types: Option<Vec<String>>,
    #[serde(default)]
    pub blocked_edge_labels: Option<Vec<String>>,
    #[serde(default)]
    pub max_facts: Option<u32>,
    #[serde(default = "default_true")]
    pub include_narrative: bool,
    #[serde(default)]
    pub temporal_scope: Option<TemporalScope>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_constraints_allows_entity_classification() {
        let vc = ViewConstraints::default_for_caller(Classification::Internal);
        assert!(vc.allows_entity(Classification::Public, "person"));
        assert!(vc.allows_entity(Classification::Internal, "person"));
        assert!(!vc.allows_entity(Classification::Confidential, "person"));
        assert!(!vc.allows_entity(Classification::Restricted, "person"));
    }

    #[test]
    fn view_constraints_allows_entity_type_whitelist() {
        let vc = ViewConstraints {
            max_classification: Classification::Restricted,
            allowed_entity_types: Some(vec!["person".into(), "product".into()]),
            blocked_edge_labels: None,
            max_facts: None,
            include_narrative: true,
            temporal_scope: None,
            view_name: None,
        };
        assert!(vc.allows_entity(Classification::Internal, "person"));
        assert!(vc.allows_entity(Classification::Internal, "Product")); // case-insensitive
        assert!(!vc.allows_entity(Classification::Internal, "organization"));
    }

    #[test]
    fn view_constraints_blocks_edge_labels() {
        let vc = ViewConstraints {
            max_classification: Classification::Restricted,
            allowed_entity_types: None,
            blocked_edge_labels: Some(vec!["salary".into(), "ssn".into()]),
            max_facts: None,
            include_narrative: true,
            temporal_scope: None,
            view_name: None,
        };
        assert!(vc.allows_edge(Classification::Internal, "prefers"));
        assert!(!vc.allows_edge(Classification::Internal, "salary"));
        assert!(!vc.allows_edge(Classification::Internal, "SSN")); // case-insensitive
    }

    #[test]
    fn view_constraints_edge_classification_ceiling() {
        let vc = ViewConstraints::default_for_caller(Classification::Internal);
        assert!(vc.allows_edge(Classification::Public, "works_at"));
        assert!(vc.allows_edge(Classification::Internal, "works_at"));
        assert!(!vc.allows_edge(Classification::Confidential, "works_at"));
    }

    #[test]
    fn view_constraints_temporal_last_n_days() {
        let vc = ViewConstraints {
            max_classification: Classification::Restricted,
            allowed_entity_types: None,
            blocked_edge_labels: None,
            max_facts: None,
            include_narrative: true,
            temporal_scope: Some(TemporalScope::LastNDays { days: 30 }),
            view_name: None,
        };
        let recent = Utc::now() - chrono::Duration::days(10);
        let old = Utc::now() - chrono::Duration::days(60);
        assert!(vc.allows_time(recent));
        assert!(!vc.allows_time(old));
    }

    #[test]
    fn view_constraints_from_view_caps_classification() {
        let view = MemoryView {
            id: Uuid::from_u128(1),
            name: "wide_view".into(),
            description: "allows confidential".into(),
            max_classification: Classification::Confidential,
            allowed_entity_types: None,
            blocked_edge_labels: None,
            max_facts: None,
            include_narrative: true,
            temporal_scope: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        // Caller can only see up to Internal — should cap to Internal
        let vc = ViewConstraints::from_view(&view, Classification::Internal);
        assert_eq!(vc.max_classification, Classification::Internal);

        // Caller can see Restricted — should use view's Confidential
        let vc = ViewConstraints::from_view(&view, Classification::Restricted);
        assert_eq!(vc.max_classification, Classification::Confidential);
    }

    #[test]
    fn temporal_scope_serde_roundtrip() {
        let scopes = vec![
            TemporalScope::LastNDays { days: 30 },
            TemporalScope::Since { since: Utc::now() },
            TemporalScope::CurrentOnly,
        ];
        for scope in scopes {
            let json = serde_json::to_string(&scope).unwrap();
            let parsed: TemporalScope = serde_json::from_str(&json).unwrap();
            assert_eq!(scope, parsed);
        }
    }

    #[test]
    fn memory_view_serde_roundtrip() {
        let view = MemoryView {
            id: Uuid::from_u128(42),
            name: "test_view".into(),
            description: "A test view".into(),
            max_classification: Classification::Internal,
            allowed_entity_types: Some(vec!["person".into()]),
            blocked_edge_labels: Some(vec!["salary".into()]),
            max_facts: Some(50),
            include_narrative: false,
            temporal_scope: Some(TemporalScope::LastNDays { days: 7 }),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_value(&view).unwrap();
        let parsed: MemoryView = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.name, "test_view");
        assert_eq!(parsed.max_classification, Classification::Internal);
        assert_eq!(parsed.max_facts, Some(50));
        assert!(!parsed.include_narrative);
    }

    #[test]
    fn create_view_request_serde() {
        let json = serde_json::json!({
            "name": "support_safe",
            "description": "Safe for customer-facing agents",
            "max_classification": "internal",
            "blocked_edge_labels": ["salary", "ssn"],
            "max_facts": 100,
        });
        let req: CreateViewRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.name, "support_safe");
        assert_eq!(req.max_classification, Classification::Internal);
        assert!(req.include_narrative); // default true
        assert!(req.allowed_entity_types.is_none());
        assert_eq!(req.blocked_edge_labels.as_ref().unwrap().len(), 2);
    }
}
