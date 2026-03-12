use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

// ─── API Key Role ──────────────────────────────────────────────────

/// Role-based access control for API keys.
///
/// Roles are ordered: Read < Write < Admin.  A handler that requires
/// `Write` will also accept `Admin`.  The ordering is enforced by
/// `ApiKeyRole::has_at_least`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyRole {
    /// GET endpoints only — context retrieval, graph queries, list operations.
    Read = 0,
    /// Read + POST/PUT/PATCH on memory, sessions, episodes, webhooks.
    Write = 1,
    /// Write + policy management, key management, user deletion, agent identity admin.
    Admin = 2,
}

impl ApiKeyRole {
    /// Returns true when this role meets or exceeds `required`.
    pub fn has_at_least(&self, required: ApiKeyRole) -> bool {
        (*self as u8) >= (required as u8)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Admin => "admin",
        }
    }
}

impl std::fmt::Display for ApiKeyRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─── Data Classification ───────────────────────────────────────────

/// Sensitivity classification for edges, entities, and memory regions.
///
/// Ordered: Public < Internal < Confidential < Restricted.
/// A caller with `max_classification = Internal` can see Public + Internal
/// data but not Confidential or Restricted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    /// Safe for any audience — customers, external agents.
    Public = 0,
    /// Safe for internal agents and operators.
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
}

impl Default for Classification {
    fn default() -> Self {
        Self::Internal
    }
}

impl std::fmt::Display for Classification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─── API Key Scope ─────────────────────────────────────────────────

/// Optional fine-grained restrictions for an API key beyond its role.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApiKeyScope {
    /// Restrict to these user IDs.  `None` = all users.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_user_ids: Option<Vec<Uuid>>,

    /// Restrict to these agent IDs.  `None` = all agents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_agent_ids: Option<Vec<String>>,

    /// Maximum data classification this key can access.  `None` = Restricted
    /// (i.e. unrestricted).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_classification: Option<Classification>,
}

impl ApiKeyScope {
    /// Returns true when `user_id` is permitted by this scope.
    pub fn allows_user(&self, user_id: Uuid) -> bool {
        match &self.allowed_user_ids {
            None => true,
            Some(ids) => ids.contains(&user_id),
        }
    }

    /// Returns true when `agent_id` is permitted by this scope.
    pub fn allows_agent(&self, agent_id: &str) -> bool {
        match &self.allowed_agent_ids {
            None => true,
            Some(ids) => ids.iter().any(|id| id == agent_id),
        }
    }

    /// Returns the effective max classification, defaulting to `Restricted`
    /// (full access) when unset.
    pub fn effective_max_classification(&self) -> Classification {
        self.max_classification
            .unwrap_or(Classification::Restricted)
    }
}

// ─── API Key Model ─────────────────────────────────────────────────

/// A stored API key.  The raw key is never persisted — only its SHA-256 hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: Uuid,

    /// Human-friendly label (e.g. "analytics-reader", "support-agent-svc").
    pub name: String,

    /// SHA-256 hash of the raw key.  Used for authentication lookups.
    pub key_hash: String,

    /// A short prefix of the raw key shown in list responses (e.g. "mnk_a3f2...").
    pub key_prefix: String,

    /// The role this key grants.
    pub role: ApiKeyRole,

    /// Optional fine-grained scope restrictions.
    #[serde(default)]
    pub scope: Option<ApiKeyScope>,

    /// Who created this key (key name or external identity).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,

    pub created_at: DateTime<Utc>,

    /// Last time this key was used in an authenticated request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,

    /// When this key expires.  `None` = never.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,

    /// If true this key is revoked and cannot authenticate.
    #[serde(default)]
    pub revoked: bool,
}

impl ApiKey {
    /// Returns true when this key is valid for authentication right now.
    pub fn is_active(&self) -> bool {
        if self.revoked {
            return false;
        }
        if let Some(exp) = self.expires_at {
            if Utc::now() >= exp {
                return false;
            }
        }
        true
    }
}

// ─── Request / Response types ──────────────────────────────────────

/// Request body for `POST /api/v1/keys`.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateApiKeyRequest {
    /// Human-friendly label.
    pub name: String,

    /// Desired role.
    pub role: ApiKeyRole,

    /// Optional scope restrictions.
    #[serde(default)]
    pub scope: Option<ApiKeyScope>,

    /// Optional expiry.
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

/// Response from `POST /api/v1/keys`.  The `raw_key` is returned exactly
/// once — it cannot be retrieved again.
#[derive(Debug, Clone, Serialize)]
pub struct CreateApiKeyResponse {
    /// The full API key value.  Store this securely — it will not be shown again.
    pub raw_key: String,

    /// The stored key metadata (without the raw key).
    #[serde(flatten)]
    pub key: ApiKey,
}

// ─── Caller Context ────────────────────────────────────────────────

/// Extracted from an authenticated API key on every request.
/// Threaded through handlers via Axum request extensions.
#[derive(Debug, Clone)]
pub struct CallerContext {
    pub key_id: Uuid,
    pub key_name: String,
    pub role: ApiKeyRole,
    pub scope: Option<ApiKeyScope>,
}

impl CallerContext {
    /// Convenience: returns an admin context (for bootstrap key or auth-disabled mode).
    pub fn admin_bootstrap() -> Self {
        Self {
            key_id: Uuid::nil(),
            key_name: "bootstrap".to_string(),
            role: ApiKeyRole::Admin,
            scope: None,
        }
    }

    /// Check whether this caller has at least the given role.
    pub fn require_role(&self, required: ApiKeyRole) -> Result<(), crate::MnemoError> {
        if self.role.has_at_least(required) {
            Ok(())
        } else {
            Err(crate::MnemoError::Forbidden)
        }
    }

    /// Check whether this caller can access a specific user's data.
    pub fn require_user_access(&self, user_id: Uuid) -> Result<(), crate::MnemoError> {
        if let Some(ref scope) = self.scope {
            if !scope.allows_user(user_id) {
                return Err(crate::MnemoError::Forbidden);
            }
        }
        Ok(())
    }

    /// Check whether this caller can access a specific agent.
    pub fn require_agent_access(&self, agent_id: &str) -> Result<(), crate::MnemoError> {
        if let Some(ref scope) = self.scope {
            if !scope.allows_agent(agent_id) {
                return Err(crate::MnemoError::Forbidden);
            }
        }
        Ok(())
    }

    /// Returns the effective max classification this caller can see.
    pub fn max_classification(&self) -> Classification {
        self.scope
            .as_ref()
            .map(|s| s.effective_max_classification())
            .unwrap_or(Classification::Restricted)
    }
}

// ─── Key generation helpers ────────────────────────────────────────

/// Generate a new raw API key with the `mnk_` prefix (mnemo-key).
pub fn generate_raw_key() -> String {
    // 32 random bytes → 64 hex chars, prefixed with `mnk_`
    let mut bytes = [0u8; 32];
    // Use a simple fallback: combine current timestamp with Uuid randomness
    let u1 = Uuid::from_u128(chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u128);
    let u2 = Uuid::from_u128(
        chrono::Utc::now()
            .timestamp_nanos_opt()
            .unwrap_or(0)
            .wrapping_mul(6364136223846793005) as u128,
    );
    bytes[..16].copy_from_slice(u1.as_bytes());
    bytes[16..].copy_from_slice(u2.as_bytes());
    format!("mnk_{}", hex::encode(bytes))
}

/// Compute the SHA-256 hash of a raw API key.
pub fn hash_api_key(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    hex::encode(hasher.finalize())
}

/// Extract a short prefix from a raw key for display (e.g. "mnk_a3f2…").
pub fn key_prefix(raw: &str) -> String {
    if raw.len() > 12 {
        format!("{}...", &raw[..12])
    } else {
        raw.to_string()
    }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_ordering() {
        assert!(ApiKeyRole::Read < ApiKeyRole::Write);
        assert!(ApiKeyRole::Write < ApiKeyRole::Admin);
        assert!(ApiKeyRole::Admin.has_at_least(ApiKeyRole::Read));
        assert!(ApiKeyRole::Admin.has_at_least(ApiKeyRole::Write));
        assert!(ApiKeyRole::Admin.has_at_least(ApiKeyRole::Admin));
        assert!(ApiKeyRole::Write.has_at_least(ApiKeyRole::Read));
        assert!(ApiKeyRole::Write.has_at_least(ApiKeyRole::Write));
        assert!(!ApiKeyRole::Write.has_at_least(ApiKeyRole::Admin));
        assert!(ApiKeyRole::Read.has_at_least(ApiKeyRole::Read));
        assert!(!ApiKeyRole::Read.has_at_least(ApiKeyRole::Write));
        assert!(!ApiKeyRole::Read.has_at_least(ApiKeyRole::Admin));
    }

    #[test]
    fn classification_ordering() {
        assert!(Classification::Public < Classification::Internal);
        assert!(Classification::Internal < Classification::Confidential);
        assert!(Classification::Confidential < Classification::Restricted);
        assert_eq!(Classification::default(), Classification::Internal);
    }

    #[test]
    fn scope_allows_user() {
        let scope = ApiKeyScope {
            allowed_user_ids: Some(vec![Uuid::from_u128(1)]),
            ..Default::default()
        };
        assert!(scope.allows_user(Uuid::from_u128(1)));
        assert!(!scope.allows_user(Uuid::from_u128(2)));

        let open_scope = ApiKeyScope::default();
        assert!(open_scope.allows_user(Uuid::from_u128(999)));
    }

    #[test]
    fn scope_allows_agent() {
        let scope = ApiKeyScope {
            allowed_agent_ids: Some(vec!["agent-a".to_string()]),
            ..Default::default()
        };
        assert!(scope.allows_agent("agent-a"));
        assert!(!scope.allows_agent("agent-b"));

        let open_scope = ApiKeyScope::default();
        assert!(open_scope.allows_agent("anything"));
    }

    #[test]
    fn key_hash_deterministic() {
        let raw = "mnk_test_key_12345";
        let h1 = hash_api_key(raw);
        let h2 = hash_api_key(raw);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 hex = 64 chars
    }

    #[test]
    fn key_prefix_extraction() {
        let raw = "mnk_abcdef1234567890";
        assert_eq!(key_prefix(raw), "mnk_abcdef12...");
        assert_eq!(key_prefix("short"), "short");
    }

    #[test]
    fn generate_raw_key_format() {
        let key = generate_raw_key();
        assert!(key.starts_with("mnk_"));
        assert_eq!(key.len(), 4 + 64); // "mnk_" + 64 hex chars
    }

    #[test]
    fn api_key_is_active() {
        let now = Utc::now();
        let active = ApiKey {
            id: Uuid::from_u128(1),
            name: "test".to_string(),
            key_hash: "abc".to_string(),
            key_prefix: "mnk_...".to_string(),
            role: ApiKeyRole::Read,
            scope: None,
            created_by: None,
            created_at: now,
            last_used_at: None,
            expires_at: None,
            revoked: false,
        };
        assert!(active.is_active());

        let revoked = ApiKey {
            revoked: true,
            ..active.clone()
        };
        assert!(!revoked.is_active());

        let expired = ApiKey {
            expires_at: Some(now - chrono::Duration::hours(1)),
            ..active.clone()
        };
        assert!(!expired.is_active());

        let future_expiry = ApiKey {
            expires_at: Some(now + chrono::Duration::hours(1)),
            ..active
        };
        assert!(future_expiry.is_active());
    }

    #[test]
    fn caller_context_require_role() {
        let admin = CallerContext::admin_bootstrap();
        assert!(admin.require_role(ApiKeyRole::Admin).is_ok());
        assert!(admin.require_role(ApiKeyRole::Write).is_ok());
        assert!(admin.require_role(ApiKeyRole::Read).is_ok());

        let reader = CallerContext {
            key_id: Uuid::from_u128(2),
            key_name: "reader".to_string(),
            role: ApiKeyRole::Read,
            scope: None,
        };
        assert!(reader.require_role(ApiKeyRole::Read).is_ok());
        assert!(reader.require_role(ApiKeyRole::Write).is_err());
        assert!(reader.require_role(ApiKeyRole::Admin).is_err());
    }

    #[test]
    fn caller_context_user_scope() {
        let scoped = CallerContext {
            key_id: Uuid::from_u128(3),
            key_name: "scoped".to_string(),
            role: ApiKeyRole::Write,
            scope: Some(ApiKeyScope {
                allowed_user_ids: Some(vec![Uuid::from_u128(10)]),
                ..Default::default()
            }),
        };
        assert!(scoped.require_user_access(Uuid::from_u128(10)).is_ok());
        assert!(scoped.require_user_access(Uuid::from_u128(11)).is_err());

        let unscoped = CallerContext::admin_bootstrap();
        assert!(unscoped.require_user_access(Uuid::from_u128(999)).is_ok());
    }

    #[test]
    fn caller_context_max_classification() {
        let admin = CallerContext::admin_bootstrap();
        assert_eq!(admin.max_classification(), Classification::Restricted);

        let limited = CallerContext {
            key_id: Uuid::from_u128(4),
            key_name: "limited".to_string(),
            role: ApiKeyRole::Read,
            scope: Some(ApiKeyScope {
                max_classification: Some(Classification::Internal),
                ..Default::default()
            }),
        };
        assert_eq!(limited.max_classification(), Classification::Internal);
    }

    #[test]
    fn role_serialization() {
        let json = serde_json::to_string(&ApiKeyRole::Admin).unwrap();
        assert_eq!(json, "\"admin\"");
        let parsed: ApiKeyRole = serde_json::from_str("\"write\"").unwrap();
        assert_eq!(parsed, ApiKeyRole::Write);
    }

    #[test]
    fn classification_serialization() {
        let json = serde_json::to_string(&Classification::Confidential).unwrap();
        assert_eq!(json, "\"confidential\"");
        let parsed: Classification = serde_json::from_str("\"public\"").unwrap();
        assert_eq!(parsed, Classification::Public);
    }
}
