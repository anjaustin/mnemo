//! Mnemo latency benchmarks.
//!
//! Prerequisites:
//!   docker compose up -d redis qdrant
//!
//! Run:
//!   cargo bench

use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use tokio::runtime::Runtime;
use uuid::Uuid;

use mnemo_core::models::episode::{CreateEpisodeRequest, EpisodeType, MessageRole};
use mnemo_core::models::session::CreateSessionRequest;
use mnemo_core::models::user::CreateUserRequest;
use mnemo_core::traits::storage::*;

use mnemo_storage::RedisStateStore;

fn create_runtime() -> Runtime {
    Runtime::new().unwrap()
}

async fn setup_store() -> (RedisStateStore, Uuid, Uuid) {
    let prefix = format!("bench:{}:", Uuid::now_v7());
    let store = RedisStateStore::new("redis://localhost:6379", &prefix)
        .await
        .expect("Redis not available for benchmarks");

    let user = store.create_user(CreateUserRequest {
        id: None, name: "Bench User".into(), email: None, external_id: None,
        metadata: serde_json::json!({}),
    }).await.unwrap();

    let session = store.create_session(CreateSessionRequest {
        id: None, user_id: user.id, name: None, metadata: serde_json::json!({}),
    }).await.unwrap();

    (store, user.id, session.id)
}

fn bench_episode_ingestion(c: &mut Criterion) {
    let rt = create_runtime();
    let (store, user_id, session_id) = rt.block_on(setup_store());

    c.bench_function("episode_ingestion", |b| {
        b.iter(|| {
            rt.block_on(async {
                store.create_episode(
                    CreateEpisodeRequest {
                        id: None,
                        episode_type: EpisodeType::Message,
                        content: "I love hiking in the Rockies on weekends with my dog Luna".into(),
                        role: Some(MessageRole::User),
                        name: Some("Bench User".into()),
                        metadata: serde_json::json!({}),
                        created_at: None,
                    },
                    session_id,
                    user_id,
                ).await.unwrap()
            })
        })
    });
}

fn bench_user_crud(c: &mut Criterion) {
    let rt = create_runtime();
    let (store, _, _) = rt.block_on(setup_store());

    c.bench_function("user_create", |b| {
        b.iter(|| {
            rt.block_on(async {
                store.create_user(CreateUserRequest {
                    id: None, name: "Bench".into(), email: None, external_id: None,
                    metadata: serde_json::json!({}),
                }).await.unwrap()
            })
        })
    });
}

fn bench_entity_lookup(c: &mut Criterion) {
    let rt = create_runtime();
    let (store, user_id, _) = rt.block_on(setup_store());

    // Seed some entities
    rt.block_on(async {
        for i in 0..100 {
            let entity = mnemo_core::models::entity::Entity::from_extraction(
                &mnemo_core::models::entity::ExtractedEntity {
                    name: format!("Entity_{}", i),
                    entity_type: mnemo_core::models::entity::EntityType::Person,
                    summary: Some(format!("Description of entity {}", i)),
                },
                user_id,
                Uuid::now_v7(),
            );
            store.create_entity(entity).await.unwrap();
        }
    });

    c.bench_function("entity_find_by_name", |b| {
        b.iter(|| {
            rt.block_on(async {
                store.find_entity_by_name(user_id, "Entity_50").await.unwrap()
            })
        })
    });

    c.bench_function("entity_list_100", |b| {
        b.iter(|| {
            rt.block_on(async {
                store.list_entities(user_id, 100, None).await.unwrap()
            })
        })
    });
}

criterion_group!(benches, bench_episode_ingestion, bench_user_crud, bench_entity_lookup);
criterion_main!(benches);
