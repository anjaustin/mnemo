use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

use mnemo_core::traits::fulltext::FullTextStore;
use mnemo_core::traits::llm::EmbeddingConfig;
use mnemo_graph::GraphEngine;
use mnemo_llm::OpenAiCompatibleEmbedder;
use mnemo_retrieval::RetrievalEngine;
use mnemo_server::routes::build_router;
use mnemo_server::state::AppState;
use mnemo_storage::{QdrantVectorStore, RedisStateStore};

async fn build_test_app() -> axum::Router {
    let redis_url = std::env::var("MNEMO_TEST_REDIS_URL")
        .unwrap_or_else(|_| "redis://localhost:6379".to_string());
    let qdrant_url = std::env::var("MNEMO_TEST_QDRANT_URL")
        .unwrap_or_else(|_| "http://localhost:6334".to_string());

    let uid = Uuid::now_v7();
    let redis_prefix = format!("memory_api_test:{}:", uid);
    let qdrant_prefix = std::env::var("MNEMO_TEST_QDRANT_PREFIX")
        .unwrap_or_else(|_| "mnemo_".to_string());

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

    build_router(AppState {
        state_store,
        vector_store,
        retrieval,
        graph,
    })
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
