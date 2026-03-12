use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::context::FactSummary;

/// A hypothetical fact override for counterfactual simulation.
///
/// Represents a "what if" condition: "what if this entity had this attribute
/// set to this value?" The counterfactual engine injects these into the
/// retrieval pipeline, replacing any matching real facts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HypotheticalFact {
    /// The entity this override applies to (by name, matched case-insensitively).
    pub entity: String,

    /// The attribute/relationship label being overridden (e.g., "brand_preference").
    pub attribute: String,

    /// The hypothetical value (e.g., "Adidas").
    pub value: String,

    /// Confidence of the hypothetical fact. Default: 0.9.
    #[serde(default = "default_confidence")]
    pub confidence: f32,
}

fn default_confidence() -> f32 {
    0.9
}

/// A record of a real fact that was overridden by a hypothetical.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverriddenFact {
    /// The real fact that was replaced.
    pub original: FactSummary,

    /// Which hypothetical replaced it (index into the `hypotheticals` array).
    pub replaced_by_index: usize,

    /// The hypothetical that replaced it.
    pub hypothetical: HypotheticalFact,
}

/// The diff between real and counterfactual context — shows what changed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterfactualDiff {
    /// Real facts that were overridden by hypotheticals.
    pub overridden_facts: Vec<OverriddenFact>,

    /// Hypotheticals that were injected (some may not have matched any real facts).
    pub injected_count: usize,

    /// Hypotheticals that did not match any existing entity/attribute and were
    /// added as entirely new facts.
    pub novel_hypotheticals: Vec<HypotheticalFact>,
}

/// Request body for `POST /api/v1/memory/:user/counterfactual`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterfactualRequest {
    /// The query to retrieve context for (same as normal context request).
    pub query: String,

    /// Optional session scope.
    #[serde(default)]
    pub session: Option<String>,

    /// Maximum tokens for the context response.
    #[serde(default)]
    pub max_tokens: Option<u32>,

    /// Minimum relevance threshold.
    #[serde(default)]
    pub min_relevance: Option<f32>,

    /// The hypothetical fact overrides.
    pub hypotheticals: Vec<HypotheticalFact>,
}

/// Response from the counterfactual endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterfactualResponse {
    /// The assembled context string with hypotheticals applied.
    pub context: String,

    /// Token count of the context.
    pub token_count: u32,

    /// Facts in the counterfactual context (with hypotheticals replacing real facts).
    pub facts: Vec<FactSummary>,

    /// The diff showing what changed.
    pub counterfactual_diff: CounterfactualDiff,

    /// How long the retrieval + simulation took (ms).
    pub latency_ms: u64,
}

/// Apply hypothetical fact overrides to a set of retrieved facts.
///
/// This is the core counterfactual engine logic. It:
/// 1. Scans the real facts for matches against each hypothetical (by entity name + label)
/// 2. Replaces matched facts with synthetic facts built from hypotheticals
/// 3. Tracks the diff (which facts were overridden, which hypotheticals are novel)
///
/// Returns the modified fact list and the diff.
pub fn apply_hypotheticals(
    mut facts: Vec<FactSummary>,
    hypotheticals: &[HypotheticalFact],
) -> (Vec<FactSummary>, CounterfactualDiff) {
    let mut overridden_facts = Vec::new();
    let mut novel_hypotheticals = Vec::new();
    let mut injected_count = 0;

    for (idx, hyp) in hypotheticals.iter().enumerate() {
        let hyp_entity_lower = hyp.entity.to_lowercase();
        let hyp_attr_lower = hyp.attribute.to_lowercase();

        // Find matching real facts (same entity + label, case-insensitive)
        let mut matched = false;
        for fact in &mut facts {
            let source_lower = fact.source_entity.to_lowercase();
            let label_lower = fact.label.to_lowercase();

            if source_lower == hyp_entity_lower && label_lower == hyp_attr_lower {
                // Record the override
                overridden_facts.push(OverriddenFact {
                    original: fact.clone(),
                    replaced_by_index: idx,
                    hypothetical: hyp.clone(),
                });

                // Replace the fact with the hypothetical
                fact.fact = format!("{} {} {}", hyp.entity, hyp.attribute, hyp.value);
                fact.relevance = hyp.confidence;
                // Mark as hypothetical by setting invalid_at to a sentinel
                fact.invalid_at = None;
                matched = true;
                injected_count += 1;
            }
        }

        if !matched {
            // This hypothetical doesn't match any existing fact — add as novel
            novel_hypotheticals.push(hyp.clone());

            // Inject as a new synthetic fact
            let synthetic = FactSummary {
                id: Uuid::from_u128(0x_CF_00_00_00 + idx as u128), // deterministic counterfactual ID
                source_entity: hyp.entity.clone(),
                target_entity: hyp.value.clone(),
                label: hyp.attribute.clone(),
                fact: format!("{} {} {}", hyp.entity, hyp.attribute, hyp.value),
                valid_at: Utc::now(),
                invalid_at: None,
                relevance: hyp.confidence,
            };
            facts.push(synthetic);
            injected_count += 1;
        }
    }

    // Re-sort by relevance descending
    facts.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let diff = CounterfactualDiff {
        overridden_facts,
        injected_count,
        novel_hypotheticals,
    };

    (facts, diff)
}

/// Rebuild the context string from modified facts (simple concatenation).
pub fn rebuild_context_string(facts: &[FactSummary]) -> String {
    let mut parts = Vec::with_capacity(facts.len());
    for fact in facts {
        parts.push(format!("- {}", fact.fact));
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fact(entity: &str, label: &str, fact_text: &str, relevance: f32) -> FactSummary {
        FactSummary {
            id: Uuid::from_u128(1),
            source_entity: entity.into(),
            target_entity: "target".into(),
            label: label.into(),
            fact: fact_text.into(),
            valid_at: Utc::now(),
            invalid_at: None,
            relevance,
        }
    }

    #[test]
    fn test_hypothetical_fact_serde() {
        let hyp = HypotheticalFact {
            entity: "user".into(),
            attribute: "brand_preference".into(),
            value: "Adidas".into(),
            confidence: 0.9,
        };
        let json = serde_json::to_string(&hyp).unwrap();
        let de: HypotheticalFact = serde_json::from_str(&json).unwrap();
        assert_eq!(de, hyp);
    }

    #[test]
    fn test_hypothetical_fact_default_confidence() {
        let json = r#"{"entity": "user", "attribute": "brand", "value": "Nike"}"#;
        let hyp: HypotheticalFact = serde_json::from_str(json).unwrap();
        assert_eq!(hyp.confidence, 0.9);
    }

    #[test]
    fn test_counterfactual_request_serde() {
        let json = r#"{
            "query": "What brand does the user prefer?",
            "hypotheticals": [
                {"entity": "user", "attribute": "brand_preference", "value": "Adidas", "confidence": 0.95}
            ]
        }"#;
        let req: CounterfactualRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.query, "What brand does the user prefer?");
        assert_eq!(req.hypotheticals.len(), 1);
        assert_eq!(req.hypotheticals[0].value, "Adidas");
    }

    #[test]
    fn test_apply_hypotheticals_replaces_matching_fact() {
        let facts = vec![
            make_fact("User", "brand_preference", "User prefers Nike", 0.8),
            make_fact("User", "color_preference", "User likes blue", 0.7),
        ];
        let hypotheticals = vec![HypotheticalFact {
            entity: "User".into(),
            attribute: "brand_preference".into(),
            value: "Adidas".into(),
            confidence: 0.95,
        }];

        let (result, diff) = apply_hypotheticals(facts, &hypotheticals);

        assert_eq!(result.len(), 2);
        assert_eq!(diff.overridden_facts.len(), 1);
        assert_eq!(diff.overridden_facts[0].original.fact, "User prefers Nike");
        assert_eq!(diff.novel_hypotheticals.len(), 0);
        assert_eq!(diff.injected_count, 1);

        // Check the replaced fact has the new value
        let brand_fact = result
            .iter()
            .find(|f| f.label == "brand_preference")
            .unwrap();
        assert!(brand_fact.fact.contains("Adidas"));
        assert_eq!(brand_fact.relevance, 0.95);
    }

    #[test]
    fn test_apply_hypotheticals_adds_novel_fact() {
        let facts = vec![make_fact("User", "color", "User likes blue", 0.7)];
        let hypotheticals = vec![HypotheticalFact {
            entity: "User".into(),
            attribute: "size_preference".into(),
            value: "Large".into(),
            confidence: 0.85,
        }];

        let (result, diff) = apply_hypotheticals(facts, &hypotheticals);

        assert_eq!(result.len(), 2); // original + novel
        assert_eq!(diff.overridden_facts.len(), 0);
        assert_eq!(diff.novel_hypotheticals.len(), 1);
        assert_eq!(diff.injected_count, 1);
    }

    #[test]
    fn test_apply_hypotheticals_case_insensitive_matching() {
        let facts = vec![make_fact(
            "USER",
            "Brand_Preference",
            "USER prefers Nike",
            0.8,
        )];
        let hypotheticals = vec![HypotheticalFact {
            entity: "user".into(),
            attribute: "brand_preference".into(),
            value: "Adidas".into(),
            confidence: 0.9,
        }];

        let (_, diff) = apply_hypotheticals(facts, &hypotheticals);
        assert_eq!(diff.overridden_facts.len(), 1);
        assert_eq!(diff.novel_hypotheticals.len(), 0);
    }

    #[test]
    fn test_apply_hypotheticals_empty() {
        let facts = vec![make_fact("User", "color", "likes blue", 0.7)];
        let (result, diff) = apply_hypotheticals(facts, &[]);

        assert_eq!(result.len(), 1);
        assert_eq!(diff.overridden_facts.len(), 0);
        assert_eq!(diff.novel_hypotheticals.len(), 0);
        assert_eq!(diff.injected_count, 0);
    }

    #[test]
    fn test_apply_hypotheticals_multiple() {
        let facts = vec![
            make_fact("User", "brand", "likes Nike", 0.8),
            make_fact("User", "color", "likes blue", 0.7),
        ];
        let hypotheticals = vec![
            HypotheticalFact {
                entity: "User".into(),
                attribute: "brand".into(),
                value: "Adidas".into(),
                confidence: 0.9,
            },
            HypotheticalFact {
                entity: "User".into(),
                attribute: "color".into(),
                value: "red".into(),
                confidence: 0.85,
            },
        ];

        let (result, diff) = apply_hypotheticals(facts, &hypotheticals);
        assert_eq!(result.len(), 2);
        assert_eq!(diff.overridden_facts.len(), 2);
        assert_eq!(diff.injected_count, 2);
    }

    #[test]
    fn test_apply_hypotheticals_results_sorted_by_relevance() {
        let facts = vec![
            make_fact("User", "a", "fact a", 0.3),
            make_fact("User", "b", "fact b", 0.9),
        ];
        let hypotheticals = vec![HypotheticalFact {
            entity: "User".into(),
            attribute: "a".into(),
            value: "overridden".into(),
            confidence: 0.95,
        }];

        let (result, _) = apply_hypotheticals(facts, &hypotheticals);
        // Should be sorted: 0.95 (overridden a), 0.9 (b)
        assert!(result[0].relevance >= result[1].relevance);
    }

    #[test]
    fn test_rebuild_context_string() {
        let facts = vec![
            make_fact("User", "brand", "User prefers Nike", 0.8),
            make_fact("User", "color", "User likes blue", 0.7),
        ];
        let ctx = rebuild_context_string(&facts);
        assert!(ctx.contains("- User prefers Nike"));
        assert!(ctx.contains("- User likes blue"));
        assert!(ctx.contains('\n'));
    }

    #[test]
    fn test_rebuild_context_string_empty() {
        let ctx = rebuild_context_string(&[]);
        assert!(ctx.is_empty());
    }

    #[test]
    fn test_counterfactual_diff_serde() {
        let diff = CounterfactualDiff {
            overridden_facts: vec![],
            injected_count: 2,
            novel_hypotheticals: vec![HypotheticalFact {
                entity: "x".into(),
                attribute: "y".into(),
                value: "z".into(),
                confidence: 0.5,
            }],
        };
        let json = serde_json::to_string(&diff).unwrap();
        let de: CounterfactualDiff = serde_json::from_str(&json).unwrap();
        assert_eq!(de.injected_count, 2);
        assert_eq!(de.novel_hypotheticals.len(), 1);
    }
}
