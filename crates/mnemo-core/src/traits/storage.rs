use uuid::Uuid;

use crate::error::MnemoError;
use crate::models::{
    agent::{
        AgentIdentityProfile, CreateExperienceRequest, ExperienceEvent, UpdateAgentIdentityRequest,
    },
    edge::{Edge, EdgeFilter},
    entity::Entity,
    episode::{CreateEpisodeRequest, Episode, ListEpisodesParams},
    session::{CreateSessionRequest, ListSessionsParams, Session, UpdateSessionRequest},
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
    async fn list_experience_events(
        &self,
        agent_id: &str,
        limit: u32,
    ) -> StorageResult<Vec<ExperienceEvent>>;
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

    /// Delete all vectors for a user (GDPR hard delete).
    async fn delete_user_vectors(&self, user_id: Uuid) -> StorageResult<()>;
}

// ─── Composite Traits ──────────────────────────────────────────────

/// Combines all state-based storage (Redis side).
/// Users, sessions, episodes, entities, edges — anything that's JSON/structured data.
pub trait StateStore:
    UserStore + SessionStore + EpisodeStore + EntityStore + EdgeStore + AgentStore
{
}

/// Blanket implementation for StateStore.
impl<T> StateStore for T where
    T: UserStore + SessionStore + EpisodeStore + EntityStore + EdgeStore + AgentStore
{
}

// Note: `VectorStore` stands on its own (Qdrant side).
// The server layer composes `StateStore` + `VectorStore` rather than
// forcing a single struct to implement both.
// This reflects the reality that Redis and Qdrant are separate backends.
