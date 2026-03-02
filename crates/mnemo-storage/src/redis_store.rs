use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};
use serde::de::DeserializeOwned;
use serde::Serialize;
use uuid::Uuid;

use mnemo_core::error::MnemoError;
use mnemo_core::models::{
    edge::{Edge, EdgeFilter},
    entity::Entity,
    episode::{CreateEpisodeRequest, Episode, ListEpisodesParams, ProcessingStatus},
    session::{CreateSessionRequest, ListSessionsParams, Session, UpdateSessionRequest},
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
/// {prefix}adj_out:{entity_id}         → Sorted Set (score=valid_at, member=edge_id)
/// {prefix}adj_in:{entity_id}          → Sorted Set (score=valid_at, member=edge_id)
/// {prefix}user_edges:{user_id}        → Sorted Set (score=timestamp, member=edge_id)
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
        redis::cmd("SET")
            .arg(key)
            .arg(&json)
            .exec_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;
        Ok(())
    }

    async fn get_json<T: DeserializeOwned>(&self, key: &str) -> StorageResult<Option<T>> {
        let mut conn = self.conn.clone();
        let result: Option<String> = conn
            .get(key)
            .await
            .map_err(|e| MnemoError::Redis(e.to_string()))?;
        match result {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    async fn get_json_required<T: DeserializeOwned>(
        &self,
        key: &str,
        not_found_err: MnemoError,
    ) -> StorageResult<T> {
        self.get_json(key)
            .await?
            .ok_or(not_found_err)
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
            let cursor_key = format!("{}{}", item_prefix, cursor_id);
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
}

// ─── UserStore ─────────────────────────────────────────────────────

impl UserStore for RedisStateStore {
    async fn create_user(&self, req: CreateUserRequest) -> StorageResult<User> {
        let user = User::from_request(req);
        let key = self.key(&["user", &user.id.to_string()]);

        // Check for duplicate
        if self.get_json::<User>(&key).await?.is_some() {
            return Err(MnemoError::Duplicate(format!("User {} already exists", user.id)));
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
        self.get_json_required(&key, MnemoError::UserNotFound(id)).await
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
        self.get_json_required(&key, MnemoError::SessionNotFound(id)).await
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
        self.list_from_zset(&zset_key, &prefix, params.limit, params.after).await
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
        session.record_episode();
        let sess_key = self.key(&["session", &session_id.to_string()]);
        self.set_json(&sess_key, &session).await?;

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
        self.get_json_required(&key, MnemoError::EpisodeNotFound(id)).await
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
        self.list_from_zset(&zset_key, &prefix, params.limit, params.after).await
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
        self.get_json_required(&key, MnemoError::EntityNotFound(id)).await
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
        let name_key = self.key(&[
            "entity_name",
            &user_id.to_string(),
            &name.to_lowercase(),
        ]);
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
        self.get_json_required(&key, MnemoError::EdgeNotFound(id)).await
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
        let candidates: Vec<Edge> = self
            .list_from_zset(&zset_key, &prefix, filter.limit * 3, None)
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
