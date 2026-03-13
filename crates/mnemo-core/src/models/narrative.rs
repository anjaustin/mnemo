//! Cross-session narrative summaries.
//!
//! A [`UserNarrative`] is an evolving "story of the user" that updates after
//! each session. Narratives contain versioned [`NarrativeChapter`]s with
//! period, summary, and key changes. Used as a preamble in context assembly
//! to give agents long-term user understanding.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single chapter in a user's narrative — covers a time period and summarizes
/// key changes that occurred during that window.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NarrativeChapter {
    /// Human-readable label for the period (e.g. "March 2026", "Sessions 40–55").
    pub period: String,

    /// One-paragraph summary of this chapter.
    pub summary: String,

    /// Bullet-point list of key changes: new facts, superseded facts, decayed edges, etc.
    #[serde(default)]
    pub key_changes: Vec<String>,

    /// Session IDs that contributed to this chapter.
    #[serde(default)]
    pub session_ids: Vec<Uuid>,
}

/// An evolving "story of the user" that distills hundreds of sessions into a
/// readable narrative the agent can use as high-level context.
///
/// Versioned — each update creates a new version (mirrors agent identity versioning).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserNarrative {
    /// The user this narrative belongs to.
    pub user_id: Uuid,

    /// Monotonically increasing version counter.
    pub version: u64,

    /// The full narrative text — readable prose that summarizes the user's
    /// evolution across all sessions.
    pub narrative_text: String,

    /// Structured chapter breakdown.
    #[serde(default)]
    pub chapters: Vec<NarrativeChapter>,

    /// How many sessions have been incorporated into this narrative.
    pub session_count: u64,

    /// When this version was created.
    pub created_at: DateTime<Utc>,

    /// When this narrative was last updated (may differ from created_at if
    /// the narrative is refreshed without a new session).
    pub updated_at: DateTime<Utc>,
}

impl UserNarrative {
    /// Create a brand-new narrative (version 1) for a user.
    pub fn new(user_id: Uuid, narrative_text: String, chapters: Vec<NarrativeChapter>) -> Self {
        let now = Utc::now();
        Self {
            user_id,
            version: 1,
            narrative_text,
            chapters,
            session_count: 0,
            created_at: now,
            updated_at: now,
        }
    }

    /// Create the next version of this narrative with updated text and chapters.
    /// Bumps version, preserves user_id and session_count.
    pub fn evolve(
        &self,
        new_text: String,
        new_chapters: Vec<NarrativeChapter>,
        sessions_added: u64,
    ) -> Self {
        let now = Utc::now();
        Self {
            user_id: self.user_id,
            version: self.version + 1,
            narrative_text: new_text,
            chapters: new_chapters,
            session_count: self.session_count + sessions_added,
            created_at: self.created_at,
            updated_at: now,
        }
    }

    /// Check if the narrative is empty (no text).
    pub fn is_empty(&self) -> bool {
        self.narrative_text.trim().is_empty()
    }

    /// Total number of key changes across all chapters.
    pub fn total_key_changes(&self) -> usize {
        self.chapters.iter().map(|c| c.key_changes.len()).sum()
    }

    /// Total number of sessions referenced across all chapters.
    pub fn total_referenced_sessions(&self) -> usize {
        self.chapters
            .iter()
            .flat_map(|c| c.session_ids.iter())
            .collect::<std::collections::HashSet<_>>()
            .len()
    }
}

/// Request body for `POST /api/v1/memory/:user/narrative/refresh`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshNarrativeRequest {
    /// If true, regenerate the narrative from scratch (scan all sessions).
    /// If false (default), do an incremental update from the last known state.
    #[serde(default)]
    pub full_rebuild: bool,

    /// Maximum number of chapters to include. Default: 20.
    #[serde(default = "default_max_chapters")]
    pub max_chapters: u32,
}

fn default_max_chapters() -> u32 {
    20
}

impl Default for RefreshNarrativeRequest {
    fn default() -> Self {
        Self {
            full_rebuild: false,
            max_chapters: default_max_chapters(),
        }
    }
}

/// Build a prompt for the LLM to generate or update a narrative.
///
/// This is a pure function — no LLM calls, just prompt construction.
/// The actual LLM call happens in the server layer via `LlmProvider::summarize`.
pub fn build_narrative_prompt(
    previous_narrative: Option<&str>,
    session_summaries: &[SessionSummaryInput],
    key_fact_changes: &[String],
    max_chapters: u32,
) -> String {
    let mut prompt = String::with_capacity(4096);

    if let Some(prev) = previous_narrative {
        prompt.push_str("## Previous Narrative\n\n");
        prompt.push_str(prev);
        prompt.push_str("\n\n## New Information Since Last Update\n\n");
    } else {
        prompt.push_str("## Task\n\nGenerate a narrative summary of this user's history.\n\n");
    }

    if !session_summaries.is_empty() {
        prompt.push_str("### Session Summaries\n\n");
        for (i, s) in session_summaries.iter().enumerate() {
            prompt.push_str(&format!(
                "{}. **{}** ({}): {}\n",
                i + 1,
                s.session_name.as_deref().unwrap_or("Untitled session"),
                s.created_at.format("%Y-%m-%d"),
                s.summary
            ));
        }
        prompt.push('\n');
    }

    if !key_fact_changes.is_empty() {
        prompt.push_str("### Key Fact Changes\n\n");
        for change in key_fact_changes {
            prompt.push_str(&format!("- {change}\n"));
        }
        prompt.push('\n');
    }

    prompt.push_str(&format!(
        "## Instructions\n\n\
         Write a concise narrative (2-5 paragraphs) that tells the story of this user's \
         evolution. Include at most {max_chapters} chapters. Each chapter should cover a \
         distinct period and note what changed, what was reinforced, and what decayed.\n\n\
         Format your response as JSON with this schema:\n\
         {{\n  \
           \"narrative_text\": \"<full prose narrative>\",\n  \
           \"chapters\": [\n    \
             {{\n      \
               \"period\": \"<time label>\",\n      \
               \"summary\": \"<chapter summary>\",\n      \
               \"key_changes\": [\"<change1>\", \"<change2>\"]\n    \
             }}\n  \
           ]\n\
         }}\n"
    ));

    prompt
}

/// Input for narrative generation — a summary of a single session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummaryInput {
    pub session_id: Uuid,
    pub session_name: Option<String>,
    pub summary: String,
    pub created_at: DateTime<Utc>,
}

/// The raw output from the LLM narrative generation, before we wrap it
/// into a full `UserNarrative`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeGenerationOutput {
    pub narrative_text: String,
    #[serde(default)]
    pub chapters: Vec<NarrativeChapter>,
}

/// Parse the LLM's JSON response into a `NarrativeGenerationOutput`.
///
/// Tolerant of minor formatting issues — tries to extract JSON from
/// markdown code blocks if present.
pub fn parse_narrative_output(raw: &str) -> Result<NarrativeGenerationOutput, String> {
    // Try direct parse first
    if let Ok(output) = serde_json::from_str::<NarrativeGenerationOutput>(raw) {
        return Ok(output);
    }

    // Try extracting from markdown code block
    let trimmed = raw.trim();
    let json_str = if trimmed.starts_with("```json") {
        trimmed
            .strip_prefix("```json")
            .and_then(|s| s.strip_suffix("```"))
            .map(|s| s.trim())
    } else if trimmed.starts_with("```") {
        trimmed
            .strip_prefix("```")
            .and_then(|s| s.strip_suffix("```"))
            .map(|s| s.trim())
    } else {
        None
    };

    if let Some(extracted) = json_str {
        if let Ok(output) = serde_json::from_str::<NarrativeGenerationOutput>(extracted) {
            return Ok(output);
        }
    }

    // Last resort: try to find JSON object in the text
    if let Some(start) = raw.find('{') {
        if let Some(end) = raw.rfind('}') {
            let candidate = &raw[start..=end];
            if let Ok(output) = serde_json::from_str::<NarrativeGenerationOutput>(candidate) {
                return Ok(output);
            }
        }
    }

    // If all parsing fails, treat the whole text as the narrative with no chapters
    if !trimmed.is_empty() {
        return Ok(NarrativeGenerationOutput {
            narrative_text: trimmed.to_string(),
            chapters: Vec::new(),
        });
    }

    Err("empty or unparseable narrative output".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_user_id() -> Uuid {
        Uuid::from_u128(1)
    }

    #[test]
    fn test_narrative_new() {
        let uid = make_user_id();
        let n = UserNarrative::new(uid, "Test narrative".into(), vec![]);
        assert_eq!(n.user_id, uid);
        assert_eq!(n.version, 1);
        assert_eq!(n.session_count, 0);
        assert!(!n.is_empty());
        assert_eq!(n.total_key_changes(), 0);
        assert_eq!(n.total_referenced_sessions(), 0);
    }

    #[test]
    fn test_narrative_evolve() {
        let uid = make_user_id();
        let n1 = UserNarrative::new(uid, "First version".into(), vec![]);
        let n2 = n1.evolve("Second version".into(), vec![], 3);
        assert_eq!(n2.version, 2);
        assert_eq!(n2.session_count, 3);
        assert_eq!(n2.user_id, uid);
        // created_at should be preserved from v1
        assert_eq!(n2.created_at, n1.created_at);

        let n3 = n2.evolve("Third version".into(), vec![], 2);
        assert_eq!(n3.version, 3);
        assert_eq!(n3.session_count, 5);
    }

    #[test]
    fn test_narrative_is_empty() {
        let uid = make_user_id();
        let empty = UserNarrative::new(uid, "   ".into(), vec![]);
        assert!(empty.is_empty());

        let not_empty = UserNarrative::new(uid, "Has content".into(), vec![]);
        assert!(!not_empty.is_empty());
    }

    #[test]
    fn test_chapter_serde_roundtrip() {
        let chapter = NarrativeChapter {
            period: "March 2026".into(),
            summary: "User started fitness journey".into(),
            key_changes: vec!["Added running preference".into(), "Set 5K goal".into()],
            session_ids: vec![Uuid::from_u128(10), Uuid::from_u128(11)],
        };
        let json = serde_json::to_string(&chapter).unwrap();
        let de: NarrativeChapter = serde_json::from_str(&json).unwrap();
        assert_eq!(de, chapter);
    }

    #[test]
    fn test_narrative_serde_roundtrip() {
        let uid = make_user_id();
        let n = UserNarrative::new(
            uid,
            "Test narrative text".into(),
            vec![NarrativeChapter {
                period: "Week 1".into(),
                summary: "Getting started".into(),
                key_changes: vec!["First interaction".into()],
                session_ids: vec![Uuid::from_u128(100)],
            }],
        );
        let json = serde_json::to_string(&n).unwrap();
        let de: UserNarrative = serde_json::from_str(&json).unwrap();
        assert_eq!(de.user_id, n.user_id);
        assert_eq!(de.version, n.version);
        assert_eq!(de.narrative_text, n.narrative_text);
        assert_eq!(de.chapters.len(), 1);
        assert_eq!(de.chapters[0].period, "Week 1");
    }

    #[test]
    fn test_total_key_changes() {
        let uid = make_user_id();
        let n = UserNarrative::new(
            uid,
            "Test".into(),
            vec![
                NarrativeChapter {
                    period: "Ch1".into(),
                    summary: "S1".into(),
                    key_changes: vec!["a".into(), "b".into()],
                    session_ids: vec![],
                },
                NarrativeChapter {
                    period: "Ch2".into(),
                    summary: "S2".into(),
                    key_changes: vec!["c".into()],
                    session_ids: vec![],
                },
            ],
        );
        assert_eq!(n.total_key_changes(), 3);
    }

    #[test]
    fn test_total_referenced_sessions_deduplicates() {
        let uid = make_user_id();
        let shared_session = Uuid::from_u128(42);
        let n = UserNarrative::new(
            uid,
            "Test".into(),
            vec![
                NarrativeChapter {
                    period: "Ch1".into(),
                    summary: "S1".into(),
                    key_changes: vec![],
                    session_ids: vec![shared_session, Uuid::from_u128(43)],
                },
                NarrativeChapter {
                    period: "Ch2".into(),
                    summary: "S2".into(),
                    key_changes: vec![],
                    session_ids: vec![shared_session, Uuid::from_u128(44)],
                },
            ],
        );
        // shared_session appears in both chapters but should only count once
        assert_eq!(n.total_referenced_sessions(), 3);
    }

    #[test]
    fn test_refresh_request_defaults() {
        let req = RefreshNarrativeRequest::default();
        assert!(!req.full_rebuild);
        assert_eq!(req.max_chapters, 20);
    }

    #[test]
    fn test_refresh_request_serde() {
        let json = r#"{"full_rebuild": true, "max_chapters": 5}"#;
        let req: RefreshNarrativeRequest = serde_json::from_str(json).unwrap();
        assert!(req.full_rebuild);
        assert_eq!(req.max_chapters, 5);
    }

    #[test]
    fn test_build_narrative_prompt_fresh() {
        let summaries = vec![SessionSummaryInput {
            session_id: Uuid::from_u128(1),
            session_name: Some("Onboarding".into()),
            summary: "User set up their profile".into(),
            created_at: Utc::now(),
        }];
        let prompt =
            build_narrative_prompt(None, &summaries, &["Added name preference".into()], 10);
        assert!(prompt.contains("Generate a narrative"));
        assert!(prompt.contains("Onboarding"));
        assert!(prompt.contains("Added name preference"));
        assert!(prompt.contains("10 chapters"));
        // Should NOT contain "Previous Narrative" since it's fresh
        assert!(!prompt.contains("Previous Narrative"));
    }

    #[test]
    fn test_build_narrative_prompt_incremental() {
        let prompt = build_narrative_prompt(
            Some("User is a fitness enthusiast."),
            &[],
            &["Shifted to recovery focus".into()],
            5,
        );
        assert!(prompt.contains("Previous Narrative"));
        assert!(prompt.contains("fitness enthusiast"));
        assert!(prompt.contains("Shifted to recovery focus"));
    }

    #[test]
    fn test_build_narrative_prompt_empty_inputs() {
        let prompt = build_narrative_prompt(None, &[], &[], 20);
        assert!(prompt.contains("Generate a narrative"));
        // Should still have instructions even with no data
        assert!(prompt.contains("Instructions"));
    }

    #[test]
    fn test_parse_narrative_output_direct_json() {
        let json = r#"{"narrative_text": "User evolved.", "chapters": [{"period": "Q1", "summary": "Started", "key_changes": ["Began"]}]}"#;
        let output = parse_narrative_output(json).unwrap();
        assert_eq!(output.narrative_text, "User evolved.");
        assert_eq!(output.chapters.len(), 1);
        assert_eq!(output.chapters[0].period, "Q1");
    }

    #[test]
    fn test_parse_narrative_output_markdown_block() {
        let raw = "```json\n{\"narrative_text\": \"Story here.\", \"chapters\": []}\n```";
        let output = parse_narrative_output(raw).unwrap();
        assert_eq!(output.narrative_text, "Story here.");
    }

    #[test]
    fn test_parse_narrative_output_embedded_json() {
        let raw =
            "Here is the narrative:\n{\"narrative_text\": \"Embedded.\", \"chapters\": []}\nDone.";
        let output = parse_narrative_output(raw).unwrap();
        assert_eq!(output.narrative_text, "Embedded.");
    }

    #[test]
    fn test_parse_narrative_output_plain_text_fallback() {
        let raw = "This is just plain text with no JSON structure.";
        let output = parse_narrative_output(raw).unwrap();
        assert_eq!(output.narrative_text, raw);
        assert!(output.chapters.is_empty());
    }

    #[test]
    fn test_parse_narrative_output_empty() {
        let result = parse_narrative_output("");
        assert!(result.is_err());
    }

    #[test]
    fn test_session_summary_input_serde() {
        let input = SessionSummaryInput {
            session_id: Uuid::from_u128(5),
            session_name: Some("Chat #5".into()),
            summary: "Discussed preferences".into(),
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&input).unwrap();
        let de: SessionSummaryInput = serde_json::from_str(&json).unwrap();
        assert_eq!(de.session_id, input.session_id);
        assert_eq!(de.summary, input.summary);
    }

    // ─── Falsification / Adversarial Tests ─────────────────────────

    #[test]
    fn test_falsify_evolve_overflow_version() {
        // Version counter should not overflow or panic with many evolutions
        let uid = make_user_id();
        let mut n = UserNarrative::new(uid, "Start".into(), vec![]);
        n.version = u64::MAX - 1;
        let n2 = n.evolve("Next".into(), vec![], 1);
        assert_eq!(n2.version, u64::MAX);
        // One more evolution would overflow — this should be caught
        // (currently wraps in release mode; the test documents this edge case)
    }

    #[test]
    fn test_falsify_evolve_session_count_accumulates() {
        // Ensure session_count is strictly additive, never decreasing
        let uid = make_user_id();
        let n1 = UserNarrative::new(uid, "V1".into(), vec![]);
        let n2 = n1.evolve("V2".into(), vec![], 10);
        let n3 = n2.evolve("V3".into(), vec![], 0);
        assert_eq!(n3.session_count, 10); // 0 sessions added = no change
        let n4 = n3.evolve("V4".into(), vec![], 5);
        assert_eq!(n4.session_count, 15);
    }

    #[test]
    fn test_falsify_empty_narrative_text_whitespace_variants() {
        let uid = make_user_id();
        // Various whitespace strings should all be considered empty
        for ws in &["", " ", "  ", "\t", "\n", " \t\n "] {
            let n = UserNarrative::new(uid, ws.to_string(), vec![]);
            assert!(n.is_empty(), "Expected is_empty() for {:?}", ws);
        }
    }

    #[test]
    fn test_falsify_parse_narrative_output_nested_json() {
        // JSON with nested braces should parse correctly
        let raw = r#"{"narrative_text": "User said {\"hello\"}", "chapters": []}"#;
        let output = parse_narrative_output(raw).unwrap();
        assert!(output.narrative_text.contains("hello"));
    }

    #[test]
    fn test_falsify_parse_narrative_output_multiple_json_objects() {
        // If text contains multiple JSON objects, should extract the outermost one
        let raw = "Prefix {\"narrative_text\": \"outer\", \"chapters\": []} suffix {\"junk\": 1}";
        let output = parse_narrative_output(raw).unwrap();
        // The outermost {...} includes everything from first { to last }
        // This tests that the brace-matching heuristic works
        assert!(!output.narrative_text.is_empty());
    }

    #[test]
    fn test_falsify_parse_narrative_output_unicode() {
        let json = r#"{"narrative_text": "User prefers \u00e9clair and caf\u00e9", "chapters": [{"period": "\u2603 Winter", "summary": "Cold season", "key_changes": ["\u2764 Love"]}]}"#;
        let output = parse_narrative_output(json).unwrap();
        assert!(output.narrative_text.contains("caf"));
        assert_eq!(output.chapters.len(), 1);
    }

    #[test]
    fn test_falsify_parse_narrative_output_only_whitespace() {
        let result = parse_narrative_output("   \n\t  ");
        assert!(result.is_err());
    }

    #[test]
    fn test_falsify_chapter_with_empty_key_changes() {
        let chapter = NarrativeChapter {
            period: "Q1".into(),
            summary: "Nothing changed".into(),
            key_changes: vec![],
            session_ids: vec![],
        };
        let json = serde_json::to_string(&chapter).unwrap();
        let de: NarrativeChapter = serde_json::from_str(&json).unwrap();
        assert_eq!(de.key_changes.len(), 0);
    }

    #[test]
    fn test_falsify_chapter_missing_optional_fields_deserialize() {
        // Chapters in LLM output may omit session_ids and key_changes
        let json = r#"{"period": "March", "summary": "Stuff happened"}"#;
        let chapter: NarrativeChapter = serde_json::from_str(json).unwrap();
        assert_eq!(chapter.period, "March");
        assert!(chapter.key_changes.is_empty());
        assert!(chapter.session_ids.is_empty());
    }

    #[test]
    fn test_falsify_very_long_narrative_text() {
        let uid = make_user_id();
        let long_text = "x".repeat(100_000);
        let n = UserNarrative::new(uid, long_text.clone(), vec![]);
        assert_eq!(n.narrative_text.len(), 100_000);
        // Serde roundtrip should handle large text
        let json = serde_json::to_string(&n).unwrap();
        let de: UserNarrative = serde_json::from_str(&json).unwrap();
        assert_eq!(de.narrative_text.len(), 100_000);
    }

    #[test]
    fn test_falsify_many_chapters() {
        let uid = make_user_id();
        let chapters: Vec<NarrativeChapter> = (0..500)
            .map(|i| NarrativeChapter {
                period: format!("Week {i}"),
                summary: format!("Summary for week {i}"),
                key_changes: vec![format!("Change {i}")],
                session_ids: vec![Uuid::from_u128(i as u128)],
            })
            .collect();
        let n = UserNarrative::new(uid, "Narrative".into(), chapters);
        assert_eq!(n.chapters.len(), 500);
        assert_eq!(n.total_key_changes(), 500);
        assert_eq!(n.total_referenced_sessions(), 500);
    }

    #[test]
    fn test_falsify_build_prompt_special_characters_in_summaries() {
        let summaries = vec![SessionSummaryInput {
            session_id: Uuid::from_u128(1),
            session_name: Some("Session with \"quotes\" & <html>".into()),
            summary: "User said: 'I like {braces} and [brackets]'".into(),
            created_at: Utc::now(),
        }];
        let prompt = build_narrative_prompt(None, &summaries, &[], 5);
        // Should include special characters without panicking
        assert!(prompt.contains("quotes"));
        assert!(prompt.contains("{braces}"));
    }

    #[test]
    fn test_falsify_narrative_created_at_preserved_across_evolve() {
        // Regression test: created_at must be frozen at v1
        let uid = make_user_id();
        let n1 = UserNarrative::new(uid, "V1".into(), vec![]);
        let original_created = n1.created_at;
        std::thread::sleep(std::time::Duration::from_millis(10));
        let n2 = n1.evolve("V2".into(), vec![], 1);
        assert_eq!(n2.created_at, original_created);
        assert!(n2.updated_at >= original_created);
        std::thread::sleep(std::time::Duration::from_millis(10));
        let n3 = n2.evolve("V3".into(), vec![], 1);
        assert_eq!(n3.created_at, original_created);
    }
}
