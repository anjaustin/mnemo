//! Context assembly for LLM prompts.
//!
//! Defines [`ContextRequest`] parameters and [`ContextBlock`] output used to
//! assemble memory-aware prompts with token budgeting, temporal intent
//! detection, entity/fact summaries, routing decisions, and optional narrative
//! preamble.
//!
//! Spec 04 additions:
//! - [`StructuredContext`] — structured breakdown of facts, changes, relationships
//! - [`RetrievalExplanation`] / [`ExplanationCollector`] — why each fact was included
//! - Tiered token budgeting in [`ContextBlock::assemble_tiered`]
//!
//! Multi-modal additions:
//! - `include_modalities` — filter retrieval by content modality (text, image, audio, document)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::attachment::Modality;

/// A context block is the primary output of Mnemo — a pre-assembled,
/// token-efficient representation of relevant knowledge for an AI agent.
///
/// Context blocks combine results from multiple retrieval strategies
/// (semantic search, full-text search, graph traversal) into a single
/// string optimized for LLM consumption.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ContextBlock {
    /// The assembled context string, ready to inject into a system prompt.
    pub context: String,

    /// Approximate token count of the context string.
    pub token_count: u32,

    /// Entities included in this context.
    pub entities: Vec<EntitySummary>,

    /// Facts/edges included in this context.
    pub facts: Vec<FactSummary>,

    /// Source episodes referenced.
    pub episodes: Vec<EpisodeSummary>,

    /// How long the retrieval took (ms).
    pub latency_ms: u64,

    /// Which retrieval strategies contributed to this context.
    pub sources: Vec<RetrievalSource>,

    /// Optional diagnostics for temporal scoring and intent resolution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_diagnostics: Option<TemporalDiagnostics>,

    /// Optional semantic routing diagnostics (strategy selection, confidence, alternatives).
    /// Set by the server layer when the semantic router auto-classifies the query.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub routing_decision: Option<serde_json::Value>,

    // ── Spec 04 additions ──────────────────────────────────────────
    /// Structured breakdown of the context (opt-in via `?structured=true`).
    /// Separates key facts, recent changes, relationships, and open questions
    /// so callers can render them distinctly instead of parsing flat text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured: Option<StructuredContext>,

    /// Per-fact retrieval explanations (opt-in via `?explain=true`).
    /// Documents why each fact/entity was selected by the retrieval pipeline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanations: Option<Vec<RetrievalExplanation>>,

    /// The query intent detected by the classifier (always present).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_type: Option<String>,

    // ── Multi-modal additions ──────────────────────────────────────
    /// Attachment sources referenced in this context (multi-modal memories).
    /// Only populated when include_modalities includes non-text modalities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentSource>,
}

// ─── D2: Structured Context ────────────────────────────────────────

/// A structured breakdown of the assembled context (Spec 04 D2).
///
/// Populated when `structured=true` is passed in the request. Callers
/// can use these fields to render context distinctly (e.g., show recent
/// changes in a diff view, relationships in a graph, etc.) rather than
/// parsing the flat `context` string.
#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct StructuredContext {
    /// The top-ranked currently-valid facts by relevance.
    pub key_facts: Vec<KeyFact>,

    /// Facts that were superseded within the last 30 days (belief changes).
    pub recent_changes: Vec<RecentChange>,

    /// Graph edges between query-relevant entities.
    pub relationships: Vec<RelationshipSummary>,

    /// Entities or topics mentioned in the query but not found in memory,
    /// or facts with very low confidence. Empty when everything is covered.
    pub open_questions: Vec<String>,
}

/// A key fact extracted from the top-ranked valid edges.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct KeyFact {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub valid_at: DateTime<Utc>,
    pub confidence: f32,
    pub source_fact_id: Uuid,
}

/// A recent change: a fact that was superseded within the last 30 days.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RecentChange {
    /// Human-readable description of the change.
    pub fact: String,
    /// When the new fact became valid (i.e., when the change occurred).
    pub changed_at: DateTime<Utc>,
    /// The fact ID of the superseding edge.
    pub source_fact_id: Uuid,
}

/// A relationship between two entities from graph traversal.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RelationshipSummary {
    pub from: String,
    pub relation: String,
    pub to: String,
    pub confidence: f32,
}

// ─── D3: Retrieval Explanations ────────────────────────────────────

/// Why a specific fact or entity was included in the context (Spec 04 D3).
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RetrievalExplanation {
    /// The fact or entity ID this explanation refers to.
    pub id: Uuid,
    /// Primary reason this item was selected.
    pub reason: RetrievalReason,
    /// Human-readable detail about the reason.
    pub detail: String,
}

/// The reason a retrieval result was included in the context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalReason {
    /// Vector similarity above threshold.
    SemanticMatch,
    /// Connected to a query entity via graph traversal.
    GraphConnection,
    /// Latest valid fact for this subject-predicate pair.
    MostRecentBelief,
    /// Within the requested time window.
    TemporalRelevance,
    /// Included because the active memory contract mandates it.
    ContractRequired,
    /// High access count (frequently referenced).
    Reinforced,
    /// Matched a full-text keyword search.
    FullTextMatch,
}

/// Accumulates retrieval explanations as results flow through the pipeline.
///
/// Only instantiated when `explain=true` is requested — zero overhead
/// for the common case.
#[derive(Debug, Default)]
pub struct ExplanationCollector {
    pub explanations: Vec<RetrievalExplanation>,
}

impl ExplanationCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that a fact was included for a given reason.
    pub fn record(&mut self, id: Uuid, reason: RetrievalReason, detail: impl Into<String>) {
        // Avoid duplicate entries for the same id (graph traversal may revisit)
        if self.explanations.iter().any(|e| e.id == id) {
            return;
        }
        self.explanations.push(RetrievalExplanation {
            id,
            reason,
            detail: detail.into(),
        });
    }

    /// Consume the collector, returning the accumulated explanations.
    pub fn finish(self) -> Vec<RetrievalExplanation> {
        self.explanations
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TemporalDiagnostics {
    pub resolved_intent: TemporalIntent,
    pub temporal_weight: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<DateTime<Utc>>,
    pub entities_scored: u32,
    pub facts_scored: u32,
    pub episodes_scored: u32,
}

/// Lightweight entity reference included in context responses.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct EntitySummary {
    pub id: Uuid,
    pub name: String,
    pub entity_type: String,
    /// Classification level of this entity (for view-scoped filtering).
    #[serde(default)]
    pub classification: super::classification::Classification,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Relevance score from retrieval (0.0–1.0).
    pub relevance: f32,
}

/// Lightweight fact reference included in context responses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
pub struct FactSummary {
    pub id: Uuid,
    pub source_entity: String,
    pub target_entity: String,
    pub label: String,
    pub fact: String,
    /// Classification level of this fact/edge (for view-scoped filtering).
    #[serde(default)]
    pub classification: super::classification::Classification,
    pub valid_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_at: Option<DateTime<Utc>>,
    pub relevance: f32,
    /// How many times this fact has been returned in retrieval (Spec 03 D3).
    #[serde(default)]
    pub access_count: u32,
    /// When this fact was last returned in retrieval (Spec 03 D3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_accessed_at: Option<DateTime<Utc>>,
    /// Temporal scope of this fact (Spec 03 D2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_scope: Option<String>,
}

/// Lightweight episode reference included in context responses.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct EpisodeSummary {
    pub id: Uuid,
    pub session_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Truncated content preview.
    pub preview: String,
    pub created_at: DateTime<Utc>,
    pub relevance: f32,
}

/// A multi-modal attachment source referenced in the context.
///
/// Returned in `ContextBlock::attachments` when the retrieval includes
/// non-text modalities (images, audio, documents).
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AttachmentSource {
    /// The attachment ID.
    pub attachment_id: Uuid,
    /// The episode this attachment belongs to.
    pub episode_id: Uuid,
    /// The modality of this attachment.
    pub modality: Modality,
    /// MIME type of the attachment.
    pub mime_type: String,
    /// Pre-signed download URL (if available, expires in 15 min).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    /// Pre-signed thumbnail URL for images (if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    /// Vision-generated description or transcript excerpt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Relevance score (0.0–1.0).
    pub relevance_score: f32,
}

/// Which retrieval strategy contributed results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalSource {
    SemanticSearch,
    FullTextSearch,
    GraphTraversal,
    TemporalScoring,
    EpisodeRecall,
    SessionSummary,
}

/// The search types a caller can request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchType {
    Semantic,
    FullText,
    Graph,
    Hybrid,
}

/// Temporal intent guides how recency and validity are weighted during retrieval.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TemporalIntent {
    /// Let Mnemo infer intent from the query text.
    #[default]
    Auto,
    /// Prefer currently valid state.
    Current,
    /// Prefer recently changed/mentioned information.
    Recent,
    /// Prefer historical validity alignment (often with `as_of`).
    Historical,
}

/// Request for context retrieval.
#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ContextRequest {
    /// The session to retrieve context for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,

    /// Recent messages to use as the query context.
    /// Mnemo uses the last N messages to formulate the retrieval query.
    #[serde(default)]
    pub messages: Vec<ContextMessage>,

    /// Maximum token budget for the returned context.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,

    /// Which retrieval strategies to use.
    #[serde(default = "default_search_types")]
    pub search_types: Vec<SearchType>,

    /// Optional: only return facts valid at this point in time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_filter: Option<DateTime<Utc>>,

    /// Optional explicit point-in-time target for historical recall.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<DateTime<Utc>>,

    /// How temporal relevance should influence ranking.
    #[serde(default = "default_temporal_intent")]
    pub time_intent: TemporalIntent,

    /// Optional override for temporal weighting (0.0–1.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_weight: Option<f32>,

    /// Minimum relevance threshold (0.0–1.0). Results below this are dropped.
    #[serde(default = "default_min_relevance")]
    pub min_relevance: f32,

    /// Restrict retrieval to memories produced by a specific agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,

    /// Restrict retrieval to specific memory regions by ID.
    #[serde(default)]
    pub region_ids: Vec<Uuid>,

    // ── Spec 04 additions ──────────────────────────────────────────
    /// When `true`, the response includes a `structured` field with
    /// key_facts, recent_changes, relationships, and open_questions.
    /// Opt-in (default false) to avoid response size bloat.
    #[serde(default)]
    pub structured: bool,

    /// When `true`, the response includes an `explanations` field that
    /// documents why each fact/entity was included in the context.
    /// Opt-in (default false).
    #[serde(default)]
    pub explain: bool,

    /// When `true`, use tiered token budgeting (verbatim → compressed → references)
    /// instead of binary include/exclude. Requires `structured=false` to work
    /// independently. Default: false (uses existing `assemble()` behaviour).
    #[serde(default)]
    pub tiered_budget: bool,

    // ── Multi-modal additions ──────────────────────────────────────
    /// Filter retrieval to only include memories from specific modalities.
    /// When empty (default), all modalities are included.
    /// Example: `["text", "image"]` returns only text and image memories.
    #[serde(default)]
    pub include_modalities: Vec<Modality>,
}

/// A message provided as query context for retrieval.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ContextMessage {
    pub role: String,
    pub content: String,
}

fn default_max_tokens() -> u32 {
    500
}

fn default_search_types() -> Vec<SearchType> {
    vec![SearchType::Hybrid]
}

fn default_min_relevance() -> f32 {
    0.3
}

fn default_temporal_intent() -> TemporalIntent {
    TemporalIntent::Auto
}

/// Generic search request for the search API.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SearchRequest {
    /// The search query text.
    pub query: String,

    /// Search type to use.
    #[serde(default = "default_search_type")]
    pub search_type: SearchType,

    /// Maximum number of results.
    #[serde(default = "default_search_limit")]
    pub limit: u32,

    /// Minimum relevance threshold.
    #[serde(default = "default_min_relevance")]
    pub min_relevance: f32,

    /// Optional temporal filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_filter: Option<DateTime<Utc>>,
}

fn default_search_type() -> SearchType {
    SearchType::Hybrid
}

fn default_search_limit() -> u32 {
    10
}

/// A search result with score and source.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SearchResult {
    /// What kind of object was matched.
    pub result_type: SearchResultType,

    /// The matched object's ID.
    pub id: Uuid,

    /// Relevance score (0.0–1.0).
    pub score: f32,

    /// The content/fact text that matched.
    pub content: String,

    /// Which retrieval source found this result.
    pub source: RetrievalSource,

    /// Additional details depending on result_type.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchResultType {
    Entity,
    Edge,
    Episode,
}

/// Approximate token counting.
/// Uses the rough heuristic of ~4 chars per token (GPT-family average).
/// For production, swap in tiktoken or a proper tokenizer.
pub fn estimate_tokens(text: &str) -> u32 {
    (text.len() as f64 / 4.0).ceil() as u32
}

impl ContextBlock {
    /// Create an empty context block.
    pub fn empty() -> Self {
        Self {
            context: String::new(),
            token_count: 0,
            entities: Vec::new(),
            facts: Vec::new(),
            episodes: Vec::new(),
            latency_ms: 0,
            sources: Vec::new(),
            temporal_diagnostics: None,
            routing_decision: None,
            structured: None,
            explanations: None,
            query_type: None,
            attachments: Vec::new(),
        }
    }

    /// Build the context string from the assembled components.
    ///
    /// Format is designed to be LLM-friendly: structured but natural language.
    pub fn assemble(&mut self, max_tokens: u32) {
        let mut parts: Vec<String> = Vec::new();
        let mut running_tokens: u32 = 0;

        // 1. Entity summaries (most compact, highest signal)
        if !self.entities.is_empty() {
            let header = "Known entities:\n";
            let header_tokens = estimate_tokens(header);
            if running_tokens + header_tokens <= max_tokens {
                let mut entity_section = String::from(header);
                let mut items_added = 0;
                running_tokens += header_tokens;

                for e in &self.entities {
                    let line = match &e.summary {
                        Some(s) => format!("- {} ({}): {}\n", e.name, e.entity_type, s),
                        None => format!("- {} ({})\n", e.name, e.entity_type),
                    };
                    let tokens = estimate_tokens(&line);
                    if running_tokens + tokens > max_tokens {
                        break;
                    }
                    entity_section.push_str(&line);
                    running_tokens += tokens;
                    items_added += 1;
                }

                if items_added > 0 {
                    parts.push(entity_section);
                } else {
                    running_tokens -= header_tokens; // Reclaim header tokens
                }
            }
        }

        // 2. Current facts (temporal edges)
        if !self.facts.is_empty() {
            let header = "Current facts:\n";
            let header_tokens = estimate_tokens(header);
            if running_tokens + header_tokens <= max_tokens {
                let mut facts_section = String::from(header);
                let mut items_added = 0;
                running_tokens += header_tokens;

                for f in &self.facts {
                    let validity = if f.invalid_at.is_some() {
                        " [superseded]"
                    } else {
                        ""
                    };
                    let line = format!("- {}{}\n", f.fact, validity);
                    let tokens = estimate_tokens(&line);
                    if running_tokens + tokens > max_tokens {
                        break;
                    }
                    facts_section.push_str(&line);
                    running_tokens += tokens;
                    items_added += 1;
                }

                if items_added > 0 {
                    parts.push(facts_section);
                } else {
                    running_tokens -= header_tokens;
                }
            }
        }

        // 3. Relevant conversation excerpts
        if !self.episodes.is_empty() {
            let header = "Relevant conversation history:\n";
            let header_tokens = estimate_tokens(header);
            if running_tokens + header_tokens <= max_tokens {
                let mut episode_section = String::from(header);
                let mut items_added = 0;
                running_tokens += header_tokens;

                for ep in &self.episodes {
                    let role = ep.role.as_deref().unwrap_or("unknown");
                    let line = format!(
                        "- [{}] {}: {}\n",
                        ep.created_at.format("%Y-%m-%d"),
                        role,
                        ep.preview
                    );
                    let tokens = estimate_tokens(&line);
                    if running_tokens + tokens > max_tokens {
                        break;
                    }
                    episode_section.push_str(&line);
                    running_tokens += tokens;
                    items_added += 1;
                }

                if items_added > 0 {
                    parts.push(episode_section);
                }
            }
        }

        self.context = parts.join("\n");
        self.token_count = estimate_tokens(&self.context);
    }

    // ── D4: Tiered token budgeting ─────────────────────────────────

    /// Assemble the context string with three-tier token budgeting (Spec 04 D4).
    ///
    /// Partition results by relevance score into:
    ///   - Tier 1 (top `tier1_ratio` of budget): verbatim, highest-ranked facts
    ///   - Tier 2 (next `tier2_ratio` of budget): compressed via `summarize_fn` callback
    ///     (first-sentence fallback when LLM is unavailable)
    ///   - Tier 3 (remaining budget): one-line references `entity pred object (date)`
    ///
    /// `summarize_fn` receives the fact text and a max_tokens budget for Tier 2.
    /// When `None`, Tier 2 falls back to first-sentence truncation.
    ///
    /// Tier ratios are configurable via `TierConfig` (defaults: 0.60 / 0.25 / 0.15).
    #[allow(clippy::type_complexity)]
    pub fn assemble_tiered(
        &mut self,
        max_tokens: u32,
        tier_config: &TierConfig,
        mut summarize_fn: Option<&mut dyn FnMut(&str, u32) -> String>,
    ) {
        let tier1_budget = (max_tokens as f32 * tier_config.tier1_ratio) as u32;
        let tier2_budget = (max_tokens as f32 * tier_config.tier2_ratio) as u32;
        let _tier3_budget = max_tokens.saturating_sub(tier1_budget + tier2_budget);

        // Sort facts by relevance descending (should already be sorted, but ensure)
        let mut facts = self.facts.clone();
        facts.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Partition into tiers by score thresholds
        // Tier 1: top 60% by relevance score (score > tier2_threshold)
        // We use score-based partition rather than count to respect relative quality.
        let max_rel = facts.first().map(|f| f.relevance).unwrap_or(1.0);
        let tier1_threshold = max_rel * (1.0 - tier_config.tier1_ratio);
        let tier2_threshold = max_rel * tier_config.tier3_ratio;

        let (tier1, rest): (Vec<FactSummary>, Vec<FactSummary>) = facts
            .into_iter()
            .partition(|f| f.relevance >= tier1_threshold);
        let (tier2, tier3): (Vec<FactSummary>, Vec<FactSummary>) = rest
            .into_iter()
            .partition(|f| f.relevance >= tier2_threshold);

        let mut parts: Vec<String> = Vec::new();
        let mut running = 0u32;

        // ── Tier 1: verbatim ──────────────────────────────────────
        if !tier1.is_empty() {
            let header = "Key facts:\n";
            let htok = estimate_tokens(header);
            if running + htok <= tier1_budget {
                let mut section = String::from(header);
                running += htok;
                for f in &tier1 {
                    let line = format!("- {}\n", f.fact);
                    let tok = estimate_tokens(&line);
                    if running + tok > tier1_budget {
                        break;
                    }
                    section.push_str(&line);
                    running += tok;
                }
                parts.push(section);
            }
        }

        // ── Tier 2: compressed ────────────────────────────────────
        if !tier2.is_empty() && running < tier1_budget + tier2_budget {
            let t2_remaining = (tier1_budget + tier2_budget).saturating_sub(running);
            let header = "Additional context:\n";
            let htok = estimate_tokens(header);
            if t2_remaining > htok {
                let mut section = String::from(header);
                running += htok;
                for f in &tier2 {
                    let available = (tier1_budget + tier2_budget).saturating_sub(running);
                    if available == 0 {
                        break;
                    }
                    let compressed = if let Some(ref mut summarize) = summarize_fn {
                        // LLM-assisted: ask for ~30% of original token count
                        let target = (estimate_tokens(&f.fact) as f32 * 0.30).ceil() as u32;
                        let target = target.max(10).min(available);
                        summarize(&f.fact, target)
                    } else {
                        // Fallback: first sentence or first 80 chars
                        first_sentence(&f.fact, available)
                    };
                    let line = format!("- {}\n", compressed);
                    let tok = estimate_tokens(&line);
                    if running + tok > tier1_budget + tier2_budget {
                        break;
                    }
                    section.push_str(&line);
                    running += tok;
                }
                parts.push(section);
            }
        }

        // ── Tier 3: one-line references ────────────────────────────
        if !tier3.is_empty() && running < max_tokens {
            let header = "Also relevant:\n";
            let htok = estimate_tokens(header);
            if running + htok < max_tokens {
                let mut section = String::from(header);
                running += htok;
                for f in &tier3 {
                    let line = format!(
                        "- {} {} {} ({})\n",
                        f.source_entity,
                        f.label,
                        f.target_entity,
                        f.valid_at.format("%b %Y")
                    );
                    let tok = estimate_tokens(&line);
                    if running + tok > max_tokens {
                        break;
                    }
                    section.push_str(&line);
                    running += tok;
                }
                parts.push(section);
            }
        }

        // Include entities and episodes in any remaining budget (same as assemble())
        let entity_budget = max_tokens.saturating_sub(running);
        if !self.entities.is_empty() && entity_budget > 0 {
            let header = "Known entities:\n";
            let htok = estimate_tokens(header);
            if running + htok <= max_tokens {
                let mut section = String::from(header);
                running += htok;
                for e in &self.entities {
                    let line = match &e.summary {
                        Some(s) => format!("- {} ({}): {}\n", e.name, e.entity_type, s),
                        None => format!("- {} ({})\n", e.name, e.entity_type),
                    };
                    let tok = estimate_tokens(&line);
                    if running + tok > max_tokens {
                        break;
                    }
                    section.push_str(&line);
                    running += tok;
                }
                parts.push(section);
            }
        }

        self.context = parts.join("\n");
        self.token_count = estimate_tokens(&self.context);
    }

    // ── D2: Build structured context ──────────────────────────────

    /// Build the `StructuredContext` supplement from this block's facts and entities.
    ///
    /// Call after the retrieval pipeline has populated `facts` and `entities`.
    /// The caller decides whether to attach the result to `self.structured`.
    pub fn build_structured(&self) -> StructuredContext {
        let now = chrono::Utc::now();
        let thirty_days_ago = now - chrono::Duration::days(30);

        // key_facts: top-ranked valid facts (no invalid_at), up to 10
        let key_facts: Vec<KeyFact> = self
            .facts
            .iter()
            .filter(|f| f.invalid_at.is_none())
            .take(10)
            .map(|f| KeyFact {
                subject: f.source_entity.clone(),
                predicate: f.label.clone(),
                object: f.target_entity.clone(),
                valid_at: f.valid_at,
                confidence: f.relevance,
                source_fact_id: f.id,
            })
            .collect();

        // recent_changes: facts that became valid in the last 30 days
        let recent_changes: Vec<RecentChange> = self
            .facts
            .iter()
            .filter(|f| f.valid_at >= thirty_days_ago)
            .take(5)
            .map(|f| RecentChange {
                fact: f.fact.clone(),
                changed_at: f.valid_at,
                source_fact_id: f.id,
            })
            .collect();

        // relationships: facts that look like entity→entity edges (from graph traversal)
        let relationships: Vec<RelationshipSummary> = self
            .facts
            .iter()
            .filter(|f| !f.source_entity.is_empty() && !f.target_entity.is_empty())
            .take(8)
            .map(|f| RelationshipSummary {
                from: f.source_entity.clone(),
                relation: f.label.clone(),
                to: f.target_entity.clone(),
                confidence: f.relevance,
            })
            .collect();

        StructuredContext {
            key_facts,
            recent_changes,
            relationships,
            open_questions: Vec::new(), // populated by caller when absent detection fires
        }
    }
}

/// Configuration for tiered token budgeting (Spec 04 D4).
///
/// Ratios must sum to approximately 1.0. Values are clamped and normalised
/// at runtime if they don't sum exactly.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TierConfig {
    /// Fraction of budget for verbatim (Tier 1). Default: 0.60.
    pub tier1_ratio: f32,
    /// Fraction of budget for compressed (Tier 2). Default: 0.25.
    pub tier2_ratio: f32,
    /// Fraction of budget for one-line references (Tier 3). Default: 0.15.
    pub tier3_ratio: f32,
}

impl Default for TierConfig {
    fn default() -> Self {
        Self {
            tier1_ratio: 0.60,
            tier2_ratio: 0.25,
            tier3_ratio: 0.15,
        }
    }
}

impl TierConfig {
    /// Build from environment variables, falling back to defaults.
    ///
    /// Reads `MNEMO_CONTEXT_TIER1_RATIO`, `MNEMO_CONTEXT_TIER2_RATIO`,
    /// `MNEMO_CONTEXT_TIER3_RATIO`.
    pub fn from_env() -> Self {
        let tier1 = std::env::var("MNEMO_CONTEXT_TIER1_RATIO")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.60);
        let tier2 = std::env::var("MNEMO_CONTEXT_TIER2_RATIO")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.25);
        let tier3 = std::env::var("MNEMO_CONTEXT_TIER3_RATIO")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.15);
        // Clamp each ratio, then normalize so they sum to 1.0
        let (t1, t2, t3) = (
            tier1.clamp(0.05, 0.90),
            tier2.clamp(0.05, 0.90),
            tier3.clamp(0.05, 0.90),
        );
        let total = t1 + t2 + t3;
        Self {
            tier1_ratio: t1 / total,
            tier2_ratio: t2 / total,
            tier3_ratio: t3 / total,
        }
    }
}

/// Extract the first sentence (up to max_tokens worth of text).
fn first_sentence(text: &str, max_tokens: u32) -> String {
    let end = text
        .find(['.', '!', '?'])
        .map(|i| i + 1)
        .unwrap_or(text.len());
    let sentence = &text[..end];
    // Truncate to max_tokens budget
    let max_chars = (max_tokens * 4) as usize; // ~4 chars per token
    if sentence.len() > max_chars {
        format!("{}…", &sentence[..max_chars.min(sentence.len())])
    } else {
        sentence.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        // ~4 chars per token
        assert_eq!(estimate_tokens("hello world"), 3); // 11 chars / 4 = 2.75 → 3
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1);
    }

    #[test]
    fn test_context_block_assembly() {
        let mut block = ContextBlock::empty();
        block.entities.push(EntitySummary {
            id: Uuid::now_v7(),
            name: "Kendra".to_string(),
            entity_type: "person".to_string(),
            classification: Default::default(),
            summary: Some("A customer who likes athletic shoes".to_string()),
            relevance: 0.95,
        });
        block.facts.push(FactSummary {
            id: Uuid::now_v7(),
            source_entity: "Kendra".to_string(),
            target_entity: "Nike shoes".to_string(),
            label: "prefers".to_string(),
            fact: "Kendra now prefers Nike shoes after switching from Adidas".to_string(),
            classification: Default::default(),
            valid_at: Utc::now(),
            invalid_at: None,
            relevance: 0.92,
            access_count: 0,
            last_accessed_at: None,
            temporal_scope: None,
        });

        block.assemble(500);

        assert!(!block.context.is_empty());
        assert!(block.context.contains("Kendra"));
        assert!(block.context.contains("Nike"));
        assert!(block.token_count > 0);
    }

    #[test]
    fn test_context_block_respects_token_budget() {
        let mut block = ContextBlock::empty();
        // Add many facts to exceed budget
        for i in 0..100 {
            block.facts.push(FactSummary {
                id: Uuid::now_v7(),
                source_entity: format!("Entity{}", i),
                target_entity: format!("Target{}", i),
                label: "related_to".to_string(),
                fact: format!("Entity{} has a very long and detailed relationship with Target{} that includes many specific details about their interaction history", i, i),
                classification: Default::default(),
                valid_at: Utc::now(),
                invalid_at: None,
                relevance: 0.5,
                access_count: 0,
                last_accessed_at: None,
                temporal_scope: None,
            });
        }

        block.assemble(50); // Very tight budget

        // Should have truncated significantly
        assert!(block.token_count <= 60); // Allow some overshoot from the last item
    }

    #[test]
    fn test_empty_context_block() {
        let mut block = ContextBlock::empty();
        block.assemble(500);
        assert!(block.context.is_empty());
        assert_eq!(block.token_count, 0);
        assert!(block.temporal_diagnostics.is_none());
    }

    #[test]
    fn test_context_request_defaults() {
        let req: ContextRequest = serde_json::from_str(
            r#"{
            "messages": [{"role": "user", "content": "What shoes does Kendra like?"}]
        }"#,
        )
        .unwrap();

        assert_eq!(req.max_tokens, 500);
        assert_eq!(req.search_types, vec![SearchType::Hybrid]);
        assert_eq!(req.min_relevance, 0.3);
        assert_eq!(req.time_intent, TemporalIntent::Auto);
        assert_eq!(req.as_of, None);
        assert_eq!(req.temporal_weight, None);
    }

    #[test]
    fn test_search_result_serialization() {
        let result = SearchResult {
            result_type: SearchResultType::Edge,
            id: Uuid::now_v7(),
            score: 0.87,
            content: "Kendra loves Nike shoes".to_string(),
            source: RetrievalSource::SemanticSearch,
            metadata: serde_json::json!({}),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("semantic_search"));
        assert!(json.contains("edge"));
    }
}
