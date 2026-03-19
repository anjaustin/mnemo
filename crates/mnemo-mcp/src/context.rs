//! Agent context management for MCP sessions.
//!
//! Each MCP connection can be bound to an agent identity, enabling:
//! - Agent-scoped memory (separate from user memory)
//! - Experience recording for identity evolution
//! - Session continuity across reconnects

use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Context for the current MCP session, optionally bound to an agent.
///
/// The agent context provides:
/// - Agent identification (agent_id)
/// - Optional user scoping (user_id)
/// - Session management (session_id)
/// - Cached identity profile for efficient access
#[derive(Debug)]
pub struct AgentContext {
    /// The agent this session belongs to (if any).
    /// Set via `MNEMO_MCP_AGENT_ID` environment variable.
    pub agent_id: Option<String>,

    /// Optional user scope. If set, memory operations are filtered
    /// to this user only. Set via `MNEMO_MCP_DEFAULT_USER`.
    pub user_id: Option<String>,

    /// Current session ID (created on first memory operation).
    session_id: RwLock<Option<Uuid>>,

    /// Session name for grouping operations.
    pub session_name: String,
}

impl AgentContext {
    /// Create a new agent context from environment variables.
    ///
    /// Reads:
    /// - `MNEMO_MCP_AGENT_ID` — Agent identifier (optional)
    /// - `MNEMO_MCP_DEFAULT_USER` — Default user for memory ops
    /// - `MNEMO_MCP_SESSION` — Session name (default: "mcp-session")
    pub fn from_env() -> Self {
        Self {
            agent_id: std::env::var("MNEMO_MCP_AGENT_ID").ok(),
            user_id: std::env::var("MNEMO_MCP_DEFAULT_USER").ok(),
            session_id: RwLock::new(None),
            session_name: std::env::var("MNEMO_MCP_SESSION")
                .unwrap_or_else(|_| "mcp-session".to_string()),
        }
    }

    /// Create context with explicit values.
    pub fn new(agent_id: Option<String>, user_id: Option<String>, session_name: String) -> Self {
        Self {
            agent_id,
            user_id,
            session_id: RwLock::new(None),
            session_name,
        }
    }

    /// Check if this context is bound to an agent.
    pub fn has_agent(&self) -> bool {
        self.agent_id.is_some()
    }

    /// Get the agent ID, returning an error if not set.
    pub fn require_agent(&self) -> Result<&str, String> {
        self.agent_id.as_deref().ok_or_else(|| {
            "No agent_id configured. Set MNEMO_MCP_AGENT_ID to bind this MCP session to an agent."
                .to_string()
        })
    }

    /// Get the user ID, returning an error if not set.
    pub fn require_user(&self) -> Result<&str, String> {
        self.user_id.as_deref().ok_or_else(|| {
            "No user specified and MNEMO_MCP_DEFAULT_USER not set. \
             Provide a 'user' argument or set the environment variable."
                .to_string()
        })
    }

    /// Resolve a user: use provided value or fall back to default.
    pub fn resolve_user<'a>(&'a self, provided: Option<&'a str>) -> Result<&'a str, String> {
        provided.or(self.user_id.as_deref()).ok_or_else(|| {
            "No user specified and MNEMO_MCP_DEFAULT_USER not set. \
             Provide a 'user' argument or set the environment variable."
                .to_string()
        })
    }

    /// Get the current session ID.
    pub async fn session_id(&self) -> Option<Uuid> {
        *self.session_id.read().await
    }

    /// Set the session ID (called after session creation).
    pub async fn set_session_id(&self, id: Uuid) {
        *self.session_id.write().await = Some(id);
    }

    /// Generate a deterministic user ID from agent_id (for agent-only memory).
    ///
    /// This allows agents to have their own memory space without a real user.
    pub fn synthetic_user_id(&self) -> Option<Uuid> {
        self.agent_id
            .as_ref()
            .map(|agent_id| Uuid::new_v5(&Uuid::NAMESPACE_OID, agent_id.as_bytes()))
    }
}

/// Shared context wrapped in Arc for concurrent access.
pub type SharedAgentContext = Arc<AgentContext>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_from_env_defaults() {
        // Clear env vars
        std::env::remove_var("MNEMO_MCP_AGENT_ID");
        std::env::remove_var("MNEMO_MCP_DEFAULT_USER");
        std::env::remove_var("MNEMO_MCP_SESSION");

        let ctx = AgentContext::from_env();
        assert!(ctx.agent_id.is_none());
        assert!(ctx.user_id.is_none());
        assert_eq!(ctx.session_name, "mcp-session");
    }

    #[test]
    fn test_context_require_agent_fails_when_none() {
        let ctx = AgentContext::new(None, None, "test".to_string());
        let result = ctx.require_agent();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("MNEMO_MCP_AGENT_ID"));
    }

    #[test]
    fn test_context_require_agent_succeeds_when_set() {
        let ctx = AgentContext::new(Some("my-agent".to_string()), None, "test".to_string());
        let result = ctx.require_agent();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "my-agent");
    }

    #[test]
    fn test_resolve_user_prefers_provided() {
        let ctx = AgentContext::new(None, Some("default-user".to_string()), "test".to_string());
        let result = ctx.resolve_user(Some("provided-user"));
        assert_eq!(result.unwrap(), "provided-user");
    }

    #[test]
    fn test_resolve_user_falls_back_to_default() {
        let ctx = AgentContext::new(None, Some("default-user".to_string()), "test".to_string());
        let result = ctx.resolve_user(None);
        assert_eq!(result.unwrap(), "default-user");
    }

    #[test]
    fn test_resolve_user_fails_when_neither() {
        let ctx = AgentContext::new(None, None, "test".to_string());
        let result = ctx.resolve_user(None);
        assert!(result.is_err());
    }

    #[test]
    fn test_synthetic_user_id_deterministic() {
        let ctx = AgentContext::new(Some("my-agent".to_string()), None, "test".to_string());
        let id1 = ctx.synthetic_user_id().unwrap();
        let id2 = ctx.synthetic_user_id().unwrap();
        assert_eq!(id1, id2);

        // Different agent = different ID
        let ctx2 = AgentContext::new(Some("other-agent".to_string()), None, "test".to_string());
        let id3 = ctx2.synthetic_user_id().unwrap();
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_synthetic_user_id_none_when_no_agent() {
        let ctx = AgentContext::new(None, None, "test".to_string());
        assert!(ctx.synthetic_user_id().is_none());
    }

    #[tokio::test]
    async fn test_session_id_lifecycle() {
        let ctx = AgentContext::new(None, None, "test".to_string());
        assert!(ctx.session_id().await.is_none());

        let id = Uuid::now_v7();
        ctx.set_session_id(id).await;
        assert_eq!(ctx.session_id().await, Some(id));
    }
}
