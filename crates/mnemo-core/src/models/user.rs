//! User identity and metadata.
//!
//! A [`User`] is the top-level tenant in Mnemo. All sessions, episodes,
//! entities, edges, narratives, goals, and memory regions are scoped to a
//! user. Users are identified by both an internal UUID and an external ID
//! for integration with upstream systems.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A user represents an end-user of an AI agent application.
///
/// Users own sessions, episodes, and their associated knowledge graph.
/// All graph data is isolated per-user (multi-tenant by default).
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct User {
    /// Unique identifier. UUIDv7 for time-ordered sorting.
    pub id: Uuid,

    /// External identifier from the host application.
    /// Allows mapping between Mnemo users and your app's user IDs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,

    /// User's display name. Used for entity resolution in graph construction.
    pub name: String,

    /// Optional email. Aids entity resolution when users are mentioned in conversations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,

    /// Arbitrary key-value metadata.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub metadata: serde_json::Value,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request to create a new user.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateUserRequest {
    /// Optional: provide your own ID. If omitted, a UUIDv7 is generated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Uuid>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,

    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,

    #[serde(default)]
    #[schema(value_type = Object)]
    pub metadata: serde_json::Value,
}

/// Request to update an existing user. All fields optional — only provided fields are updated.
#[derive(Debug, Clone, Serialize, Deserialize, Default, utoipa::ToSchema)]
pub struct UpdateUserRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub metadata: Option<serde_json::Value>,
}

impl User {
    /// Create a new User from a request, generating defaults for omitted fields.
    pub fn from_request(req: CreateUserRequest) -> Self {
        let now = Utc::now();
        Self {
            id: req.id.unwrap_or_else(Uuid::now_v7),
            external_id: req.external_id,
            name: req.name,
            email: req.email,
            metadata: req.metadata,
            created_at: now,
            updated_at: now,
        }
    }

    /// Apply a partial update, returning the modified user.
    pub fn apply_update(mut self, update: UpdateUserRequest) -> Self {
        if let Some(name) = update.name {
            self.name = name;
        }
        if let Some(email) = update.email {
            self.email = Some(email);
        }
        if let Some(external_id) = update.external_id {
            self.external_id = Some(external_id);
        }
        if let Some(metadata) = update.metadata {
            self.metadata = metadata;
        }
        self.updated_at = Utc::now();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_from_request_generates_id() {
        let req = CreateUserRequest {
            id: None,
            external_id: None,
            name: "Kendra".to_string(),
            email: Some("kendra@example.com".to_string()),
            metadata: serde_json::json!({}),
        };
        let user = User::from_request(req);
        assert_eq!(user.name, "Kendra");
        assert!(user.email.is_some());
        // UUIDv7 version nibble
        assert_eq!(user.id.get_version_num(), 7);
    }

    #[test]
    fn test_user_from_request_preserves_provided_id() {
        let custom_id = Uuid::now_v7();
        let req = CreateUserRequest {
            id: Some(custom_id),
            external_id: Some("ext_123".to_string()),
            name: "Robbie".to_string(),
            email: None,
            metadata: serde_json::json!({"tier": "premium"}),
        };
        let user = User::from_request(req);
        assert_eq!(user.id, custom_id);
        assert_eq!(user.external_id.as_deref(), Some("ext_123"));
    }

    #[test]
    fn test_apply_update_partial() {
        let user = User::from_request(CreateUserRequest {
            id: None,
            external_id: None,
            name: "Kendra".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        });

        let original_id = user.id;
        let updated = user.apply_update(UpdateUserRequest {
            name: Some("Kendra Smith".to_string()),
            email: None,
            external_id: None,
            metadata: None,
        });

        assert_eq!(updated.id, original_id); // ID never changes
        assert_eq!(updated.name, "Kendra Smith");
        assert!(updated.email.is_none()); // Wasn't set, stays None
    }

    #[test]
    fn test_user_serialization_roundtrip() {
        let user = User::from_request(CreateUserRequest {
            id: None,
            external_id: None,
            name: "Test".to_string(),
            email: None,
            metadata: serde_json::json!({"key": "value"}),
        });
        let json = serde_json::to_string(&user).unwrap();
        let deserialized: User = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, user.name);
        assert_eq!(deserialized.id, user.id);
    }
}
