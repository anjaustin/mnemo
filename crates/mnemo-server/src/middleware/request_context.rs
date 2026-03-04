use std::sync::atomic::Ordering;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, Request};
use axum::middleware::Next;
use axum::response::Response;
use uuid::Uuid;

use crate::state::AppState;

pub const REQUEST_ID_HEADER: &str = "x-mnemo-request-id";

#[derive(Clone, Debug)]
pub struct RequestContext {
    pub request_id: String,
}

pub async fn request_context_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let request_id = req
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| Uuid::now_v7().to_string());

    req.extensions_mut().insert(RequestContext {
        request_id: request_id.clone(),
    });

    state
        .metrics
        .http_requests_total
        .fetch_add(1, Ordering::Relaxed);

    let mut response = next.run(req).await;

    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }

    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        state
            .metrics
            .http_responses_2xx
            .fetch_add(1, Ordering::Relaxed);
    } else if (400..500).contains(&status) {
        state
            .metrics
            .http_responses_4xx
            .fetch_add(1, Ordering::Relaxed);
    } else if status >= 500 {
        state
            .metrics
            .http_responses_5xx
            .fetch_add(1, Ordering::Relaxed);
    }

    response
}
