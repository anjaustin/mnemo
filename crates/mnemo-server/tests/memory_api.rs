use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use chrono::Utc;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

use mnemo_core::models::edge::{Edge, ExtractedRelationship};
use mnemo_core::models::entity::{Entity, EntityType, ExtractedEntity};
use mnemo_core::traits::fulltext::FullTextStore;
use mnemo_core::traits::llm::EmbeddingConfig;
use mnemo_core::traits::storage::{EdgeStore, EntityStore};
use mnemo_graph::GraphEngine;
use mnemo_llm::OpenAiCompatibleEmbedder;
use mnemo_retrieval::RetrievalEngine;
use mnemo_server::routes::build_router;
use mnemo_server::state::{AppState, MetadataPrefilterConfig};
use mnemo_storage::{QdrantVectorStore, RedisStateStore};

async fn build_test_app() -> axum::Router {
    build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await
    .0
}

async fn build_test_app_with_prefilter(prefilter: MetadataPrefilterConfig) -> axum::Router {
    build_test_harness_with_prefilter(prefilter).await.0
}

async fn build_test_harness_with_prefilter(
    prefilter: MetadataPrefilterConfig,
) -> (axum::Router, Arc<RedisStateStore>) {
    let redis_url = std::env::var("MNEMO_TEST_REDIS_URL")
        .unwrap_or_else(|_| "redis://localhost:6379".to_string());
    let qdrant_url = std::env::var("MNEMO_TEST_QDRANT_URL")
        .unwrap_or_else(|_| "http://localhost:6334".to_string());

    let uid = Uuid::now_v7();
    let redis_prefix = format!("memory_api_test:{}:", uid);
    let qdrant_prefix =
        std::env::var("MNEMO_TEST_QDRANT_PREFIX").unwrap_or_else(|_| "mnemo_".to_string());

    let state_store = Arc::new(
        RedisStateStore::new(&redis_url, &redis_prefix)
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "Redis required for memory API tests. Run: docker compose -f docker-compose.test.yml up -d. Error: {e}"
                )
            }),
    );
    state_store.ensure_indexes().await.unwrap();

    let vector_store = Arc::new(
        QdrantVectorStore::new(&qdrant_url, &qdrant_prefix, 1536)
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "Qdrant required for memory API tests. Run: docker compose -f docker-compose.test.yml up -d. Error: {e}"
                )
            }),
    );

    let embedder = Arc::new(OpenAiCompatibleEmbedder::new(EmbeddingConfig {
        provider: "openai".to_string(),
        api_key: None,
        model: "text-embedding-3-small".to_string(),
        base_url: None,
        dimensions: 1536,
    }));

    let retrieval = Arc::new(RetrievalEngine::new(
        state_store.clone(),
        vector_store.clone(),
        embedder,
    ));
    let graph = Arc::new(GraphEngine::new(state_store.clone()));

    let app = build_router(AppState {
        state_store: state_store.clone(),
        vector_store,
        retrieval,
        graph,
        metadata_prefilter: prefilter,
        import_jobs: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        import_idempotency: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
    });

    (app, state_store.clone())
}

async fn json_request(
    app: &axum::Router,
    method: &str,
    path: &str,
    payload: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed = if body.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice::<Value>(&body).unwrap()
    };
    (status, parsed)
}

async fn wait_for_import_job(app: &axum::Router, job_id: &str) -> Value {
    for _ in 0..80 {
        let (status, job) = json_request(
            app,
            "GET",
            &format!("/api/v1/import/jobs/{job_id}"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        if job["status"] == "completed" || job["status"] == "failed" {
            return job;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    panic!("import job {job_id} did not reach terminal state in time");
}

#[tokio::test]
async fn test_memory_api_validation_and_resolution() {
    let app = build_test_app().await;

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({"user": "   ", "text": "hello"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "validation_error");

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({"user": "falsify-user", "text": "   "}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "validation_error");

    let (status, first) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({"user": "falsify-user", "text": "first fact", "session": "   "}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = first["user_id"].as_str().unwrap().to_string();
    let session_id = first["session_id"].as_str().unwrap().to_string();

    let (status, second) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({"user": "falsify-user", "text": "second fact"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(second["session_id"], session_id);

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({"user": "falsify-user", "text": "assistant says hi", "role": "assistant"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, episodes) = json_request(
        &app,
        "GET",
        &format!("/api/v1/sessions/{session_id}/episodes?limit=10"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let roles: Vec<String> = episodes["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["role"].as_str().map(|r| r.to_string()))
        .collect();
    assert!(roles.contains(&"assistant".to_string()));
    assert!(roles.contains(&"user".to_string()));

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory/no-such-user/context",
        serde_json::json!({"query": "hi"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");

    let (status, body) = json_request(
        &app,
        "POST",
        &format!("/api/v1/memory/{user_id}/context"),
        serde_json::json!({"query": "what do i know?"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("context").is_some());
}

#[tokio::test]
async fn test_memory_changes_since_reports_episode_and_head_changes() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "diff-user",
            "external_id": "diff-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "diff-session" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "older event",
            "created_at": "2025-01-01T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "newer event",
            "created_at": "2025-03-01T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, diff) = json_request(
        &app,
        "POST",
        "/api/v1/memory/diff-user/changes_since",
        serde_json::json!({
            "from": "2025-02-01T00:00:00Z",
            "to": "2025-04-01T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!diff["added_episodes"].as_array().unwrap().is_empty());
    assert!(!diff["head_changes"].as_array().unwrap().is_empty());
    assert!(diff["summary"]
        .as_str()
        .unwrap_or_default()
        .contains("added episodes"));
}

#[tokio::test]
async fn test_memory_changes_since_rejects_invalid_window() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory/nope/changes_since",
        serde_json::json!({
            "from": "2025-04-01T00:00:00Z",
            "to": "2025-01-01T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_conflict_radar_detects_active_fact_conflict() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "conflict-user",
            "external_id": "conflict-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = Uuid::parse_str(user["id"].as_str().unwrap()).unwrap();

    let episode_id = Uuid::now_v7();
    let src = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Kendra".to_string(),
                entity_type: EntityType::Person,
                summary: None,
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();
    let adidas = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Adidas".to_string(),
                entity_type: EntityType::Organization,
                summary: None,
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();
    let nike = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Nike".to_string(),
                entity_type: EntityType::Organization,
                summary: None,
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();

    let now = Utc::now();
    state_store
        .create_edge(Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "Kendra".to_string(),
                target_name: "Adidas".to_string(),
                label: "prefers".to_string(),
                fact: "Kendra prefers Adidas".to_string(),
                confidence: 0.8,
                valid_at: Some(now - chrono::Duration::days(2)),
            },
            user_id,
            src.id,
            adidas.id,
            episode_id,
            now,
        ))
        .await
        .unwrap();
    state_store
        .create_edge(Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "Kendra".to_string(),
                target_name: "Nike".to_string(),
                label: "prefers".to_string(),
                fact: "Kendra prefers Nike".to_string(),
                confidence: 0.85,
                valid_at: Some(now - chrono::Duration::days(1)),
            },
            user_id,
            src.id,
            nike.id,
            episode_id,
            now,
        ))
        .await
        .unwrap();

    let (status, radar) = json_request(
        &app,
        "POST",
        "/api/v1/memory/conflict-user/conflict_radar",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let clusters = radar["conflicts"].as_array().unwrap();
    assert!(!clusters.is_empty());
    let first = &clusters[0];
    assert_eq!(first["label"], "prefers");
    assert!(first["active_edge_count"].as_u64().unwrap_or(0) >= 2);
    assert!(first["needs_resolution"].as_bool().unwrap_or(false));
    assert!(first["severity"].as_f64().unwrap_or(0.0) >= 0.8);
}

#[tokio::test]
async fn test_memory_api_immediate_recall_fallback_contains_recent_text() {
    let app = build_test_app().await;
    let marker = "falsify-anchovy-orbit-9271";

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "timing-user",
            "text": format!("My secret marker is {marker}.")
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/timing-user/context",
        serde_json::json!({"query": "What is my secret marker?"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let text = context["context"].as_str().unwrap_or_default();
    assert!(
        text.contains(marker),
        "expected immediate recall to include marker, got context: {text}"
    );
}

#[tokio::test]
async fn test_chat_history_import_ndjson_pathway() {
    let app = build_test_app().await;

    let (status, import_resp) = json_request(
        &app,
        "POST",
        "/api/v1/import/chat-history",
        serde_json::json!({
            "user": "import-user-1",
            "source": "ndjson",
            "payload": [
                {
                    "session": "Imported Thread",
                    "role": "user",
                    "content": "Imported message one",
                    "created_at": "2025-01-01T00:00:00Z"
                },
                {
                    "session": "Imported Thread",
                    "role": "assistant",
                    "content": "Imported response one",
                    "created_at": "2025-01-01T00:00:10Z"
                }
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let job_id = import_resp["job_id"].as_str().unwrap().to_string();

    let mut completed = false;
    for _ in 0..40 {
        let (job_status, job) = json_request(
            &app,
            "GET",
            &format!("/api/v1/import/jobs/{job_id}"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(job_status, StatusCode::OK);

        if job["status"] == "completed" {
            completed = true;
            assert_eq!(job["imported_messages"], 2);
            assert_eq!(job["failed_messages"], 0);
            break;
        }
        if job["status"] == "failed" {
            panic!("import job failed unexpectedly: {job}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(completed, "import job did not complete in time");

    let (status, user) = json_request(
        &app,
        "GET",
        "/api/v1/users/external/import-user-1",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_id = user["id"].as_str().unwrap();

    let (status, context) = json_request(
        &app,
        "POST",
        &format!("/api/v1/memory/{user_id}/context"),
        serde_json::json!({"query": "What was imported?", "session": "Imported Thread"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let context_text = context["context"].as_str().unwrap_or_default();
    assert!(
        context_text.contains("Imported message one")
            || context_text.contains("Imported response one"),
        "expected imported content in context, got: {}",
        context_text
    );
}

#[tokio::test]
async fn test_chat_history_import_rejects_malformed_rows() {
    let app = build_test_app().await;

    let (status, import_resp) = json_request(
        &app,
        "POST",
        "/api/v1/import/chat-history",
        serde_json::json!({
            "user": "import-user-malformed",
            "source": "ndjson",
            "payload": [
                {
                    "session": "broken",
                    "role": "user",
                    "created_at": "2025-01-01T00:00:00Z"
                }
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let job_id = import_resp["job_id"].as_str().unwrap();
    let job = wait_for_import_job(&app, job_id).await;
    assert_eq!(job["status"], "failed");
    assert_eq!(job["total_messages"], 0);
    assert_eq!(job["imported_messages"], 0);
    assert!(job["errors"]
        .as_array()
        .map(|errors| !errors.is_empty())
        .unwrap_or(false));
}

#[tokio::test]
async fn test_chat_history_import_supports_mixed_timestamp_quality() {
    let app = build_test_app().await;

    let (status, import_resp) = json_request(
        &app,
        "POST",
        "/api/v1/import/chat-history",
        serde_json::json!({
            "user": "import-user-timestamps",
            "source": "ndjson",
            "default_session": "Timestamp Mix",
            "payload": [
                {
                    "role": "user",
                    "content": "RFC3339 timestamp row",
                    "created_at": "2025-01-01T00:00:00Z"
                },
                {
                    "role": "assistant",
                    "content": "Unix timestamp row",
                    "created_at": "1735689605"
                },
                {
                    "role": "user",
                    "content": "Missing timestamp row"
                }
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let job_id = import_resp["job_id"].as_str().unwrap();
    let job = wait_for_import_job(&app, job_id).await;
    assert_eq!(job["status"], "completed");
    assert_eq!(job["total_messages"], 3);
    assert_eq!(job["imported_messages"], 3);
    assert_eq!(job["failed_messages"], 0);
}

#[tokio::test]
async fn test_chat_history_import_idempotency_prevents_duplicate_replay() {
    let app = build_test_app().await;

    let payload = serde_json::json!({
        "user": "import-user-idempotent",
        "source": "ndjson",
        "idempotency_key": "replay-key-42",
        "default_session": "Idempotent Session",
        "payload": [
            {
                "role": "user",
                "content": "Import exactly once",
                "created_at": "2025-01-01T00:00:00Z"
            }
        ]
    });

    let (status, first) =
        json_request(&app, "POST", "/api/v1/import/chat-history", payload.clone()).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let first_job_id = first["job_id"].as_str().unwrap().to_string();

    let first_job = wait_for_import_job(&app, &first_job_id).await;
    assert_eq!(first_job["status"], "completed");
    assert_eq!(first_job["imported_messages"], 1);

    let (status, second) = json_request(&app, "POST", "/api/v1/import/chat-history", payload).await;
    assert_eq!(status, StatusCode::OK);
    let second_job_id = second["job_id"].as_str().unwrap().to_string();
    assert_eq!(first_job_id, second_job_id);

    let (status, user) = json_request(
        &app,
        "GET",
        "/api/v1/users/external/import-user-idempotent",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_id = user["id"].as_str().unwrap();

    let (status, sessions) = json_request(
        &app,
        "GET",
        &format!("/api/v1/users/{user_id}/sessions?limit=20"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session_id = sessions["data"][0]["id"].as_str().unwrap();

    let (status, episodes) = json_request(
        &app,
        "GET",
        &format!("/api/v1/sessions/{session_id}/episodes?limit=50"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        episodes["count"], 1,
        "idempotent replay must not duplicate episodes"
    );
}

#[tokio::test]
async fn test_chat_history_import_chatgpt_export_pathway() {
    let app = build_test_app().await;

    let (status, import_resp) = json_request(
        &app,
        "POST",
        "/api/v1/import/chat-history",
        serde_json::json!({
            "user": "import-user-chatgpt",
            "source": "chatgpt_export",
            "payload": {
                "title": "Lab Notebook",
                "mapping": {
                    "m1": {
                        "message": {
                            "author": {"role": "user"},
                            "create_time": 1735689600,
                            "content": {"parts": ["first exported message"]}
                        }
                    },
                    "m2": {
                        "message": {
                            "author": {"role": "assistant"},
                            "create_time": 1735689610,
                            "content": {"parts": ["assistant exported reply"]}
                        }
                    }
                }
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let job = wait_for_import_job(&app, import_resp["job_id"].as_str().unwrap()).await;
    assert_eq!(job["status"], "completed");
    assert_eq!(job["imported_messages"], 2);

    let (status, user) = json_request(
        &app,
        "GET",
        "/api/v1/users/external/import-user-chatgpt",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_id = user["id"].as_str().unwrap();

    let (status, context) = json_request(
        &app,
        "POST",
        &format!("/api/v1/memory/{user_id}/context"),
        serde_json::json!({"query": "what did we import?", "session": "Lab Notebook"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = context["context"].as_str().unwrap_or_default();
    assert!(
        text.contains("first exported message") || text.contains("assistant exported reply"),
        "expected chatgpt import content in context, got: {text}"
    );
}

#[tokio::test]
async fn test_chat_history_import_gemini_export_pathway() {
    let app = build_test_app().await;

    let (status, import_resp) = json_request(
        &app,
        "POST",
        "/api/v1/import/chat-history",
        serde_json::json!({
            "user": "import-user-gemini",
            "source": "gemini_export",
            "payload": {
                "chunkedPrompt": {
                    "chunks": [
                        {"text": "hello from gemini user", "role": "user"},
                        {"text": "internal thought should be skipped", "role": "model", "isThought": true},
                        {"text": "hello from gemini model", "role": "model"}
                    ]
                }
            },
            "default_session": "Gemini Import"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let job = wait_for_import_job(&app, import_resp["job_id"].as_str().unwrap()).await;
    assert_eq!(job["status"], "completed");
    assert_eq!(job["imported_messages"], 2);
    assert_eq!(job["failed_messages"], 0);

    let (status, user) = json_request(
        &app,
        "GET",
        "/api/v1/users/external/import-user-gemini",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_id = user["id"].as_str().unwrap();

    let (status, episodes) = json_request(
        &app,
        "GET",
        &format!("/api/v1/users/{user_id}/sessions?limit=20"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session_id = episodes["data"][0]["id"].as_str().unwrap();

    let (status, rows) = json_request(
        &app,
        "GET",
        &format!("/api/v1/sessions/{session_id}/episodes?limit=20"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rows["count"], 2);
    let rendered = rows["data"].as_array().unwrap();
    let contents: Vec<&str> = rendered
        .iter()
        .filter_map(|r| r["content"].as_str())
        .collect();
    assert!(contents
        .iter()
        .any(|c| c.contains("hello from gemini user")));
    assert!(contents
        .iter()
        .any(|c| c.contains("hello from gemini model")));
    assert!(contents
        .iter()
        .all(|c| !c.contains("internal thought should be skipped")));
}

#[tokio::test]
async fn test_chat_history_import_dry_run_writes_no_data() {
    let app = build_test_app().await;

    let (status, import_resp) = json_request(
        &app,
        "POST",
        "/api/v1/import/chat-history",
        serde_json::json!({
            "user": "import-user-dry-run",
            "source": "ndjson",
            "dry_run": true,
            "payload": [
                {
                    "role": "user",
                    "content": "dry run row",
                    "created_at": "2025-01-01T00:00:00Z"
                }
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let job = wait_for_import_job(&app, import_resp["job_id"].as_str().unwrap()).await;
    assert_eq!(job["status"], "completed");
    assert_eq!(job["total_messages"], 1);
    assert_eq!(job["imported_messages"], 1);
    assert_eq!(job["failed_messages"], 0);

    let (status, body) = json_request(
        &app,
        "GET",
        "/api/v1/users/external/import-user-dry-run",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn test_scientific_current_context_includes_episode_provenance() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "science-provenance-user",
            "external_id": "science-provenance-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({"user_id": user_id, "name": "research-log"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "Legacy model: regenerative patterning is gene-expression only.",
            "created_at": "2021-03-11T09:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "Current model: regenerative patterning uses stable bioelectric prepatterns.",
            "created_at": "2025-08-19T10:15:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/science-provenance-user/context",
        serde_json::json!({
            "query": "What is our current model of regenerative patterning?",
            "time_intent": "current",
            "mode": "head",
            "max_tokens": 700
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let episodes = context["episodes"].as_array().unwrap();
    assert!(
        !episodes.is_empty(),
        "expected episode provenance in response"
    );
    assert!(episodes.iter().all(|e| e["id"].is_string()));
    assert!(episodes.iter().all(|e| e["session_id"].is_string()));
    assert!(episodes.iter().all(|e| e["created_at"].is_string()));
    assert!(
        episodes.iter().any(|e| {
            e["preview"]
                .as_str()
                .unwrap_or_default()
                .contains("stable bioelectric prepatterns")
        }),
        "expected provenance to cite current scientific claim"
    );
}

#[tokio::test]
async fn test_scientific_historical_context_cites_historical_episode() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "science-historical-user",
            "external_id": "science-historical-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({"user_id": user_id, "name": "research-log"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "Old framing: target morphology is a static endpoint snapshot only.",
            "created_at": "2022-05-09T13:20:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "Updated framing: target morphology behaves as an attractor-like setpoint in morphospace.",
            "created_at": "2025-01-21T15:45:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/science-historical-user/context",
        serde_json::json!({
            "query": "What was our framing in 2022?",
            "mode": "historical",
            "time_intent": "historical",
            "as_of": "2022-09-01T00:00:00Z",
            "max_tokens": 700
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let episodes = context["episodes"].as_array().unwrap();
    assert!(
        !episodes.is_empty(),
        "expected historical provenance episodes"
    );
    assert!(
        episodes.iter().any(|e| {
            e["preview"]
                .as_str()
                .unwrap_or_default()
                .contains("static endpoint snapshot")
        }),
        "expected historical provenance to include the historical claim"
    );
}

#[tokio::test]
async fn test_memory_api_head_mode_returns_thread_head_diagnostics() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "head-mode-user",
            "external_id": "head-mode-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session_a) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "a" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_a_id = session_a["id"].as_str().unwrap().to_string();

    let (status, session_b) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "b" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_b_id = session_b["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_a_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "old-session-marker",
            "created_at": "2024-01-10T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_b_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "new-session-marker",
            "created_at": "2026-03-01T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/head-mode-user/context",
        serde_json::json!({
            "query": "what is current?",
            "mode": "head",
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(context["mode"], "head");
    assert_eq!(context["head"]["session_id"], session_b_id);
    assert!(context["head"]["episode_id"].is_string());
    assert_eq!(context["head"]["version"], 1);
    assert!(
        context["context"]
            .as_str()
            .unwrap_or_default()
            .contains("new-session-marker"),
        "expected head mode context to include latest session marker"
    );
}

#[tokio::test]
async fn test_memory_api_head_mode_with_explicit_session_override() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "head-override-user",
            "external_id": "head-override-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session_a) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "a" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_a_id = session_a["id"].as_str().unwrap().to_string();

    let (status, session_b) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "b" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_b_id = session_b["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_a_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "override-marker-a",
            "created_at": "2024-01-10T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_b_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "latest-marker-b",
            "created_at": "2026-03-01T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/head-override-user/context",
        serde_json::json!({
            "query": "what should head use?",
            "mode": "head",
            "session": "a",
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(context["mode"], "head");
    assert_eq!(context["head"]["session_id"], session_a_id);
    assert!(context["context"]
        .as_str()
        .unwrap_or_default()
        .contains("override-marker-a"));
}

#[tokio::test]
async fn test_memory_api_head_mode_without_sessions_returns_empty_head() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "head-empty-user",
            "external_id": "head-empty-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/head-empty-user/context",
        serde_json::json!({
            "query": "what is current?",
            "mode": "head",
            "max_tokens": 300
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(context["mode"], "head");
    assert!(context.get("head").is_none() || context["head"].is_null());
}

#[tokio::test]
async fn test_memory_api_temporal_intent_changes_rank_order() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "temporal-intent-user",
            "external_id": "temporal-intent-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "default" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "I prefer Adidas running shoes.",
            "created_at": "2024-01-10T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "I switched and now prefer Nike running shoes.",
            "created_at": "2026-03-01T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, current) = json_request(
        &app,
        "POST",
        "/api/v1/memory/temporal-intent-user/context",
        serde_json::json!({
            "query": "What shoes do I prefer now?",
            "session": "default",
            "time_intent": "current",
            "temporal_weight": 0.9,
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, historical) = json_request(
        &app,
        "POST",
        "/api/v1/memory/temporal-intent-user/context",
        serde_json::json!({
            "query": "What shoes did I prefer as of 2024?",
            "session": "default",
            "time_intent": "historical",
            "as_of": "2024-06-01T00:00:00Z",
            "temporal_weight": 0.9,
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let current_context = current["context"].as_str().unwrap_or_default();
    let historical_context = historical["context"].as_str().unwrap_or_default();
    let current_top = current_context
        .lines()
        .find(|l| l.starts_with("- ["))
        .unwrap_or_default();
    let historical_top = historical_context
        .lines()
        .find(|l| l.starts_with("- ["))
        .unwrap_or_default();

    assert!(
        current_top.contains("Nike"),
        "expected current intent to rank Nike first, got: {current_top}"
    );
    assert!(
        historical_top.contains("Adidas"),
        "expected historical intent to rank Adidas first, got: {historical_top}"
    );

    assert_eq!(
        current["temporal_diagnostics"]["resolved_intent"],
        "current"
    );
    assert_eq!(
        historical["temporal_diagnostics"]["resolved_intent"],
        "historical"
    );
}

#[tokio::test]
async fn test_memory_api_metadata_filters_and_diagnostics() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "metadata-filter-user",
            "external_id": "metadata-filter-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "default" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "Priority outage in payments pipeline",
            "metadata": {"tags": ["priority", "incident"]},
            "created_at": "2026-03-01T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "assistant",
            "content": "Normal weekly standup update",
            "metadata": {"tags": ["routine"]},
            "created_at": "2026-03-01T13:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/metadata-filter-user/context",
        serde_json::json!({
            "query": "What priority incidents did we discuss?",
            "session": "default",
            "filters": {
                "roles": ["user"],
                "tags_all": ["priority"],
                "created_after": "2026-03-01T00:00:00Z"
            },
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(context["metadata_filter_diagnostics"].is_object());
    assert_eq!(
        context["metadata_filter_diagnostics"]["candidate_count_before_filters"],
        2
    );
    assert_eq!(
        context["metadata_filter_diagnostics"]["candidate_count_after_filters"],
        1
    );
    assert_eq!(
        context["metadata_filter_diagnostics"]["prefilter_enabled"],
        true
    );
    assert!(context["metadata_filter_diagnostics"]["planner_latency_ms"].is_number());

    let episodes = context["episodes"].as_array().cloned().unwrap_or_default();
    if !episodes.is_empty() {
        let top_preview = episodes[0]["preview"].as_str().unwrap_or_default();
        assert!(top_preview.contains("Priority outage"));
    }
}

#[tokio::test]
async fn test_memory_api_metadata_prefilter_disabled_diagnostics() {
    let app = build_test_app_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 200,
        relax_if_empty: false,
    })
    .await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "prefilter-disabled-user",
            "text": "Priority incident happened today"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/prefilter-disabled-user/context",
        serde_json::json!({
            "query": "What incidents happened?",
            "filters": {"tags_any": ["priority"]},
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        context["metadata_filter_diagnostics"]["prefilter_enabled"],
        false
    );
}

#[tokio::test]
async fn test_memory_api_metadata_prefilter_relax_if_empty() {
    let app = build_test_app_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: true,
    })
    .await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "prefilter-relax-user",
            "text": "Priority incident happened today",
            "session": "default"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/prefilter-relax-user/context",
        serde_json::json!({
            "query": "What happened?",
            "session": "default",
            "filters": {"tags_all": ["does-not-exist"]},
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        context["metadata_filter_diagnostics"]["relaxed_fallback_applied"],
        true
    );
    assert_eq!(
        context["metadata_filter_diagnostics"]["candidate_count_after_filters"],
        1
    );
}

#[tokio::test]
async fn test_memory_api_metadata_scan_limit_applies() {
    let app = build_test_app_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 1,
        relax_if_empty: false,
    })
    .await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "prefilter-scan-limit-user",
            "text": "Episode one",
            "session": "default"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "prefilter-scan-limit-user",
            "text": "Episode two",
            "session": "default"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/prefilter-scan-limit-user/context",
        serde_json::json!({
            "query": "What happened?",
            "session": "default",
            "filters": {"roles": ["user"]},
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let scanned = context["metadata_filter_diagnostics"]["candidate_count_before_filters"]
        .as_u64()
        .unwrap_or(0);
    assert!(
        scanned <= 1,
        "expected scan limit to cap candidates, got {scanned}"
    );
}

#[tokio::test]
async fn test_agent_identity_substrate_endpoints_prototype() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "identity-user",
            "external_id": "identity-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "default" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap().to_string();

    let (status, identity) = json_request(
        &app,
        "GET",
        "/api/v1/agents/support-agent/identity",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity["agent_id"], "support-agent");

    let (status, identity) = json_request(
        &app,
        "PUT",
        "/api/v1/agents/support-agent/identity",
        serde_json::json!({
            "core": {
                "mission": "Resolve user issues accurately.",
                "boundaries": {"never_claim_human_experience": true}
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity["version"], 2);

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/agents/support-agent/experience",
        serde_json::json!({
            "user_id": user_id,
            "session_id": session_id,
            "category": "interaction_pattern",
            "signal": "user_prefers_bulleted_action_plans",
            "confidence": 0.8,
            "weight": 0.7,
            "decay_half_life_days": 30
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "I prefer concise checklists.",
            "created_at": "2026-03-02T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/agents/support-agent/context",
        serde_json::json!({
            "user": "identity-user",
            "query": "How should I respond to this user?",
            "session": "default",
            "mode": "head"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(context["identity_version"], 2);
    assert_eq!(context["experience_events_used"], 1);
    assert_eq!(
        context["attribution_guards"]["self_user_separation_enforced"],
        true
    );
    assert!(context["context"]
        .as_str()
        .unwrap_or_default()
        .contains("Agent Identity Core"));
}

#[tokio::test]
async fn test_agent_identity_rejects_user_memory_contamination_keys() {
    let app = build_test_app().await;

    let (status, body) = json_request(
        &app,
        "PUT",
        "/api/v1/agents/guard-test/identity",
        serde_json::json!({
            "core": {
                "mission": "be useful",
                "user_fact": "I am a doctor"
            }
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "validation_error");
}

#[tokio::test]
async fn test_agent_identity_versions_audit_and_rollback() {
    let app = build_test_app().await;

    let (status, identity_v1) = json_request(
        &app,
        "GET",
        "/api/v1/agents/rollback-test/identity",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity_v1["version"], 1);

    let (status, identity_v2) = json_request(
        &app,
        "PUT",
        "/api/v1/agents/rollback-test/identity",
        serde_json::json!({
            "core": {"mission": "version-2"}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity_v2["version"], 2);

    let (status, identity_v3) = json_request(
        &app,
        "PUT",
        "/api/v1/agents/rollback-test/identity",
        serde_json::json!({
            "core": {"mission": "version-3"}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity_v3["version"], 3);

    let (status, versions) = json_request(
        &app,
        "GET",
        "/api/v1/agents/rollback-test/identity/versions?limit=10",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let version_count = versions.as_array().map(|a| a.len()).unwrap_or(0);
    assert!(version_count >= 3);

    let (status, rolled) = json_request(
        &app,
        "POST",
        "/api/v1/agents/rollback-test/identity/rollback",
        serde_json::json!({
            "target_version": 2,
            "reason": "revert to stable"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rolled["version"], 4);
    assert_eq!(rolled["core"]["mission"], "version-2");

    let (status, audit) = json_request(
        &app,
        "GET",
        "/api/v1/agents/rollback-test/identity/audit?limit=20",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let events = audit.as_array().cloned().unwrap_or_default();
    assert!(!events.is_empty());
    assert!(events.iter().any(|e| e["action"] == "rolled_back"));
}

#[tokio::test]
async fn test_agent_promotion_gating_and_approval_flow() {
    let app = build_test_app().await;

    let (status, identity) = json_request(
        &app,
        "GET",
        "/api/v1/agents/promo-agent/identity",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity["version"], 1);

    // insufficient evidence should be rejected
    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/agents/promo-agent/promotions",
        serde_json::json!({
            "proposal": "increase directness",
            "candidate_core": {"mission": "new-mission"},
            "reason": "single anecdote",
            "source_event_ids": [uuid::Uuid::now_v7()]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "validation_error");

    let source_ids = vec![
        uuid::Uuid::now_v7(),
        uuid::Uuid::now_v7(),
        uuid::Uuid::now_v7(),
    ];

    let (status, proposal) = json_request(
        &app,
        "POST",
        "/api/v1/agents/promo-agent/promotions",
        serde_json::json!({
            "proposal": "increase directness",
            "candidate_core": {"mission": "new-mission"},
            "reason": "repeated positive outcomes",
            "source_event_ids": source_ids
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(proposal["status"], "pending");
    let proposal_id = proposal["id"].as_str().unwrap().to_string();

    let (status, approved) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/promo-agent/promotions/{proposal_id}/approve"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(approved["status"], "approved");

    let (status, identity_after) = json_request(
        &app,
        "GET",
        "/api/v1/agents/promo-agent/identity",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity_after["core"]["mission"], "new-mission");
    assert_eq!(identity_after["version"], 2);
}

#[tokio::test]
async fn test_agent_identity_drift_resistance_blocks_repeated_adversarial_mutations() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "GET",
        "/api/v1/agents/drift-agent/identity",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    for _ in 0..20 {
        let (status, body) = json_request(
            &app,
            "PUT",
            "/api/v1/agents/drift-agent/identity",
            serde_json::json!({
                "core": {
                    "mission": "safe",
                    "user_profile": "I am definitely a doctor"
                }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "unexpected body: {body:?}");
    }

    let (status, identity_after) = json_request(
        &app,
        "GET",
        "/api/v1/agents/drift-agent/identity",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity_after["version"], 1);
}
