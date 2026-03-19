//! SSE (Server-Sent Events) transport for the MCP server.
//!
//! This provides an HTTP-based transport for web-based agents and remote
//! integrations. The MCP protocol over SSE uses:
//!
//! - POST /message - Send JSON-RPC requests, receive JSON-RPC responses
//! - GET /sse - SSE stream for server-initiated notifications (subscriptions)
//!
//! This is complementary to the stdio transport — use stdio for local
//! tool integration (Claude Code, Cursor) and SSE for remote/web clients.

#![cfg(feature = "sse")]

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::transport::handle_message;
use crate::McpServer;

/// Configuration for the SSE transport.
#[derive(Debug, Clone)]
pub struct SseConfig {
    /// Host to bind the HTTP server to (default: "127.0.0.1").
    pub host: String,
    /// Port to bind the HTTP server to (default: 3000).
    pub port: u16,
    /// Optional CORS origins to allow (if empty, CORS is permissive).
    pub cors_origins: Vec<String>,
}

impl Default for SseConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3000,
            cors_origins: vec![],
        }
    }
}

impl SseConfig {
    pub fn from_env() -> Self {
        Self {
            host: std::env::var("MNEMO_MCP_SSE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: std::env::var("MNEMO_MCP_SSE_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(3000),
            cors_origins: std::env::var("MNEMO_MCP_SSE_CORS")
                .ok()
                .map(|s| s.split(',').map(String::from).collect())
                .unwrap_or_default(),
        }
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Server-sent notification for subscribed resources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseNotification {
    /// JSON-RPC 2.0 notification (no id).
    pub jsonrpc: String,
    /// Notification method (e.g., "notifications/resources/updated").
    pub method: String,
    /// Notification parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl SseNotification {
    pub fn resource_updated(uri: &str) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: "notifications/resources/updated".to_string(),
            params: Some(serde_json::json!({ "uri": uri })),
        }
    }
}

/// Shared state for SSE connections.
pub struct SseState {
    /// The MCP server instance.
    pub mcp: Arc<McpServer>,
    /// Broadcast channel for SSE notifications.
    pub notifications: broadcast::Sender<SseNotification>,
    /// Active session IDs (for future use with subscriptions).
    pub sessions: RwLock<HashMap<String, SessionInfo>>,
}

/// Information about an active SSE session.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Session identifier.
    pub id: String,
    /// Subscribed resource URIs.
    pub subscriptions: Vec<String>,
    /// When the session was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl SseState {
    pub fn new(mcp: Arc<McpServer>) -> Self {
        let (notifications, _) = broadcast::channel(256);
        Self {
            mcp,
            notifications,
            sessions: RwLock::new(HashMap::new()),
        }
    }

    /// Broadcast a notification to all SSE clients.
    pub fn broadcast(&self, notification: SseNotification) {
        // Ignore send errors (no receivers is fine)
        let _ = self.notifications.send(notification);
    }
}

/// Maximum allowed length for session IDs.
const MAX_SESSION_ID_LENGTH: usize = 256;

/// Query parameters for the SSE endpoint.
#[derive(Debug, Deserialize)]
pub struct SseQuery {
    /// Optional session ID for resuming a session.
    #[serde(default)]
    pub session_id: Option<String>,
}

/// Validate a session ID for security issues.
fn validate_session_id(id: &str) -> Result<(), &'static str> {
    // Check length
    if id.len() > MAX_SESSION_ID_LENGTH {
        return Err("Session ID exceeds maximum length");
    }

    // Check for path traversal
    if id.contains("..") || id.contains('/') || id.contains('\\') {
        return Err("Session ID contains invalid characters");
    }

    // Check for null bytes
    if id.contains('\0') {
        return Err("Session ID contains null bytes");
    }

    // Check for control characters
    if id.chars().any(|c| c.is_control()) {
        return Err("Session ID contains control characters");
    }

    Ok(())
}

/// Build the SSE transport router.
pub fn router(state: Arc<SseState>) -> Router {
    Router::new()
        .route("/message", post(handle_post_message))
        .route("/sse", get(handle_sse_stream))
        .route("/health", get(handle_health))
        .with_state(state)
}

/// POST /message - Handle JSON-RPC requests.
///
/// Accepts a JSON-RPC request body and returns a JSON-RPC response.
/// This is the primary endpoint for MCP tool calls over HTTP.
async fn handle_post_message(State(state): State<Arc<SseState>>, body: String) -> Response {
    let body = body.trim();
    if body.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {
                    "code": -32700,
                    "message": "Parse error: empty request body"
                }
            })),
        )
            .into_response();
    }

    match handle_message(&state.mcp, body).await {
        Some(response) => {
            // Parse the response to return proper JSON
            match serde_json::from_str::<serde_json::Value>(&response) {
                Ok(json) => (StatusCode::OK, Json(json)).into_response(),
                Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, response).into_response(),
            }
        }
        None => {
            // Notification — no response body
            StatusCode::NO_CONTENT.into_response()
        }
    }
}

/// GET /sse - SSE stream for server-initiated notifications.
///
/// Clients connect here to receive real-time notifications about
/// resource updates, subscription events, etc.
async fn handle_sse_stream(
    State(state): State<Arc<SseState>>,
    Query(query): Query<SseQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, (StatusCode, String)>
{
    // Validate session ID if provided
    let session_id = match query.session_id {
        Some(ref id) => {
            if let Err(e) = validate_session_id(id) {
                return Err((StatusCode::BAD_REQUEST, e.to_string()));
            }
            id.clone()
        }
        None => uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string(),
    };

    // Register session
    {
        let mut sessions = state.sessions.write().await;
        sessions.insert(
            session_id.clone(),
            SessionInfo {
                id: session_id.clone(),
                subscriptions: vec![],
                created_at: chrono::Utc::now(),
            },
        );
    }

    // Subscribe to notifications
    let rx = state.notifications.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(move |result| {
        match result {
            Ok(notification) => {
                let json = serde_json::to_string(&notification).ok()?;
                Some(Ok(Event::default().data(json)))
            }
            Err(_) => None, // Lagged — skip
        }
    });

    // Send initial connected event
    let session_id_clone = session_id.clone();
    let initial = futures::stream::once(async move {
        Ok(Event::default()
            .event("connected")
            .data(serde_json::json!({ "sessionId": session_id_clone }).to_string()))
    });

    let combined = initial.chain(stream);

    Ok(Sse::new(combined).keep_alive(KeepAlive::default()))
}

/// GET /health - Health check endpoint.
async fn handle_health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "transport": "sse",
        "version": crate::SERVER_VERSION
    }))
}

/// Run the SSE transport server.
///
/// This starts an HTTP server that handles MCP requests over HTTP/SSE.
/// Use this for web-based agents or remote integrations.
pub async fn run_sse(
    mcp: Arc<McpServer>,
    config: SseConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = Arc::new(SseState::new(mcp));
    let app = router(state);

    let addr = config.bind_addr();
    tracing::info!("Starting MCP SSE transport on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_state() -> Arc<SseState> {
        let mcp = Arc::new(McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        }));
        Arc::new(SseState::new(mcp))
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let state = test_state();
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_message_endpoint_ping() {
        let state = test_state();
        let app = router(state);

        let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_message_endpoint_empty_body() {
        let state = test_state();
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_message_endpoint_notification() {
        let state = test_state();
        let app = router(state);

        // Notification (no id) should return 204 No Content
        let body = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn test_message_endpoint_tools_list() {
        let state = test_state();
        let app = router(state);

        let body = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(json["result"]["tools"].is_array());
    }

    #[tokio::test]
    async fn test_sse_config_from_env() {
        // Test defaults
        let config = SseConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 3000);
        assert!(config.cors_origins.is_empty());
    }

    #[tokio::test]
    async fn test_sse_notification_resource_updated() {
        let notification = SseNotification::resource_updated("mnemo://users/alice/memory");
        assert_eq!(notification.jsonrpc, "2.0");
        assert_eq!(notification.method, "notifications/resources/updated");
        assert!(notification.params.is_some());
    }

    #[tokio::test]
    async fn test_broadcast_notification() {
        let state = test_state();

        // Subscribe before broadcasting
        let mut rx = state.notifications.subscribe();

        // Broadcast a notification
        state.broadcast(SseNotification::resource_updated(
            "mnemo://users/test/memory",
        ));

        // Should receive the notification
        let received = rx.try_recv();
        assert!(received.is_ok());
        assert_eq!(received.unwrap().method, "notifications/resources/updated");
    }
}
