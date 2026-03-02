use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A context block is the primary output of Mnemo — a pre-assembled,
/// token-efficient representation of relevant knowledge for an AI agent.
///
/// Context blocks combine results from multiple retrieval strategies
/// (semantic search, full-text search, graph traversal) into a single
/// string optimized for LLM consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// Lightweight entity reference included in context responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySummary {
    pub id: Uuid,
    pub name: String,
    pub entity_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Relevance score from retrieval (0.0–1.0).
    pub relevance: f32,
}

/// Lightweight fact reference included in context responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactSummary {
    pub id: Uuid,
    pub source_entity: String,
    pub target_entity: String,
    pub label: String,
    pub fact: String,
    pub valid_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_at: Option<DateTime<Utc>>,
    pub relevance: f32,
}

/// Lightweight episode reference included in context responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Which retrieval strategy contributed results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalSource {
    SemanticSearch,
    FullTextSearch,
    GraphTraversal,
    EpisodeRecall,
    SessionSummary,
}

/// The search types a caller can request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchType {
    Semantic,
    FullText,
    Graph,
    Hybrid,
}

/// Request for context retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Minimum relevance threshold (0.0–1.0). Results below this are dropped.
    #[serde(default = "default_min_relevance")]
    pub min_relevance: f32,
}

/// A message provided as query context for retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Generic search request for the search API.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
                    let line = format!("- [{}] {}: {}\n", ep.created_at.format("%Y-%m-%d"), role, ep.preview);
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
                } else {
                    running_tokens -= header_tokens;
                }
            }
        }

        self.context = parts.join("\n");
        self.token_count = estimate_tokens(&self.context);
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
            summary: Some("A customer who likes athletic shoes".to_string()),
            relevance: 0.95,
        });
        block.facts.push(FactSummary {
            id: Uuid::now_v7(),
            source_entity: "Kendra".to_string(),
            target_entity: "Nike shoes".to_string(),
            label: "prefers".to_string(),
            fact: "Kendra now prefers Nike shoes after switching from Adidas".to_string(),
            valid_at: Utc::now(),
            invalid_at: None,
            relevance: 0.92,
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
                valid_at: Utc::now(),
                invalid_at: None,
                relevance: 0.5,
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
    }

    #[test]
    fn test_context_request_defaults() {
        let req: ContextRequest = serde_json::from_str(r#"{
            "messages": [{"role": "user", "content": "What shoes does Kendra like?"}]
        }"#).unwrap();

        assert_eq!(req.max_tokens, 500);
        assert_eq!(req.search_types, vec![SearchType::Hybrid]);
        assert_eq!(req.min_relevance, 0.3);
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
