//! Security red-team tests for Phase 2 features.
//!
//! These tests verify that the SSE transport, subscriptions, and new
//! resource handlers are resistant to common attacks.

#[cfg(test)]
mod tests {
    use crate::transport::handle_message;
    use crate::{McpConfig, McpServer};
    use std::sync::Arc;

    fn test_server() -> Arc<McpServer> {
        Arc::new(McpServer::new(McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        }))
    }

    // ─── Subscription Security Tests ──────────────────────────────

    #[tokio::test]
    async fn test_falsify_subscribe_oversized_uri() {
        let server = test_server();
        // Attempt to subscribe with an extremely long URI
        let huge_uri = format!("mnemo://users/{}/memory", "x".repeat(10000));
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{{"uri":"{}"}}}}"#,
            huge_uri
        );
        let resp = handle_message(&server, &msg).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

        // Should reject oversized URIs
        assert!(
            parsed.get("error").is_some(),
            "Should reject subscription URIs over 2048 characters"
        );
    }

    #[tokio::test]
    async fn test_falsify_subscribe_path_traversal_in_uri() {
        let server = test_server();
        // Attempt path traversal in subscription URI
        let msg = r#"{"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{"uri":"mnemo://users/../admin/memory"}}"#;
        let resp = handle_message(&server, &msg).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

        // Should reject path traversal patterns
        assert!(
            parsed.get("error").is_some(),
            "Should reject URIs with path traversal"
        );
    }

    #[tokio::test]
    async fn test_falsify_subscribe_null_bytes_in_uri() {
        let server = test_server();
        // Attempt null byte injection in subscription URI
        let msg = r#"{"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{"uri":"mnemo://users/alice\u0000evil/memory"}}"#;
        let resp = handle_message(&server, &msg).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

        // Should reject URIs with null bytes
        assert!(
            parsed.get("error").is_some(),
            "Should reject URIs with null bytes"
        );
    }

    #[tokio::test]
    async fn test_falsify_unsubscribe_missing_validation() {
        let server = test_server();
        // Unsubscribe should also validate the URI
        let msg = r#"{"jsonrpc":"2.0","id":1,"method":"resources/unsubscribe","params":{"uri":"https://evil.com/steal"}}"#;
        let resp = handle_message(&server, &msg).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

        // Should reject non-mnemo URIs
        assert!(
            parsed.get("error").is_some(),
            "Unsubscribe should validate URI prefix"
        );
    }

    #[tokio::test]
    async fn test_falsify_unsubscribe_oversized_uri() {
        let server = test_server();
        let huge_uri = format!("mnemo://users/{}/memory", "x".repeat(10000));
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"resources/unsubscribe","params":{{"uri":"{}"}}}}"#,
            huge_uri
        );
        let resp = handle_message(&server, &msg).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

        // Should reject oversized URIs
        assert!(
            parsed.get("error").is_some(),
            "Unsubscribe should reject oversized URIs"
        );
    }

    // ─── Search Resource Security Tests ───────────────────────────

    #[tokio::test]
    async fn test_falsify_search_query_injection() {
        let server = test_server();
        // Try to inject additional parameters via the query
        let msg = r#"{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"mnemo://users/alice/search?q=test&admin=true&bypass=1"}}"#;
        let resp = handle_message(&server, &msg).await;

        // The request will fail at HTTP level (server not running) but should not panic
        // and should properly parse the query parameter
        assert!(resp.is_some());
    }

    #[tokio::test]
    async fn test_falsify_search_encoded_traversal() {
        let server = test_server();
        // URL-encoded path traversal in query
        let msg = r#"{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"mnemo://users/alice/search?q=%2e%2e%2f%2e%2e%2fetc%2fpasswd"}}"#;
        let resp = handle_message(&server, &msg).await;

        // Should handle encoded input safely
        assert!(resp.is_some());
    }

    // ─── JSON-RPC Protocol Security ───────────────────────────────

    #[tokio::test]
    async fn test_falsify_deeply_nested_json() {
        let server = test_server();
        // Create deeply nested JSON to test stack overflow protection
        let mut nested = String::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":"#);
        for _ in 0..100 {
            nested.push_str(r#"{"a":"#);
        }
        nested.push_str(r#""b""#);
        for _ in 0..100 {
            nested.push('}');
        }
        nested.push('}');

        let resp = handle_message(&server, &nested).await;
        // Should either parse successfully or return parse error, not crash
        assert!(resp.is_some());
    }

    #[tokio::test]
    async fn test_falsify_unicode_edge_cases() {
        let server = test_server();
        // Test various Unicode edge cases
        let cases = [
            // Zero-width characters
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{"uri":"mnemo://users/ali\u200Bce/memory"}}"#,
            // Right-to-left override
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{"uri":"mnemo://users/\u202Eecila/memory"}}"#,
            // Homoglyph attack (Cyrillic 'а' looks like Latin 'a')
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{"uri":"mnemo://users/\u0430lice/memory"}}"#,
        ];

        for case in cases {
            let resp = handle_message(&server, case).await;
            // Should handle without panic
            assert!(resp.is_some(), "Should handle Unicode edge case");
        }
    }
}

#[cfg(all(test, feature = "sse"))]
mod sse_tests {
    use crate::sse::{router, SseState};
    use crate::{McpConfig, McpServer};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;
    use tower::ServiceExt;

    fn test_state() -> Arc<SseState> {
        let mcp = Arc::new(McpServer::new(McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        }));
        Arc::new(SseState::new(mcp))
    }

    #[tokio::test]
    async fn test_falsify_sse_session_id_path_traversal() {
        let state = test_state();
        let app = router(state);

        // Try path traversal in session_id query param
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/sse?session_id=../../../etc/passwd")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should either reject or sanitize, not use the malicious session_id
        // For now, we just verify it doesn't panic
        assert!(
            response.status() == StatusCode::OK
                || response.status() == StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn test_falsify_sse_session_id_oversized() {
        let state = test_state();
        let app = router(state);

        // Try very long session_id
        let huge_id = "x".repeat(10000);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(&format!("/sse?session_id={}", huge_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should reject oversized session IDs
        assert!(
            response.status() == StatusCode::BAD_REQUEST
                || response.status() == StatusCode::URI_TOO_LONG,
            "Should reject oversized session IDs, got {}",
            response.status()
        );
    }

    #[tokio::test]
    async fn test_falsify_message_oversized_body() {
        let state = test_state();
        let app = router(state);

        // Send very large request body
        let huge_body = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"ping","params":{{"data":"{}"}}}}"#,
            "x".repeat(10 * 1024 * 1024) // 10MB
        );

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/message")
                    .header("content-type", "application/json")
                    .body(Body::from(huge_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should reject or handle gracefully
        assert!(
            response.status() == StatusCode::PAYLOAD_TOO_LARGE
                || response.status() == StatusCode::BAD_REQUEST
                || response.status() == StatusCode::OK,
            "Should handle large payloads, got {}",
            response.status()
        );
    }

    #[tokio::test]
    async fn test_falsify_sse_session_id_null_bytes() {
        let state = test_state();
        let app = router(state);

        // Try null byte in session_id (URL encoded)
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/sse?session_id=abc%00def")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should reject or sanitize null bytes
        assert!(
            response.status() == StatusCode::OK
                || response.status() == StatusCode::BAD_REQUEST
        );
    }
}
