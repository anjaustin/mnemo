//! OpenAPI specification for the Mnemo REST API.
//!
//! Generates the OpenAPI 3.1 JSON spec at `/api/v1/openapi.json` and serves
//! Swagger UI at `/swagger-ui/`.

use utoipa::OpenApi;

// ─── Core model schemas ─────────────────────────────────────────────
// Only import types that actually exist and have ToSchema derives.
use mnemo_core::models::{
    agent::{
        AgentIdentityAuditAction, AgentIdentityAuditEvent, AgentIdentityProfile,
        AllowlistMembershipProof, AllowlistMerkleTree, ApprovalPolicy, BranchInfo, BranchMetadata,
        ConflictAnalysis, CreateBranchRequest, CreateExperienceRequest,
        CreatePromotionProposalRequest, ExperienceEvent, ForkAgentRequest, ForkLineage, ForkResult,
        IdentityRollbackRequest, IdentityUpdateProof, MergeResult, PromotionProposal,
        PromotionStatus, SetApprovalPolicyRequest, UpdateAgentIdentityRequest,
        VerifiedIdentityUpdateRequest,
    },
    api_key::{ApiKey, ApiKeyRole, ApiKeyScope, CreateApiKeyRequest, CreateApiKeyResponse},
    clarification::{ClarificationRequest, ClarificationStatus, ResolveClarificationRequest},
    classification::Classification,
    context::{
        ContextBlock, ContextMessage, ContextRequest, EpisodeSummary, FactSummary, RetrievalSource,
        TemporalIntent,
    },
    counterfactual::{
        CounterfactualDiff, CounterfactualRequest, CounterfactualResponse, HypotheticalFact,
    },
    digest::MemoryDigest,
    edge::{Edge, EdgeFilter},
    entity::{Entity, EntityType},
    episode::{
        BatchCreateEpisodesRequest, CreateEpisodeRequest, Episode, EpisodeType, ListEpisodesParams,
        MessageRole, ProcessingStatus,
    },
    goal::GoalProfile,
    guardrail::{
        CreateGuardrailRequest, EvaluateGuardrailsRequest, EvaluateGuardrailsResponse,
        GuardrailAction, GuardrailCondition, GuardrailRule,
    },
    narrative::{NarrativeChapter, UserNarrative},
    region::{
        CreateRegionRequest, GrantRegionAccessRequest, MemoryRegion, MemoryRegionAcl,
        RegionPermission, UpdateRegionRequest,
    },
    session::{CreateSessionRequest, ListSessionsParams, Session, UpdateSessionRequest},
    span::LlmSpan,
    user::{CreateUserRequest, UpdateUserRequest, User},
    view::{CreateViewRequest, MemoryView, TemporalScope},
    webhook_event::IngestWebhookEvent,
};

// ─── Server-specific schemas ────────────────────────────────────────
use crate::state::{
    GovernanceAuditRecord, ImportJobRecord, ImportJobStatus, MemoryWebhookAuditRecord,
    MemoryWebhookEventRecord, MemoryWebhookEventType, MemoryWebhookSubscription, UserPolicyRecord,
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Mnemo API",
        version = "0.7.0",
        description = "Memory infrastructure for production AI agents — temporal knowledge graph, hybrid retrieval, enterprise access control.\n\n## Authentication\n\nAll endpoints (except `/health`, `/healthz`, `/metrics`, and `/_/*`) require authentication via one of:\n- `Authorization: Bearer <api-key>` header\n- `X-Api-Key: <api-key>` header\n\n## gRPC\n\nMnemo also serves gRPC on the same port. See the proto schema at `proto/mnemo/v1/memory.proto` or use gRPC reflection.",
        license(name = "Apache-2.0", url = "https://www.apache.org/licenses/LICENSE-2.0"),
        contact(name = "Mnemo", url = "https://github.com/anjaustin/mnemo"),
    ),
    servers(
        (url = "/", description = "Current server"),
    ),
    components(schemas(
        // Users
        User, CreateUserRequest, UpdateUserRequest,
        // Sessions
        Session, CreateSessionRequest, UpdateSessionRequest, ListSessionsParams,
        // Episodes
        Episode, CreateEpisodeRequest, BatchCreateEpisodesRequest,
        ListEpisodesParams, EpisodeType, MessageRole, ProcessingStatus,
        // Entities
        Entity, EntityType,
        // Edges
        Edge, EdgeFilter,
        // Context & retrieval
        ContextBlock, ContextMessage, ContextRequest,
        EpisodeSummary, FactSummary, RetrievalSource, TemporalIntent,
        // Classification
        Classification,
        // Agent identity
        AgentIdentityProfile, ExperienceEvent, CreateExperienceRequest,
        AgentIdentityAuditEvent, AgentIdentityAuditAction,
        UpdateAgentIdentityRequest, VerifiedIdentityUpdateRequest,
        IdentityUpdateProof, IdentityRollbackRequest,
        AllowlistMembershipProof, AllowlistMerkleTree,
        BranchMetadata, BranchInfo, CreateBranchRequest, MergeResult,
        ForkAgentRequest, ForkResult, ForkLineage,
        // Promotions
        PromotionProposal, PromotionStatus, CreatePromotionProposalRequest,
        ApprovalPolicy, SetApprovalPolicyRequest,
        ConflictAnalysis,
        // API keys
        CreateApiKeyRequest, CreateApiKeyResponse, ApiKey, ApiKeyRole, ApiKeyScope,
        // Views
        CreateViewRequest, MemoryView, TemporalScope,
        // Guardrails
        CreateGuardrailRequest, GuardrailRule, GuardrailCondition, GuardrailAction,
        EvaluateGuardrailsRequest, EvaluateGuardrailsResponse,
        // Regions
        CreateRegionRequest, MemoryRegion, MemoryRegionAcl,
        GrantRegionAccessRequest, UpdateRegionRequest, RegionPermission,
        // Goals
        GoalProfile,
        // Narrative
        UserNarrative, NarrativeChapter,
        // Clarifications
        ClarificationRequest, ClarificationStatus, ResolveClarificationRequest,
        // Counterfactual
        CounterfactualRequest, CounterfactualResponse, CounterfactualDiff, HypotheticalFact,
        // Digest
        MemoryDigest,
        // Spans
        LlmSpan,
        // Webhooks
        IngestWebhookEvent, MemoryWebhookSubscription, MemoryWebhookEventRecord,
        MemoryWebhookEventType, MemoryWebhookAuditRecord,
        // State
        ImportJobRecord, ImportJobStatus, UserPolicyRecord, GovernanceAuditRecord,
    )),
    tags(
        (name = "health", description = "Health check endpoints"),
        (name = "users", description = "User lifecycle management"),
        (name = "sessions", description = "Session management"),
        (name = "episodes", description = "Episode (message) ingestion and retrieval"),
        (name = "memory", description = "High-level memory operations — context, search, time travel"),
        (name = "entities", description = "Knowledge graph entity operations"),
        (name = "edges", description = "Knowledge graph edge (relationship) operations"),
        (name = "graph", description = "Graph traversal — neighbors, community detection, shortest path"),
        (name = "agents", description = "Agent identity, experience, promotions, branches, forking"),
        (name = "keys", description = "API key management (RBAC)"),
        (name = "views", description = "Policy-scoped memory views"),
        (name = "guardrails", description = "Memory guardrails engine"),
        (name = "regions", description = "Multi-agent shared memory regions with ACLs"),
        (name = "webhooks", description = "Webhook subscription and event management"),
        (name = "goals", description = "Goal-conditioned memory profiles"),
        (name = "narrative", description = "Cross-session narrative summaries"),
        (name = "clarifications", description = "Self-healing memory — conflict clarification"),
        (name = "ops", description = "Operator and observability endpoints"),
        (name = "import", description = "Chat history import"),
        (name = "vectors", description = "Raw vector API for direct Qdrant access"),
    ),
)]
pub struct MnemoApiDoc;
