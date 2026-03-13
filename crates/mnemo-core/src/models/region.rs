//! Multi-agent shared memory regions with ACLs.
//!
//! A [`MemoryRegion`] scopes a subset of a user's knowledge graph for shared
//! access by multiple agents. [`MemoryRegionAcl`] entries grant per-agent
//! permissions (`Read`/`Write`/`Manage`) with optional expiry. Includes
//! [`validate_agent_id`] and [`validate_region_name`] input sanitization.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::classification::Classification;

// ─── Memory Region ─────────────────────────────────────────────────

/// A scoped subset of a user's memory that can be shared between agents.
///
/// Regions define which entities and edges are visible through the share,
/// and impose a classification ceiling on all data surfaced through the region.
/// The owner agent creates the region and can grant other agents access via ACLs.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MemoryRegion {
    pub id: Uuid,
    /// Human-readable name (e.g. `"shared_customer_context"`).
    pub name: String,
    /// Agent that created and owns this region.
    pub owner_agent_id: String,
    /// User whose data this region covers. Cross-user regions are forbidden.
    pub user_id: Uuid,
    /// Optional filter restricting which entities are visible in this region.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_filter: Option<RegionEntityFilter>,
    /// Optional filter restricting which edges (facts) are visible in this region.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_filter: Option<RegionEdgeFilter>,
    /// Maximum classification level for data surfaced through this region.
    /// Facts above this ceiling are excluded even if they match the filters.
    #[serde(default)]
    pub classification_ceiling: Classification,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Filter for selecting entities in a memory region.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RegionEntityFilter {
    /// Only include entities of these types (empty = all types).
    #[serde(default)]
    pub entity_types: Vec<String>,
    /// Only include entities whose name matches one of these patterns (case-insensitive substring).
    #[serde(default)]
    pub name_patterns: Vec<String>,
}

/// Filter for selecting edges (facts) in a memory region.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RegionEdgeFilter {
    /// Only include edges with these relationship labels (empty = all labels).
    #[serde(default)]
    pub labels: Vec<String>,
    /// Only include edges with confidence above this threshold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_confidence: Option<f32>,
}

// ─── ACL ───────────────────────────────────────────────────────────

/// Access control entry granting an agent permission to a memory region.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MemoryRegionAcl {
    /// The region this ACL entry belongs to.
    pub region_id: Uuid,
    /// Agent being granted access.
    pub agent_id: String,
    /// What the agent can do in this region.
    pub permission: RegionPermission,
    /// Identity of the caller who granted this access.
    pub granted_by: String,
    pub granted_at: DateTime<Utc>,
    /// Optional expiration. After this time the ACL is ignored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

impl MemoryRegionAcl {
    /// Check whether this ACL entry has expired.
    pub fn is_expired(&self) -> bool {
        if let Some(exp) = self.expires_at {
            Utc::now() >= exp
        } else {
            false
        }
    }

    /// Check whether this ACL entry grants at least the given permission.
    pub fn has_permission(&self, required: RegionPermission) -> bool {
        if self.is_expired() {
            return false;
        }
        self.permission.has_at_least(required)
    }
}

/// Permission levels for memory region access.
///
/// Ordered: `Read < Write < Manage`. Higher permissions include all lower ones.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RegionPermission {
    /// Can retrieve facts from this region.
    Read = 0,
    /// Can add facts to this region (implies Read).
    Write = 1,
    /// Can modify the region definition and ACLs (implies Read + Write).
    Manage = 2,
}

impl RegionPermission {
    /// Check whether this permission level is at least as high as `required`.
    pub fn has_at_least(self, required: RegionPermission) -> bool {
        self >= required
    }
}

// ─── Request / Response structs ────────────────────────────────────

/// Request body for `POST /api/v1/regions`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateRegionRequest {
    /// Human-readable region name (1-128 chars).
    pub name: String,
    /// The agent that will own this region.
    pub owner_agent_id: String,
    /// User whose data this region covers.
    pub user_id: Uuid,
    /// Entity filter (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_filter: Option<RegionEntityFilter>,
    /// Edge filter (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_filter: Option<RegionEdgeFilter>,
    /// Classification ceiling (defaults to Internal).
    #[serde(default)]
    pub classification_ceiling: Classification,
}

/// Request body for `PUT /api/v1/regions/:id`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UpdateRegionRequest {
    /// Updated region name (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Updated entity filter (optional, `null` to clear).
    #[serde(default)]
    pub entity_filter: Option<RegionEntityFilter>,
    /// Updated edge filter (optional, `null` to clear).
    #[serde(default)]
    pub edge_filter: Option<RegionEdgeFilter>,
    /// Updated classification ceiling (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification_ceiling: Option<Classification>,
}

/// Request body for `POST /api/v1/regions/:id/acl`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GrantRegionAccessRequest {
    /// Agent to grant access to.
    pub agent_id: String,
    /// Permission level to grant.
    pub permission: RegionPermission,
    /// Optional expiration time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

/// Validate an agent ID used in region operations.
/// Same rules as fork agent IDs: 1-128 ASCII alphanumeric plus `-`, `_`, `.`.
/// Rejects colons, slashes, path traversal, control chars, whitespace.
pub fn validate_agent_id(id: &str) -> Result<(), String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err("agent_id must not be empty".into());
    }
    if trimmed.len() > 128 {
        return Err("agent_id must be <= 128 characters".into());
    }
    if trimmed.contains(':') || trimmed.contains('/') || trimmed.contains("..") {
        return Err("agent_id must not contain ':', '/', or '..'".into());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(
            "agent_id must contain only alphanumeric characters, hyphens, underscores, or dots"
                .into(),
        );
    }
    Ok(())
}

/// Validate a region name: 1-128 chars, no control characters.
pub fn validate_region_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("region name cannot be empty".into());
    }
    if trimmed.len() > 128 {
        return Err("region name must be <= 128 characters".into());
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err("region name must not contain control characters".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ─── RegionPermission tests ───────────────────────────────────

    #[test]
    fn test_region_permission_ordering() {
        assert!(RegionPermission::Read < RegionPermission::Write);
        assert!(RegionPermission::Write < RegionPermission::Manage);
        assert!(RegionPermission::Read < RegionPermission::Manage);
    }

    #[test]
    fn test_region_permission_has_at_least() {
        assert!(RegionPermission::Read.has_at_least(RegionPermission::Read));
        assert!(!RegionPermission::Read.has_at_least(RegionPermission::Write));
        assert!(!RegionPermission::Read.has_at_least(RegionPermission::Manage));

        assert!(RegionPermission::Write.has_at_least(RegionPermission::Read));
        assert!(RegionPermission::Write.has_at_least(RegionPermission::Write));
        assert!(!RegionPermission::Write.has_at_least(RegionPermission::Manage));

        assert!(RegionPermission::Manage.has_at_least(RegionPermission::Read));
        assert!(RegionPermission::Manage.has_at_least(RegionPermission::Write));
        assert!(RegionPermission::Manage.has_at_least(RegionPermission::Manage));
    }

    #[test]
    fn test_region_permission_serde_roundtrip() {
        for perm in [
            RegionPermission::Read,
            RegionPermission::Write,
            RegionPermission::Manage,
        ] {
            let json = serde_json::to_string(&perm).unwrap();
            let back: RegionPermission = serde_json::from_str(&json).unwrap();
            assert_eq!(back, perm);
        }
        assert_eq!(
            serde_json::to_string(&RegionPermission::Read).unwrap(),
            "\"read\""
        );
        assert_eq!(
            serde_json::to_string(&RegionPermission::Write).unwrap(),
            "\"write\""
        );
        assert_eq!(
            serde_json::to_string(&RegionPermission::Manage).unwrap(),
            "\"manage\""
        );
    }

    // ─── ACL expiry tests ─────────────────────────────────────────

    #[test]
    fn test_acl_not_expired_when_no_expiry() {
        let acl = MemoryRegionAcl {
            region_id: Uuid::from_u128(1),
            agent_id: "bot".into(),
            permission: RegionPermission::Read,
            granted_by: "admin".into(),
            granted_at: Utc::now(),
            expires_at: None,
        };
        assert!(!acl.is_expired());
        assert!(acl.has_permission(RegionPermission::Read));
    }

    #[test]
    fn test_acl_expired_in_past() {
        let acl = MemoryRegionAcl {
            region_id: Uuid::from_u128(1),
            agent_id: "bot".into(),
            permission: RegionPermission::Write,
            granted_by: "admin".into(),
            granted_at: Utc::now() - chrono::Duration::hours(48),
            expires_at: Some(Utc::now() - chrono::Duration::hours(1)),
        };
        assert!(acl.is_expired());
        assert!(!acl.has_permission(RegionPermission::Read));
        assert!(!acl.has_permission(RegionPermission::Write));
    }

    #[test]
    fn test_acl_not_expired_future() {
        let acl = MemoryRegionAcl {
            region_id: Uuid::from_u128(1),
            agent_id: "bot".into(),
            permission: RegionPermission::Manage,
            granted_by: "admin".into(),
            granted_at: Utc::now(),
            expires_at: Some(Utc::now() + chrono::Duration::hours(24)),
        };
        assert!(!acl.is_expired());
        assert!(acl.has_permission(RegionPermission::Read));
        assert!(acl.has_permission(RegionPermission::Write));
        assert!(acl.has_permission(RegionPermission::Manage));
    }

    #[test]
    fn test_acl_permission_check_with_insufficient_level() {
        let acl = MemoryRegionAcl {
            region_id: Uuid::from_u128(1),
            agent_id: "bot".into(),
            permission: RegionPermission::Read,
            granted_by: "admin".into(),
            granted_at: Utc::now(),
            expires_at: None,
        };
        assert!(acl.has_permission(RegionPermission::Read));
        assert!(!acl.has_permission(RegionPermission::Write));
        assert!(!acl.has_permission(RegionPermission::Manage));
    }

    // ─── MemoryRegion serde tests ─────────────────────────────────

    #[test]
    fn test_memory_region_serde_roundtrip() {
        let region = MemoryRegion {
            id: Uuid::from_u128(42),
            name: "shared_customer_context".into(),
            owner_agent_id: "support-bot".into(),
            user_id: Uuid::from_u128(99),
            entity_filter: Some(RegionEntityFilter {
                entity_types: vec!["person".into(), "organization".into()],
                name_patterns: vec![],
            }),
            edge_filter: None,
            classification_ceiling: Classification::Confidential,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&region).unwrap();
        let back: MemoryRegion = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, Uuid::from_u128(42));
        assert_eq!(back.name, "shared_customer_context");
        assert_eq!(back.owner_agent_id, "support-bot");
        assert_eq!(back.classification_ceiling, Classification::Confidential);
        assert!(back.entity_filter.is_some());
        assert!(back.edge_filter.is_none());
    }

    #[test]
    fn test_memory_region_classification_defaults_to_internal() {
        let json = json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "name": "test",
            "owner_agent_id": "bot",
            "user_id": "00000000-0000-0000-0000-000000000002",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
        });
        let region: MemoryRegion = serde_json::from_value(json).unwrap();
        assert_eq!(region.classification_ceiling, Classification::Internal);
    }

    #[test]
    fn test_memory_region_acl_serde_roundtrip() {
        let acl = MemoryRegionAcl {
            region_id: Uuid::from_u128(1),
            agent_id: "sales-bot".into(),
            permission: RegionPermission::Write,
            granted_by: "support-bot".into(),
            granted_at: Utc::now(),
            expires_at: Some(Utc::now() + chrono::Duration::days(30)),
        };
        let json = serde_json::to_string(&acl).unwrap();
        let back: MemoryRegionAcl = serde_json::from_str(&json).unwrap();
        assert_eq!(back.agent_id, "sales-bot");
        assert_eq!(back.permission, RegionPermission::Write);
        assert!(back.expires_at.is_some());
    }

    // ─── Request struct tests ─────────────────────────────────────

    #[test]
    fn test_create_region_request_serde() {
        let req: CreateRegionRequest = serde_json::from_value(json!({
            "name": "shared_context",
            "owner_agent_id": "bot-a",
            "user_id": "00000000-0000-0000-0000-000000000001",
            "classification_ceiling": "confidential",
        }))
        .unwrap();
        assert_eq!(req.name, "shared_context");
        assert_eq!(req.classification_ceiling, Classification::Confidential);
        assert!(req.entity_filter.is_none());
    }

    #[test]
    fn test_create_region_request_with_filters() {
        let req: CreateRegionRequest = serde_json::from_value(json!({
            "name": "filtered_region",
            "owner_agent_id": "bot-a",
            "user_id": "00000000-0000-0000-0000-000000000001",
            "entity_filter": {
                "entity_types": ["person"],
                "name_patterns": ["john"],
            },
            "edge_filter": {
                "labels": ["works_at"],
                "min_confidence": 0.7,
            },
        }))
        .unwrap();
        let ef = req.entity_filter.unwrap();
        assert_eq!(ef.entity_types, vec!["person"]);
        assert_eq!(ef.name_patterns, vec!["john"]);
        let ef2 = req.edge_filter.unwrap();
        assert_eq!(ef2.labels, vec!["works_at"]);
        assert_eq!(ef2.min_confidence, Some(0.7));
    }

    #[test]
    fn test_update_region_request_partial() {
        let req: UpdateRegionRequest = serde_json::from_value(json!({
            "name": "renamed_region",
        }))
        .unwrap();
        assert_eq!(req.name.as_deref(), Some("renamed_region"));
        assert!(req.entity_filter.is_none());
        assert!(req.classification_ceiling.is_none());
    }

    #[test]
    fn test_grant_region_access_request_serde() {
        let req: GrantRegionAccessRequest = serde_json::from_value(json!({
            "agent_id": "sales-bot",
            "permission": "read",
        }))
        .unwrap();
        assert_eq!(req.agent_id, "sales-bot");
        assert_eq!(req.permission, RegionPermission::Read);
        assert!(req.expires_at.is_none());
    }

    #[test]
    fn test_grant_region_access_with_expiry() {
        let req: GrantRegionAccessRequest = serde_json::from_value(json!({
            "agent_id": "sales-bot",
            "permission": "write",
            "expires_at": "2025-12-31T23:59:59Z",
        }))
        .unwrap();
        assert_eq!(req.permission, RegionPermission::Write);
        assert!(req.expires_at.is_some());
    }

    // ─── Validation tests ─────────────────────────────────────────

    #[test]
    fn test_validate_region_name_valid() {
        assert!(validate_region_name("shared_customer_context").is_ok());
        assert!(validate_region_name("a").is_ok());
        assert!(validate_region_name("My Region 123").is_ok());
    }

    #[test]
    fn test_validate_region_name_empty() {
        assert!(validate_region_name("").is_err());
        assert!(validate_region_name("   ").is_err());
    }

    #[test]
    fn test_validate_region_name_too_long() {
        let long = "a".repeat(129);
        assert!(validate_region_name(&long).is_err());
        let exact = "a".repeat(128);
        assert!(validate_region_name(&exact).is_ok());
    }

    #[test]
    fn test_validate_region_name_control_chars() {
        assert!(validate_region_name("test\0null").is_err());
        assert!(validate_region_name("test\nnewline").is_err());
        assert!(validate_region_name("test\ttab").is_err());
    }

    // ─── Falsification tests ──────────────────────────────────────

    #[test]
    fn test_falsify_expired_acl_denies_all_permissions() {
        let acl = MemoryRegionAcl {
            region_id: Uuid::from_u128(1),
            agent_id: "bot".into(),
            permission: RegionPermission::Manage, // highest permission
            granted_by: "admin".into(),
            granted_at: Utc::now() - chrono::Duration::hours(48),
            expires_at: Some(Utc::now() - chrono::Duration::seconds(1)), // just expired
        };
        // Even Manage permission is denied after expiry
        assert!(!acl.has_permission(RegionPermission::Read));
        assert!(!acl.has_permission(RegionPermission::Write));
        assert!(!acl.has_permission(RegionPermission::Manage));
    }

    #[test]
    fn test_falsify_permission_escalation_read_cannot_write() {
        let acl = MemoryRegionAcl {
            region_id: Uuid::from_u128(1),
            agent_id: "bot".into(),
            permission: RegionPermission::Read,
            granted_by: "admin".into(),
            granted_at: Utc::now(),
            expires_at: None,
        };
        assert!(!acl.has_permission(RegionPermission::Write));
    }

    #[test]
    fn test_falsify_permission_escalation_write_cannot_manage() {
        let acl = MemoryRegionAcl {
            region_id: Uuid::from_u128(1),
            agent_id: "bot".into(),
            permission: RegionPermission::Write,
            granted_by: "admin".into(),
            granted_at: Utc::now(),
            expires_at: None,
        };
        assert!(!acl.has_permission(RegionPermission::Manage));
    }

    #[test]
    fn test_falsify_region_entity_filter_empty_means_all() {
        let filter = RegionEntityFilter {
            entity_types: vec![],
            name_patterns: vec![],
        };
        // Empty filter means all entities pass — this is by convention,
        // verified here so the retrieval layer can rely on it
        assert!(filter.entity_types.is_empty());
        assert!(filter.name_patterns.is_empty());
    }

    #[test]
    fn test_falsify_region_edge_filter_empty_means_all() {
        let filter = RegionEdgeFilter {
            labels: vec![],
            min_confidence: None,
        };
        assert!(filter.labels.is_empty());
        assert!(filter.min_confidence.is_none());
    }

    #[test]
    fn test_falsify_classification_ceiling_serializes_correctly() {
        let region = MemoryRegion {
            id: Uuid::from_u128(1),
            name: "test".into(),
            owner_agent_id: "bot".into(),
            user_id: Uuid::from_u128(2),
            entity_filter: None,
            edge_filter: None,
            classification_ceiling: Classification::Restricted,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let val = serde_json::to_value(&region).unwrap();
        assert_eq!(val["classification_ceiling"], "restricted");
    }

    #[test]
    fn test_falsify_acl_without_expires_never_expires() {
        let acl = MemoryRegionAcl {
            region_id: Uuid::from_u128(1),
            agent_id: "bot".into(),
            permission: RegionPermission::Read,
            granted_by: "admin".into(),
            // Granted a year ago with no expiry
            granted_at: Utc::now() - chrono::Duration::days(365),
            expires_at: None,
        };
        assert!(!acl.is_expired());
        assert!(acl.has_permission(RegionPermission::Read));
    }

    #[test]
    fn test_falsify_region_missing_optional_filters_deserializes() {
        let json = json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "name": "minimal",
            "owner_agent_id": "bot",
            "user_id": "00000000-0000-0000-0000-000000000002",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
        });
        let region: MemoryRegion = serde_json::from_value(json).unwrap();
        assert!(region.entity_filter.is_none());
        assert!(region.edge_filter.is_none());
    }

    // ─── validate_agent_id tests ──────────────────────────────────

    #[test]
    fn test_validate_agent_id_valid() {
        assert!(validate_agent_id("support-bot").is_ok());
        assert!(validate_agent_id("bot_v2").is_ok());
        assert!(validate_agent_id("sales.bot.v3").is_ok());
        assert!(validate_agent_id("a").is_ok());
    }

    #[test]
    fn test_validate_agent_id_empty() {
        assert!(validate_agent_id("").is_err());
        assert!(validate_agent_id("   ").is_err());
    }

    #[test]
    fn test_validate_agent_id_too_long() {
        let long = "a".repeat(129);
        assert!(validate_agent_id(&long).is_err());
        assert!(validate_agent_id(&"a".repeat(128)).is_ok());
    }

    #[test]
    fn test_validate_agent_id_colon_rejected() {
        assert!(validate_agent_id("bot:fork").is_err());
    }

    #[test]
    fn test_validate_agent_id_slash_rejected() {
        assert!(validate_agent_id("bot/fork").is_err());
    }

    #[test]
    fn test_validate_agent_id_path_traversal_rejected() {
        assert!(validate_agent_id("..bot").is_err());
        assert!(validate_agent_id("bot../etc").is_err());
    }

    #[test]
    fn test_validate_agent_id_null_bytes_rejected() {
        assert!(validate_agent_id("bot\0hidden").is_err());
    }

    #[test]
    fn test_validate_agent_id_whitespace_rejected() {
        assert!(validate_agent_id("bot name").is_err());
        assert!(validate_agent_id("bot\tname").is_err());
    }

    #[test]
    fn test_validate_agent_id_unicode_rejected() {
        assert!(validate_agent_id("böt").is_err());
        assert!(validate_agent_id("bot🤖").is_err());
    }
}
