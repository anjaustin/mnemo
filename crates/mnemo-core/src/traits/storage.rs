use uuid::Uuid;

use crate::error::MnemoError;
use crate::models::{
    agent::{
        AgentIdentityAuditEvent, AgentIdentityProfile, AuditChainVerification,
        BranchInfo, BranchMetadata, CreateBranchRequest, CreateExperienceRequest,
        CreatePromotionProposalRequest, ExperienceEvent, MergeResult,
        PromotionProposal, UpdateAgentIdentityRequest,
    },
    digest::MemoryDigest,
    edge::{Edge, EdgeFilter},
    entity::Entity,
    episode::{CreateEpisodeRequest, Episode, ListEpisodesParams},
    session::{CreateSessionRequest, ListSessionsParams, Session, UpdateSessionRequest},
    span::LlmSpan,
    user::{CreateUserRequest, UpdateUserRequest, User},
};

// ─── Agent Identity Storage ────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait AgentStore: Send + Sync {
    async fn get_agent_identity(&self, agent_id: &str) -> StorageResult<AgentIdentityProfile>;
    async fn update_agent_identity(
        &self,
        agent_id: &str,
        req: UpdateAgentIdentityRequest,
    ) -> StorageResult<AgentIdentityProfile>;
    async fn add_experience_event(
        &self,
        agent_id: &str,
        req: CreateExperienceRequest,
    ) -> StorageResult<ExperienceEvent>;
    async fn get_experience_event(&self, event_id: Uuid) -> StorageResult<Option<ExperienceEvent>>;
    async fn update_experience_event(&self, event: &ExperienceEvent) -> StorageResult<()>;
    async fn list_experience_events(
        &self,
        agent_id: &str,
        limit: u32,
    ) -> StorageResult<Vec<ExperienceEvent>>;
    async fn list_agent_identity_versions(
        &self,
        agent_id: &str,
        limit: u32,
    ) -> StorageResult<Vec<AgentIdentityProfile>>;
    async fn rollback_agent_identity(
        &self,
        agent_id: &str,
        target_version: u64,
        reason: Option<String>,
    ) -> StorageResult<AgentIdentityProfile>;
    async fn list_agent_identity_audit(
        &self,
        agent_id: &str,
        limit: u32,
    ) -> StorageResult<Vec<AgentIdentityAuditEvent>>;
    /// Walk the full audit chain and verify hash integrity.
    async fn verify_agent_audit_chain(
        &self,
        agent_id: &str,
    ) -> StorageResult<AuditChainVerification>;
    async fn create_promotion_proposal(
        &self,
        agent_id: &str,
        req: CreatePromotionProposalRequest,
    ) -> StorageResult<PromotionProposal>;
    async fn list_promotion_proposals(
        &self,
        agent_id: &str,
        limit: u32,
    ) -> StorageResult<Vec<PromotionProposal>>;
    async fn get_promotion_proposal(
        &self,
        agent_id: &str,
        proposal_id: Uuid,
    ) -> StorageResult<PromotionProposal>;
    async fn update_promotion_proposal(&self, proposal: &PromotionProposal) -> StorageResult<()>;

    // ─── COW Branching ──────────────────────────────────────────
    /// Create a branch from the agent's current identity.
    async fn create_agent_branch(
        &self,
        agent_id: &str,
        req: CreateBranchRequest,
    ) -> StorageResult<BranchInfo>;
    /// List all branches for an agent.
    async fn list_agent_branches(
        &self,
        agent_id: &str,
    ) -> StorageResult<Vec<BranchMetadata>>;
    /// Get a specific branch's info (metadata + current identity).
    async fn get_agent_branch(
        &self,
        agent_id: &str,
        branch_name: &str,
    ) -> StorageResult<BranchInfo>;
    /// Update a branch's identity core.
    async fn update_agent_branch(
        &self,
        agent_id: &str,
        branch_name: &str,
        req: UpdateAgentIdentityRequest,
    ) -> StorageResult<AgentIdentityProfile>;
    /// Merge a branch back into the parent's main identity.
    async fn merge_agent_branch(
        &self,
        agent_id: &str,
        branch_name: &str,
    ) -> StorageResult<MergeResult>;
    /// Delete a branch without merging.
    async fn delete_agent_branch(
        &self,
        agent_id: &str,
        branch_name: &str,
    ) -> StorageResult<()>;
}

/// Result type for all storage operations.
pub type StorageResult<T> = Result<T, MnemoError>;

// ─── User Storage ──────────────────────────────────────────────────

/// Trait for user persistence operations.
#[allow(async_fn_in_trait)]
pub trait UserStore: Send + Sync {
    async fn create_user(&self, req: CreateUserRequest) -> StorageResult<User>;
    async fn get_user(&self, id: Uuid) -> StorageResult<User>;
    async fn get_user_by_external_id(&self, external_id: &str) -> StorageResult<User>;
    async fn update_user(&self, id: Uuid, req: UpdateUserRequest) -> StorageResult<User>;
    async fn delete_user(&self, id: Uuid) -> StorageResult<()>;
    async fn list_users(&self, limit: u32, after: Option<Uuid>) -> StorageResult<Vec<User>>;
}

// ─── Session Storage ───────────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait SessionStore: Send + Sync {
    async fn create_session(&self, req: CreateSessionRequest) -> StorageResult<Session>;
    async fn get_session(&self, id: Uuid) -> StorageResult<Session>;
    async fn update_session(&self, id: Uuid, req: UpdateSessionRequest) -> StorageResult<Session>;
    async fn delete_session(&self, id: Uuid) -> StorageResult<()>;
    async fn list_sessions(
        &self,
        user_id: Uuid,
        params: ListSessionsParams,
    ) -> StorageResult<Vec<Session>>;
}

// ─── Episode Storage ───────────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait EpisodeStore: Send + Sync {
    async fn create_episode(
        &self,
        req: CreateEpisodeRequest,
        session_id: Uuid,
        user_id: Uuid,
    ) -> StorageResult<Episode>;

    async fn create_episodes_batch(
        &self,
        episodes: Vec<CreateEpisodeRequest>,
        session_id: Uuid,
        user_id: Uuid,
    ) -> StorageResult<Vec<Episode>>;

    async fn get_episode(&self, id: Uuid) -> StorageResult<Episode>;

    async fn update_episode(&self, episode: &Episode) -> StorageResult<()>;

    async fn list_episodes(
        &self,
        session_id: Uuid,
        params: ListEpisodesParams,
    ) -> StorageResult<Vec<Episode>>;

    /// Get episodes that are pending processing (for the ingestion pipeline).
    async fn get_pending_episodes(&self, limit: u32) -> StorageResult<Vec<Episode>>;

    /// Atomically claim an episode for processing (prevents double-processing).
    async fn claim_episode(&self, id: Uuid) -> StorageResult<bool>;

    /// Re-add an episode to the pending queue with a future timestamp for delayed retry.
    /// `delay_ms` is how far in the future the episode should become eligible for processing.
    async fn requeue_episode(&self, id: Uuid, delay_ms: u64) -> StorageResult<()>;

    /// Delete a single episode by ID and remove it from its session's episode set.
    async fn delete_episode(&self, id: Uuid) -> StorageResult<()>;

    /// Delete all episodes in a session (clear messages) without deleting the session itself.
    /// Returns the number of episodes deleted.
    async fn delete_session_episodes(&self, session_id: Uuid) -> StorageResult<u32>;
}

// ─── Entity Storage ────────────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait EntityStore: Send + Sync {
    async fn create_entity(&self, entity: Entity) -> StorageResult<Entity>;
    async fn get_entity(&self, id: Uuid) -> StorageResult<Entity>;
    async fn update_entity(&self, entity: &Entity) -> StorageResult<()>;
    async fn delete_entity(&self, id: Uuid) -> StorageResult<()>;

    /// Find an existing entity by name or alias within a user's graph.
    /// Used during deduplication in the ingestion pipeline.
    async fn find_entity_by_name(&self, user_id: Uuid, name: &str)
        -> StorageResult<Option<Entity>>;

    /// List all entities for a user.
    async fn list_entities(
        &self,
        user_id: Uuid,
        limit: u32,
        after: Option<Uuid>,
    ) -> StorageResult<Vec<Entity>>;
}

// ─── Edge Storage ──────────────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait EdgeStore: Send + Sync {
    async fn create_edge(&self, edge: Edge) -> StorageResult<Edge>;
    async fn get_edge(&self, id: Uuid) -> StorageResult<Edge>;
    async fn update_edge(&self, edge: &Edge) -> StorageResult<()>;
    async fn delete_edge(&self, id: Uuid) -> StorageResult<()>;

    /// Query edges with filtering.
    async fn query_edges(&self, user_id: Uuid, filter: EdgeFilter) -> StorageResult<Vec<Edge>>;

    /// Get all outgoing edges from an entity.
    async fn get_outgoing_edges(&self, entity_id: Uuid) -> StorageResult<Vec<Edge>>;

    /// Get all incoming edges to an entity.
    async fn get_incoming_edges(&self, entity_id: Uuid) -> StorageResult<Vec<Edge>>;

    /// Find edges that might conflict with a new relationship
    /// (same source, target, and label — candidates for invalidation).
    async fn find_conflicting_edges(
        &self,
        user_id: Uuid,
        source_entity_id: Uuid,
        target_entity_id: Uuid,
        label: &str,
    ) -> StorageResult<Vec<Edge>>;
}

// ─── Vector Storage ────────────────────────────────────────────────

/// Trait for vector embedding storage and similarity search (Qdrant).
#[allow(async_fn_in_trait)]
pub trait VectorStore: Send + Sync {
    /// Store an embedding for an entity.
    async fn upsert_entity_embedding(
        &self,
        entity_id: Uuid,
        user_id: Uuid,
        embedding: Vec<f32>,
        payload: serde_json::Value,
    ) -> StorageResult<()>;

    /// Store an embedding for an edge/fact.
    async fn upsert_edge_embedding(
        &self,
        edge_id: Uuid,
        user_id: Uuid,
        embedding: Vec<f32>,
        payload: serde_json::Value,
    ) -> StorageResult<()>;

    /// Store an embedding for an episode.
    async fn upsert_episode_embedding(
        &self,
        episode_id: Uuid,
        user_id: Uuid,
        embedding: Vec<f32>,
        payload: serde_json::Value,
    ) -> StorageResult<()>;

    /// Semantic search over entities.
    async fn search_entities(
        &self,
        user_id: Uuid,
        query_embedding: Vec<f32>,
        limit: u32,
        min_score: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>>;

    /// Semantic search over edges/facts.
    async fn search_edges(
        &self,
        user_id: Uuid,
        query_embedding: Vec<f32>,
        limit: u32,
        min_score: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>>;

    /// Semantic search over episodes.
    async fn search_episodes(
        &self,
        user_id: Uuid,
        query_embedding: Vec<f32>,
        limit: u32,
        min_score: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>>;

    /// Update payload fields on an entity point without re-sending the embedding vector.
    /// Used by proactive re-ranking to write relevance scores to Qdrant.
    async fn set_entity_payload(
        &self,
        entity_id: Uuid,
        payload: serde_json::Value,
    ) -> StorageResult<()>;

    /// Update payload fields on an edge point without re-sending the embedding vector.
    /// Used by proactive re-ranking to write relevance scores to Qdrant.
    async fn set_edge_payload(
        &self,
        edge_id: Uuid,
        payload: serde_json::Value,
    ) -> StorageResult<()>;

    /// Delete all vectors for a user (GDPR hard delete).
    async fn delete_user_vectors(&self, user_id: Uuid) -> StorageResult<()>;
}

// ─── Raw Vector Storage ────────────────────────────────────────────

/// A scored search hit from raw vector queries.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VectorHit {
    pub id: String,
    pub score: f32,
    pub payload: serde_json::Value,
}

/// Trait for raw (namespace-based) vector storage.
///
/// This enables external systems (e.g. AnythingLLM) to use Mnemo as a
/// pluggable vector database. Namespaces map to dynamically-created Qdrant
/// collections and are fully isolated from Mnemo's internal entity/edge/episode
/// collections.
#[allow(async_fn_in_trait)]
pub trait RawVectorStore: Send + Sync {
    /// Ensure a namespace (collection) exists, creating it if needed.
    /// `dimensions` specifies the vector size for collection creation.
    async fn ensure_namespace(&self, namespace: &str, dimensions: u32) -> StorageResult<()>;

    /// Check whether a namespace (collection) exists.
    async fn has_namespace(&self, namespace: &str) -> StorageResult<bool>;

    /// Delete an entire namespace (collection) and all its vectors.
    async fn delete_namespace(&self, namespace: &str) -> StorageResult<()>;

    /// Upsert a batch of vectors into a namespace.
    async fn upsert_vectors(
        &self,
        namespace: &str,
        vectors: Vec<(String, Vec<f32>, serde_json::Value)>,
    ) -> StorageResult<()>;

    /// Search a namespace by vector similarity.
    async fn search_vectors(
        &self,
        namespace: &str,
        query_vector: Vec<f32>,
        top_k: u32,
        min_score: f32,
    ) -> StorageResult<Vec<VectorHit>>;

    /// Delete specific vectors by ID from a namespace.
    async fn delete_vectors(&self, namespace: &str, ids: Vec<String>) -> StorageResult<()>;

    /// Count total vectors in a namespace.
    async fn count_vectors(&self, namespace: &str) -> StorageResult<u64>;
}

// ─── Digest Storage ────────────────────────────────────────────────

/// Storage for memory digests (sleep-time compute summaries).
#[allow(async_fn_in_trait)]
pub trait DigestStore: Send + Sync {
    /// Persist a memory digest for a user (overwrites any previous digest).
    async fn save_digest(&self, digest: &MemoryDigest) -> StorageResult<()>;

    /// Load a single user's memory digest. Returns `None` if no digest exists.
    async fn get_digest(&self, user_id: Uuid) -> StorageResult<Option<MemoryDigest>>;

    /// Load all stored digests (used on startup to warm the in-memory cache).
    async fn list_digests(&self) -> StorageResult<Vec<MemoryDigest>>;

    /// Delete a user's memory digest.
    async fn delete_digest(&self, user_id: Uuid) -> StorageResult<()>;
}

// ─── Span Storage ──────────────────────────────────────────────────

/// Storage for LLM call spans (tracing/observability).
///
/// Spans are persisted with a 7-day TTL by default. The store supports
/// querying by request correlation ID, user ID, or listing recent spans.
#[allow(async_fn_in_trait)]
pub trait SpanStore: Send + Sync {
    /// Persist an LLM span. Implementations should apply a TTL (e.g. 7 days).
    async fn save_span(&self, span: &LlmSpan) -> StorageResult<()>;

    /// Load spans by request correlation ID, ordered by `started_at` ascending.
    async fn get_spans_by_request(&self, request_id: &str) -> StorageResult<Vec<LlmSpan>>;

    /// Load recent spans for a user, ordered by `started_at` descending.
    /// Returns at most `limit` spans.
    async fn get_spans_by_user(&self, user_id: Uuid, limit: usize) -> StorageResult<Vec<LlmSpan>>;

    /// Load recent spans across all users, ordered by `started_at` descending.
    /// Returns at most `limit` spans.
    async fn list_recent_spans(&self, limit: usize) -> StorageResult<Vec<LlmSpan>>;
}

// ─── Composite Traits ──────────────────────────────────────────────

/// Combines all state-based storage (Redis side).
/// Users, sessions, episodes, entities, edges — anything that's JSON/structured data.
pub trait StateStore:
    UserStore
    + SessionStore
    + EpisodeStore
    + EntityStore
    + EdgeStore
    + AgentStore
    + DigestStore
    + SpanStore
{
}

/// Blanket implementation for StateStore.
impl<T> StateStore for T where
    T: UserStore
        + SessionStore
        + EpisodeStore
        + EntityStore
        + EdgeStore
        + AgentStore
        + DigestStore
        + SpanStore
{
}

// Note: `VectorStore` stands on its own (Qdrant side).
// The server layer composes `StateStore` + `VectorStore` rather than
// forcing a single struct to implement both.
// This reflects the reality that Redis and Qdrant are separate backends.
