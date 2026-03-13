use chrono::Utc;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};
use serde::de::DeserializeOwned;
use serde::Serialize;
use uuid::Uuid;

use mnemo_core::error::MnemoError;
use mnemo_core::models::{
    agent::{
        validate_branch_name, validate_fork_agent_id, AgentIdentityAuditAction,
        AgentIdentityAuditEvent, AgentIdentityProfile, ApprovalPolicy, AuditChainVerification,
        BranchInfo, BranchMetadata, CreateBranchRequest, CreateExperienceRequest,
        CreatePromotionProposalRequest, ExperienceEvent, ForkAgentRequest, ForkLineage, ForkResult,
        MergeResult, PromotionProposal, UpdateAgentIdentityRequest,
    },
    edge::{Edge, EdgeFilter},
    entity::Entity,
    episode::{CreateEpisodeRequest, Episode, ListEpisodesParams, ProcessingStatus},
    guardrail::GuardrailRule,
    region::{MemoryRegion, MemoryRegionAcl},
    session::{CreateSessionRequest, ListSessionsParams, Session, UpdateSessionRequest},
    span::LlmSpan,
    user::{CreateUserRequest, UpdateUserRequest, User},
};
use mnemo_core::traits::storage::*;

/// Redis-backed state store for all structured data.
///
/// Key schema:
/// ```text
/// {prefix}user:{id}                   → JSON User
/// {prefix}user_ext:{external_id}      → user UUID (index)
/// {prefix}users                       → Sorted Set (score=timestamp, member=user_id)
/// {prefix}session:{id}                → JSON Session
/// {prefix}user_sessions:{user_id}     → Sorted Set (score=timestamp, member=session_id)
/// {prefix}episode:{id}                → JSON Episode
/// {prefix}session_episodes:{sess_id}  → Sorted Set (score=timestamp, member=episode_id)
/// {prefix}pending_episodes            → Sorted Set (score=timestamp, member=episode_id)
/// {prefix}entity:{id}                 → JSON Entity
/// {prefix}user_entities:{user_id}     → Sorted Set (score=timestamp, member=entity_id)
/// {prefix}entity_name:{user_id}:{lc_name} → entity UUID (name index)
/// {prefix}entity_episodes:{entity_id} → Sorted Set (score=timestamp, member=episode_id)
/// {prefix}edge:{id}                   → JSON Edge
/// {prefix}adj_out:{entity_id}          → Sorted Set (score=valid_at, member=edge_id)
/// {prefix}adj_in:{entity_id}           → Sorted Set (score=valid_at, member=edge_id)
/// {prefix}user_edges:{user_id}         → Sorted Set (score=timestamp, member=edge_id)
/// {prefix}rid_episodes:{request_id}   → Sorted Set (score=epoch_ms, member="{ep_id}:{user_id}:{sess_id}")
/// {prefix}digest:{user_id}            → JSON MemoryDigest
/// {prefix}digests                      → Sorted Set (score=generated_at_ms, member=user_id)
/// {prefix}span:{id}                   → JSON LlmSpan (TTL: 7 days)
/// {prefix}spans                        → Sorted Set (score=started_at_ms, member=span_id)
/// {prefix}spans_request:{request_id}  → Sorted Set (score=started_at_ms, member=span_id)
/// {prefix}spans_user:{user_id}        → Sorted Set (score=started_at_ms, member=span_id)
/// ```
#[derive(Clone)]
pub struct RedisStateStore {
    pub(crate) conn: ConnectionManager,
    pub(crate) prefix: String,
}

impl RedisStateStore {
    /// Connect to Redis and return a new store.
    pub async fn new(url: &str, prefix: &str) -> Result<Self, MnemoError> {
        let client = Client::open(url)
            .map_err(|e| MnemoError::Redis(format!("Failed to create client: {}", e)))?;
        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| MnemoError::Redis(format!("Failed to connect: {}", e)))?;
        Ok(Self {
            conn,
            prefix: prefix.to_string(),
        })
    }

    pub(crate) fn key(&self, parts: &[&str]) -> String {
        format!("{}{}", self.prefix, parts.join(":"))
    }

    async fn set_json<T: Serialize>(&self, key: &str, value: &T) -> StorageResult<()> {
        let json = serde_json::to_string(value)?;
        let mut conn = self.conn.clone();
        // Use JSON.SET so that RediSearch ON JSON indexes can scan these documents.
        redis::cmd("JSON.SET")
            .arg(key)
            .arg("$")
            .arg(&json)
            .exec_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;
        Ok(())
    }

    async fn get_json<T: DeserializeOwned>(&self, key: &str) -> StorageResult<Option<T>> {
        let mut conn = self.conn.clone();
        // JSON.GET returns a JSON array wrapping the root value: ["<value>"]
        // We unwrap the outer array to get the actual document.
        let result: Option<String> = redis::cmd("JSON.GET")
            .arg(key)
            .arg("$")
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;
        match result {
            Some(raw) => {
                // JSON.GET with path "$" returns an array: [<document>]
                let arr: Vec<serde_json::Value> = serde_json::from_str(&raw)?;
                match arr.into_iter().next() {
                    Some(val) => Ok(Some(serde_json::from_value(val)?)),
                    None => Ok(None),
                }
            }
            None => Ok(None),
        }
    }

    async fn get_json_required<T: DeserializeOwned>(
        &self,
        key: &str,
        not_found_err: MnemoError,
    ) -> StorageResult<T> {
        self.get_json(key).await?.ok_or(not_found_err)
    }

    async fn del(&self, key: &str) -> StorageResult<()> {
        let mut conn = self.conn.clone();
        conn.del::<_, ()>(key)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;
        Ok(())
    }

    /// Fetch multiple items from a sorted set with cursor-based pagination.
    async fn list_from_zset<T: DeserializeOwned>(
        &self,
        zset_key: &str,
        item_prefix: &str,
        limit: u32,
        after: Option<Uuid>,
    ) -> StorageResult<Vec<T>> {
        let mut conn = self.conn.clone();

        // Get member IDs from sorted set (newest first)
        let ids: Vec<String> = if let Some(cursor_id) = after {
            // Get the score of the cursor, then fetch items with lower scores
            let score: Option<f64> = redis::cmd("ZSCORE")
                .arg(zset_key)
                .arg(cursor_id.to_string())
                .query_async(&mut conn)
                .await
                .map_err(|e| MnemoError::Redis(e.to_string()))?;

            match score {
                Some(s) => {
                    redis::cmd("ZREVRANGEBYSCORE")
                        .arg(zset_key)
                        .arg(format!("({}", s)) // exclusive
                        .arg("-inf")
                        .arg("LIMIT")
                        .arg(0)
                        .arg(limit)
                        .query_async(&mut conn)
                        .await
                        .map_err(|e| MnemoError::Redis(e.to_string()))?
                }
                None => Vec::new(),
            }
        } else {
            redis::cmd("ZREVRANGE")
                .arg(zset_key)
                .arg(0)
                .arg(limit as isize - 1)
                .query_async(&mut conn)
                .await
                .map_err(|e| MnemoError::Redis(e.to_string()))?
        };

        let mut items = Vec::with_capacity(ids.len());
        for id in &ids {
            let key = format!("{}{}", item_prefix, id);
            if let Some(item) = self.get_json::<T>(&key).await? {
                items.push(item);
            }
        }
        Ok(items)
    }

    /// Look up episodes by request correlation ID using the secondary index.
    ///
    /// Returns a list of `(episode_id, user_id, session_id)` tuples in
    /// ascending timestamp order, scoped to `[from_ms, to_ms]` and capped at
    /// `limit` results.
    ///
    /// Key schema: `{prefix}rid_episodes:{request_id}` → SortedSet where
    /// score = created_at epoch_ms and member = `{episode_id}:{user_id}:{session_id}`.
    pub async fn get_episodes_by_request_id(
        &self,
        request_id: &str,
        from_ms: i64,
        to_ms: i64,
        limit: usize,
    ) -> StorageResult<Vec<(Uuid, Uuid, Uuid)>> {
        let rid_key = self.key(&["rid_episodes", request_id]);
        let mut conn = self.conn.clone();

        let members: Vec<String> = redis::cmd("ZRANGEBYSCORE")
            .arg(&rid_key)
            .arg(from_ms)
            .arg(to_ms)
            .arg("LIMIT")
            .arg(0)
            .arg(limit as isize)
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut result = Vec::with_capacity(members.len());
        for member in &members {
            // member format: "{episode_id}:{user_id}:{session_id}"
            let parts: Vec<&str> = member.splitn(3, ':').collect();
            if parts.len() != 3 {
                continue;
            }
            let ep_id = Uuid::parse_str(parts[0])
                .map_err(|e| MnemoError::Storage(format!("Invalid UUID in rid index: {}", e)))?;
            let user_id = Uuid::parse_str(parts[1])
                .map_err(|e| MnemoError::Storage(format!("Invalid UUID in rid index: {}", e)))?;
            let sess_id = Uuid::parse_str(parts[2])
                .map_err(|e| MnemoError::Storage(format!("Invalid UUID in rid index: {}", e)))?;
            result.push((ep_id, user_id, sess_id));
        }
        Ok(result)
    }
}

// ─── UserStore ─────────────────────────────────────────────────────

impl UserStore for RedisStateStore {
    async fn create_user(&self, req: CreateUserRequest) -> StorageResult<User> {
        let user = User::from_request(req);
        let key = self.key(&["user", &user.id.to_string()]);

        // Check for duplicate
        if self.get_json::<User>(&key).await?.is_some() {
            return Err(MnemoError::Duplicate(format!(
                "User {} already exists",
                user.id
            )));
        }

        // Store user
        self.set_json(&key, &user).await?;

        // Index by external_id
        if let Some(ref ext_id) = user.external_id {
            let ext_key = self.key(&["user_ext", ext_id]);
            let mut conn = self.conn.clone();
            conn.set::<_, _, ()>(&ext_key, user.id.to_string())
                .await
                .map_err(|e| MnemoError::Redis(e.to_string()))?;
        }

        // Add to sorted set for listing
        let zset_key = self.key(&["users"]);
        let mut conn = self.conn.clone();
        conn.zadd::<_, _, _, ()>(
            &zset_key,
            user.id.to_string(),
            user.created_at.timestamp_millis() as f64,
        )
        .await
        .map_err(|e| MnemoError::Redis(e.to_string()))?;

        tracing::debug!(user_id = %user.id, "Created user");
        Ok(user)
    }

    async fn get_user(&self, id: Uuid) -> StorageResult<User> {
        let key = self.key(&["user", &id.to_string()]);
        self.get_json_required(&key, MnemoError::UserNotFound(id))
            .await
    }

    async fn get_user_by_external_id(&self, external_id: &str) -> StorageResult<User> {
        let ext_key = self.key(&["user_ext", external_id]);
        let mut conn = self.conn.clone();
        let id_str: Option<String> = conn
            .get(&ext_key)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        match id_str {
            Some(s) => {
                let id = Uuid::parse_str(&s)
                    .map_err(|e| MnemoError::Storage(format!("Invalid UUID in index: {}", e)))?;
                self.get_user(id).await
            }
            None => Err(MnemoError::NotFound {
                resource_type: "User".into(),
                id: external_id.into(),
            }),
        }
    }

    async fn update_user(&self, id: Uuid, req: UpdateUserRequest) -> StorageResult<User> {
        let user = self.get_user(id).await?;
        let updated = user.apply_update(req);
        let key = self.key(&["user", &id.to_string()]);
        self.set_json(&key, &updated).await?;
        Ok(updated)
    }

    async fn delete_user(&self, id: Uuid) -> StorageResult<()> {
        let user = self.get_user(id).await?;
        let key = self.key(&["user", &id.to_string()]);
        self.del(&key).await?;

        if let Some(ref ext_id) = user.external_id {
            self.del(&self.key(&["user_ext", ext_id])).await?;
        }

        let mut conn = self.conn.clone();
        let zset_key = self.key(&["users"]);
        conn.zrem::<_, _, ()>(&zset_key, id.to_string())
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        tracing::debug!(user_id = %id, "Deleted user");
        Ok(())
    }

    async fn list_users(&self, limit: u32, after: Option<Uuid>) -> StorageResult<Vec<User>> {
        let zset_key = self.key(&["users"]);
        let prefix = self.key(&["user:"]);
        self.list_from_zset(&zset_key, &prefix, limit, after).await
    }
}

// ─── SessionStore ──────────────────────────────────────────────────

impl SessionStore for RedisStateStore {
    async fn create_session(&self, req: CreateSessionRequest) -> StorageResult<Session> {
        // Verify user exists
        self.get_user(req.user_id).await?;

        let session = Session::from_request(req);
        let key = self.key(&["session", &session.id.to_string()]);
        self.set_json(&key, &session).await?;

        let zset_key = self.key(&["user_sessions", &session.user_id.to_string()]);
        let mut conn = self.conn.clone();
        conn.zadd::<_, _, _, ()>(
            &zset_key,
            session.id.to_string(),
            session.created_at.timestamp_millis() as f64,
        )
        .await
        .map_err(|e| MnemoError::Redis(e.to_string()))?;

        tracing::debug!(session_id = %session.id, user_id = %session.user_id, "Created session");
        Ok(session)
    }

    async fn get_session(&self, id: Uuid) -> StorageResult<Session> {
        let key = self.key(&["session", &id.to_string()]);
        self.get_json_required(&key, MnemoError::SessionNotFound(id))
            .await
    }

    async fn update_session(&self, id: Uuid, req: UpdateSessionRequest) -> StorageResult<Session> {
        let session = self.get_session(id).await?;
        let updated = session.apply_update(req);
        let key = self.key(&["session", &id.to_string()]);
        self.set_json(&key, &updated).await?;
        Ok(updated)
    }

    async fn delete_session(&self, id: Uuid) -> StorageResult<()> {
        let session = self.get_session(id).await?;
        self.del(&self.key(&["session", &id.to_string()])).await?;

        let mut conn = self.conn.clone();
        let zset_key = self.key(&["user_sessions", &session.user_id.to_string()]);
        conn.zrem::<_, _, ()>(&zset_key, id.to_string())
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn list_sessions(
        &self,
        user_id: Uuid,
        params: ListSessionsParams,
    ) -> StorageResult<Vec<Session>> {
        let zset_key = self.key(&["user_sessions", &user_id.to_string()]);
        let prefix = self.key(&["session:"]);
        self.list_from_zset(&zset_key, &prefix, params.limit, params.after)
            .await
    }
}

// ─── EpisodeStore ──────────────────────────────────────────────────

impl EpisodeStore for RedisStateStore {
    async fn create_episode(
        &self,
        req: CreateEpisodeRequest,
        session_id: Uuid,
        user_id: Uuid,
    ) -> StorageResult<Episode> {
        let episode = Episode::from_request(req, session_id, user_id);
        let key = self.key(&["episode", &episode.id.to_string()]);
        self.set_json(&key, &episode).await?;

        let mut conn = self.conn.clone();

        // Add to session's episode list
        let zset_key = self.key(&["session_episodes", &session_id.to_string()]);
        conn.zadd::<_, _, _, ()>(
            &zset_key,
            episode.id.to_string(),
            episode.created_at.timestamp_millis() as f64,
        )
        .await
        .map_err(|e| MnemoError::Redis(e.to_string()))?;

        // Add to pending queue if it should be processed
        if episode.should_process() {
            let pending_key = self.key(&["pending_episodes"]);
            conn.zadd::<_, _, _, ()>(
                &pending_key,
                episode.id.to_string(),
                episode.ingested_at.timestamp_millis() as f64,
            )
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;
        }

        // Update session episode count
        let mut session = self.get_session(session_id).await?;
        session.record_episode(episode.id, episode.created_at);
        let sess_key = self.key(&["session", &session_id.to_string()]);
        self.set_json(&sess_key, &session).await?;

        // Index by request_id (O(1) trace lookup)
        if let Some(rid) = episode
            .metadata
            .get("request_id")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            let rid_key = self.key(&["rid_episodes", rid]);
            let member = format!("{}:{}:{}", episode.id, user_id, session_id);
            let score = episode.created_at.timestamp_millis() as f64;
            conn.zadd::<_, _, _, ()>(&rid_key, member, score)
                .await
                .map_err(|e| MnemoError::Redis(e.to_string()))?;
        }

        tracing::debug!(episode_id = %episode.id, session_id = %session_id, "Created episode");
        Ok(episode)
    }

    async fn create_episodes_batch(
        &self,
        episodes: Vec<CreateEpisodeRequest>,
        session_id: Uuid,
        user_id: Uuid,
    ) -> StorageResult<Vec<Episode>> {
        let mut results = Vec::with_capacity(episodes.len());
        for req in episodes {
            let ep = self.create_episode(req, session_id, user_id).await?;
            results.push(ep);
        }
        Ok(results)
    }

    async fn get_episode(&self, id: Uuid) -> StorageResult<Episode> {
        let key = self.key(&["episode", &id.to_string()]);
        self.get_json_required(&key, MnemoError::EpisodeNotFound(id))
            .await
    }

    async fn update_episode(&self, episode: &Episode) -> StorageResult<()> {
        let key = self.key(&["episode", &episode.id.to_string()]);
        self.set_json(&key, episode).await
    }

    async fn list_episodes(
        &self,
        session_id: Uuid,
        params: ListEpisodesParams,
    ) -> StorageResult<Vec<Episode>> {
        let zset_key = self.key(&["session_episodes", &session_id.to_string()]);
        let prefix = self.key(&["episode:"]);
        self.list_from_zset(&zset_key, &prefix, params.limit, params.after)
            .await
    }

    async fn get_pending_episodes(&self, limit: u32) -> StorageResult<Vec<Episode>> {
        let pending_key = self.key(&["pending_episodes"]);
        let mut conn = self.conn.clone();
        let ids: Vec<String> = redis::cmd("ZRANGE")
            .arg(&pending_key)
            .arg(0)
            .arg(limit as isize - 1)
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut episodes = Vec::with_capacity(ids.len());
        for id_str in &ids {
            let id = Uuid::parse_str(id_str)
                .map_err(|e| MnemoError::Storage(format!("Invalid UUID: {}", e)))?;
            if let Ok(ep) = self.get_episode(id).await {
                if ep.processing_status == ProcessingStatus::Pending {
                    episodes.push(ep);
                }
            }
        }
        Ok(episodes)
    }

    async fn claim_episode(&self, id: Uuid) -> StorageResult<bool> {
        // Atomic claim: remove from pending set. If removed, we own it.
        let pending_key = self.key(&["pending_episodes"]);
        let mut conn = self.conn.clone();
        let removed: u32 = conn
            .zrem(&pending_key, id.to_string())
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        if removed > 0 {
            let mut episode = self.get_episode(id).await?;
            episode.mark_processing();
            self.update_episode(&episode).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn requeue_episode(&self, id: Uuid, delay_ms: u64) -> StorageResult<()> {
        let pending_key = self.key(&["pending_episodes"]);
        let mut conn = self.conn.clone();
        let future_score = (chrono::Utc::now().timestamp_millis() + delay_ms as i64) as f64;
        conn.zadd::<_, _, _, ()>(&pending_key, id.to_string(), future_score)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;
        Ok(())
    }

    async fn delete_episode(&self, id: Uuid) -> StorageResult<()> {
        // Get the episode to find its session_id
        let episode = self.get_episode(id).await?;

        // Remove from session's episode sorted set
        let zset_key = self.key(&["session_episodes", &episode.session_id.to_string()]);
        let mut conn = self.conn.clone();
        conn.zrem::<_, _, ()>(&zset_key, id.to_string())
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        // Remove from pending queue (if present)
        let pending_key = self.key(&["pending_episodes"]);
        let _: () = conn
            .zrem(&pending_key, id.to_string())
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        // Delete the episode data
        let key = self.key(&["episode", &id.to_string()]);
        self.del(&key).await?;

        tracing::debug!(episode_id = %id, session_id = %episode.session_id, "Deleted episode");
        Ok(())
    }

    async fn delete_session_episodes(&self, session_id: Uuid) -> StorageResult<u32> {
        let zset_key = self.key(&["session_episodes", &session_id.to_string()]);
        let mut conn = self.conn.clone();

        // Get all episode IDs in this session
        let ids: Vec<String> = redis::cmd("ZRANGE")
            .arg(&zset_key)
            .arg(0)
            .arg(-1)
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let count = ids.len() as u32;

        // Delete each episode's data and remove from pending queue
        let pending_key = self.key(&["pending_episodes"]);
        for id_str in &ids {
            let key = self.key(&["episode", id_str]);
            self.del(&key).await?;
            let _: () = conn
                .zrem(&pending_key, id_str)
                .await
                .map_err(|e| MnemoError::Redis(e.to_string()))?;
        }

        // Clear the session's episode sorted set
        self.del(&zset_key).await?;

        tracing::debug!(session_id = %session_id, deleted = count, "Cleared session episodes");
        Ok(count)
    }
}

// ─── EntityStore ───────────────────────────────────────────────────

impl EntityStore for RedisStateStore {
    async fn create_entity(&self, entity: Entity) -> StorageResult<Entity> {
        let key = self.key(&["entity", &entity.id.to_string()]);
        self.set_json(&key, &entity).await?;

        let mut conn = self.conn.clone();

        // User entity index
        let zset_key = self.key(&["user_entities", &entity.user_id.to_string()]);
        conn.zadd::<_, _, _, ()>(
            &zset_key,
            entity.id.to_string(),
            entity.created_at.timestamp_millis() as f64,
        )
        .await
        .map_err(|e| MnemoError::Redis(e.to_string()))?;

        // Name index for dedup
        let name_key = self.key(&[
            "entity_name",
            &entity.user_id.to_string(),
            &entity.name.to_lowercase(),
        ]);
        conn.set::<_, _, ()>(&name_key, entity.id.to_string())
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        tracing::debug!(entity_id = %entity.id, name = %entity.name, "Created entity");
        Ok(entity)
    }

    async fn get_entity(&self, id: Uuid) -> StorageResult<Entity> {
        let key = self.key(&["entity", &id.to_string()]);
        self.get_json_required(&key, MnemoError::EntityNotFound(id))
            .await
    }

    async fn update_entity(&self, entity: &Entity) -> StorageResult<()> {
        let key = self.key(&["entity", &entity.id.to_string()]);
        self.set_json(&key, entity).await
    }

    async fn delete_entity(&self, id: Uuid) -> StorageResult<()> {
        let entity = self.get_entity(id).await?;
        self.del(&self.key(&["entity", &id.to_string()])).await?;

        let mut conn = self.conn.clone();
        let zset_key = self.key(&["user_entities", &entity.user_id.to_string()]);
        conn.zrem::<_, _, ()>(&zset_key, id.to_string())
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let name_key = self.key(&[
            "entity_name",
            &entity.user_id.to_string(),
            &entity.name.to_lowercase(),
        ]);
        self.del(&name_key).await?;

        Ok(())
    }

    async fn find_entity_by_name(
        &self,
        user_id: Uuid,
        name: &str,
    ) -> StorageResult<Option<Entity>> {
        let name_key = self.key(&["entity_name", &user_id.to_string(), &name.to_lowercase()]);
        let mut conn = self.conn.clone();
        let id_str: Option<String> = conn
            .get(&name_key)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        match id_str {
            Some(s) => {
                let id = Uuid::parse_str(&s)
                    .map_err(|e| MnemoError::Storage(format!("Invalid UUID: {}", e)))?;
                Ok(Some(self.get_entity(id).await?))
            }
            None => Ok(None),
        }
    }

    async fn list_entities(
        &self,
        user_id: Uuid,
        limit: u32,
        after: Option<Uuid>,
    ) -> StorageResult<Vec<Entity>> {
        let zset_key = self.key(&["user_entities", &user_id.to_string()]);
        let prefix = self.key(&["entity:"]);
        self.list_from_zset(&zset_key, &prefix, limit, after).await
    }
}

// ─── EdgeStore ─────────────────────────────────────────────────────

impl EdgeStore for RedisStateStore {
    async fn create_edge(&self, edge: Edge) -> StorageResult<Edge> {
        let key = self.key(&["edge", &edge.id.to_string()]);
        self.set_json(&key, &edge).await?;

        let mut conn = self.conn.clone();
        let score = edge.valid_at.timestamp_millis() as f64;

        // Adjacency lists
        let out_key = self.key(&["adj_out", &edge.source_entity_id.to_string()]);
        conn.zadd::<_, _, _, ()>(&out_key, edge.id.to_string(), score)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let in_key = self.key(&["adj_in", &edge.target_entity_id.to_string()]);
        conn.zadd::<_, _, _, ()>(&in_key, edge.id.to_string(), score)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        // User edge index
        let user_key = self.key(&["user_edges", &edge.user_id.to_string()]);
        conn.zadd::<_, _, _, ()>(&user_key, edge.id.to_string(), score)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        tracing::debug!(edge_id = %edge.id, label = %edge.label, "Created edge");
        Ok(edge)
    }

    async fn get_edge(&self, id: Uuid) -> StorageResult<Edge> {
        let key = self.key(&["edge", &id.to_string()]);
        self.get_json_required(&key, MnemoError::EdgeNotFound(id))
            .await
    }

    async fn update_edge(&self, edge: &Edge) -> StorageResult<()> {
        let key = self.key(&["edge", &edge.id.to_string()]);
        self.set_json(&key, edge).await
    }

    async fn delete_edge(&self, id: Uuid) -> StorageResult<()> {
        let edge = self.get_edge(id).await?;
        self.del(&self.key(&["edge", &id.to_string()])).await?;

        let mut conn = self.conn.clone();
        let out_key = self.key(&["adj_out", &edge.source_entity_id.to_string()]);
        conn.zrem::<_, _, ()>(&out_key, id.to_string()).await.ok();
        let in_key = self.key(&["adj_in", &edge.target_entity_id.to_string()]);
        conn.zrem::<_, _, ()>(&in_key, id.to_string()).await.ok();
        let user_key = self.key(&["user_edges", &edge.user_id.to_string()]);
        conn.zrem::<_, _, ()>(&user_key, id.to_string()).await.ok();

        Ok(())
    }

    async fn query_edges(&self, user_id: Uuid, filter: EdgeFilter) -> StorageResult<Vec<Edge>> {
        let zset_key = self.key(&["user_edges", &user_id.to_string()]);
        let prefix = self.key(&["edge:"]);

        // Fetch candidates (overfetch, then filter in-memory)
        let fetch_limit = filter.limit.saturating_mul(3);
        let candidates: Vec<Edge> = self
            .list_from_zset(&zset_key, &prefix, fetch_limit, None)
            .await?;

        Ok(candidates
            .into_iter()
            .filter(|e| filter.matches(e))
            .take(filter.limit as usize)
            .collect())
    }

    async fn get_outgoing_edges(&self, entity_id: Uuid) -> StorageResult<Vec<Edge>> {
        let zset_key = self.key(&["adj_out", &entity_id.to_string()]);
        let prefix = self.key(&["edge:"]);
        self.list_from_zset(&zset_key, &prefix, 1000, None).await
    }

    async fn get_incoming_edges(&self, entity_id: Uuid) -> StorageResult<Vec<Edge>> {
        let zset_key = self.key(&["adj_in", &entity_id.to_string()]);
        let prefix = self.key(&["edge:"]);
        self.list_from_zset(&zset_key, &prefix, 1000, None).await
    }

    async fn find_conflicting_edges(
        &self,
        user_id: Uuid,
        source_entity_id: Uuid,
        target_entity_id: Uuid,
        label: &str,
    ) -> StorageResult<Vec<Edge>> {
        // Get outgoing edges from source and filter
        let edges = self.get_outgoing_edges(source_entity_id).await?;
        Ok(edges
            .into_iter()
            .filter(|e| {
                e.target_entity_id == target_entity_id
                    && e.label == label
                    && e.user_id == user_id
                    && e.is_valid()
            })
            .collect())
    }
}

impl AgentStore for RedisStateStore {
    async fn get_agent_identity(&self, agent_id: &str) -> StorageResult<AgentIdentityProfile> {
        let key = self.key(&["agent_identity", agent_id]);
        match self.get_json::<AgentIdentityProfile>(&key).await? {
            Some(identity) => Ok(identity),
            None => {
                let identity = AgentIdentityProfile::new(agent_id.to_string());
                self.persist_identity_snapshot(agent_id, &identity).await?;
                self.append_identity_audit(
                    agent_id,
                    AgentIdentityAuditAction::Created,
                    None,
                    identity.version,
                    None,
                    None,
                )
                .await?;
                Ok(identity)
            }
        }
    }

    async fn update_agent_identity(
        &self,
        agent_id: &str,
        req: UpdateAgentIdentityRequest,
    ) -> StorageResult<AgentIdentityProfile> {
        let mut identity = self.get_agent_identity(agent_id).await?;
        let from_version = identity.version;
        identity.apply_update(req);
        self.persist_identity_snapshot(agent_id, &identity).await?;
        self.append_identity_audit(
            agent_id,
            AgentIdentityAuditAction::Updated,
            Some(from_version),
            identity.version,
            None,
            None,
        )
        .await?;
        Ok(identity)
    }

    async fn add_experience_event(
        &self,
        agent_id: &str,
        req: CreateExperienceRequest,
    ) -> StorageResult<ExperienceEvent> {
        let event = ExperienceEvent::from_request(agent_id, req);
        let event_key = self.key(&["experience", &event.id.to_string()]);
        self.set_json(&event_key, &event).await?;

        let zset_key = self.key(&["agent_experience", agent_id]);
        let mut conn = self.conn.clone();
        conn.zadd::<_, _, _, ()>(
            &zset_key,
            event.id.to_string(),
            event.created_at.timestamp_millis() as f64,
        )
        .await
        .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(event)
    }

    async fn get_experience_event(&self, event_id: Uuid) -> StorageResult<Option<ExperienceEvent>> {
        let key = self.key(&["experience", &event_id.to_string()]);
        self.get_json(&key).await
    }

    async fn update_experience_event(&self, event: &ExperienceEvent) -> StorageResult<()> {
        let event_key = self.key(&["experience", &event.id.to_string()]);
        self.set_json(&event_key, event).await?;
        Ok(())
    }

    async fn list_experience_events(
        &self,
        agent_id: &str,
        limit: u32,
    ) -> StorageResult<Vec<ExperienceEvent>> {
        let zset_key = self.key(&["agent_experience", agent_id]);
        let mut conn = self.conn.clone();
        let ids: Vec<String> = redis::cmd("ZREVRANGE")
            .arg(&zset_key)
            .arg(0)
            .arg(limit as isize - 1)
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut events = Vec::with_capacity(ids.len());
        for id in ids {
            let key = self.key(&["experience", &id]);
            if let Some(event) = self.get_json::<ExperienceEvent>(&key).await? {
                events.push(event);
            }
        }
        Ok(events)
    }

    async fn list_agent_identity_versions(
        &self,
        agent_id: &str,
        limit: u32,
    ) -> StorageResult<Vec<AgentIdentityProfile>> {
        let zset_key = self.key(&["agent_identity_versions", agent_id]);
        let mut conn = self.conn.clone();
        let versions: Vec<String> = redis::cmd("ZREVRANGE")
            .arg(&zset_key)
            .arg(0)
            .arg(limit as isize - 1)
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut out = Vec::with_capacity(versions.len());
        for version in versions {
            let key = self.key(&["agent_identity_version", agent_id, &version]);
            if let Some(identity) = self.get_json::<AgentIdentityProfile>(&key).await? {
                out.push(identity);
            }
        }
        Ok(out)
    }

    async fn rollback_agent_identity(
        &self,
        agent_id: &str,
        target_version: u64,
        reason: Option<String>,
    ) -> StorageResult<AgentIdentityProfile> {
        let target_key = self.key(&[
            "agent_identity_version",
            agent_id,
            &target_version.to_string(),
        ]);
        let target = self
            .get_json::<AgentIdentityProfile>(&target_key)
            .await?
            .ok_or(MnemoError::NotFound {
                resource_type: "AgentIdentityVersion".into(),
                id: format!("{}:{}", agent_id, target_version),
            })?;

        let current = self.get_agent_identity(agent_id).await?;
        let mut rolled = target;
        rolled.agent_id = agent_id.to_string();
        rolled.version = current.version + 1;
        rolled.updated_at = Utc::now();

        self.persist_identity_snapshot(agent_id, &rolled).await?;
        self.append_identity_audit(
            agent_id,
            AgentIdentityAuditAction::RolledBack,
            Some(current.version),
            rolled.version,
            Some(target_version),
            reason,
        )
        .await?;

        Ok(rolled)
    }

    async fn list_agent_identity_audit(
        &self,
        agent_id: &str,
        limit: u32,
    ) -> StorageResult<Vec<AgentIdentityAuditEvent>> {
        let zset_key = self.key(&["agent_identity_audit", agent_id]);
        let mut conn = self.conn.clone();
        let ids: Vec<String> = redis::cmd("ZREVRANGE")
            .arg(&zset_key)
            .arg(0)
            .arg(limit as isize - 1)
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let key = self.key(&["agent_identity_audit_event", &id]);
            if let Some(event) = self.get_json::<AgentIdentityAuditEvent>(&key).await? {
                out.push(event);
            }
        }
        Ok(out)
    }

    async fn verify_agent_audit_chain(
        &self,
        agent_id: &str,
    ) -> StorageResult<AuditChainVerification> {
        // Fetch ALL audit events in chronological (oldest-first) order using ZRANGE
        let zset_key = self.key(&["agent_identity_audit", agent_id]);
        let mut conn = self.conn.clone();
        let ids: Vec<String> = redis::cmd("ZRANGE")
            .arg(&zset_key)
            .arg(0)
            .arg(-1) // all elements, oldest first
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut events = Vec::with_capacity(ids.len());
        for id in ids {
            let key = self.key(&["agent_identity_audit_event", &id]);
            if let Some(event) = self.get_json::<AgentIdentityAuditEvent>(&key).await? {
                events.push(event);
            }
        }

        Ok(mnemo_core::models::agent::verify_audit_chain(&events))
    }

    async fn create_promotion_proposal(
        &self,
        agent_id: &str,
        req: CreatePromotionProposalRequest,
    ) -> StorageResult<PromotionProposal> {
        let proposal = PromotionProposal::from_request(agent_id, req);
        let key = self.key(&["promotion_proposal", &proposal.id.to_string()]);
        self.set_json(&key, &proposal).await?;

        let zset_key = self.key(&["agent_promotion_proposals", agent_id]);
        let mut conn = self.conn.clone();
        conn.zadd::<_, _, _, ()>(
            &zset_key,
            proposal.id.to_string(),
            proposal.created_at.timestamp_millis() as f64,
        )
        .await
        .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(proposal)
    }

    async fn list_promotion_proposals(
        &self,
        agent_id: &str,
        limit: u32,
    ) -> StorageResult<Vec<PromotionProposal>> {
        let zset_key = self.key(&["agent_promotion_proposals", agent_id]);
        let mut conn = self.conn.clone();
        let ids: Vec<String> = redis::cmd("ZREVRANGE")
            .arg(&zset_key)
            .arg(0)
            .arg(limit as isize - 1)
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let key = self.key(&["promotion_proposal", &id]);
            if let Some(proposal) = self.get_json::<PromotionProposal>(&key).await? {
                out.push(proposal);
            }
        }
        Ok(out)
    }

    async fn get_promotion_proposal(
        &self,
        agent_id: &str,
        proposal_id: Uuid,
    ) -> StorageResult<PromotionProposal> {
        let key = self.key(&["promotion_proposal", &proposal_id.to_string()]);
        let proposal =
            self.get_json::<PromotionProposal>(&key)
                .await?
                .ok_or(MnemoError::NotFound {
                    resource_type: "PromotionProposal".into(),
                    id: proposal_id.to_string(),
                })?;
        if proposal.agent_id != agent_id {
            return Err(MnemoError::NotFound {
                resource_type: "PromotionProposal".into(),
                id: proposal_id.to_string(),
            });
        }
        Ok(proposal)
    }

    async fn update_promotion_proposal(&self, proposal: &PromotionProposal) -> StorageResult<()> {
        let key = self.key(&["promotion_proposal", &proposal.id.to_string()]);
        self.set_json(&key, proposal).await
    }

    // ─── COW Branching ──────────────────────────────────────────

    async fn create_agent_branch(
        &self,
        agent_id: &str,
        req: CreateBranchRequest,
    ) -> StorageResult<BranchInfo> {
        validate_branch_name(&req.branch_name).map_err(|e| MnemoError::Validation(e))?;

        // Check if branch already exists
        let meta_key = self.key(&["agent_branch", agent_id, &req.branch_name]);
        if self.get_json::<BranchMetadata>(&meta_key).await?.is_some() {
            return Err(MnemoError::Validation(format!(
                "branch '{}' already exists for agent '{}'",
                req.branch_name, agent_id
            )));
        }

        // Get the parent identity to fork from
        let parent = self.get_agent_identity(agent_id).await?;

        // Create branch identity: starts with parent's core (or override)
        let branch_core = req.core_override.unwrap_or_else(|| parent.core.clone());
        let branch_agent_id = format!("{}:branch:{}", agent_id, req.branch_name);
        let branch_identity = AgentIdentityProfile {
            agent_id: branch_agent_id.clone(),
            version: parent.version,
            core: branch_core,
            updated_at: chrono::Utc::now(),
        };

        // Store the branch identity
        let branch_identity_key = self.key(&["agent_identity", &branch_agent_id]);
        self.set_json(&branch_identity_key, &branch_identity)
            .await?;

        // Store branch metadata
        let metadata = BranchMetadata {
            branch_name: req.branch_name.clone(),
            parent_agent_id: agent_id.to_string(),
            fork_version: parent.version,
            created_at: chrono::Utc::now(),
            description: req.description,
            merged: false,
        };
        self.set_json(&meta_key, &metadata).await?;

        // Add to branch index
        let index_key = self.key(&["agent_branches", agent_id]);
        let mut conn = self.conn.clone();
        let _: () = redis::cmd("ZADD")
            .arg(&index_key)
            .arg(metadata.created_at.timestamp_millis() as f64)
            .arg(&req.branch_name)
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(BranchInfo {
            metadata,
            identity: branch_identity,
        })
    }

    async fn list_agent_branches(&self, agent_id: &str) -> StorageResult<Vec<BranchMetadata>> {
        let index_key = self.key(&["agent_branches", agent_id]);
        let mut conn = self.conn.clone();

        let members: Vec<String> = redis::cmd("ZRANGE")
            .arg(&index_key)
            .arg(0i64)
            .arg(-1i64)
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut results = Vec::new();
        for branch_name in members {
            let meta_key = self.key(&["agent_branch", agent_id, &branch_name]);
            if let Some(meta) = self.get_json::<BranchMetadata>(&meta_key).await? {
                results.push(meta);
            }
        }
        Ok(results)
    }

    async fn get_agent_branch(
        &self,
        agent_id: &str,
        branch_name: &str,
    ) -> StorageResult<BranchInfo> {
        let meta_key = self.key(&["agent_branch", agent_id, branch_name]);
        let metadata = self
            .get_json::<BranchMetadata>(&meta_key)
            .await?
            .ok_or_else(|| MnemoError::NotFound {
                resource_type: "AgentBranch".into(),
                id: format!("{}:{}", agent_id, branch_name),
            })?;

        let branch_agent_id = format!("{}:branch:{}", agent_id, branch_name);
        let identity = self.get_agent_identity(&branch_agent_id).await?;

        Ok(BranchInfo { metadata, identity })
    }

    async fn update_agent_branch(
        &self,
        agent_id: &str,
        branch_name: &str,
        req: UpdateAgentIdentityRequest,
    ) -> StorageResult<AgentIdentityProfile> {
        // Verify branch exists
        let meta_key = self.key(&["agent_branch", agent_id, branch_name]);
        let metadata = self
            .get_json::<BranchMetadata>(&meta_key)
            .await?
            .ok_or_else(|| MnemoError::NotFound {
                resource_type: "AgentBranch".into(),
                id: format!("{}:{}", agent_id, branch_name),
            })?;
        if metadata.merged {
            return Err(MnemoError::Validation(format!(
                "branch '{}' has already been merged",
                branch_name
            )));
        }

        let branch_agent_id = format!("{}:branch:{}", agent_id, branch_name);
        self.update_agent_identity(&branch_agent_id, req).await
    }

    async fn merge_agent_branch(
        &self,
        agent_id: &str,
        branch_name: &str,
    ) -> StorageResult<MergeResult> {
        // Get branch
        let meta_key = self.key(&["agent_branch", agent_id, branch_name]);
        let mut metadata = self
            .get_json::<BranchMetadata>(&meta_key)
            .await?
            .ok_or_else(|| MnemoError::NotFound {
                resource_type: "AgentBranch".into(),
                id: format!("{}:{}", agent_id, branch_name),
            })?;
        if metadata.merged {
            return Err(MnemoError::Validation(format!(
                "branch '{}' has already been merged",
                branch_name
            )));
        }

        // Get the branch's current identity
        let branch_agent_id = format!("{}:branch:{}", agent_id, branch_name);
        let branch_identity = self.get_agent_identity(&branch_agent_id).await?;
        let branch_core = branch_identity.core.clone();

        // Apply the branch's core to the parent as a normal update
        let parent_before = self.get_agent_identity(agent_id).await?;
        let parent_version_before = parent_before.version;
        let merged_identity = self
            .update_agent_identity(
                agent_id,
                UpdateAgentIdentityRequest {
                    core: branch_core.clone(),
                },
            )
            .await?;

        // Mark branch as merged
        metadata.merged = true;
        self.set_json(&meta_key, &metadata).await?;

        Ok(MergeResult {
            branch_name: branch_name.to_string(),
            merged_identity,
            parent_version_before,
            branch_core_applied: branch_core,
        })
    }

    async fn delete_agent_branch(&self, agent_id: &str, branch_name: &str) -> StorageResult<()> {
        // Verify branch exists
        let meta_key = self.key(&["agent_branch", agent_id, branch_name]);
        if self.get_json::<BranchMetadata>(&meta_key).await?.is_none() {
            return Err(MnemoError::NotFound {
                resource_type: "AgentBranch".into(),
                id: format!("{}:{}", agent_id, branch_name),
            });
        }

        // Delete branch identity
        let branch_agent_id = format!("{}:branch:{}", agent_id, branch_name);
        let identity_key = self.key(&["agent_identity", &branch_agent_id]);
        let mut conn = self.conn.clone();
        let _: () = redis::cmd("DEL")
            .arg(&identity_key)
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        // Delete metadata
        let mut conn2 = self.conn.clone();
        let _: () = redis::cmd("DEL")
            .arg(&meta_key)
            .query_async(&mut conn2)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        // Remove from index
        let index_key = self.key(&["agent_branches", agent_id]);
        let mut conn3 = self.conn.clone();
        let _: () = redis::cmd("ZREM")
            .arg(&index_key)
            .arg(branch_name)
            .query_async(&mut conn3)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn fork_agent(
        &self,
        source_agent_id: &str,
        req: ForkAgentRequest,
    ) -> StorageResult<ForkResult> {
        // 1. Validate new agent ID
        validate_fork_agent_id(&req.new_agent_id).map_err(|e| MnemoError::Validation(e))?;

        // 2. Check new agent doesn't already exist
        let new_identity_key = self.key(&["agent_identity", &req.new_agent_id]);
        if self
            .get_json::<AgentIdentityProfile>(&new_identity_key)
            .await?
            .is_some()
        {
            return Err(MnemoError::Validation(format!(
                "agent '{}' already exists; cannot fork to an existing agent_id",
                req.new_agent_id
            )));
        }

        // 3. Get source agent identity
        let source = self.get_agent_identity(source_agent_id).await?;

        // 4. Create new identity (use override or copy parent core)
        let new_core = req.core_override.unwrap_or_else(|| source.core.clone());
        let new_identity = AgentIdentityProfile {
            agent_id: req.new_agent_id.clone(),
            version: 1, // New agent starts at version 1
            core: new_core,
            updated_at: Utc::now(),
        };

        // 5. Persist new identity
        self.set_json(&new_identity_key, &new_identity).await?;

        // 6. Persist initial version snapshot for the new agent
        self.persist_identity_snapshot(&req.new_agent_id, &new_identity)
            .await?;

        // 7. Transfer experience events (filtered)
        let source_events = self.list_experience_events(source_agent_id, 10000).await?;
        let filter = req.experience_filter.clone().unwrap_or_default();
        let max_events = filter.max_events.unwrap_or(u32::MAX) as usize;

        let mut transferred_count: u32 = 0;
        for event in source_events.iter().filter(|e| filter.matches(e)) {
            if transferred_count as usize >= max_events {
                break;
            }
            // Create a copy under the new agent_id with a new UUID
            let new_event = ExperienceEvent {
                id: Uuid::now_v7(),
                agent_id: req.new_agent_id.clone(),
                user_id: event.user_id,
                session_id: event.session_id,
                category: event.category.clone(),
                signal: event.signal.clone(),
                confidence: event.confidence,
                weight: event.weight,
                decay_half_life_days: event.decay_half_life_days,
                evidence_episode_ids: event.evidence_episode_ids.clone(),
                fisher_importance: event.fisher_importance,
                created_at: event.created_at,
            };

            let event_key = self.key(&[
                "agent_experience",
                &req.new_agent_id,
                &new_event.id.to_string(),
            ]);
            self.set_json(&event_key, &new_event).await?;

            // Add to sorted-set index
            let index_key = self.key(&["agent_experiences", &req.new_agent_id]);
            let score = new_event.created_at.timestamp_millis() as f64;
            let mut conn = self.conn.clone();
            let _: () = redis::cmd("ZADD")
                .arg(&index_key)
                .arg(score)
                .arg(new_event.id.to_string())
                .query_async(&mut conn)
                .await
                .map_err(|e| MnemoError::Redis(e.to_string()))?;

            transferred_count += 1;
        }

        // 8. Store lineage metadata on the new agent
        let lineage = ForkLineage {
            parent_agent_id: source_agent_id.to_string(),
            parent_version: source.version,
            forked_at: Utc::now(),
            description: req.description.clone(),
            experience_events_transferred: transferred_count,
            experience_filter: req.experience_filter,
        };

        let lineage_key = self.key(&["agent_lineage", &req.new_agent_id]);
        self.set_json(&lineage_key, &lineage).await?;

        // 9. Create audit event for the new agent
        let mut audit_event = AgentIdentityAuditEvent {
            id: Uuid::now_v7(),
            agent_id: req.new_agent_id.clone(),
            action: AgentIdentityAuditAction::Created,
            from_version: None,
            to_version: 1,
            rollback_to_version: None,
            reason: req.description.clone(),
            created_at: Utc::now(),
            prev_hash: None,
            event_hash: String::new(),
        };
        audit_event.event_hash = audit_event.compute_hash();
        let audit_key = self.key(&[
            "agent_audit",
            &req.new_agent_id,
            &audit_event.id.to_string(),
        ]);
        self.set_json(&audit_key, &audit_event).await?;

        let audit_index_key = self.key(&["agent_audit_events", &req.new_agent_id]);
        let audit_score = audit_event.created_at.timestamp_millis() as f64;
        let mut conn = self.conn.clone();
        let _: () = redis::cmd("ZADD")
            .arg(&audit_index_key)
            .arg(audit_score)
            .arg(audit_event.id.to_string())
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(ForkResult {
            new_agent: new_identity,
            lineage,
        })
    }

    // ─── Approval Policy ────────────────────────────────────────

    async fn save_approval_policy(&self, policy: &ApprovalPolicy) -> StorageResult<()> {
        let key = self.key(&["approval_policy", &policy.agent_id]);
        self.set_json(&key, policy).await
    }

    async fn get_approval_policy(&self, agent_id: &str) -> StorageResult<Option<ApprovalPolicy>> {
        let key = self.key(&["approval_policy", agent_id]);
        self.get_json::<ApprovalPolicy>(&key).await
    }
}

impl RedisStateStore {
    async fn persist_identity_snapshot(
        &self,
        agent_id: &str,
        identity: &AgentIdentityProfile,
    ) -> StorageResult<()> {
        let current_key = self.key(&["agent_identity", agent_id]);
        self.set_json(&current_key, identity).await?;

        let version_key = self.key(&[
            "agent_identity_version",
            agent_id,
            &identity.version.to_string(),
        ]);
        self.set_json(&version_key, identity).await?;

        let versions_key = self.key(&["agent_identity_versions", agent_id]);
        let mut conn = self.conn.clone();
        conn.zadd::<_, _, _, ()>(
            &versions_key,
            identity.version.to_string(),
            identity.version as f64,
        )
        .await
        .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn append_identity_audit(
        &self,
        agent_id: &str,
        action: AgentIdentityAuditAction,
        from_version: Option<u64>,
        to_version: u64,
        rollback_to_version: Option<u64>,
        reason: Option<String>,
    ) -> StorageResult<()> {
        // ─── Witness chain: fetch the latest event's hash ─────
        let prev_hash = {
            let zset_key = self.key(&["agent_identity_audit", agent_id]);
            let mut conn = self.conn.clone();
            let latest_ids: Vec<String> = redis::cmd("ZREVRANGE")
                .arg(&zset_key)
                .arg(0)
                .arg(0) // just the most recent
                .query_async(&mut conn)
                .await
                .map_err(|e| MnemoError::Redis(e.to_string()))?;

            if let Some(latest_id) = latest_ids.first() {
                let key = self.key(&["agent_identity_audit_event", latest_id]);
                if let Some(latest_event) = self.get_json::<AgentIdentityAuditEvent>(&key).await? {
                    Some(latest_event.event_hash)
                } else {
                    None
                }
            } else {
                None
            }
        };

        let mut event = AgentIdentityAuditEvent {
            id: Uuid::now_v7(),
            agent_id: agent_id.to_string(),
            action,
            from_version,
            to_version,
            rollback_to_version,
            reason,
            created_at: Utc::now(),
            prev_hash,
            event_hash: String::new(), // computed below
        };
        event.event_hash = event.compute_hash();

        let event_key = self.key(&["agent_identity_audit_event", &event.id.to_string()]);
        self.set_json(&event_key, &event).await?;

        let zset_key = self.key(&["agent_identity_audit", agent_id]);
        let mut conn = self.conn.clone();
        conn.zadd::<_, _, _, ()>(
            &zset_key,
            event.id.to_string(),
            event.created_at.timestamp_millis() as f64,
        )
        .await
        .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }
}

// ─── DigestStore ───────────────────────────────────────────────────

use mnemo_core::models::digest::MemoryDigest;
use mnemo_core::traits::storage::DigestStore;

impl DigestStore for RedisStateStore {
    async fn save_digest(&self, digest: &MemoryDigest) -> StorageResult<()> {
        let key = self.key(&["digest", &digest.user_id.to_string()]);
        let zset_key = self.key(&["digests"]);
        let json = serde_json::to_string(digest)?;
        let score = digest.generated_at.timestamp_millis() as f64;
        let member = digest.user_id.to_string();

        // Atomic: both JSON.SET and ZADD in a single pipeline (executed as one round-trip).
        let mut conn = self.conn.clone();
        redis::pipe()
            .atomic()
            .cmd("JSON.SET")
            .arg(&key)
            .arg("$")
            .arg(&json)
            .ignore()
            .cmd("ZADD")
            .arg(&zset_key)
            .arg(score)
            .arg(&member)
            .ignore()
            .exec_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn get_digest(&self, user_id: Uuid) -> StorageResult<Option<MemoryDigest>> {
        let key = self.key(&["digest", &user_id.to_string()]);
        self.get_json(&key).await
    }

    async fn list_digests(&self) -> StorageResult<Vec<MemoryDigest>> {
        let zset_key = self.key(&["digests"]);
        let mut conn = self.conn.clone();
        let user_ids: Vec<String> = conn
            .zrange(&zset_key, 0, -1)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut digests = Vec::with_capacity(user_ids.len());
        for uid_str in user_ids {
            let key = self.key(&["digest", &uid_str]);
            if let Some(digest) = self.get_json(&key).await? {
                digests.push(digest);
            }
        }
        Ok(digests)
    }

    async fn delete_digest(&self, user_id: Uuid) -> StorageResult<()> {
        let key = self.key(&["digest", &user_id.to_string()]);
        let zset_key = self.key(&["digests"]);

        // Atomic: DEL and ZREM in a single pipeline
        let mut conn = self.conn.clone();
        redis::pipe()
            .atomic()
            .del(&key)
            .ignore()
            .cmd("ZREM")
            .arg(&zset_key)
            .arg(user_id.to_string())
            .ignore()
            .exec_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }
}

// ─── SpanStore ─────────────────────────────────────────────────────

/// 7-day TTL for span keys (in seconds).
const SPAN_TTL_SECS: i64 = 7 * 24 * 60 * 60;

impl SpanStore for RedisStateStore {
    async fn save_span(&self, span: &LlmSpan) -> StorageResult<()> {
        let span_id = span.id.to_string();
        let key = self.key(&["span", &span_id]);
        let global_zset = self.key(&["spans"]);
        let json =
            serde_json::to_string(span).map_err(|e| MnemoError::Serialization(e.to_string()))?;
        let score = span.started_at.timestamp_millis() as f64;

        let mut conn = self.conn.clone();

        // Atomic pipeline: JSON.SET + EXPIRE + ZADD global + optional index sets
        let mut pipe = redis::pipe();
        pipe.atomic()
            .cmd("JSON.SET")
            .arg(&key)
            .arg("$")
            .arg(&json)
            .ignore()
            .cmd("EXPIRE")
            .arg(&key)
            .arg(SPAN_TTL_SECS)
            .ignore()
            .cmd("ZADD")
            .arg(&global_zset)
            .arg(score)
            .arg(&span_id)
            .ignore();

        // Index by request_id if present
        if let Some(ref rid) = span.request_id {
            let req_zset = self.key(&["spans_request", rid]);
            pipe.cmd("ZADD")
                .arg(&req_zset)
                .arg(score)
                .arg(&span_id)
                .ignore()
                .cmd("EXPIRE")
                .arg(&req_zset)
                .arg(SPAN_TTL_SECS)
                .ignore();
        }

        // Index by user_id if present
        if let Some(uid) = span.user_id {
            let user_zset = self.key(&["spans_user", &uid.to_string()]);
            pipe.cmd("ZADD")
                .arg(&user_zset)
                .arg(score)
                .arg(&span_id)
                .ignore()
                .cmd("EXPIRE")
                .arg(&user_zset)
                .arg(SPAN_TTL_SECS)
                .ignore();
        }

        pipe.exec_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn get_spans_by_request(&self, request_id: &str) -> StorageResult<Vec<LlmSpan>> {
        let req_zset = self.key(&["spans_request", request_id]);
        let mut conn = self.conn.clone();

        let span_ids: Vec<String> = conn
            .zrange(&req_zset, 0, -1)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut spans = Vec::with_capacity(span_ids.len());
        for sid in &span_ids {
            let key = self.key(&["span", sid]);
            if let Some(span) = self.get_json::<LlmSpan>(&key).await? {
                spans.push(span);
            }
        }
        Ok(spans)
    }

    async fn get_spans_by_user(&self, user_id: Uuid, limit: usize) -> StorageResult<Vec<LlmSpan>> {
        let user_zset = self.key(&["spans_user", &user_id.to_string()]);
        let mut conn = self.conn.clone();

        let clamped = limit.clamp(1, 1000) as isize;
        let span_ids: Vec<String> = conn
            .zrevrange(&user_zset, 0, clamped - 1)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut spans = Vec::with_capacity(span_ids.len());
        for sid in &span_ids {
            let key = self.key(&["span", sid]);
            if let Some(span) = self.get_json::<LlmSpan>(&key).await? {
                spans.push(span);
            }
        }
        Ok(spans)
    }

    async fn list_recent_spans(&self, limit: usize) -> StorageResult<Vec<LlmSpan>> {
        let global_zset = self.key(&["spans"]);
        let mut conn = self.conn.clone();

        let clamped = limit.clamp(1, 1000) as isize;
        let span_ids: Vec<String> = conn
            .zrevrange(&global_zset, 0, clamped - 1)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut spans = Vec::with_capacity(span_ids.len());
        for sid in &span_ids {
            let key = self.key(&["span", sid]);
            if let Some(span) = self.get_json::<LlmSpan>(&key).await? {
                spans.push(span);
            }
        }
        Ok(spans)
    }
}

// ─── ClarificationStore ───────────────────────────────────────────

use mnemo_core::models::clarification::ClarificationRequest;

impl ClarificationStore for RedisStateStore {
    async fn save_clarification(&self, req: &ClarificationRequest) -> Result<(), MnemoError> {
        let key = self.key(&["clarification", &req.id.to_string()]);
        self.set_json(&key, req).await?;

        // Add to user's clarification sorted set (score = severity * 1000 for ordering)
        let zset_key = self.key(&["user_clarifications", &req.user_id.to_string()]);
        let score = (req.severity * 1000.0) as f64;
        let mut conn = self.conn.clone();
        redis::cmd("ZADD")
            .arg(&zset_key)
            .arg(score)
            .arg(req.id.to_string())
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn get_clarification(
        &self,
        id: Uuid,
    ) -> Result<Option<ClarificationRequest>, MnemoError> {
        let key = self.key(&["clarification", &id.to_string()]);
        self.get_json::<ClarificationRequest>(&key).await
    }

    async fn list_clarifications(
        &self,
        user_id: Uuid,
        pending_only: bool,
        limit: usize,
    ) -> Result<Vec<ClarificationRequest>, MnemoError> {
        let zset_key = self.key(&["user_clarifications", &user_id.to_string()]);
        let mut conn = self.conn.clone();

        // Overfetch to account for filtering
        let fetch_limit = if pending_only { limit * 3 } else { limit };
        let clamped = fetch_limit.min(1000) as isize;

        let ids: Vec<String> = redis::cmd("ZREVRANGE")
            .arg(&zset_key)
            .arg(0)
            .arg(clamped - 1)
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut results = Vec::with_capacity(ids.len());
        for id_str in &ids {
            let key = self.key(&["clarification", id_str]);
            if let Some(mut clar) = self.get_json::<ClarificationRequest>(&key).await? {
                // Auto-expire pending clarifications that have passed their TTL
                if clar.is_expired() {
                    clar.expire();
                    self.set_json(&key, &clar).await?;
                }
                if pending_only {
                    if clar.status
                        == mnemo_core::models::clarification::ClarificationStatus::Pending
                    {
                        results.push(clar);
                    }
                } else {
                    results.push(clar);
                }
                if results.len() >= limit {
                    break;
                }
            }
        }
        Ok(results)
    }

    async fn delete_clarification(&self, id: Uuid) -> Result<(), MnemoError> {
        let key = self.key(&["clarification", &id.to_string()]);

        // Get the clarification first to find the user_id for index cleanup
        if let Some(clar) = self.get_json::<ClarificationRequest>(&key).await? {
            let zset_key = self.key(&["user_clarifications", &clar.user_id.to_string()]);
            let mut conn = self.conn.clone();
            redis::cmd("ZREM")
                .arg(&zset_key)
                .arg(id.to_string())
                .query_async::<()>(&mut conn)
                .await
                .map_err(|e| MnemoError::Redis(e.to_string()))?;
        }

        // Delete the JSON document
        let mut conn = self.conn.clone();
        redis::cmd("DEL")
            .arg(&key)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }
}

// ─── NarrativeStore ───────────────────────────────────────────────

use mnemo_core::models::narrative::UserNarrative;

impl NarrativeStore for RedisStateStore {
    async fn save_narrative(&self, narrative: &UserNarrative) -> Result<(), MnemoError> {
        let key = self.key(&["narrative", &narrative.user_id.to_string()]);
        self.set_json(&key, narrative).await
    }

    async fn get_narrative(&self, user_id: Uuid) -> Result<Option<UserNarrative>, MnemoError> {
        let key = self.key(&["narrative", &user_id.to_string()]);
        self.get_json::<UserNarrative>(&key).await
    }

    async fn delete_narrative(&self, user_id: Uuid) -> Result<(), MnemoError> {
        let key = self.key(&["narrative", &user_id.to_string()]);
        self.del(&key).await
    }
}

// ─── GoalStore ────────────────────────────────────────────────────

use mnemo_core::models::goal::GoalProfile;

impl GoalStore for RedisStateStore {
    async fn save_goal_profile(&self, profile: &GoalProfile) -> Result<(), MnemoError> {
        let key = self.key(&["goal", &profile.id.to_string()]);
        self.set_json(&key, profile).await?;

        // Index by user_id (or "global" for system-wide profiles)
        let owner = profile
            .user_id
            .map(|u| u.to_string())
            .unwrap_or_else(|| "global".to_string());
        let zset_key = self.key(&["user_goals", &owner]);
        let score = profile.created_at.timestamp_millis() as f64;
        let mut conn = self.conn.clone();
        redis::cmd("ZADD")
            .arg(&zset_key)
            .arg(score)
            .arg(profile.id.to_string())
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        // Also index by name for fast lookup
        let name_key = self.key(&["goal_name", &owner, &profile.name]);
        let mut conn = self.conn.clone();
        conn.set::<_, _, ()>(&name_key, profile.id.to_string())
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn get_goal_profile(&self, id: Uuid) -> Result<Option<GoalProfile>, MnemoError> {
        let key = self.key(&["goal", &id.to_string()]);
        self.get_json::<GoalProfile>(&key).await
    }

    async fn get_goal_profile_by_name(
        &self,
        user_id: Uuid,
        name: &str,
    ) -> Result<Option<GoalProfile>, MnemoError> {
        // Check user-specific first
        let user_name_key = self.key(&["goal_name", &user_id.to_string(), name]);
        let mut conn = self.conn.clone();
        let user_id_result: Option<String> = conn
            .get(&user_name_key)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        if let Some(id_str) = user_id_result {
            if let Ok(id) = Uuid::parse_str(&id_str) {
                if let Some(profile) = self.get_goal_profile(id).await? {
                    return Ok(Some(profile));
                }
            }
        }

        // Fallback to global
        let global_name_key = self.key(&["goal_name", "global", name]);
        let mut conn = self.conn.clone();
        let global_id_result: Option<String> = conn
            .get(&global_name_key)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        if let Some(id_str) = global_id_result {
            if let Ok(id) = Uuid::parse_str(&id_str) {
                return self.get_goal_profile(id).await;
            }
        }

        Ok(None)
    }

    async fn list_goal_profiles(
        &self,
        user_id: Uuid,
        limit: usize,
    ) -> Result<Vec<GoalProfile>, MnemoError> {
        let clamped = limit.min(500) as isize;
        let mut results = Vec::new();

        // Fetch user-specific goals
        let user_zset = self.key(&["user_goals", &user_id.to_string()]);
        let mut conn = self.conn.clone();
        let user_ids: Vec<String> = redis::cmd("ZREVRANGE")
            .arg(&user_zset)
            .arg(0)
            .arg(clamped - 1)
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        for id_str in &user_ids {
            let key = self.key(&["goal", id_str]);
            if let Some(profile) = self.get_json::<GoalProfile>(&key).await? {
                results.push(profile);
            }
        }

        // Also fetch global goals
        let global_zset = self.key(&["user_goals", "global"]);
        let mut conn = self.conn.clone();
        let global_ids: Vec<String> = redis::cmd("ZREVRANGE")
            .arg(&global_zset)
            .arg(0)
            .arg(clamped - 1)
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        for id_str in &global_ids {
            let key = self.key(&["goal", id_str]);
            if let Some(profile) = self.get_json::<GoalProfile>(&key).await? {
                results.push(profile);
            }
        }

        results.truncate(limit);
        Ok(results)
    }

    async fn delete_goal_profile(&self, id: Uuid) -> Result<(), MnemoError> {
        let key = self.key(&["goal", &id.to_string()]);

        if let Some(profile) = self.get_json::<GoalProfile>(&key).await? {
            // Clean up indexes
            let owner = profile
                .user_id
                .map(|u| u.to_string())
                .unwrap_or_else(|| "global".to_string());

            let zset_key = self.key(&["user_goals", &owner]);
            let mut conn = self.conn.clone();
            redis::cmd("ZREM")
                .arg(&zset_key)
                .arg(id.to_string())
                .query_async::<()>(&mut conn)
                .await
                .map_err(|e| MnemoError::Redis(e.to_string()))?;

            let name_key = self.key(&["goal_name", &owner, &profile.name]);
            self.del(&name_key).await?;
        }

        self.del(&key).await
    }
}

// ─── API Key Storage ───────────────────────────────────────────────
//
// Key schema:
//   {prefix}api_key:{id}                → JSON ApiKey
//   {prefix}api_keys                    → Sorted Set (score=created_at_ms, member=key_id)
//   {prefix}api_key_hash:{sha256_hash}  → key UUID (hash → id lookup index)

use mnemo_core::models::api_key::ApiKey as ApiKeyModel;

impl ApiKeyStore for RedisStateStore {
    async fn save_api_key(&self, key: &ApiKeyModel) -> Result<(), MnemoError> {
        let pk = self.key(&["api_key", &key.id.to_string()]);
        self.set_json(&pk, key).await?;

        // Index: sorted set by created_at
        let zset = self.key(&["api_keys"]);
        let score = key.created_at.timestamp_millis() as f64;
        let mut conn = self.conn.clone();
        redis::cmd("ZADD")
            .arg(&zset)
            .arg(score)
            .arg(key.id.to_string())
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        // Index: hash → id lookup
        let hash_key = self.key(&["api_key_hash", &key.key_hash]);
        let mut conn = self.conn.clone();
        conn.set::<_, _, ()>(&hash_key, key.id.to_string())
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn get_api_key(&self, id: Uuid) -> Result<Option<ApiKeyModel>, MnemoError> {
        let pk = self.key(&["api_key", &id.to_string()]);
        self.get_json::<ApiKeyModel>(&pk).await
    }

    async fn get_api_key_by_hash(&self, hash: &str) -> Result<Option<ApiKeyModel>, MnemoError> {
        let hash_key = self.key(&["api_key_hash", hash]);
        let mut conn = self.conn.clone();
        let id_str: Option<String> = conn
            .get(&hash_key)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        match id_str {
            None => Ok(None),
            Some(s) => {
                let id: Uuid = s
                    .parse()
                    .map_err(|e| MnemoError::Redis(format!("bad api_key UUID: {e}")))?;
                self.get_api_key(id).await
            }
        }
    }

    async fn list_api_keys(&self, limit: usize) -> Result<Vec<ApiKeyModel>, MnemoError> {
        let zset = self.key(&["api_keys"]);
        let mut conn = self.conn.clone();
        let ids: Vec<String> = redis::cmd("ZREVRANGE")
            .arg(&zset)
            .arg(0isize)
            .arg((limit as isize) - 1)
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut results = Vec::with_capacity(ids.len());
        for id_str in ids {
            let pk = self.key(&["api_key", &id_str]);
            if let Some(key) = self.get_json::<ApiKeyModel>(&pk).await? {
                results.push(key);
            }
        }
        Ok(results)
    }

    async fn update_api_key(&self, key: &ApiKeyModel) -> Result<(), MnemoError> {
        let pk = self.key(&["api_key", &key.id.to_string()]);
        self.set_json(&pk, key).await
    }

    async fn delete_api_key(&self, id: Uuid) -> Result<(), MnemoError> {
        // Load key first to clean up hash index
        let pk = self.key(&["api_key", &id.to_string()]);
        if let Some(key) = self.get_json::<ApiKeyModel>(&pk).await? {
            let hash_key = self.key(&["api_key_hash", &key.key_hash]);
            self.del(&hash_key).await?;
        }

        // Remove from sorted set
        let zset = self.key(&["api_keys"]);
        let mut conn = self.conn.clone();
        redis::cmd("ZREM")
            .arg(&zset)
            .arg(id.to_string())
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        self.del(&pk).await
    }
}

// ─── ViewStore ─────────────────────────────────────────────────────

use mnemo_core::models::view::MemoryView;
use mnemo_core::traits::storage::ViewStore;

impl ViewStore for RedisStateStore {
    async fn save_view(&self, view: &MemoryView) -> StorageResult<()> {
        let pk = self.key(&["memory_view", &view.name]);
        self.set_json(&pk, view).await?;

        // Sorted set for listing (score = created_at millis)
        let idx = self.key(&["memory_views"]);
        let score = view.created_at.timestamp_millis() as f64;
        let mut conn = self.conn.clone();
        redis::cmd("ZADD")
            .arg(&idx)
            .arg(score)
            .arg(&pk)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;
        Ok(())
    }

    async fn get_view(&self, name: &str) -> StorageResult<Option<MemoryView>> {
        let pk = self.key(&["memory_view", name]);
        self.get_json::<MemoryView>(&pk).await
    }

    async fn list_views(&self) -> StorageResult<Vec<MemoryView>> {
        let idx = self.key(&["memory_views"]);
        let mut conn = self.conn.clone();
        let keys: Vec<String> = redis::cmd("ZRANGEBYSCORE")
            .arg(&idx)
            .arg("-inf")
            .arg("+inf")
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut views = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(v) = self.get_json::<MemoryView>(&key).await? {
                views.push(v);
            }
        }
        Ok(views)
    }

    async fn update_view(&self, view: &MemoryView) -> StorageResult<()> {
        let pk = self.key(&["memory_view", &view.name]);
        self.set_json(&pk, view).await
    }

    async fn delete_view(&self, name: &str) -> StorageResult<()> {
        let pk = self.key(&["memory_view", name]);
        let idx = self.key(&["memory_views"]);
        let mut conn = self.conn.clone();
        redis::cmd("ZREM")
            .arg(&idx)
            .arg(&pk)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;
        self.del(&pk).await
    }
}

// ─── Guardrail Store ───────────────────────────────────────────────

impl GuardrailStore for RedisStateStore {
    async fn save_guardrail(&self, rule: &GuardrailRule) -> StorageResult<()> {
        let pk = self.key(&["guardrail", &rule.id.to_string()]);
        self.set_json(&pk, rule).await?;

        // Global sorted set (score = priority for ordering)
        let idx = self.key(&["guardrails"]);
        let score = rule.priority as f64;
        let mut conn = self.conn.clone();
        redis::cmd("ZADD")
            .arg(&idx)
            .arg(score)
            .arg(&pk)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        // Per-user sorted set (if user-scoped)
        if let mnemo_core::models::guardrail::GuardrailScope::User { user_id } = &rule.scope {
            let user_idx = self.key(&["guardrails_user", &user_id.to_string()]);
            let mut conn = self.conn.clone();
            redis::cmd("ZADD")
                .arg(&user_idx)
                .arg(score)
                .arg(&pk)
                .query_async::<()>(&mut conn)
                .await
                .map_err(|e| MnemoError::Redis(e.to_string()))?;
        }

        Ok(())
    }

    async fn get_guardrail(&self, id: Uuid) -> StorageResult<Option<GuardrailRule>> {
        let pk = self.key(&["guardrail", &id.to_string()]);
        self.get_json::<GuardrailRule>(&pk).await
    }

    async fn list_guardrails(&self) -> StorageResult<Vec<GuardrailRule>> {
        let idx = self.key(&["guardrails"]);
        let mut conn = self.conn.clone();
        let keys: Vec<String> = redis::cmd("ZRANGEBYSCORE")
            .arg(&idx)
            .arg("-inf")
            .arg("+inf")
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut rules = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(r) = self.get_json::<GuardrailRule>(&key).await? {
                rules.push(r);
            }
        }
        // Sort by priority (ZADD score = priority, but be defensive)
        rules.sort_by_key(|r| r.priority);
        Ok(rules)
    }

    async fn list_guardrails_for_user(&self, user_id: Uuid) -> StorageResult<Vec<GuardrailRule>> {
        // Load all rules and filter to global + matching user scope
        let all = self.list_guardrails().await?;
        let mut applicable: Vec<GuardrailRule> = all
            .into_iter()
            .filter(|r| match &r.scope {
                mnemo_core::models::guardrail::GuardrailScope::Global => true,
                mnemo_core::models::guardrail::GuardrailScope::User { user_id: uid } => {
                    *uid == user_id
                }
            })
            .collect();
        applicable.sort_by_key(|r| r.priority);
        Ok(applicable)
    }

    async fn update_guardrail(&self, rule: &GuardrailRule) -> StorageResult<()> {
        // Overwrite the JSON document
        let pk = self.key(&["guardrail", &rule.id.to_string()]);
        self.set_json(&pk, rule).await?;

        // Update score in global sorted set (priority may have changed)
        let idx = self.key(&["guardrails"]);
        let score = rule.priority as f64;
        let mut conn = self.conn.clone();
        redis::cmd("ZADD")
            .arg(&idx)
            .arg(score)
            .arg(&pk)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn delete_guardrail(&self, id: Uuid) -> StorageResult<()> {
        let pk = self.key(&["guardrail", &id.to_string()]);

        // Try to read the rule first so we can clean up user-scoped index
        if let Some(rule) = self.get_json::<GuardrailRule>(&pk).await? {
            if let mnemo_core::models::guardrail::GuardrailScope::User { user_id } = &rule.scope {
                let user_idx = self.key(&["guardrails_user", &user_id.to_string()]);
                let mut conn = self.conn.clone();
                redis::cmd("ZREM")
                    .arg(&user_idx)
                    .arg(&pk)
                    .query_async::<()>(&mut conn)
                    .await
                    .map_err(|e| MnemoError::Redis(e.to_string()))?;
            }
        }

        // Remove from global index
        let idx = self.key(&["guardrails"]);
        let mut conn = self.conn.clone();
        redis::cmd("ZREM")
            .arg(&idx)
            .arg(&pk)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        // Delete the JSON document
        self.del(&pk).await
    }
}

// ─── RegionStore ───────────────────────────────────────────────────
//
// Key schema:
//   {prefix}region:{id}                          — JSON document
//   {prefix}regions                              — sorted set (all region IDs, score = created_at ms)
//   {prefix}user_regions:{user_id}               — sorted set (region IDs for a user)
//   {prefix}agent_regions:{agent_id}             — sorted set (region IDs an agent owns or has ACL for)
//   {prefix}region_acl:{region_id}               — sorted set (agent_ids with ACLs, score = granted_at ms)
//   {prefix}region_acl_entry:{region_id}:{agent} — JSON document for each ACL entry

impl RegionStore for RedisStateStore {
    async fn create_region(&self, region: &MemoryRegion) -> StorageResult<()> {
        let pk = self.key(&["region", &region.id.to_string()]);
        let json = serde_json::to_string(region)?;
        let global_idx = self.key(&["regions"]);
        let user_idx = self.key(&["user_regions", &region.user_id.to_string()]);
        let owner_idx = self.key(&["agent_regions", &region.owner_agent_id]);
        let score = region.created_at.timestamp_millis() as f64;
        let id_str = region.id.to_string();

        // Atomic: JSON.SET + 3× ZADD (global, user, owner indices)
        let mut conn = self.conn.clone();
        redis::pipe()
            .atomic()
            .cmd("JSON.SET")
            .arg(&pk)
            .arg("$")
            .arg(&json)
            .ignore()
            .cmd("ZADD")
            .arg(&global_idx)
            .arg(score)
            .arg(&id_str)
            .ignore()
            .cmd("ZADD")
            .arg(&user_idx)
            .arg(score)
            .arg(&id_str)
            .ignore()
            .cmd("ZADD")
            .arg(&owner_idx)
            .arg(score)
            .arg(&id_str)
            .ignore()
            .exec_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn get_region(&self, id: Uuid) -> StorageResult<Option<MemoryRegion>> {
        let pk = self.key(&["region", &id.to_string()]);
        self.get_json::<MemoryRegion>(&pk).await
    }

    async fn list_regions(
        &self,
        user_id: Option<Uuid>,
        agent_id: Option<&str>,
    ) -> StorageResult<Vec<MemoryRegion>> {
        // If agent_id is provided, delegate to list_agent_accessible_regions
        // (which checks ACL expiry) then optionally post-filter by user_id.
        if let Some(aid) = agent_id {
            let mut regions = self.list_agent_accessible_regions(aid).await?;
            if let Some(uid) = user_id {
                regions.retain(|r| r.user_id == uid);
            }
            return Ok(regions);
        }

        // No agent_id — list by user or all.
        let idx = match user_id {
            Some(uid) => self.key(&["user_regions", &uid.to_string()]),
            None => self.key(&["regions"]),
        };
        let mut conn = self.conn.clone();
        let ids: Vec<String> = redis::cmd("ZRANGEBYSCORE")
            .arg(&idx)
            .arg("-inf")
            .arg("+inf")
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut regions = Vec::new();
        for id_str in &ids {
            if let Ok(id) = id_str.parse::<Uuid>() {
                if let Some(r) = self.get_region(id).await? {
                    regions.push(r);
                }
            }
        }
        Ok(regions)
    }

    async fn update_region(&self, region: &MemoryRegion) -> StorageResult<()> {
        let pk = self.key(&["region", &region.id.to_string()]);
        self.set_json(&pk, region).await
    }

    async fn delete_region(&self, id: Uuid) -> StorageResult<()> {
        // Phase 1: Read region + ACL agent list (non-atomic, needed for key names)
        let region = match self.get_region(id).await? {
            Some(r) => r,
            None => return Ok(()),
        };

        let acl_idx_key = self.key(&["region_acl", &id.to_string()]);
        let mut conn = self.conn.clone();
        let acl_agent_ids: Vec<String> = redis::cmd("ZRANGEBYSCORE")
            .arg(&acl_idx_key)
            .arg("-inf")
            .arg("+inf")
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        // Phase 2: Atomic delete of everything
        let pk = self.key(&["region", &id.to_string()]);
        let global_idx = self.key(&["regions"]);
        let user_idx = self.key(&["user_regions", &region.user_id.to_string()]);
        let owner_idx = self.key(&["agent_regions", &region.owner_agent_id]);
        let id_str = id.to_string();

        let mut pipe = redis::pipe();
        pipe.atomic()
            // Delete region document
            .del(&pk)
            .ignore()
            // Remove from global index
            .cmd("ZREM")
            .arg(&global_idx)
            .arg(&id_str)
            .ignore()
            // Remove from user index
            .cmd("ZREM")
            .arg(&user_idx)
            .arg(&id_str)
            .ignore()
            // Remove from owner reverse index
            .cmd("ZREM")
            .arg(&owner_idx)
            .arg(&id_str)
            .ignore();

        // Delete each ACL entry + remove from each agent's reverse index
        for agent_id in &acl_agent_ids {
            let acl_entry_key = self.key(&["region_acl_entry", &id_str, agent_id]);
            let agent_idx = self.key(&["agent_regions", agent_id]);
            pipe.del(&acl_entry_key)
                .ignore()
                .cmd("ZREM")
                .arg(&agent_idx)
                .arg(&id_str)
                .ignore();
        }

        // Delete the ACL index itself
        pipe.del(&acl_idx_key).ignore();

        let mut conn = self.conn.clone();
        pipe.exec_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn grant_region_access(&self, acl: &MemoryRegionAcl) -> StorageResult<()> {
        let acl_key = self.key(&[
            "region_acl_entry",
            &acl.region_id.to_string(),
            &acl.agent_id,
        ]);
        let json = serde_json::to_string(acl)?;
        let acl_idx = self.key(&["region_acl", &acl.region_id.to_string()]);
        let agent_idx = self.key(&["agent_regions", &acl.agent_id]);
        let score = acl.granted_at.timestamp_millis() as f64;
        let region_id_str = acl.region_id.to_string();

        // Atomic: JSON.SET + 2× ZADD (region ACL index + agent reverse index)
        let mut conn = self.conn.clone();
        redis::pipe()
            .atomic()
            .cmd("JSON.SET")
            .arg(&acl_key)
            .arg("$")
            .arg(&json)
            .ignore()
            .cmd("ZADD")
            .arg(&acl_idx)
            .arg(score)
            .arg(&acl.agent_id)
            .ignore()
            .cmd("ZADD")
            .arg(&agent_idx)
            .arg(score)
            .arg(&region_id_str)
            .ignore()
            .exec_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn list_region_acls(&self, region_id: Uuid) -> StorageResult<Vec<MemoryRegionAcl>> {
        let acl_idx = self.key(&["region_acl", &region_id.to_string()]);
        let mut conn = self.conn.clone();
        let agent_ids: Vec<String> = redis::cmd("ZRANGEBYSCORE")
            .arg(&acl_idx)
            .arg("-inf")
            .arg("+inf")
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut acls = Vec::new();
        for agent_id in &agent_ids {
            let acl_key = self.key(&["region_acl_entry", &region_id.to_string(), agent_id]);
            if let Some(acl) = self.get_json::<MemoryRegionAcl>(&acl_key).await? {
                acls.push(acl);
            }
        }
        Ok(acls)
    }

    async fn revoke_region_access(&self, region_id: Uuid, agent_id: &str) -> StorageResult<()> {
        let acl_key = self.key(&["region_acl_entry", &region_id.to_string(), agent_id]);
        let acl_idx = self.key(&["region_acl", &region_id.to_string()]);
        let agent_idx = self.key(&["agent_regions", agent_id]);
        let region_id_str = region_id.to_string();

        // Atomic: DEL + 2× ZREM (ACL index + agent reverse index)
        let mut conn = self.conn.clone();
        redis::pipe()
            .atomic()
            .del(&acl_key)
            .ignore()
            .cmd("ZREM")
            .arg(&acl_idx)
            .arg(agent_id)
            .ignore()
            .cmd("ZREM")
            .arg(&agent_idx)
            .arg(&region_id_str)
            .ignore()
            .exec_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn list_agent_accessible_regions(
        &self,
        agent_id: &str,
    ) -> StorageResult<Vec<MemoryRegion>> {
        let agent_idx = self.key(&["agent_regions", agent_id]);
        let mut conn = self.conn.clone();
        let region_ids: Vec<String> = redis::cmd("ZRANGEBYSCORE")
            .arg(&agent_idx)
            .arg("-inf")
            .arg("+inf")
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;

        let mut regions = Vec::new();
        // Collect expired ACL entries for lazy cleanup
        let mut expired_cleanup: Vec<(String, String)> = Vec::new();

        for id_str in &region_ids {
            if let Ok(id) = id_str.parse::<Uuid>() {
                let acl_key = self.key(&["region_acl_entry", id_str, agent_id]);
                if let Some(acl) = self.get_json::<MemoryRegionAcl>(&acl_key).await? {
                    if acl.is_expired() {
                        // Collect for lazy cleanup
                        expired_cleanup.push((id_str.clone(), agent_id.to_string()));
                    } else if let Some(region) = self.get_region(id).await? {
                        regions.push(region);
                    }
                } else {
                    // No ACL entry — might be the owner.
                    if let Some(region) = self.get_region(id).await? {
                        if region.owner_agent_id == agent_id {
                            regions.push(region);
                        }
                    }
                }
            }
        }

        // Lazy cleanup: remove expired ACL entries from indices (fire-and-forget)
        if !expired_cleanup.is_empty() {
            let mut pipe = redis::pipe();
            // Don't wrap cleanup in atomic — best-effort is fine
            for (region_id_str, aid) in &expired_cleanup {
                let acl_key = self.key(&["region_acl_entry", region_id_str, &aid]);
                let acl_idx = self.key(&["region_acl", region_id_str]);
                let agent_rev_idx = self.key(&["agent_regions", &aid]);
                pipe.del(&acl_key)
                    .ignore()
                    .cmd("ZREM")
                    .arg(&acl_idx)
                    .arg(aid.as_str())
                    .ignore()
                    .cmd("ZREM")
                    .arg(&agent_rev_idx)
                    .arg(region_id_str.as_str())
                    .ignore();
            }
            let mut conn = self.conn.clone();
            // Best-effort — ignore errors from cleanup
            let _ = pipe.exec_async(&mut conn).await;
        }

        Ok(regions)
    }
}
