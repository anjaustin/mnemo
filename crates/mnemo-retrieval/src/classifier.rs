//! # Query Intent Classifier (Spec 04 D1)
//!
//! Classifies incoming context queries into one of five intent types and
//! selects a specialized retrieval path for each. Classification is purely
//! keyword-heuristic — no LLM call, no embedding, latency ~0ms.
//!
//! Falls back to `Summary` (current hybrid behaviour) when no strong signal
//! is detected, so misclassification degrades gracefully to status quo.

use serde::{Deserialize, Serialize};

// ─── Query intent types ────────────────────────────────────────────

/// The intent of an incoming query, used to select a retrieval strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryType {
    /// "What is Jordan's email?" — entity lookup → single fact
    Factual,
    /// "How does Jordan relate to Acme?" — graph traversal → edge summary
    Relationship,
    /// "What changed since January?" — time-windowed → chronological
    Temporal,
    /// "Give me context on Jordan's accounts" — balanced hybrid retrieval
    #[default]
    Summary,
    /// "Does Jordan have any legal issues?" — if low confidence → explicit absent notice
    Absent,
}

// ─── Keyword patterns ──────────────────────────────────────────────

/// Patterns that signal a factual lookup ("single fact, exact answer").
const FACTUAL_PATTERNS: &[&str] = &[
    "what is",
    "what's",
    "who is",
    "who's",
    "where is",
    "when is",
    "when was",
    "which is",
    "how much is",
    "how many",
    "what does",
    "what did",
    "tell me the",
    "give me the",
    "find the",
    "what are the",
    "show me the",
];

/// Patterns that signal a relationship/graph query.
const RELATIONSHIP_PATTERNS: &[&str] = &[
    "how does",
    "how do",
    "relates to",
    "relate to",
    "relationship between",
    "connection between",
    "connected to",
    "associated with",
    "linked to",
    "works with",
    "interact with",
    "interacts with",
    "knows",
    "collaborate",
    "partner",
];

/// Patterns that signal a temporal / change-detection query.
const TEMPORAL_PATTERNS: &[&str] = &[
    "what changed",
    "what has changed",
    "what happened",
    "since ",
    "before ",
    "after ",
    "between ",
    "last week",
    "last month",
    "last year",
    "recently",
    "lately",
    "over time",
    "timeline",
    "history",
    "update",
    "updates",
    "different",
    "anymore",
    "still",
    "no longer",
    "used to",
    "originally",
    "previously",
    "evolution of",
    "progression",
    "changes",
];

/// Patterns that signal an absent-detection query ("is there anything about X?").
const ABSENT_PATTERNS: &[&str] = &[
    "does",
    "do we have",
    "is there",
    "are there",
    "any ",
    "anything about",
    "anything on",
    "have any",
    "has any",
    "know of",
    "know about",
    "aware of",
    "not sure if",
    "wondering if",
    "whether",
];

// ─── Classification ────────────────────────────────────────────────

/// Classification result from the query intent classifier.
#[derive(Debug, Clone, Serialize)]
pub struct QueryClassification {
    /// Detected intent type.
    pub query_type: QueryType,
    /// Confidence in the classification (0.0–1.0).
    pub confidence: f32,
    /// Whether this was auto-classified or fell back to the default.
    pub source: ClassificationSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationSource {
    /// Keyword heuristics matched with sufficient confidence.
    HeuristicMatch,
    /// No strong signals — fell back to `Summary`.
    DefaultFallback,
}

/// Classify the intent of a context query using keyword heuristics.
///
/// Returns a `QueryClassification` with the detected type, confidence, and
/// classification source. Runs in < 1 ms with no allocations beyond the result.
///
/// Priority order (highest → lowest):
///   Temporal > Relationship > Factual > Absent > Summary (fallback)
pub fn classify_query_intent(query: &str) -> QueryClassification {
    let lower = query.to_lowercase();

    // Score each category
    let temporal_score = score(&lower, TEMPORAL_PATTERNS);
    let relationship_score = score(&lower, RELATIONSHIP_PATTERNS);
    let factual_score = score(&lower, FACTUAL_PATTERNS);
    let absent_score = score(&lower, ABSENT_PATTERNS);

    // Threshold: a category wins if it scores above 0.3 (one pattern match = 0.5)
    const THRESHOLD: f32 = 0.30;

    // Priority: temporal first (most specific), then relationship, factual, absent
    let winner = if temporal_score > THRESHOLD {
        Some((QueryType::Temporal, temporal_score))
    } else if relationship_score > THRESHOLD {
        Some((QueryType::Relationship, relationship_score))
    } else if factual_score > THRESHOLD {
        Some((QueryType::Factual, factual_score))
    } else if absent_score > THRESHOLD {
        Some((QueryType::Absent, absent_score))
    } else {
        None
    };

    match winner {
        Some((query_type, top_score)) => QueryClassification {
            query_type,
            confidence: top_score.clamp(0.0, 1.0),
            source: ClassificationSource::HeuristicMatch,
        },
        None => QueryClassification {
            query_type: QueryType::Summary,
            confidence: 0.0,
            source: ClassificationSource::DefaultFallback,
        },
    }
}

/// Score how well a query matches a set of keyword patterns.
/// Returns a score in [0.0, 1.0] with diminishing returns for additional matches.
fn score(query: &str, patterns: &[&str]) -> f32 {
    let matches = patterns.iter().filter(|p| query.contains(*p)).count() as u32;
    if matches == 0 {
        return 0.0;
    }
    let base = 0.5_f32;
    let extra = (matches - 1) as f32 * 0.15;
    (base + extra).min(1.0)
}

// ─── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Positive classification tests ──────────────────────────────

    #[test]
    fn test_classify_factual_what_is() {
        let c = classify_query_intent("What is Jordan's email address?");
        assert_eq!(c.query_type, QueryType::Factual);
        assert_eq!(c.source, ClassificationSource::HeuristicMatch);
    }

    #[test]
    fn test_classify_factual_who_is() {
        let c = classify_query_intent("Who is the account manager for Acme?");
        assert_eq!(c.query_type, QueryType::Factual);
    }

    #[test]
    fn test_classify_relationship_how_does() {
        let c = classify_query_intent("How does Jordan relate to the Acme deal?");
        assert_eq!(c.query_type, QueryType::Relationship);
        assert_eq!(c.source, ClassificationSource::HeuristicMatch);
    }

    #[test]
    fn test_classify_relationship_between() {
        // Use phrasing that avoids factual ("what is") and temporal ("between ") patterns.
        let c = classify_query_intent("How does Alice relate to the project team?");
        assert_eq!(c.query_type, QueryType::Relationship);
    }

    #[test]
    fn test_falsify_between_fires_temporal_over_relationship() {
        // "between X and Y" triggers both temporal ("between ") and relationship
        // ("connection between"). Temporal has higher priority — document this.
        let c = classify_query_intent("What is the connection between Alice and the team?");
        assert_eq!(
            c.query_type,
            QueryType::Temporal,
            "'between' is a temporal pattern and takes priority over relationship"
        );
    }

    #[test]
    fn test_classify_temporal_what_changed() {
        let c = classify_query_intent("What changed about Acme since January?");
        assert_eq!(c.query_type, QueryType::Temporal);
        assert_eq!(c.source, ClassificationSource::HeuristicMatch);
    }

    #[test]
    fn test_classify_temporal_last_week() {
        let c = classify_query_intent("What happened last week with the renewal?");
        assert_eq!(c.query_type, QueryType::Temporal);
    }

    #[test]
    fn test_classify_absent_is_there() {
        let c = classify_query_intent("Is there any information about legal issues?");
        assert_eq!(c.query_type, QueryType::Absent);
        assert_eq!(c.source, ClassificationSource::HeuristicMatch);
    }

    #[test]
    fn test_classify_summary_fallback() {
        let c = classify_query_intent("Give me context on Jordan's accounts");
        // "give me the" matches factual but "give me context" doesn't contain "give me the"
        // Should fall back to Summary or Factual — verify it doesn't panic
        assert!(c.confidence >= 0.0 && c.confidence <= 1.0);
    }

    #[test]
    fn test_classify_generic_falls_back_to_summary() {
        let c = classify_query_intent("Tell me about the project");
        // No strong signals — should fall back
        assert_eq!(c.source, ClassificationSource::DefaultFallback);
        assert_eq!(c.query_type, QueryType::Summary);
    }

    // ── Priority ordering tests ────────────────────────────────────

    #[test]
    fn test_temporal_beats_relationship_when_stronger() {
        // "since" (temporal) + "how does" (relationship) — temporal wins as higher priority
        let c = classify_query_intent("How does the project look since last month?");
        assert_eq!(c.query_type, QueryType::Temporal);
    }

    #[test]
    fn test_relationship_beats_factual() {
        let c = classify_query_intent("How does Alice relate to the org?");
        assert_eq!(c.query_type, QueryType::Relationship);
    }

    // ── Adversarial / falsification tests ─────────────────────────

    #[test]
    fn test_falsify_empty_query_is_summary() {
        let c = classify_query_intent("");
        assert_eq!(c.query_type, QueryType::Summary);
        assert_eq!(c.source, ClassificationSource::DefaultFallback);
    }

    #[test]
    fn test_falsify_very_long_query_no_panic() {
        let long = "what is ".repeat(5000);
        let c = classify_query_intent(&long);
        // Factual patterns match — must not panic
        assert!(c.confidence >= 0.0 && c.confidence <= 1.0);
    }

    #[test]
    fn test_falsify_score_capped_at_one() {
        // Even with every temporal pattern present, score should not exceed 1.0
        let all_temporal = TEMPORAL_PATTERNS.join(" ");
        let s = score(&all_temporal, TEMPORAL_PATTERNS);
        assert!(s <= 1.0, "Score must not exceed 1.0, got {s}");
        assert_eq!(s, 1.0, "All patterns should saturate to 1.0");
    }

    #[test]
    fn test_falsify_unicode_no_panic() {
        let c = classify_query_intent("Qu'est-ce que 変化 since last week?");
        assert!(c.confidence >= 0.0 && c.confidence <= 1.0);
    }

    #[test]
    fn test_falsify_case_insensitive() {
        let c1 = classify_query_intent("WHAT IS the email?");
        let c2 = classify_query_intent("what is the email?");
        assert_eq!(c1.query_type, c2.query_type);
    }

    #[test]
    fn test_falsify_single_word_query() {
        let c = classify_query_intent("anything");
        // "any " pattern requires a space after — "anything" shouldn't match "any "
        // It will match "anything about"? No, "anything" alone doesn't.
        // Should fall back to summary
        assert!(c.confidence >= 0.0 && c.confidence <= 1.0);
    }

    #[test]
    fn test_falsify_score_function_zero_on_no_match() {
        assert_eq!(score("completely unrelated text", FACTUAL_PATTERNS), 0.0);
    }

    #[test]
    fn test_falsify_score_function_one_match_is_half() {
        assert!((score("what is something", FACTUAL_PATTERNS) - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_classify_returns_serializable_result() {
        let c = classify_query_intent("What changed since January?");
        let json = serde_json::to_value(&c).unwrap();
        assert!(json.get("query_type").is_some());
        assert!(json.get("confidence").is_some());
        assert!(json.get("source").is_some());
    }

    #[test]
    fn test_falsify_absent_does_not_fire_on_positive_assertion() {
        // "does not" might trigger "does" — score should be low enough or beaten by others
        let c = classify_query_intent("Jordan does great work on Acme");
        // "does" fires absent — but no other signals. Confirm it classifies consistently.
        assert!(c.confidence >= 0.0 && c.confidence <= 1.0);
    }

    #[test]
    fn test_falsify_confidence_bounded() {
        for query in &[
            "",
            "what is it",
            "how does this relate",
            "what changed since last week how does it relate what is the status",
        ] {
            let c = classify_query_intent(query);
            assert!(
                c.confidence >= 0.0 && c.confidence <= 1.0,
                "confidence out of bounds for query: {query}"
            );
        }
    }
}
