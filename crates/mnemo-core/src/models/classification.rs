//! Data classification labels for access control.
//!
//! Four-tier classification system: `Public` < `Internal` < `Confidential` <
//! `Restricted`. Applied to entities and edges at ingestion time, enforced
//! during retrieval via API key scope ceilings and memory view constraints.
//! Defaults to `Internal` for backward compatibility.

use serde::{Deserialize, Serialize};

// ─── Data Classification ──────────────────────────────────────────

/// Sensitivity classification for data (edges, entities, API key scopes).
///
/// Ordered: Public < Internal < Confidential < Restricted.
/// A caller with `max_classification = Internal` can see Public + Internal
/// data but not Confidential or Restricted.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum Classification {
    /// Safe for any audience — customers, external agents.
    Public = 0,
    /// Safe for internal agents and operators.
    #[default]
    Internal = 1,
    /// Restricted to authorized agents/users.  May contain PII.
    Confidential = 2,
    /// Highest sensitivity — financial, health, or protected-class data.
    Restricted = 3,
}

impl Classification {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Confidential => "confidential",
            Self::Restricted => "restricted",
        }
    }

    /// Parse from a string, defaulting to `Internal` for unrecognized values.
    pub fn from_str_flexible(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "public" => Self::Public,
            "internal" => Self::Internal,
            "confidential" => Self::Confidential,
            "restricted" => Self::Restricted,
            _ => Self::Internal,
        }
    }
}

impl std::fmt::Display for Classification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_ordering() {
        assert!(Classification::Public < Classification::Internal);
        assert!(Classification::Internal < Classification::Confidential);
        assert!(Classification::Confidential < Classification::Restricted);
    }

    #[test]
    fn classification_default_is_internal() {
        assert_eq!(Classification::default(), Classification::Internal);
    }

    #[test]
    fn classification_serde_roundtrip() {
        for c in [
            Classification::Public,
            Classification::Internal,
            Classification::Confidential,
            Classification::Restricted,
        ] {
            let json = serde_json::to_string(&c).unwrap();
            let parsed: Classification = serde_json::from_str(&json).unwrap();
            assert_eq!(c, parsed);
        }
    }

    #[test]
    fn classification_deserializes_from_snake_case() {
        let c: Classification = serde_json::from_str("\"public\"").unwrap();
        assert_eq!(c, Classification::Public);
        let c: Classification = serde_json::from_str("\"confidential\"").unwrap();
        assert_eq!(c, Classification::Confidential);
    }

    #[test]
    fn classification_display() {
        assert_eq!(Classification::Public.to_string(), "public");
        assert_eq!(Classification::Restricted.to_string(), "restricted");
    }

    #[test]
    fn classification_as_str() {
        assert_eq!(Classification::Internal.as_str(), "internal");
        assert_eq!(Classification::Confidential.as_str(), "confidential");
    }

    #[test]
    fn classification_missing_field_defaults_to_internal() {
        // Simulates backward compatibility: existing Redis data has no classification field
        #[derive(Deserialize, utoipa::ToSchema)]
        struct LegacyEdge {
            #[allow(dead_code)]
            name: String,
            #[serde(default)]
            classification: Classification,
        }
        let json = r#"{"name": "old-edge"}"#;
        let edge: LegacyEdge = serde_json::from_str(json).unwrap();
        assert_eq!(edge.classification, Classification::Internal);
    }
}
