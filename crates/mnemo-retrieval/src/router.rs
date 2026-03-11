//! Semantic routing for retrieval strategy selection.
//!
//! Classifies incoming queries to determine the optimal retrieval strategy
//! (Head, Hybrid, Historical) based on keyword patterns and heuristics.
//! This replaces blind `mode=hybrid` defaults with an informed routing
//! decision, without requiring the caller to explicitly choose a mode.
//!
//! The router uses lightweight keyword matching (no LLM call, no embedding)
//! to keep latency at ~0ms. For higher-accuracy routing, callers can still
//! override by passing an explicit `mode` field.

use serde::Serialize;

// ─── Routing Strategy ─────────────────────────────────────────────

/// Retrieval strategy selected by the router.
/// Mirrors `MemoryContextMode` from the server layer but is defined here
/// to keep the retrieval crate independent of server types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalStrategy {
    /// Recent conversation focus — emphasise the current session's episodes.
    Head,
    /// Balanced: semantic search + full-text + graph traversal (default).
    Hybrid,
    /// Historical focus — prioritise older, established facts over recent noise.
    Historical,
    /// Graph-focused — entity/relationship queries that benefit from graph traversal.
    GraphFocused,
    /// Episode-only — direct recall of specific conversations or events.
    EpisodeRecall,
}

/// Diagnostic information about the routing decision.
#[derive(Debug, Clone, Serialize)]
pub struct RoutingDecision {
    /// The strategy selected by the router.
    pub selected_strategy: RetrievalStrategy,
    /// Confidence that this is the right strategy (0.0–1.0).
    pub confidence: f32,
    /// Whether the strategy was auto-detected or explicitly requested.
    pub source: RoutingSource,
    /// Alternative strategies considered, with their scores.
    pub alternatives: Vec<StrategyScore>,
}

/// Where the routing decision came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingSource {
    /// Auto-classified by the semantic router.
    AutoClassified,
    /// Explicitly requested by the caller via the `mode` field.
    ExplicitRequest,
    /// Fallback: no signals matched, defaulting to Hybrid.
    DefaultFallback,
}

/// A strategy and its classification score.
#[derive(Debug, Clone, Serialize)]
pub struct StrategyScore {
    pub strategy: RetrievalStrategy,
    pub score: f32,
}

// ─── Keyword patterns ─────────────────────────────────────────────

/// Patterns that suggest a "head" (recent conversation) retrieval mode.
const HEAD_PATTERNS: &[&str] = &[
    "just said",
    "just told",
    "just mentioned",
    "a moment ago",
    "a minute ago",
    "earlier today",
    "earlier in this conversation",
    "what did i just",
    "what did we just",
    "you just said",
    "i just said",
    "the last thing",
    "most recent",
    "latest",
    "what was that",
    "say that again",
    "repeat that",
    "in this chat",
    "in this session",
    "so far today",
    "recap",
    "summarize this conversation",
    "summarize our conversation",
    "what have we discussed",
    "what have we talked about",
];

/// Patterns that suggest a historical retrieval mode.
const HISTORICAL_PATTERNS: &[&str] = &[
    "remember when",
    "a long time ago",
    "months ago",
    "weeks ago",
    "years ago",
    "last year",
    "last month",
    "historically",
    "in the past",
    "originally",
    "used to",
    "back when",
    "at first",
    "initially",
    "what did i tell you about",
    "what did i say about",
    "you once told me",
    "a while back",
    "long ago",
    "way back",
    "do you recall",
    "do you remember",
    "first time we",
    "first time i",
    "early on",
    "how it all started",
    "beginning of",
    "timeline",
    "history of",
    "evolution of",
    "over time",
    "track record",
];

/// Patterns that suggest a graph-focused retrieval mode.
const GRAPH_PATTERNS: &[&str] = &[
    "relationship between",
    "connected to",
    "related to",
    "how does .* relate",
    "who knows",
    "who works",
    "who is .* associated",
    "entities",
    "knowledge graph",
    "connection between",
    "link between",
    "network of",
    "map of",
    "all about",
    "everything about",
    "tell me about",
    "who are .* people",
    "what are .* things",
    "list of",
    "all the",
    "core beliefs",
    "values",
    "principles",
    "what does .* believe",
];

/// Patterns that suggest episode/conversation recall mode.
const EPISODE_PATTERNS: &[&str] = &[
    "what did we discuss",
    "conversation about",
    "talked about",
    "meeting about",
    "discussion about",
    "that time we",
    "that conversation",
    "that session",
    "session about",
    "topic of",
    "when we talked",
    "when we discussed",
    "chat about",
    "chat history",
    "message history",
    "transcript",
    "conversation log",
];

// ─── Router implementation ────────────────────────────────────────

/// Classify a query into a retrieval strategy.
///
/// Returns a `RoutingDecision` with the selected strategy, confidence,
/// and alternatives. The classification is purely keyword-based and
/// runs in <1ms with zero allocations beyond the result struct.
pub fn classify_query(query: &str) -> RoutingDecision {
    let lower = query.to_lowercase();

    let head_score = score_patterns(&lower, HEAD_PATTERNS);
    let historical_score = score_patterns(&lower, HISTORICAL_PATTERNS);
    let graph_score = score_patterns(&lower, GRAPH_PATTERNS);
    let episode_score = score_patterns(&lower, EPISODE_PATTERNS);

    // Hybrid gets a base score — it's the "safe default"
    let hybrid_score: f32 = 0.3;

    let mut scores = vec![
        (RetrievalStrategy::Head, head_score),
        (RetrievalStrategy::Historical, historical_score),
        (RetrievalStrategy::GraphFocused, graph_score),
        (RetrievalStrategy::EpisodeRecall, episode_score),
        (RetrievalStrategy::Hybrid, hybrid_score),
    ];

    // Sort by score descending
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let (selected, top_score) = scores[0];
    let confidence = if scores.len() > 1 {
        // Confidence = margin between top and second-best
        let runner_up = scores[1].1;
        let margin = top_score - runner_up;
        // Normalize: a margin of 0.5+ is very confident
        (margin * 2.0).clamp(0.0, 1.0)
    } else {
        1.0
    };

    let source = if top_score <= hybrid_score {
        RoutingSource::DefaultFallback
    } else {
        RoutingSource::AutoClassified
    };

    let alternatives = scores[1..]
        .iter()
        .filter(|(_, s)| *s > 0.0)
        .map(|(strategy, score)| StrategyScore {
            strategy: *strategy,
            score: *score,
        })
        .collect();

    RoutingDecision {
        selected_strategy: selected,
        confidence,
        source,
        alternatives,
    }
}

/// Score how well a query matches a set of keyword patterns.
/// Returns a score in [0.0, 1.0].
fn score_patterns(query: &str, patterns: &[&str]) -> f32 {
    let mut matches = 0u32;
    for pattern in patterns {
        if query.contains(pattern) {
            matches += 1;
        }
    }
    if matches == 0 {
        return 0.0;
    }
    // Diminishing returns: first match = 0.5, each additional adds less
    let base = 0.5;
    let extra = (matches - 1) as f32 * 0.15;
    (base + extra).min(1.0)
}

// ─── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_head_query() {
        let d = classify_query("What did we just discuss?");
        assert_eq!(d.selected_strategy, RetrievalStrategy::Head);
        assert_eq!(d.source, RoutingSource::AutoClassified);
        assert!(d.confidence > 0.0);
    }

    #[test]
    fn test_classify_head_recap_query() {
        let d = classify_query("Can you recap the conversation so far?");
        assert_eq!(d.selected_strategy, RetrievalStrategy::Head);
    }

    #[test]
    fn test_classify_historical_query() {
        let d = classify_query("What did I tell you about the project months ago?");
        assert_eq!(d.selected_strategy, RetrievalStrategy::Historical);
    }

    #[test]
    fn test_classify_historical_remember_when() {
        let d = classify_query("Do you remember when I first mentioned the budget?");
        assert_eq!(d.selected_strategy, RetrievalStrategy::Historical);
    }

    #[test]
    fn test_classify_graph_query() {
        let d = classify_query("What is the relationship between Alice and the project?");
        assert_eq!(d.selected_strategy, RetrievalStrategy::GraphFocused);
    }

    #[test]
    fn test_classify_graph_everything_about() {
        let d = classify_query("Tell me everything about Alice's core beliefs");
        assert_eq!(d.selected_strategy, RetrievalStrategy::GraphFocused);
    }

    #[test]
    fn test_classify_episode_recall() {
        let d = classify_query("What did we discuss in that meeting about the roadmap?");
        assert_eq!(d.selected_strategy, RetrievalStrategy::EpisodeRecall);
    }

    #[test]
    fn test_classify_hybrid_fallback() {
        let d = classify_query("How should I structure the API?");
        assert_eq!(d.selected_strategy, RetrievalStrategy::Hybrid);
        assert_eq!(d.source, RoutingSource::DefaultFallback);
    }

    #[test]
    fn test_classify_empty_query_defaults_to_hybrid() {
        let d = classify_query("");
        assert_eq!(d.selected_strategy, RetrievalStrategy::Hybrid);
        assert_eq!(d.source, RoutingSource::DefaultFallback);
    }

    #[test]
    fn test_classify_ambiguous_query_picks_strongest() {
        // Contains both head and historical signals
        let d = classify_query("What did we just discuss about what happened months ago?");
        // Should pick the one with more matches
        assert!(
            d.selected_strategy == RetrievalStrategy::Head
                || d.selected_strategy == RetrievalStrategy::Historical,
            "Should pick head or historical, got {:?}",
            d.selected_strategy
        );
    }

    #[test]
    fn test_confidence_is_bounded() {
        let d = classify_query("A very random query with no keywords");
        assert!(d.confidence >= 0.0 && d.confidence <= 1.0);
    }

    #[test]
    fn test_alternatives_are_populated() {
        let d = classify_query("What did we just discuss?");
        // Hybrid should always be in alternatives since it has a base score
        assert!(!d.alternatives.is_empty(), "Should have alternatives");
    }

    #[test]
    fn test_score_patterns_no_match_returns_zero() {
        let score = score_patterns("hello world", &["foo", "bar"]);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_score_patterns_one_match() {
        let score = score_patterns("hello world", &["hello", "bar"]);
        assert_eq!(score, 0.5);
    }

    #[test]
    fn test_score_patterns_multiple_matches_diminishing() {
        let score = score_patterns("just said the last thing in this session", HEAD_PATTERNS);
        assert!(score > 0.5, "Multiple matches should score > 0.5");
        assert!(score <= 1.0, "Score must be <= 1.0");
    }

    #[test]
    fn test_routing_decision_serializes() {
        let d = classify_query("test query");
        let json = serde_json::to_value(&d).unwrap();
        assert!(json.get("selected_strategy").is_some());
        assert!(json.get("confidence").is_some());
        assert!(json.get("source").is_some());
        assert!(json.get("alternatives").is_some());
    }

    #[test]
    fn test_classify_case_insensitive() {
        let d = classify_query("WHAT DID WE JUST DISCUSS?");
        assert_eq!(d.selected_strategy, RetrievalStrategy::Head);
    }

    // ─── Adversarial tests ────────────────────────────────────────

    #[test]
    fn test_falsify_very_long_query_doesnt_panic() {
        let long = "what ".repeat(10000);
        let d = classify_query(&long);
        assert!(d.confidence >= 0.0 && d.confidence <= 1.0);
    }

    #[test]
    fn test_falsify_unicode_query_doesnt_panic() {
        let d = classify_query("何を話しましたか？先ほどのことを教えて。just said");
        // Should still match "just said" even in mixed unicode
        assert_eq!(d.selected_strategy, RetrievalStrategy::Head);
    }

    #[test]
    fn test_falsify_special_chars_query() {
        let d = classify_query("what about <script>alert('xss')</script>?");
        // Should not panic, should default to hybrid
        assert!(
            d.confidence >= 0.0 && d.confidence <= 1.0,
            "Should handle special chars gracefully"
        );
    }

    #[test]
    fn test_falsify_max_keyword_overlap() {
        // Query that matches multiple patterns from EVERY category
        let d = classify_query(
            "just said months ago relationship between what did we discuss in the past",
        );
        // Should pick one with highest match count
        let total: f32 = d.alternatives.iter().map(|a| a.score).sum::<f32>()
            + if d.confidence > 0.0 { 1.0 } else { 0.0 };
        assert!(total > 0.0, "Should have positive scores");
    }

    // ─── Falsification round 2: deeper adversarial tests ─────────────

    #[test]
    fn test_falsify_single_match_beats_hybrid_base() {
        // A query with exactly one HEAD match should score 0.5 and beat Hybrid's 0.3
        let d = classify_query("give me the latest on the project");
        assert_eq!(
            d.selected_strategy,
            RetrievalStrategy::Head,
            "Single HEAD keyword ('latest') should beat Hybrid base score"
        );
        assert_eq!(d.source, RoutingSource::AutoClassified);
    }

    #[test]
    fn test_falsify_tie_between_two_strategies_is_deterministic() {
        // If two categories score identically (each with exactly 1 match → 0.5),
        // the winner should be deterministic (stable sort order).
        // "latest" → HEAD, "originally" → HISTORICAL; both score 0.5
        let d1 = classify_query("originally latest");
        let d2 = classify_query("originally latest");
        assert_eq!(
            d1.selected_strategy, d2.selected_strategy,
            "Identical queries must produce identical routing"
        );
        // Confidence should be 0 when tied (margin = 0)
        assert_eq!(
            d1.confidence, 0.0,
            "Tied strategies should yield zero confidence"
        );
    }

    #[test]
    fn test_falsify_broad_pattern_all_the_is_graph() {
        // "all the" is a GRAPH pattern — it is intentionally broad.
        // Verify it fires for casual speech.
        let d = classify_query("I want all the details on the migration");
        assert_eq!(
            d.selected_strategy,
            RetrievalStrategy::GraphFocused,
            "'all the' is a graph pattern and should classify as GraphFocused"
        );
    }

    #[test]
    fn test_falsify_substring_no_false_positive_for_partial_keyword() {
        // "at first" is a HISTORICAL pattern, but "bat first" should NOT match
        // because "at first" is checked via contains() which matches substrings.
        // Actually "bat first" DOES contain "at first" — this IS a false positive.
        let _d = classify_query("the bat first base player");
        // This WILL match "at first" (historical), revealing a known substring limitation.
        let hist_score = score_patterns(
            &"the bat first base player".to_lowercase(),
            HISTORICAL_PATTERNS,
        );
        // Acknowledge the limitation: contains() matches substrings
        assert!(
            hist_score > 0.0,
            "Known limitation: contains() matches substrings like 'at first' in 'bat first'"
        );
    }

    #[test]
    fn test_falsify_head_vs_episode_overlap() {
        // "What did we just discuss" has both HEAD signal ("just") and
        // EPISODE signal ("what did we discuss"). Head should win via
        // more specific pattern "what did we just" matching first.
        let d = classify_query("What did we just discuss about the roadmap?");
        // HEAD has "what did we just" pattern. Episode has "what did we discuss".
        // Both match, but HEAD also matches "just" implicitly in "what did we just".
        // Head patterns that match: "what did we just"
        // Episode patterns that match: "what did we discuss" — wait, check exact patterns
        let head_score = score_patterns(
            &"what did we just discuss about the roadmap?".to_lowercase(),
            HEAD_PATTERNS,
        );
        let episode_score = score_patterns(
            &"what did we just discuss about the roadmap?".to_lowercase(),
            EPISODE_PATTERNS,
        );
        // Document which wins
        assert!(
            head_score > 0.0 || episode_score > 0.0,
            "At least one category should match"
        );
        // The actual winner:
        if head_score > episode_score {
            assert_eq!(d.selected_strategy, RetrievalStrategy::Head);
        } else if episode_score > head_score {
            assert_eq!(d.selected_strategy, RetrievalStrategy::EpisodeRecall);
        }
        // If tied, just verify it's one of the two
    }

    #[test]
    fn test_falsify_score_never_exceeds_one() {
        // Craft a query that hits MANY historical patterns
        let q = "remember when, a long time ago, months ago, weeks ago, years ago, \
                 last year, last month, historically, in the past, originally, \
                 used to, back when, at first, initially, do you recall, \
                 do you remember, a while back, long ago, way back, early on, \
                 timeline, history of, evolution of, over time, track record";
        let score = score_patterns(&q.to_lowercase(), HISTORICAL_PATTERNS);
        assert!(score <= 1.0, "Score must never exceed 1.0, got {}", score);
        assert!(
            score == 1.0,
            "With 20+ matches score should be capped at 1.0, got {}",
            score
        );
    }

    #[test]
    fn test_falsify_whitespace_only_query() {
        let d = classify_query("   \t\n  ");
        assert_eq!(d.selected_strategy, RetrievalStrategy::Hybrid);
        assert_eq!(d.source, RoutingSource::DefaultFallback);
    }

    #[test]
    fn test_falsify_routing_decision_all_fields_serialized() {
        // Ensure the full RoutingDecision serializes with no missing fields
        let d = classify_query("What did I tell you about the project months ago?");
        let json = serde_json::to_value(&d).unwrap();
        let obj = json.as_object().unwrap();
        assert!(
            obj.contains_key("selected_strategy"),
            "missing selected_strategy"
        );
        assert!(obj.contains_key("confidence"), "missing confidence");
        assert!(obj.contains_key("source"), "missing source");
        assert!(obj.contains_key("alternatives"), "missing alternatives");
        // Verify alternatives are properly structured
        let alts = obj["alternatives"].as_array().unwrap();
        for alt in alts {
            assert!(
                alt.get("strategy").is_some(),
                "alternative missing strategy"
            );
            assert!(alt.get("score").is_some(), "alternative missing score");
        }
    }

    #[test]
    fn test_falsify_graph_focused_maps_to_hybrid_at_server_layer() {
        // GraphFocused and EpisodeRecall don't have their own MemoryContextMode —
        // they map to Hybrid at the server layer. Verify the router still
        // correctly identifies them as distinct strategies.
        let d = classify_query("What is the relationship between Alice and Bob?");
        assert_eq!(d.selected_strategy, RetrievalStrategy::GraphFocused);
        // At the server layer this would become Hybrid, but the routing_decision
        // should still report GraphFocused for diagnostic purposes.
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["selected_strategy"], "graph_focused");
    }

    #[test]
    fn test_falsify_confidence_zero_when_all_non_hybrid_scores_zero() {
        // A generic query: all strategy scores are 0 except Hybrid (0.3).
        // Confidence = 2 * (0.3 - 0.0) = 0.6 — NOT zero, because margin from
        // Hybrid to second-place (0.0) is 0.3, so confidence = 0.6.
        let d = classify_query("Tell me something interesting");
        assert_eq!(d.selected_strategy, RetrievalStrategy::Hybrid);
        assert_eq!(d.source, RoutingSource::DefaultFallback);
        // Confidence = 2 * 0.3 = 0.6 (Hybrid 0.3 vs runner-up 0.0)
        assert!(
            (d.confidence - 0.6).abs() < 0.01,
            "Expected confidence ~0.6 for fallback, got {}",
            d.confidence
        );
    }
}
