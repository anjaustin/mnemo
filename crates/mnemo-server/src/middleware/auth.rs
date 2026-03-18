use std::collections::HashSet;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use subtle::ConstantTimeEq;
use tower::{Layer, Service};

use mnemo_core::error::{ApiErrorDetail, ApiErrorResponse};
use mnemo_core::models::api_key::{hash_api_key, CallerContext};
use mnemo_core::traits::storage::ApiKeyStore;
use mnemo_storage::RedisStateStore;
use tokio::sync::RwLock;

/// P2-1: Constant-time comparison for bootstrap keys to prevent timing attacks.
fn constant_time_key_match(keys: &HashSet<String>, candidate: &str) -> bool {
    let candidate_bytes = candidate.as_bytes();
    for key in keys {
        let key_bytes = key.as_bytes();
        // Only compare if lengths match (length itself leaks info, but this is
        // unavoidable with variable-length keys; the comparison is still constant-time)
        if key_bytes.len() == candidate_bytes.len() {
            if key_bytes.ct_eq(candidate_bytes).into() {
                return true;
            }
        }
    }
    false
}

/// Configuration for API key authentication.
///
/// Supports three modes:
/// 1. **Disabled** — all requests get implicit Admin context.
/// 2. **Bootstrap keys only** — legacy mode: a set of raw strings treated as Admin keys.
/// 3. **Scoped keys** — keys stored in Redis with role/scope metadata.
#[derive(Clone)]
pub struct AuthConfig {
    pub enabled: bool,
    /// Legacy bootstrap keys (raw strings).  These always grant Admin role.
    pub valid_keys: HashSet<String>,
    /// Optional reference to the state store for scoped key lookups.
    pub state_store: Option<Arc<RedisStateStore>>,
    /// Cache of recently-resolved scoped keys (hash → CallerContext).
    /// Populated on first lookup, invalidated on key revocation.
    pub key_cache: Arc<RwLock<std::collections::HashMap<String, CachedKey>>>,
}

/// A cached key lookup result.
#[derive(Clone)]
pub struct CachedKey {
    pub context: CallerContext,
    pub active: bool,
    pub cached_at: chrono::DateTime<chrono::Utc>,
}

/// P3-3: Cache TTL for key lookups (seconds).
const KEY_CACHE_TTL_SECS: i64 = 30;

/// P3-3: Maximum cache entries before forced eviction.
const KEY_CACHE_MAX_ENTRIES: usize = 10_000;

impl AuthConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            valid_keys: HashSet::new(),
            state_store: None,
            key_cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    pub fn with_keys(keys: Vec<String>) -> Self {
        Self {
            enabled: true,
            valid_keys: keys.into_iter().collect(),
            state_store: None,
            key_cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    pub fn with_keys_and_store(keys: Vec<String>, store: Arc<RedisStateStore>) -> Self {
        Self {
            enabled: true,
            valid_keys: keys.into_iter().collect(),
            state_store: Some(store),
            key_cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// P3-3: Evict stale entries from the key cache.
    /// Called periodically or when cache grows too large.
    pub async fn evict_stale_cache_entries(&self) {
        let now = chrono::Utc::now();
        let mut cache = self.key_cache.write().await;

        // Remove entries older than TTL
        cache.retain(|_, cached| {
            let age = now - cached.cached_at;
            age.num_seconds() < KEY_CACHE_TTL_SECS
        });

        // If still too large, remove oldest entries
        if cache.len() > KEY_CACHE_MAX_ENTRIES {
            let mut entries: Vec<_> = cache
                .iter()
                .map(|(k, v)| (k.clone(), v.cached_at))
                .collect();
            entries.sort_by(|a, b| a.1.cmp(&b.1));

            let to_remove = cache.len() - KEY_CACHE_MAX_ENTRIES;
            for (key, _) in entries.into_iter().take(to_remove) {
                cache.remove(&key);
            }
        }
    }
}

/// Tower Layer that applies API key authentication.
#[derive(Clone)]
pub struct AuthLayer {
    config: Arc<AuthConfig>,
}

impl AuthLayer {
    pub fn new(config: AuthConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthMiddleware {
            inner,
            config: self.config.clone(),
        }
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

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        // ── Auth disabled → implicit admin ──────────────────────
        if !self.config.enabled {
            req.extensions_mut()
                .insert(CallerContext::admin_bootstrap());
            let mut inner = self.inner.clone();
            return Box::pin(async move { inner.call(req).await });
        }

        // ── Exempt paths (anonymous / read-only context) ────────
        let path = req.uri().path();
        if path == "/health"
            || path == "/healthz"
            || path == "/metrics"
            || path.starts_with("/_/")
            || path == "/_"
            || path == "/api/v1/openapi.json"
            || path.starts_with("/swagger-ui")
        {
            req.extensions_mut().insert(CallerContext::anonymous());
            let mut inner = self.inner.clone();
            return Box::pin(async move { inner.call(req).await });
        }

        // ── Extract raw key from header ─────────────────────────
        let api_key = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|s| s.to_string())
            .or_else(|| {
                req.headers()
                    .get("x-api-key")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string())
            });

        let raw_key = match api_key {
            Some(k) => k,
            None => {
                let response = unauthorized_response();
                return Box::pin(async move { Ok(response) });
            }
        };

        let config = self.config.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            // ── Check bootstrap keys first (P2-1: constant-time comparison) ──
            if constant_time_key_match(&config.valid_keys, &raw_key) {
                req.extensions_mut()
                    .insert(CallerContext::admin_bootstrap());
                return inner.call(req).await;
            }

            // ── Check scoped keys via Redis ─────────────────────
            if let Some(ref store) = config.state_store {
                let key_hash = hash_api_key(&raw_key);

                // Check cache first (TTL-based)
                {
                    let cache = config.key_cache.read().await;
                    if let Some(cached) = cache.get(&key_hash) {
                        let age = chrono::Utc::now() - cached.cached_at;
                        if age.num_seconds() < KEY_CACHE_TTL_SECS && cached.active {
                            req.extensions_mut().insert(cached.context.clone());
                            return inner.call(req).await;
                        }
                    }
                }

                // Cache miss or stale → look up in Redis
                match store.get_api_key_by_hash(&key_hash).await {
                    Ok(Some(api_key)) if api_key.is_active() => {
                        let ctx = CallerContext {
                            key_id: api_key.id,
                            key_name: api_key.name.clone(),
                            role: api_key.role,
                            scope: api_key.scope.clone(),
                        };

                        // Update cache
                        {
                            let mut cache = config.key_cache.write().await;
                            cache.insert(
                                key_hash.clone(),
                                CachedKey {
                                    context: ctx.clone(),
                                    active: true,
                                    cached_at: chrono::Utc::now(),
                                },
                            );
                        }

                        // Best-effort: update last_used_at
                        let mut updated = api_key;
                        updated.last_used_at = Some(chrono::Utc::now());
                        let _ = store.update_api_key(&updated).await;

                        req.extensions_mut().insert(ctx);
                        return inner.call(req).await;
                    }
                    Ok(Some(_)) => {
                        // Key exists but is revoked or expired
                        return Ok(unauthorized_response());
                    }
                    Ok(None) => {
                        // Not a scoped key either
                        return Ok(unauthorized_response());
                    }
                    Err(_) => {
                        // Redis error — fail open or closed?
                        // Fail closed for security.
                        return Ok(unauthorized_response());
                    }
                }
            }

            // ── No match ────────────────────────────────────────
            Ok(unauthorized_response())
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    fn test_app(config: AuthConfig) -> Router {
        Router::new()
            .route("/private", get(|| async { StatusCode::OK }))
            .route("/health", get(|| async { StatusCode::OK }))
            .route("/metrics", get(|| async { StatusCode::OK }))
            .layer(AuthLayer::new(config))
    }

    #[tokio::test]
    async fn allows_requests_when_auth_disabled() {
        let app = test_app(AuthConfig::disabled());
        let req = Request::builder()
            .uri("/private")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn bypasses_health_without_api_key() {
        let app = test_app(AuthConfig::with_keys(vec!["secret".to_string()]));
        let req = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn bypasses_metrics_without_api_key() {
        let app = test_app(AuthConfig::with_keys(vec!["secret".to_string()]));
        let req = Request::builder()
            .uri("/metrics")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_missing_key_when_auth_enabled() {
        let app = test_app(AuthConfig::with_keys(vec!["secret".to_string()]));
        let req = Request::builder()
            .uri("/private")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn accepts_bearer_authorization() {
        let app = test_app(AuthConfig::with_keys(vec!["secret".to_string()]));
        let req = Request::builder()
            .uri("/private")
            .header("authorization", "Bearer secret")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn accepts_x_api_key_header() {
        let app = test_app(AuthConfig::with_keys(vec!["secret".to_string()]));
        let req = Request::builder()
            .uri("/private")
            .header("x-api-key", "secret")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    fn test_app_with_dashboard(config: AuthConfig) -> Router {
        Router::new()
            .route("/private", get(|| async { StatusCode::OK }))
            .route("/health", get(|| async { StatusCode::OK }))
            .route("/_/", get(|| async { StatusCode::OK }))
            .route("/_/static/style.css", get(|| async { StatusCode::OK }))
            .route("/_/webhooks", get(|| async { StatusCode::OK }))
            .layer(AuthLayer::new(config))
    }

    #[tokio::test]
    async fn bypasses_dashboard_routes_without_api_key() {
        let app = test_app_with_dashboard(AuthConfig::with_keys(vec!["secret".to_string()]));

        for path in &["/_/", "/_/static/style.css", "/_/webhooks"] {
            let req = Request::builder().uri(*path).body(Body::empty()).unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "dashboard path {path} should bypass auth"
            );
        }

        // API routes should still require auth
        let req = Request::builder()
            .uri("/private")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
