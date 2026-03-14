//! Knowledge graph entities (nodes).
//!
//! An [`Entity`] represents a named concept extracted from episodes — people,
//! organizations, locations, concepts, or custom types. Entities track aliases,
//! mention counts, first/last seen timestamps, and data classification.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::classification::Classification;

/// The type of an entity in the knowledge graph.
///
/// Mnemo ships with common types and supports custom types
/// defined in configuration for domain-specific use cases.
///
/// Serializes as a plain string (e.g., `"person"`, `"organization"`, `"medication"`).
/// Deserializes flexibly: known types map to their variants, unknown strings become `Custom`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, utoipa::ToSchema)]
#[schema(as = String, example = "person")]
pub enum EntityType {
    Person,
    Organization,
    Product,
    Location,
    Event,
    Concept,
    /// Domain-specific custom entity type (e.g., "medication", "stock_ticker").
    Custom(String),
}

impl EntityType {
    /// Parse from a string, falling back to Custom for unknown types.
    pub fn from_str_flexible(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "person" => Self::Person,
            "organization" | "org" | "company" => Self::Organization,
            "product" => Self::Product,
            "location" | "place" => Self::Location,
            "event" => Self::Event,
            "concept" | "idea" | "topic" => Self::Concept,
            other => Self::Custom(other.to_string()),
        }
    }

    /// Canonical string representation.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Person => "person",
            Self::Organization => "organization",
            Self::Product => "product",
            Self::Location => "location",
            Self::Event => "event",
            Self::Concept => "concept",
            Self::Custom(s) => s.as_str(),
        }
    }
}

impl Serialize for EntityType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EntityType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::from_str_flexible(&s))
    }
}

/// An entity is a node in the temporal knowledge graph.
///
/// Entities represent people, products, organizations, concepts, etc.
/// They are automatically extracted from episodes and deduplicated
/// across the user's entire conversation history.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Entity {
    pub id: Uuid,
    pub user_id: Uuid,

    /// The canonical name of this entity (e.g., "Kendra", "Nike", "Boston").
    pub name: String,

    /// The type/category of this entity.
    pub entity_type: EntityType,

    /// An auto-generated summary of everything known about this entity.
    /// Updated incrementally as new information arrives.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,

    /// Alternative names / aliases for entity deduplication.
    /// E.g., ["Kenneth", "Ken"] for entity "Kendra".
    #[serde(default)]
    pub aliases: Vec<String>,

    /// Arbitrary metadata.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub metadata: serde_json::Value,

    /// Sensitivity classification.  Defaults to `Internal` for secure-by-default
    /// posture and backward compatibility with data created before v0.6.0.
    #[serde(default)]
    pub classification: Classification,

    /// How many times this entity has been mentioned across all episodes.
    /// Used for episode-mention reranking.
    /// Note: The entity→episode mapping is stored separately in the storage layer
    /// (e.g., a Redis sorted set) to avoid unbounded growth on the Entity struct.
    pub mention_count: u64,

    /// The community/cluster this entity belongs to (if community detection is enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub community_id: Option<Uuid>,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Represents an entity extracted by the LLM/extraction pipeline
/// before it's been deduplicated and merged into the graph.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ExtractedEntity {
    /// The name as extracted from the text.
    pub name: String,
    pub entity_type: EntityType,
    /// Optional summary/description from the extraction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// LLM-suggested classification.  Defaults to `Internal` if not provided.
    #[serde(default)]
    pub classification: Classification,
}

impl Entity {
    /// Create a new entity from an extraction result.
    pub fn from_extraction(extracted: &ExtractedEntity, user_id: Uuid, _episode_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::now_v7(),
            user_id,
            name: extracted.name.clone(),
            entity_type: extracted.entity_type.clone(),
            summary: extracted.summary.clone(),
            aliases: Vec::new(),
            metadata: serde_json::json!({}),
            classification: extracted.classification,
            mention_count: 1,
            community_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Record that this entity was mentioned in another episode.
    /// The entity→episode mapping is tracked separately in the storage layer.
    pub fn record_mention(&mut self) {
        self.mention_count += 1;
        self.updated_at = Utc::now();
    }

    /// Add an alias for this entity (for deduplication).
    pub fn add_alias(&mut self, alias: String) {
        let normalized = alias.trim().to_lowercase();
        let existing: Vec<String> = self.aliases.iter().map(|a| a.to_lowercase()).collect();
        if !existing.contains(&normalized) && normalized != self.name.to_lowercase() {
            self.aliases.push(alias);
            self.updated_at = Utc::now();
        }
    }

    /// Check if a given name matches this entity (name or any alias).
    pub fn matches_name(&self, candidate: &str) -> bool {
        let normalized = candidate.trim().to_lowercase();
        if self.name.to_lowercase() == normalized {
            return true;
        }
        self.aliases.iter().any(|a| a.to_lowercase() == normalized)
    }

    /// Update the summary with new information.
    pub fn update_summary(&mut self, new_summary: String) {
        self.summary = Some(new_summary);
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_extraction() -> ExtractedEntity {
        ExtractedEntity {
            name: "Kendra".to_string(),
            entity_type: EntityType::Person,
            summary: Some("A customer who likes athletic shoes".to_string()),
            classification: Classification::default(),
        }
    }

    #[test]
    fn test_entity_from_extraction() {
        let user_id = Uuid::now_v7();
        let episode_id = Uuid::now_v7();
        let entity = Entity::from_extraction(&sample_extraction(), user_id, episode_id);

        assert_eq!(entity.name, "Kendra");
        assert_eq!(entity.entity_type, EntityType::Person);
        assert_eq!(entity.mention_count, 1);
    }

    #[test]
    fn test_entity_record_mention_increments() {
        let user_id = Uuid::now_v7();
        let ep1 = Uuid::now_v7();

        let mut entity = Entity::from_extraction(&sample_extraction(), user_id, ep1);
        entity.record_mention();
        entity.record_mention();

        assert_eq!(entity.mention_count, 3); // 1 initial + 2 record_mention calls
    }

    #[test]
    fn test_entity_alias_matching() {
        let mut entity =
            Entity::from_extraction(&sample_extraction(), Uuid::now_v7(), Uuid::now_v7());
        entity.add_alias("Ken".to_string());
        entity.add_alias("Kenny".to_string());

        assert!(entity.matches_name("Kendra"));
        assert!(entity.matches_name("kendra")); // case-insensitive
        assert!(entity.matches_name("Ken"));
        assert!(entity.matches_name("Kenny"));
        assert!(!entity.matches_name("Kevin"));
    }

    #[test]
    fn test_entity_alias_no_duplicates() {
        let mut entity =
            Entity::from_extraction(&sample_extraction(), Uuid::now_v7(), Uuid::now_v7());
        entity.add_alias("Ken".to_string());
        entity.add_alias("ken".to_string()); // same, different case
        entity.add_alias("Kendra".to_string()); // same as name

        assert_eq!(entity.aliases.len(), 1); // only "Ken"
    }

    #[test]
    fn test_entity_type_flexible_parsing() {
        assert_eq!(EntityType::from_str_flexible("person"), EntityType::Person);
        assert_eq!(
            EntityType::from_str_flexible("company"),
            EntityType::Organization
        );
        assert_eq!(
            EntityType::from_str_flexible("org"),
            EntityType::Organization
        );
        assert_eq!(EntityType::from_str_flexible("place"), EntityType::Location);
        assert_eq!(
            EntityType::from_str_flexible("medication"),
            EntityType::Custom("medication".to_string())
        );
    }

    #[test]
    fn test_entity_serialization_roundtrip() {
        let entity = Entity::from_extraction(&sample_extraction(), Uuid::now_v7(), Uuid::now_v7());
        let json = serde_json::to_string(&entity).unwrap();
        let de: Entity = serde_json::from_str(&json).unwrap();
        assert_eq!(de.id, entity.id);
        assert_eq!(de.name, entity.name);
        assert_eq!(de.entity_type, entity.entity_type);
    }
}
