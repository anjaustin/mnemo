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
        BatchCreateEpisodesRequest, CreateEpisodeRequest, Episode, EpisodeType, MessageRole,
        ProcessingStatus,
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
    session::{CreateSessionRequest, Session, UpdateSessionRequest},
    span::LlmSpan,
    user::{CreateUserRequest, UpdateUserRequest, User},
    view::{CreateViewRequest, MemoryView, TemporalScope},
};

// ─── Route-level schemas ────────────────────────────────────────────
use crate::routes::PaginationParams;

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
        Session, CreateSessionRequest, UpdateSessionRequest,
        // Episodes
        Episode, CreateEpisodeRequest, BatchCreateEpisodesRequest,
        EpisodeType, MessageRole, ProcessingStatus,
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
        MemoryWebhookSubscription, MemoryWebhookEventRecord,
        MemoryWebhookEventType, MemoryWebhookAuditRecord,
        // Pagination
        PaginationParams,
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
    paths(
        // ── health ───────────────────────────────────────────────────
        crate::routes::health,
        crate::routes::metrics,
        // ── ops ──────────────────────────────────────────────────────
        crate::routes::get_ops_summary,
        crate::routes::get_ops_compression,
        crate::routes::get_ops_hyperbolic,
        crate::routes::get_ops_pipeline,
        crate::routes::get_ops_sync,
        crate::routes::get_ops_incidents,
        crate::routes::get_trace_by_request_id,
        crate::routes::export_webhook_evidence_bundle,
        crate::routes::export_governance_evidence_bundle,
        crate::routes::export_trace_evidence_bundle,
        crate::routes::audit_export,
        // ── keys ─────────────────────────────────────────────────────
        crate::routes::create_api_key,
        crate::routes::list_api_keys,
        crate::routes::revoke_api_key,
        crate::routes::rotate_api_key,
        // ── import ───────────────────────────────────────────────────
        crate::routes::import_chat_history,
        crate::routes::get_import_job,
        // ── users ────────────────────────────────────────────────────
        crate::routes::create_user,
        crate::routes::get_user,
        crate::routes::get_user_by_external_id,
        crate::routes::update_user,
        crate::routes::delete_user,
        crate::routes::list_users,
        // ── sessions ─────────────────────────────────────────────────
        crate::routes::create_session,
        crate::routes::get_session,
        crate::routes::update_session,
        crate::routes::delete_session,
        crate::routes::list_user_sessions,
        // ── episodes ─────────────────────────────────────────────────
        crate::routes::add_episode,
        crate::routes::add_episodes_batch,
        crate::routes::get_episode,
        crate::routes::list_episodes,
        crate::routes::get_session_messages,
        crate::routes::delete_session_messages,
        crate::routes::delete_session_message_by_idx,
        // ── memory ───────────────────────────────────────────────────
        crate::routes::get_context,
        crate::routes::remember_memory,
        crate::routes::extract_memory,
        crate::routes::get_memory_context,
        crate::routes::memory_changes_since,
        crate::routes::time_travel_trace,
        crate::routes::time_travel_summary,
        crate::routes::conflict_radar,
        crate::routes::causal_recall_chains,
        crate::routes::memory_retrieval_feedback,
        crate::routes::get_memory_digest,
        crate::routes::refresh_memory_digest,
        crate::routes::get_user_coherence,
        crate::routes::get_stale_facts,
        crate::routes::revalidate_fact,
        crate::routes::counterfactual_context,
        // ── entities ─────────────────────────────────────────────────
        crate::routes::list_entities,
        crate::routes::get_entity,
        crate::routes::delete_entity,
        crate::routes::patch_entity_classification,
        // ── edges ────────────────────────────────────────────────────
        crate::routes::query_edges,
        crate::routes::get_edge,
        crate::routes::delete_edge,
        crate::routes::patch_edge_classification,
        // ── graph ────────────────────────────────────────────────────
        crate::routes::get_subgraph,
        crate::routes::graph_list_entities,
        crate::routes::graph_get_entity,
        crate::routes::graph_list_edges,
        crate::routes::graph_neighbors,
        crate::routes::graph_community,
        crate::routes::graph_shortest_path,
        // ── spans ────────────────────────────────────────────────────
        crate::routes::list_spans_by_request,
        crate::routes::list_spans_by_user,
        // ── agents ───────────────────────────────────────────────────
        crate::routes::get_agent_identity,
        crate::routes::update_agent_identity,
        crate::routes::list_agent_identity_versions,
        crate::routes::list_agent_identity_audit,
        crate::routes::verify_agent_audit_chain,
        crate::routes::rollback_agent_identity,
        crate::routes::verified_identity_update,
        crate::routes::add_agent_experience,
        crate::routes::list_experience_importance,
        crate::routes::create_promotion_proposal,
        crate::routes::list_promotion_proposals,
        crate::routes::approve_promotion_proposal,
        crate::routes::reject_promotion_proposal,
        crate::routes::get_promotion_conflicts,
        crate::routes::set_agent_approval_policy,
        crate::routes::get_agent_approval_policy,
        crate::routes::create_agent_branch,
        crate::routes::fork_agent,
        crate::routes::list_agent_branches,
        crate::routes::get_agent_branch,
        crate::routes::update_agent_branch,
        crate::routes::merge_agent_branch,
        crate::routes::delete_agent_branch,
        crate::routes::get_agent_context,
        // ── views ────────────────────────────────────────────────────
        crate::routes::create_view,
        crate::routes::list_views,
        crate::routes::get_view,
        crate::routes::update_view,
        crate::routes::delete_view,
        // ── guardrails ───────────────────────────────────────────────
        crate::routes::create_guardrail,
        crate::routes::list_guardrails,
        crate::routes::get_guardrail,
        crate::routes::update_guardrail,
        crate::routes::delete_guardrail,
        crate::routes::evaluate_guardrails,
        // ── regions ──────────────────────────────────────────────────
        crate::routes::create_region,
        crate::routes::list_regions,
        crate::routes::get_region,
        crate::routes::update_region,
        crate::routes::delete_region,
        crate::routes::grant_region_access,
        crate::routes::list_region_acls,
        crate::routes::revoke_region_access,
        // ── webhooks ─────────────────────────────────────────────────
        crate::routes::register_memory_webhook,
        crate::routes::list_memory_webhooks,
        crate::routes::get_memory_webhook,
        crate::routes::update_memory_webhook,
        crate::routes::delete_memory_webhook,
        crate::routes::list_memory_webhook_events,
        crate::routes::replay_memory_webhook_events,
        crate::routes::retry_memory_webhook_event,
        crate::routes::list_memory_webhook_dead_letters,
        crate::routes::get_memory_webhook_stats,
        crate::routes::list_memory_webhook_audit,
        // ── goals ────────────────────────────────────────────────────
        crate::routes::list_goal_profiles,
        crate::routes::create_goal_profile,
        crate::routes::get_goal_profile,
        crate::routes::update_goal_profile,
        crate::routes::delete_goal_profile,
        // ── narrative ────────────────────────────────────────────────
        crate::routes::get_narrative,
        crate::routes::delete_narrative,
        crate::routes::refresh_narrative,
        // ── clarifications ───────────────────────────────────────────
        crate::routes::list_clarifications,
        crate::routes::generate_clarifications,
        crate::routes::resolve_clarification,
        crate::routes::dismiss_clarification,
        // ── vectors ──────────────────────────────────────────────────
        crate::routes::vectors_upsert,
        crate::routes::vectors_query,
        crate::routes::vectors_delete_ids,
        crate::routes::vectors_delete_namespace,
        crate::routes::vectors_count,
        crate::routes::vectors_exists,
    ),
    modifiers(&SecurityAddon),
)]
pub struct MnemoApiDoc;

/// Adds Bearer and API-key security schemes to the OpenAPI spec.
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            utoipa::openapi::security::SecurityScheme::Http(
                utoipa::openapi::security::HttpBuilder::new()
                    .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                    .bearer_format("Mnemo API Key")
                    .description(Some("Pass API key via `Authorization: Bearer <key>`"))
                    .build(),
            ),
        );
        components.add_security_scheme(
            "api_key",
            utoipa::openapi::security::SecurityScheme::ApiKey(
                utoipa::openapi::security::ApiKey::Header(
                    utoipa::openapi::security::ApiKeyValue::new("X-Api-Key"),
                ),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the full OpenAPI spec can be serialized to JSON without
    /// stack overflow.  This was previously impossible due to recursive
    /// `serde_json::Value` schemas; fixed by annotating all Value fields
    /// with `#[schema(value_type = Object)]`.
    #[test]
    fn openapi_to_json_does_not_overflow() {
        use utoipa::OpenApi;
        let json = MnemoApiDoc::openapi()
            .to_json()
            .expect("OpenAPI JSON serialization must not overflow");
        // Sanity: the spec must contain our title and at least one path
        assert!(json.contains("\"Mnemo API\""), "spec must contain title");
        assert!(
            json.contains("/api/v1/users"),
            "spec must contain user paths"
        );
        assert!(json.contains("/health"), "spec must contain health path");
        // Verify path count is reasonable (we registered 142 paths)
        let path_count = json.matches("\"summary\"").count();
        assert!(
            path_count >= 100,
            "expected at least 100 path summaries, got {path_count}"
        );
    }

    /// RT-11: signing_secret field must not appear in the
    /// MemoryWebhookSubscription schema.
    #[test]
    fn rt11_signing_secret_hidden_from_webhook_schema() {
        let mut collected = Vec::new();
        <MemoryWebhookSubscription as utoipa::ToSchema>::schemas(&mut collected);

        // Serialize every schema produced by the derive and verify none
        // contain "signing_secret" in the output.
        for (name, schema) in &collected {
            let json = serde_json::to_string(schema).unwrap();
            assert!(
                !json.contains("signing_secret"),
                "signing_secret must be hidden from schema '{name}' via #[schema(ignore)]"
            );
        }
    }

    /// RT-11: verify that the schemas(...) block in openapi.rs does NOT
    /// register internal types and DOES register PaginationParams.
    ///
    /// We read the source and check only the `components(schemas(...))`
    /// region (lines before the test module) to avoid false positives
    /// from assertion messages in this test.
    #[test]
    fn rt11_schema_registration_source_audit() {
        let source = include_str!("openapi.rs");

        // Extract just the portion before #[cfg(test)] — the actual
        // OpenAPI macro invocation.
        let openapi_src = source
            .split("#[cfg(test)]")
            .next()
            .expect("test module marker must exist");

        // ── Must NOT be registered ───────────────────────────────────
        assert!(
            !openapi_src.contains("IngestWebhookEvent"),
            "IngestWebhookEvent import/registration must be removed from OpenAPI spec"
        );
        assert!(
            !openapi_src.contains("ListSessionsParams"),
            "ListSessionsParams must not be registered in OpenAPI schemas"
        );
        assert!(
            !openapi_src.contains("ListEpisodesParams"),
            "ListEpisodesParams must not be registered in OpenAPI schemas"
        );

        // ── Must be registered ───────────────────────────────────────
        assert!(
            openapi_src.contains("PaginationParams"),
            "PaginationParams must be registered in OpenAPI schemas"
        );
    }
}
