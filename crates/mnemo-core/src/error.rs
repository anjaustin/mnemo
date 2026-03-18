use thiserror::Error;
use uuid::Uuid;

/// Unified error type for the entire Mnemo system.
///
/// All crates use this error type to ensure consistent error handling
/// and propagation across module boundaries.
#[derive(Debug, Error)]
pub enum MnemoError {
    // ─── Resource errors ───────────────────────────────────────
    #[error("User not found: {0}")]
    UserNotFound(Uuid),

    #[error("Session not found: {0}")]
    SessionNotFound(Uuid),

    #[error("Episode not found: {0}")]
    EpisodeNotFound(Uuid),

    #[error("Entity not found: {0}")]
    EntityNotFound(Uuid),

    #[error("Edge not found: {0}")]
    EdgeNotFound(Uuid),

    #[error("Resource not found: {resource_type} {id}")]
    NotFound { resource_type: String, id: String },

    // ─── Validation errors ─────────────────────────────────────
    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Duplicate resource: {0}")]
    Duplicate(String),

    // ─── Storage errors ────────────────────────────────────────
    #[error("Redis error: {0}")]
    Redis(String),

    #[error("Qdrant error: {0}")]
    Qdrant(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    // ─── LLM errors ────────────────────────────────────────────
    #[error("LLM provider error: {provider} - {message}")]
    LlmProvider { provider: String, message: String },

    #[error("Embedding provider error: {provider} - {message}")]
    EmbeddingProvider { provider: String, message: String },

    #[error("LLM extraction failed: {0}")]
    ExtractionFailed(String),

    #[error("LLM rate limited: retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },

    // ─── Processing errors ─────────────────────────────────────
    #[error("Episode already claimed for processing: {0}")]
    AlreadyClaimed(Uuid),

    #[error("Processing timeout for episode: {0}")]
    ProcessingTimeout(Uuid),

    // ─── Auth errors ───────────────────────────────────────────
    #[error("Authentication required")]
    Unauthorized,

    #[error("Insufficient permissions")]
    Forbidden,

    #[error("Invalid API key")]
    InvalidApiKey,

    // ─── Infrastructure errors ─────────────────────────────────
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl MnemoError {
    /// HTTP status code for this error.
    pub fn status_code(&self) -> u16 {
        match self {
            Self::UserNotFound(_)
            | Self::SessionNotFound(_)
            | Self::EpisodeNotFound(_)
            | Self::EntityNotFound(_)
            | Self::EdgeNotFound(_)
            | Self::NotFound { .. } => 404,

            Self::Validation(_) => 400,
            Self::Duplicate(_) => 409,

            Self::Unauthorized | Self::InvalidApiKey => 401,
            Self::Forbidden => 403,

            Self::RateLimited { .. } => 429,
            Self::AlreadyClaimed(_) => 409,

            Self::LlmProvider { .. }
            | Self::EmbeddingProvider { .. }
            | Self::ExtractionFailed(_) => 502,

            _ => 500,
        }
    }

    /// Error code string for API responses.
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::UserNotFound(_) => "user_not_found",
            Self::SessionNotFound(_) => "session_not_found",
            Self::EpisodeNotFound(_) => "episode_not_found",
            Self::EntityNotFound(_) => "entity_not_found",
            Self::EdgeNotFound(_) => "edge_not_found",
            Self::NotFound { .. } => "not_found",
            Self::Validation(_) => "validation_error",
            Self::Duplicate(_) => "duplicate",
            Self::Unauthorized => "unauthorized",
            Self::InvalidApiKey => "invalid_api_key",
            Self::Forbidden => "forbidden",
            Self::RateLimited { .. } => "rate_limited",
            Self::AlreadyClaimed(_) => "already_claimed",
            _ => "internal_error",
        }
    }
}

/// Standard API error response body.
#[derive(Debug, serde::Serialize)]
pub struct ApiErrorResponse {
    pub error: ApiErrorDetail,
}

#[derive(Debug, serde::Serialize)]
pub struct ApiErrorDetail {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl From<MnemoError> for ApiErrorResponse {
    fn from(err: MnemoError) -> Self {
        let retry_after_ms = match &err {
            MnemoError::RateLimited { retry_after_ms } => Some(*retry_after_ms),
            _ => None,
        };
        Self {
            error: ApiErrorDetail {
                code: err.error_code().to_string(),
                // P3-4: Sanitize error messages to avoid leaking internal details
                message: sanitize_error_message(&err),
                retry_after_ms,
            },
        }
    }
}

/// P3-4: Sanitize error messages before returning to clients.
/// Replaces internal details (Redis errors, storage paths, etc.) with generic messages.
fn sanitize_error_message(err: &MnemoError) -> String {
    match err {
        // Safe to expose: these are user-facing errors with no internal details
        MnemoError::UserNotFound(id) => format!("User not found: {}", id),
        MnemoError::SessionNotFound(id) => format!("Session not found: {}", id),
        MnemoError::EpisodeNotFound(id) => format!("Episode not found: {}", id),
        MnemoError::EntityNotFound(id) => format!("Entity not found: {}", id),
        MnemoError::EdgeNotFound(id) => format!("Edge not found: {}", id),
        MnemoError::NotFound { resource_type, id } => {
            format!("{} not found: {}", resource_type, id)
        }
        MnemoError::Validation(msg) => format!("Validation error: {}", msg),
        MnemoError::Duplicate(msg) => format!("Duplicate resource: {}", msg),
        MnemoError::Forbidden => "Insufficient permissions".to_string(),
        MnemoError::Unauthorized => "Unauthorized".to_string(),
        MnemoError::InvalidApiKey => "Invalid API key".to_string(),
        MnemoError::RateLimited { retry_after_ms } => {
            format!("Rate limit exceeded, retry after {}ms", retry_after_ms)
        }
        MnemoError::AlreadyClaimed(_) => "Resource is being processed".to_string(),
        MnemoError::ProcessingTimeout(_) => "Processing timed out".to_string(),

        // Internal errors: sanitize to avoid leaking implementation details
        MnemoError::Redis(_) => "Storage service temporarily unavailable".to_string(),
        MnemoError::Qdrant(_) => "Vector service temporarily unavailable".to_string(),
        MnemoError::Storage(_) => "Storage error".to_string(),
        MnemoError::Serialization(_) => "Data serialization error".to_string(),
        MnemoError::Config(_) => "Server configuration error".to_string(),
        MnemoError::LlmProvider { .. } => "Language model service error".to_string(),
        MnemoError::EmbeddingProvider { .. } => "Embedding service error".to_string(),
        MnemoError::ExtractionFailed(_) => "Content extraction failed".to_string(),
        MnemoError::Internal(_) => "Internal server error".to_string(),
    }
}

// Conversion from common error types
impl From<serde_json::Error> for MnemoError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_status_codes() {
        assert_eq!(MnemoError::UserNotFound(Uuid::nil()).status_code(), 404);
        assert_eq!(MnemoError::Validation("bad".into()).status_code(), 400);
        assert_eq!(MnemoError::Unauthorized.status_code(), 401);
        assert_eq!(MnemoError::Forbidden.status_code(), 403);
        assert_eq!(MnemoError::Duplicate("x".into()).status_code(), 409);
        assert_eq!(
            MnemoError::RateLimited {
                retry_after_ms: 1000
            }
            .status_code(),
            429
        );
        assert_eq!(MnemoError::Internal("oops".into()).status_code(), 500);
    }

    #[test]
    fn test_api_error_response_serialization() {
        let err = MnemoError::UserNotFound(Uuid::nil());
        let resp = ApiErrorResponse::from(err);
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("user_not_found"));
        assert!(!json.contains("404")); // status code not in body
    }

    #[test]
    fn test_rate_limit_error_includes_retry() {
        let err = MnemoError::RateLimited {
            retry_after_ms: 5000,
        };
        let resp = ApiErrorResponse::from(err);
        assert_eq!(resp.error.retry_after_ms, Some(5000));
    }
}
