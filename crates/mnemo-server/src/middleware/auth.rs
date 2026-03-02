use std::collections::HashSet;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use tower::{Layer, Service};

use mnemo_core::error::{ApiErrorDetail, ApiErrorResponse};

/// Configuration for API key authentication.
#[derive(Clone)]
pub struct AuthConfig {
    pub enabled: bool,
    pub valid_keys: HashSet<String>,
}

impl AuthConfig {
    pub fn disabled() -> Self {
        Self { enabled: false, valid_keys: HashSet::new() }
    }

    pub fn with_keys(keys: Vec<String>) -> Self {
        Self { enabled: true, valid_keys: keys.into_iter().collect() }
    }
}

/// Tower Layer that applies API key authentication.
#[derive(Clone)]
pub struct AuthLayer {
    config: Arc<AuthConfig>,
}

impl AuthLayer {
    pub fn new(config: AuthConfig) -> Self {
        Self { config: Arc::new(config) }
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthMiddleware { inner, config: self.config.clone() }
    }
}

/// The actual middleware service that checks API keys.
#[derive(Clone)]
pub struct AuthMiddleware<S> {
    inner: S,
    config: Arc<AuthConfig>,
}

impl<S> Service<Request<Body>> for AuthMiddleware<S>
where
    S: Service<Request<Body>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        if !self.config.enabled {
            let mut inner = self.inner.clone();
            return Box::pin(async move { inner.call(req).await });
        }

        let path = req.uri().path();
        if path == "/health" || path == "/healthz" {
            let mut inner = self.inner.clone();
            return Box::pin(async move { inner.call(req).await });
        }

        let api_key = req.headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|s| s.to_string())
            .or_else(|| {
                req.headers().get("x-api-key")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string())
            });

        match api_key {
            Some(key) if self.config.valid_keys.contains(&key) => {
                let mut inner = self.inner.clone();
                Box::pin(async move { inner.call(req).await })
            }
            _ => {
                let response = unauthorized_response();
                Box::pin(async move { Ok(response) })
            }
        }
    }
}

fn unauthorized_response() -> Response {
    let body = ApiErrorResponse {
        error: ApiErrorDetail {
            code: "unauthorized".to_string(),
            message: "Invalid or missing API key".to_string(),
            retry_after_ms: None,
        },
    };
    (StatusCode::UNAUTHORIZED, Json(body)).into_response()
}
