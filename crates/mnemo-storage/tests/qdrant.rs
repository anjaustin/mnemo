//! Integration tests for QdrantVectorStore.
//!
//! These tests require a real Qdrant instance at MNEMO_TEST_QDRANT_URL
//! (defaults to http://localhost:6334).
//!
//! Run:
//!   docker compose up -d qdrant
//!   cargo test -p mnemo-storage --test qdrant
//!
//! Covers: ST-08 (upsert+search roundtrip), ST-09 (tenant isolation),
//!         ST-10 (delete_user_vectors removes all collections).
//!
//! Note: All tests share a single QdrantVectorStore (same prefix/collections)
//! to avoid exhausting Qdrant's file descriptors with too many collections.
//! Tests use unique user_id/entity_id UUIDs for isolation.

use uuid::Uuid;

use mnemo_core::traits::storage::{RawVectorStore, VectorStore};
use mnemo_storage::QdrantVectorStore;

/// Dimensions for test vectors — small to keep tests fast.
const TEST_DIMS: u32 = 4;

/// Connect to Qdrant. All tests share one store instance with a fixed prefix
/// per test binary invocation to avoid creating too many collections.
async fn get_qdrant_store() -> QdrantVectorStore {
    let url = std::env::var("MNEMO_TEST_QDRANT_URL")
        .unwrap_or_else(|_| "http://localhost:6334".to_string());
    // Use a fixed short prefix — tests isolate via unique user_id UUIDs, not
    // separate collections.  This keeps the Qdrant collection count at 3.
    let prefix = "qtest_";
    match QdrantVectorStore::new(&url, prefix, TEST_DIMS).await {
        Ok(store) => store,
        Err(e) => {
            eprintln!(
                "Skipping Qdrant integration test (Qdrant not available): {}",
                e
            );
            panic!("Qdrant required for integration tests. Run: docker compose up -d qdrant");
        }
    }
}

/// Create a normalized test vector. Uses a simple pattern so cosine similarity is predictable.
fn test_vector(seed: f32) -> Vec<f32> {
    let v = vec![seed, seed + 0.1, seed + 0.2, seed + 0.3];
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.into_iter().map(|x| x / norm).collect()
}

// ── ST-08: Qdrant upsert + search roundtrip ───────────────────────

#[tokio::test]
async fn test_qdrant_upsert_and_search_roundtrip() {
    let store = get_qdrant_store().await;
    let user_id = Uuid::now_v7();
    let entity_id = Uuid::now_v7();
    let embedding = test_vector(1.0);

    // Upsert an entity embedding
    store
        .upsert_entity_embedding(
            entity_id,
            user_id,
            embedding.clone(),
            serde_json::json!({"name": "TestEntity"}),
        )
        .await
        .expect("upsert should succeed");

    // Search with the same vector — should find it
    let results = store
        .search_entities(user_id, embedding.clone(), 10, 0.0)
        .await
        .expect("search should succeed");

    assert!(
        !results.is_empty(),
        "search with identical vector should return at least one result"
    );

    // The top result should be our entity
    let (found_id, score) = &results[0];
    assert_eq!(
        *found_id, entity_id,
        "top result should match the upserted entity ID"
    );
    assert!(
        *score > 0.99,
        "identical vector should have near-perfect cosine similarity, got {}",
        score
    );
}

// ── ST-08b: Edge and episode upsert + search roundtrip ────────────

#[tokio::test]
async fn test_qdrant_edge_and_episode_roundtrip() {
    let store = get_qdrant_store().await;
    let user_id = Uuid::now_v7();
    let edge_id = Uuid::now_v7();
    let episode_id = Uuid::now_v7();
    let edge_vec = test_vector(2.0);
    let episode_vec = test_vector(3.0);

    // Upsert edge embedding
    store
        .upsert_edge_embedding(
            edge_id,
            user_id,
            edge_vec.clone(),
            serde_json::json!({"fact": "Alice works at Acme"}),
        )
        .await
        .expect("edge upsert should succeed");

    // Upsert episode embedding
    store
        .upsert_episode_embedding(
            episode_id,
            user_id,
            episode_vec.clone(),
            serde_json::json!({"content": "Today I talked to Alice"}),
        )
        .await
        .expect("episode upsert should succeed");

    // Search edges
    let edge_results = store
        .search_edges(user_id, edge_vec.clone(), 10, 0.0)
        .await
        .expect("edge search should succeed");
    assert!(!edge_results.is_empty());
    assert_eq!(edge_results[0].0, edge_id);

    // Search episodes
    let ep_results = store
        .search_episodes(user_id, episode_vec.clone(), 10, 0.0)
        .await
        .expect("episode search should succeed");
    assert!(!ep_results.is_empty());
    assert_eq!(ep_results[0].0, episode_id);
}

// ── ST-09: Tenant isolation via user_id filter ────────────────────

#[tokio::test]
async fn test_qdrant_tenant_isolation() {
    let store = get_qdrant_store().await;

    let user_a = Uuid::now_v7();
    let user_b = Uuid::now_v7();
    let entity_a = Uuid::now_v7();
    let entity_b = Uuid::now_v7();

    let vec_a = test_vector(1.0);
    let vec_b = test_vector(1.5); // slightly different but similar

    // Upsert for user A
    store
        .upsert_entity_embedding(
            entity_a,
            user_a,
            vec_a.clone(),
            serde_json::json!({"name": "A's entity"}),
        )
        .await
        .unwrap();

    // Upsert for user B
    store
        .upsert_entity_embedding(
            entity_b,
            user_b,
            vec_b.clone(),
            serde_json::json!({"name": "B's entity"}),
        )
        .await
        .unwrap();

    // Search as user A — should only find A's entity
    let results_a = store
        .search_entities(user_a, vec_a.clone(), 10, 0.0)
        .await
        .unwrap();

    let found_ids_a: Vec<Uuid> = results_a.iter().map(|(id, _)| *id).collect();
    assert!(
        found_ids_a.contains(&entity_a),
        "user A should find their own entity"
    );
    assert!(
        !found_ids_a.contains(&entity_b),
        "user A must NOT see user B's entity — tenant isolation violated"
    );

    // Search as user B — should only find B's entity
    let results_b = store
        .search_entities(user_b, vec_b.clone(), 10, 0.0)
        .await
        .unwrap();

    let found_ids_b: Vec<Uuid> = results_b.iter().map(|(id, _)| *id).collect();
    assert!(
        found_ids_b.contains(&entity_b),
        "user B should find their own entity"
    );
    assert!(
        !found_ids_b.contains(&entity_a),
        "user B must NOT see user A's entity — tenant isolation violated"
    );
}

// ── ST-10: delete_user_vectors removes from all collections ───────

#[tokio::test]
async fn test_qdrant_delete_user_vectors() {
    let store = get_qdrant_store().await;
    let user_id = Uuid::now_v7();

    let entity_id = Uuid::now_v7();
    let edge_id = Uuid::now_v7();
    let episode_id = Uuid::now_v7();

    let vec1 = test_vector(1.0);
    let vec2 = test_vector(2.0);
    let vec3 = test_vector(3.0);

    // Upsert across all 3 collections
    store
        .upsert_entity_embedding(entity_id, user_id, vec1.clone(), serde_json::json!({}))
        .await
        .unwrap();
    store
        .upsert_edge_embedding(edge_id, user_id, vec2.clone(), serde_json::json!({}))
        .await
        .unwrap();
    store
        .upsert_episode_embedding(episode_id, user_id, vec3.clone(), serde_json::json!({}))
        .await
        .unwrap();

    // Verify they exist
    let entities = store
        .search_entities(user_id, vec1.clone(), 10, 0.0)
        .await
        .unwrap();
    assert!(!entities.is_empty(), "entity should exist before delete");

    let edges = store
        .search_edges(user_id, vec2.clone(), 10, 0.0)
        .await
        .unwrap();
    assert!(!edges.is_empty(), "edge should exist before delete");

    let episodes = store
        .search_episodes(user_id, vec3.clone(), 10, 0.0)
        .await
        .unwrap();
    assert!(!episodes.is_empty(), "episode should exist before delete");

    // GDPR hard delete
    store
        .delete_user_vectors(user_id)
        .await
        .expect("delete_user_vectors should succeed");

    // Verify all gone
    let entities_after = store
        .search_entities(user_id, vec1.clone(), 10, 0.0)
        .await
        .unwrap();
    assert!(
        entities_after.is_empty(),
        "entity vectors should be gone after delete_user_vectors"
    );

    let edges_after = store
        .search_edges(user_id, vec2.clone(), 10, 0.0)
        .await
        .unwrap();
    assert!(
        edges_after.is_empty(),
        "edge vectors should be gone after delete_user_vectors"
    );

    let episodes_after = store
        .search_episodes(user_id, vec3.clone(), 10, 0.0)
        .await
        .unwrap();
    assert!(
        episodes_after.is_empty(),
        "episode vectors should be gone after delete_user_vectors"
    );
}

// ── ST-10b: delete_user_vectors doesn't affect other users ────────

#[tokio::test]
async fn test_qdrant_delete_preserves_other_users() {
    let store = get_qdrant_store().await;

    let user_delete = Uuid::now_v7();
    let user_keep = Uuid::now_v7();

    let entity_delete = Uuid::now_v7();
    let entity_keep = Uuid::now_v7();

    let vec1 = test_vector(1.0);
    let vec2 = test_vector(1.5);

    // Upsert for both users
    store
        .upsert_entity_embedding(
            entity_delete,
            user_delete,
            vec1.clone(),
            serde_json::json!({}),
        )
        .await
        .unwrap();
    store
        .upsert_entity_embedding(entity_keep, user_keep, vec2.clone(), serde_json::json!({}))
        .await
        .unwrap();

    // Delete one user
    store.delete_user_vectors(user_delete).await.unwrap();

    // Deleted user should have no results
    let gone = store
        .search_entities(user_delete, vec1.clone(), 10, 0.0)
        .await
        .unwrap();
    assert!(gone.is_empty(), "deleted user's vectors should be gone");

    // Other user's data should be intact
    let kept = store
        .search_entities(user_keep, vec2.clone(), 10, 0.0)
        .await
        .unwrap();
    assert!(
        !kept.is_empty(),
        "other user's vectors must survive a different user's deletion"
    );
    assert_eq!(kept[0].0, entity_keep);
}

// ── RawVectorStore: namespace CRUD roundtrip ──────────────────────

#[tokio::test]
async fn test_qdrant_raw_namespace_lifecycle() {
    let store = get_qdrant_store().await;
    let ns = format!("ns{}", &Uuid::now_v7().to_string()[..8]);

    // Namespace doesn't exist yet
    assert!(!store.has_namespace(&ns).await.unwrap());

    // Ensure creates it
    store.ensure_namespace(&ns, TEST_DIMS).await.unwrap();
    assert!(store.has_namespace(&ns).await.unwrap());

    // Upsert vectors
    let vectors = vec![
        (
            "doc1".to_string(),
            test_vector(1.0),
            serde_json::json!({"title": "First"}),
        ),
        (
            "doc2".to_string(),
            test_vector(2.0),
            serde_json::json!({"title": "Second"}),
        ),
    ];
    store.upsert_vectors(&ns, vectors).await.unwrap();

    // Count
    let count = store.count_vectors(&ns).await.unwrap();
    assert_eq!(count, 2, "should have 2 vectors after upsert");

    // Search
    let hits = store
        .search_vectors(&ns, test_vector(1.0), 10, 0.0)
        .await
        .unwrap();
    assert!(!hits.is_empty(), "search should return results");
    assert_eq!(
        hits[0].id, "doc1",
        "top hit should be the most similar vector"
    );

    // Delete specific vector
    store
        .delete_vectors(&ns, vec!["doc1".to_string()])
        .await
        .unwrap();
    let count_after = store.count_vectors(&ns).await.unwrap();
    assert_eq!(count_after, 1, "should have 1 vector after deleting doc1");

    // Delete namespace
    store.delete_namespace(&ns).await.unwrap();
    assert!(
        !store.has_namespace(&ns).await.unwrap(),
        "namespace should be gone"
    );
}
