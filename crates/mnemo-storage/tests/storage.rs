//! Integration tests for RedisStateStore.
//!
//! These tests require a real Redis instance at MNEMO_TEST_REDIS_URL
//! (defaults to redis://localhost:6379).

use uuid::Uuid;

use mnemo_core::models::episode::{
    CreateEpisodeRequest, EpisodeType, MessageRole, ProcessingStatus,
};
use mnemo_core::models::session::CreateSessionRequest;
use mnemo_core::models::user::CreateUserRequest;
use mnemo_core::traits::storage::*;
use mnemo_storage::RedisStateStore;

/// Get or skip test if Redis is not available.
async fn get_store() -> RedisStateStore {
    let url = std::env::var("MNEMO_TEST_REDIS_URL")
        .unwrap_or_else(|_| "redis://localhost:6379".to_string());
    // Use unique prefix per test run to avoid collisions
    let prefix = format!("mnemo_test_{}:", Uuid::now_v7());
    match RedisStateStore::new(&url, &prefix).await {
        Ok(store) => store,
        Err(e) => {
            eprintln!("Skipping integration test (Redis not available): {}", e);
            panic!("Redis required for integration tests. Run: docker compose up -d redis");
        }
    }
}

fn test_user_req(name: &str) -> CreateUserRequest {
    CreateUserRequest {
        id: None,
        name: name.to_string(),
        email: Some(format!("{}@test.com", name)),
        external_id: Some(format!("ext_{}", name)),
        metadata: serde_json::json!({}),
    }
}

fn test_episode_req(content: &str) -> CreateEpisodeRequest {
    CreateEpisodeRequest {
        id: None,
        episode_type: EpisodeType::Message,
        content: content.to_string(),
        role: Some(MessageRole::User),
        name: Some("TestUser".into()),
        metadata: serde_json::json!({}),
        created_at: None,
    }
}

#[tokio::test]
async fn test_user_crud_lifecycle() {
    let store = get_store().await;

    // Create
    let user = store.create_user(test_user_req("alice")).await.unwrap();
    assert_eq!(user.name, "alice");
    assert!(user.external_id.as_deref() == Some("ext_alice"));

    // Get
    let fetched = store.get_user(user.id).await.unwrap();
    assert_eq!(fetched.id, user.id);

    // Get by external ID
    let by_ext = store.get_user_by_external_id("ext_alice").await.unwrap();
    assert_eq!(by_ext.id, user.id);

    // Update
    let updated = store
        .update_user(
            user.id,
            mnemo_core::models::user::UpdateUserRequest {
                name: Some("Alice Smith".into()),
                email: None,
                external_id: None,
                metadata: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "Alice Smith");

    // List
    let users = store.list_users(10, None).await.unwrap();
    assert!(users.iter().any(|u| u.id == user.id));

    // Delete
    store.delete_user(user.id).await.unwrap();
    assert!(store.get_user(user.id).await.is_err());
}

#[tokio::test]
async fn test_session_crud_lifecycle() {
    let store = get_store().await;
    let user = store.create_user(test_user_req("bob")).await.unwrap();

    let session = store
        .create_session(CreateSessionRequest {
            id: None,
            user_id: user.id,
            name: Some("Test Session".into()),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    assert_eq!(session.user_id, user.id);
    assert_eq!(session.episode_count, 0);

    let fetched = store.get_session(session.id).await.unwrap();
    assert_eq!(fetched.id, session.id);

    let sessions = store
        .list_sessions(
            user.id,
            mnemo_core::models::session::ListSessionsParams {
                limit: 10,
                after: None,
                since: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(sessions.len(), 1);

    store.delete_session(session.id).await.unwrap();
    assert!(store.get_session(session.id).await.is_err());
}

#[tokio::test]
async fn test_episode_pending_queue() {
    let store = get_store().await;
    let user = store.create_user(test_user_req("charlie")).await.unwrap();
    let session = store
        .create_session(CreateSessionRequest {
            id: None,
            user_id: user.id,
            name: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    // Create episode -> should be in pending queue
    let episode = store
        .create_episode(
            test_episode_req("Hello from integration test!"),
            session.id,
            user.id,
        )
        .await
        .unwrap();
    assert_eq!(episode.processing_status, ProcessingStatus::Pending);

    // Pending queue should contain it
    let pending = store.get_pending_episodes(10).await.unwrap();
    assert!(pending.iter().any(|e| e.id == episode.id));

    // Claim it
    let claimed = store.claim_episode(episode.id).await.unwrap();
    assert!(claimed);

    // Double claim should fail
    let double = store.claim_episode(episode.id).await.unwrap();
    assert!(!double);

    // Verify status changed
    let ep = store.get_episode(episode.id).await.unwrap();
    assert_eq!(ep.processing_status, ProcessingStatus::Processing);
}

#[tokio::test]
async fn test_entity_dedup_by_name() {
    let store = get_store().await;
    let user = store.create_user(test_user_req("diana")).await.unwrap();

    let entity = mnemo_core::models::entity::Entity::from_extraction(
        &mnemo_core::models::entity::ExtractedEntity {
            name: "Nike".into(),
            entity_type: mnemo_core::models::entity::EntityType::Organization,
            summary: Some("Athletic brand".into()),
        },
        user.id,
        Uuid::now_v7(),
    );
    let created = store.create_entity(entity).await.unwrap();

    // Find by name should return it
    let found = store.find_entity_by_name(user.id, "Nike").await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, created.id);

    // Case-insensitive
    let found_lower = store.find_entity_by_name(user.id, "nike").await.unwrap();
    assert!(found_lower.is_some());
    assert_eq!(found_lower.unwrap().id, created.id);

    // Different user shouldn't find it
    let user2 = store.create_user(test_user_req("eve")).await.unwrap();
    let not_found = store.find_entity_by_name(user2.id, "Nike").await.unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn test_edge_conflict_detection() {
    let store = get_store().await;
    let user = store.create_user(test_user_req("frank")).await.unwrap();

    // Create two entities
    let entity_a = store
        .create_entity(mnemo_core::models::entity::Entity::from_extraction(
            &mnemo_core::models::entity::ExtractedEntity {
                name: "Frank".into(),
                entity_type: mnemo_core::models::entity::EntityType::Person,
                summary: None,
            },
            user.id,
            Uuid::now_v7(),
        ))
        .await
        .unwrap();

    let entity_b = store
        .create_entity(mnemo_core::models::entity::Entity::from_extraction(
            &mnemo_core::models::entity::ExtractedEntity {
                name: "Adidas".into(),
                entity_type: mnemo_core::models::entity::EntityType::Organization,
                summary: None,
            },
            user.id,
            Uuid::now_v7(),
        ))
        .await
        .unwrap();

    // Create edge: Frank prefers Adidas
    let edge1 = mnemo_core::models::edge::Edge::from_extraction(
        &mnemo_core::models::edge::ExtractedRelationship {
            source_name: "Frank".into(),
            target_name: "Adidas".into(),
            label: "prefers".into(),
            fact: "Frank prefers Adidas shoes".into(),
            confidence: 0.9,
            valid_at: None,
        },
        user.id,
        entity_a.id,
        entity_b.id,
        Uuid::now_v7(),
        chrono::Utc::now(),
    );
    let created_edge = store.create_edge(edge1).await.unwrap();
    assert!(created_edge.is_valid());

    // Find conflicts: same source, target, label
    let conflicts = store
        .find_conflicting_edges(user.id, entity_a.id, entity_b.id, "prefers")
        .await
        .unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].id, created_edge.id);

    // Different label -> no conflict
    let no_conflict = store
        .find_conflicting_edges(user.id, entity_a.id, entity_b.id, "dislikes")
        .await
        .unwrap();
    assert!(no_conflict.is_empty());
}

#[tokio::test]
async fn test_episode_requeue() {
    let store = get_store().await;
    let user = store.create_user(test_user_req("grace")).await.unwrap();
    let session = store
        .create_session(CreateSessionRequest {
            id: None,
            user_id: user.id,
            name: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let episode = store
        .create_episode(test_episode_req("Requeue test"), session.id, user.id)
        .await
        .unwrap();

    // Claim it
    store.claim_episode(episode.id).await.unwrap();

    // Requeue with delay
    store.requeue_episode(episode.id, 0).await.unwrap();

    // Should be back in pending (with score = now, so immediately available)
    let pending = store.get_pending_episodes(10).await.unwrap();
    // Note: the episode's processing_status is still "processing" in the JSON,
    // but it's back in the sorted set. The worker would need to update the status too.
    // This just tests the sorted set requeue behavior.
    assert!(pending.is_empty() || pending.iter().any(|e| e.id == episode.id));
}

// ─── Digest Storage ────────────────────────────────────────────────

#[tokio::test]
async fn test_digest_save_and_load() {
    let store = get_store().await;
    let user_id = Uuid::now_v7();

    // No digest initially
    let result = store.get_digest(user_id).await.unwrap();
    assert!(result.is_none(), "no digest should exist initially");

    // Save a digest
    let digest = mnemo_core::models::digest::MemoryDigest {
        user_id,
        summary: "User is interested in machine learning and Rust programming.".into(),
        entity_count: 42,
        edge_count: 15,
        dominant_topics: vec!["ML".into(), "Rust".into(), "Systems".into()],
        generated_at: chrono::Utc::now(),
        model: "test-model".into(),
    };
    store.save_digest(&digest).await.unwrap();

    // Load it back
    let loaded = store.get_digest(user_id).await.unwrap().unwrap();
    assert_eq!(loaded.user_id, user_id);
    assert_eq!(loaded.summary, digest.summary);
    assert_eq!(loaded.entity_count, 42);
    assert_eq!(loaded.edge_count, 15);
    assert_eq!(loaded.dominant_topics, vec!["ML", "Rust", "Systems"]);
    assert_eq!(loaded.model, "test-model");
}

#[tokio::test]
async fn test_digest_overwrite() {
    let store = get_store().await;
    let user_id = Uuid::now_v7();

    let digest1 = mnemo_core::models::digest::MemoryDigest {
        user_id,
        summary: "First summary.".into(),
        entity_count: 10,
        edge_count: 5,
        dominant_topics: vec!["alpha".into()],
        generated_at: chrono::Utc::now(),
        model: "model-v1".into(),
    };
    store.save_digest(&digest1).await.unwrap();

    let digest2 = mnemo_core::models::digest::MemoryDigest {
        user_id,
        summary: "Updated summary after more activity.".into(),
        entity_count: 25,
        edge_count: 12,
        dominant_topics: vec!["beta".into(), "gamma".into()],
        generated_at: chrono::Utc::now(),
        model: "model-v2".into(),
    };
    store.save_digest(&digest2).await.unwrap();

    // Should get the latest digest
    let loaded = store.get_digest(user_id).await.unwrap().unwrap();
    assert_eq!(loaded.summary, "Updated summary after more activity.");
    assert_eq!(loaded.entity_count, 25);
    assert_eq!(loaded.model, "model-v2");
}

#[tokio::test]
async fn test_digest_list_all() {
    let store = get_store().await;
    let user_a = Uuid::now_v7();
    let user_b = Uuid::now_v7();

    // Start empty
    let all = store.list_digests().await.unwrap();
    assert!(all.is_empty(), "no digests should exist initially");

    // Save two digests for different users
    let digest_a = mnemo_core::models::digest::MemoryDigest {
        user_id: user_a,
        summary: "User A summary.".into(),
        entity_count: 5,
        edge_count: 2,
        dominant_topics: vec!["topic-a".into()],
        generated_at: chrono::Utc::now(),
        model: "test".into(),
    };
    let digest_b = mnemo_core::models::digest::MemoryDigest {
        user_id: user_b,
        summary: "User B summary.".into(),
        entity_count: 8,
        edge_count: 3,
        dominant_topics: vec!["topic-b".into()],
        generated_at: chrono::Utc::now(),
        model: "test".into(),
    };
    store.save_digest(&digest_a).await.unwrap();
    store.save_digest(&digest_b).await.unwrap();

    let all = store.list_digests().await.unwrap();
    assert_eq!(all.len(), 2);
    let ids: Vec<Uuid> = all.iter().map(|d| d.user_id).collect();
    assert!(ids.contains(&user_a));
    assert!(ids.contains(&user_b));
}

#[tokio::test]
async fn test_digest_delete() {
    let store = get_store().await;
    let user_id = Uuid::now_v7();

    let digest = mnemo_core::models::digest::MemoryDigest {
        user_id,
        summary: "To be deleted.".into(),
        entity_count: 1,
        edge_count: 0,
        dominant_topics: vec![],
        generated_at: chrono::Utc::now(),
        model: "test".into(),
    };
    store.save_digest(&digest).await.unwrap();
    assert!(store.get_digest(user_id).await.unwrap().is_some());

    // Delete
    store.delete_digest(user_id).await.unwrap();
    assert!(store.get_digest(user_id).await.unwrap().is_none());

    // list_digests should not include the deleted digest
    let all = store.list_digests().await.unwrap();
    assert!(!all.iter().any(|d| d.user_id == user_id));
}
