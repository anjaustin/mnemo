//! Memory guardrails — rule-based policy engine.
//!
//! [`GuardrailRule`] defines composable condition predicates (classification
//! thresholds, confidence floors, entity/edge type filters, content regex,
//! caller role checks) paired with actions (block, redact, reclassify, audit,
//! warn). Rules are priority-ordered with short-circuit on block.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::api_key::ApiKeyRole;
use super::classification::Classification;

// ─── Guardrail Rule ────────────────────────────────────────────────

/// A declarative constraint rule evaluated at write time (ingestion) and/or
/// read time (retrieval).  Rules are defined per-user or globally and enforced
/// automatically by the guardrails evaluation pipeline.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GuardrailRule {
    pub id: Uuid,

    /// Human-readable name (e.g. "block_pii_storage", "restrict_health_data").
    pub name: String,

    /// Longer description of what this rule does.
    pub description: String,

    /// When this rule fires.
    pub trigger: GuardrailTrigger,

    /// What the rule checks.
    pub condition: GuardrailCondition,

    /// What happens when the condition matches.
    pub action: GuardrailAction,

    /// Lower = evaluated first.  Deterministic ordering.
    pub priority: u32,

    /// If false, the rule is skipped during evaluation.
    pub enabled: bool,

    /// Global or per-user scope.
    pub scope: GuardrailScope,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ─── Trigger ───────────────────────────────────────────────────────

/// When a guardrail rule fires.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailTrigger {
    /// Evaluated when episodes are ingested.
    OnIngest,
    /// Evaluated when edges are created from extraction.
    OnFactCreation,
    /// Evaluated during context assembly (read path).
    OnRetrieval,
    /// Evaluated when entities are created.
    OnEntityCreation,
    /// All of the above.
    OnAny,
}

impl GuardrailTrigger {
    /// Returns true when `self` should fire for the given operation type.
    pub fn matches(&self, operation: &GuardrailTrigger) -> bool {
        *self == GuardrailTrigger::OnAny || *self == *operation
    }
}

// ─── Condition ─────────────────────────────────────────────────────

/// What a guardrail rule checks.  Conditions form a composable tree via
/// `And`, `Or`, and `Not` combinators.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GuardrailCondition {
    /// Fact/entity classification is strictly above the threshold.
    ClassificationAbove { classification: Classification },
    /// Entity type is in the given list (case-insensitive).
    EntityTypeIn { entity_types: Vec<String> },
    /// Edge label is in the given list (case-insensitive).
    EdgeLabelIn { labels: Vec<String> },
    /// Content (fact text or episode content) matches a regex pattern.
    ContentMatchesRegex { pattern: String },
    /// Caller role is strictly below the required role.
    CallerRoleBelow { role: ApiKeyRole },
    /// Fact age is above N days.
    FactAgeAboveDays { days: u32 },
    /// Fact confidence is below the threshold.
    ConfidenceBelow { confidence: f32 },
    /// All sub-conditions must match.
    And {
        #[schema(no_recursion)]
        conditions: Vec<GuardrailCondition>,
    },
    /// Any sub-condition must match.
    Or {
        #[schema(no_recursion)]
        conditions: Vec<GuardrailCondition>,
    },
    /// Negate the sub-condition.
    Not {
        #[schema(no_recursion)]
        condition: Box<GuardrailCondition>,
    },
}

// ─── Action ────────────────────────────────────────────────────────

/// What happens when a guardrail condition matches.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GuardrailAction {
    /// Reject the operation with an error message.
    Block { reason: String },
    /// Remove the matching content from output.
    Redact,
    /// Upgrade the classification of the fact/entity.
    Reclassify { classification: Classification },
    /// Allow but log to governance audit.
    AuditOnly { severity: String },
    /// Allow but include a warning in the response.
    Warn { message: String },
}

// ─── Scope ─────────────────────────────────────────────────────────

/// Whether a guardrail rule applies globally or to a specific user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GuardrailScope {
    /// Applies to all users.
    Global,
    /// Applies to a specific user only.
    User { user_id: Uuid },
}

// ─── Evaluation Context ────────────────────────────────────────────

/// Data provided to the guardrail evaluator for condition checking.
/// Not all fields are populated for every operation type.
#[derive(Debug, Clone, Default)]
pub struct EvalContext {
    /// The classification of the entity/edge being evaluated.
    pub classification: Option<Classification>,
    /// Entity type (e.g. "person", "organization").
    pub entity_type: Option<String>,
    /// Edge label (e.g. "works_at", "salary").
    pub edge_label: Option<String>,
    /// Content text (episode content or fact text).
    pub content: Option<String>,
    /// Caller's role.
    pub caller_role: Option<ApiKeyRole>,
    /// Fact age in days.
    pub fact_age_days: Option<u32>,
    /// Fact confidence score.
    pub confidence: Option<f32>,
    /// The user ID for scope matching.
    pub user_id: Option<Uuid>,
}

// ─── Evaluation Result ─────────────────────────────────────────────

/// Result of evaluating a single guardrail rule.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GuardrailEvalResult {
    pub rule_id: Uuid,
    pub rule_name: String,
    pub matched: bool,
    pub action: GuardrailAction,
}

/// Aggregate result of evaluating all applicable guardrail rules.
#[derive(Debug, Clone, Default)]
pub struct GuardrailVerdict {
    /// If true, the operation should be blocked.
    pub blocked: bool,
    /// Reason for blocking (from the first Block action that matched).
    pub block_reason: Option<String>,
    /// If true, the content should be redacted from output.
    pub redact: bool,
    /// Classification to upgrade to (from the highest Reclassify action).
    pub reclassify_to: Option<Classification>,
    /// Warnings to include in the response.
    pub warnings: Vec<String>,
    /// Audit entries to log.
    pub audit_entries: Vec<(String, String)>, // (rule_name, severity)
    /// Details of each rule evaluation (for dry-run).
    pub details: Vec<GuardrailEvalResult>,
}

// ─── Condition Evaluation ──────────────────────────────────────────

impl GuardrailCondition {
    /// Evaluate this condition tree against the given context.
    pub fn evaluate(&self, ctx: &EvalContext) -> bool {
        match self {
            Self::ClassificationAbove { classification } => ctx
                .classification
                .map(|c| c > *classification)
                .unwrap_or(false),
            Self::EntityTypeIn { entity_types } => ctx
                .entity_type
                .as_ref()
                .is_some_and(|et| entity_types.iter().any(|t| t.eq_ignore_ascii_case(et))),
            Self::EdgeLabelIn { labels } => ctx
                .edge_label
                .as_ref()
                .is_some_and(|el| labels.iter().any(|l| l.eq_ignore_ascii_case(el))),
            Self::ContentMatchesRegex { pattern } => {
                ctx.content.as_ref().is_some_and(|content| {
                    // Compile regex; if invalid, treat as non-match (safe default)
                    regex::Regex::new(pattern)
                        .map(|re| re.is_match(content))
                        .unwrap_or(false)
                })
            }
            Self::CallerRoleBelow { role } => ctx
                .caller_role
                .map(|cr| (cr as u8) < (*role as u8))
                .unwrap_or(false),
            Self::FactAgeAboveDays { days } => {
                ctx.fact_age_days.map(|age| age > *days).unwrap_or(false)
            }
            Self::ConfidenceBelow { confidence } => {
                ctx.confidence.map(|c| c < *confidence).unwrap_or(false)
            }
            Self::And { conditions } => conditions.iter().all(|c| c.evaluate(ctx)),
            Self::Or { conditions } => conditions.iter().any(|c| c.evaluate(ctx)),
            Self::Not { condition } => !condition.evaluate(ctx),
        }
    }
}

// ─── Rule Evaluation ───────────────────────────────────────────────

impl GuardrailRule {
    /// Check whether this rule applies to the given operation and scope.
    pub fn applies_to(&self, trigger: &GuardrailTrigger, user_id: Option<Uuid>) -> bool {
        if !self.enabled {
            return false;
        }
        if !self.trigger.matches(trigger) {
            return false;
        }
        match &self.scope {
            GuardrailScope::Global => true,
            GuardrailScope::User { user_id: rule_user } => user_id == Some(*rule_user),
        }
    }
}

/// Evaluate a set of rules against a context for a given operation type.
///
/// Rules are assumed to be pre-sorted by priority (lower = first).
/// Evaluation stops on the first `Block` action.
pub fn evaluate_rules(
    rules: &[GuardrailRule],
    trigger: &GuardrailTrigger,
    ctx: &EvalContext,
) -> GuardrailVerdict {
    let mut verdict = GuardrailVerdict::default();

    for rule in rules {
        if !rule.applies_to(trigger, ctx.user_id) {
            continue;
        }

        let matched = rule.condition.evaluate(ctx);
        verdict.details.push(GuardrailEvalResult {
            rule_id: rule.id,
            rule_name: rule.name.clone(),
            matched,
            action: rule.action.clone(),
        });

        if !matched {
            continue;
        }

        match &rule.action {
            GuardrailAction::Block { reason } => {
                verdict.blocked = true;
                verdict.block_reason = Some(reason.clone());
                // Stop on first block — no need to evaluate further
                return verdict;
            }
            GuardrailAction::Redact => {
                verdict.redact = true;
            }
            GuardrailAction::Reclassify { classification } => {
                // Keep the highest reclassification
                match verdict.reclassify_to {
                    None => verdict.reclassify_to = Some(*classification),
                    Some(existing) if *classification > existing => {
                        verdict.reclassify_to = Some(*classification);
                    }
                    _ => {}
                }
            }
            GuardrailAction::AuditOnly { severity } => {
                verdict
                    .audit_entries
                    .push((rule.name.clone(), severity.clone()));
            }
            GuardrailAction::Warn { message } => {
                verdict.warnings.push(message.clone());
            }
        }
    }

    verdict
}

// ─── Request / Response types ──────────────────────────────────────

/// Request body for `POST /api/v1/guardrails`.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct CreateGuardrailRequest {
    pub name: String,
    pub description: String,
    pub trigger: GuardrailTrigger,
    pub condition: GuardrailCondition,
    pub action: GuardrailAction,
    #[serde(default)]
    pub priority: u32,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_scope")]
    pub scope: GuardrailScope,
}

fn default_true() -> bool {
    true
}

fn default_scope() -> GuardrailScope {
    GuardrailScope::Global
}

/// Request body for the dry-run evaluate endpoint.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct EvaluateGuardrailsRequest {
    pub trigger: GuardrailTrigger,
    #[serde(default)]
    pub classification: Option<Classification>,
    #[serde(default)]
    pub entity_type: Option<String>,
    #[serde(default)]
    pub edge_label: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub caller_role: Option<ApiKeyRole>,
    #[serde(default)]
    pub fact_age_days: Option<u32>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub user_id: Option<Uuid>,
}

impl EvaluateGuardrailsRequest {
    /// Convert into an EvalContext for rule evaluation.
    pub fn into_eval_context(self) -> EvalContext {
        EvalContext {
            classification: self.classification,
            entity_type: self.entity_type,
            edge_label: self.edge_label,
            content: self.content,
            caller_role: self.caller_role,
            fact_age_days: self.fact_age_days,
            confidence: self.confidence,
            user_id: self.user_id,
        }
    }
}

/// Response for the dry-run evaluate endpoint.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct EvaluateGuardrailsResponse {
    pub blocked: bool,
    pub block_reason: Option<String>,
    pub redact: bool,
    pub reclassify_to: Option<Classification>,
    pub warnings: Vec<String>,
    pub audit_entries: Vec<AuditEntry>,
    pub rule_results: Vec<GuardrailEvalResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AuditEntry {
    pub rule_name: String,
    pub severity: String,
}

impl From<GuardrailVerdict> for EvaluateGuardrailsResponse {
    fn from(v: GuardrailVerdict) -> Self {
        Self {
            blocked: v.blocked,
            block_reason: v.block_reason,
            redact: v.redact,
            reclassify_to: v.reclassify_to,
            warnings: v.warnings,
            audit_entries: v
                .audit_entries
                .into_iter()
                .map(|(rule_name, severity)| AuditEntry {
                    rule_name,
                    severity,
                })
                .collect(),
            rule_results: v.details,
        }
    }
}

// ─── Validation ────────────────────────────────────────────────────

/// Validate a guardrail rule name: 1–64 chars, alphanumeric + underscores + hyphens.
pub fn validate_guardrail_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err("Guardrail name must be 1-64 characters".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(
            "Guardrail name must contain only alphanumeric characters, underscores, and hyphens"
                .into(),
        );
    }
    Ok(())
}

/// Validate that a regex pattern in a ContentMatchesRegex condition compiles.
/// Returns Err with a user-friendly message if the pattern is invalid.
pub fn validate_condition_regexes(condition: &GuardrailCondition) -> Result<(), String> {
    match condition {
        GuardrailCondition::ContentMatchesRegex { pattern } => {
            regex::Regex::new(pattern)
                .map_err(|e| format!("Invalid regex pattern '{}': {}", pattern, e))?;
            Ok(())
        }
        GuardrailCondition::And { conditions } => {
            for c in conditions {
                validate_condition_regexes(c)?;
            }
            Ok(())
        }
        GuardrailCondition::Or { conditions } => {
            for c in conditions {
                validate_condition_regexes(c)?;
            }
            Ok(())
        }
        GuardrailCondition::Not { condition } => validate_condition_regexes(condition),
        _ => Ok(()),
    }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rule(
        id: u128,
        name: &str,
        trigger: GuardrailTrigger,
        condition: GuardrailCondition,
        action: GuardrailAction,
        priority: u32,
    ) -> GuardrailRule {
        let now = Utc::now();
        GuardrailRule {
            id: Uuid::from_u128(id),
            name: name.to_string(),
            description: format!("Test rule: {}", name),
            trigger,
            condition,
            action,
            priority,
            enabled: true,
            scope: GuardrailScope::Global,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn condition_classification_above() {
        let cond = GuardrailCondition::ClassificationAbove {
            classification: Classification::Internal,
        };
        let mut ctx = EvalContext::default();

        // No classification → false
        assert!(!cond.evaluate(&ctx));

        ctx.classification = Some(Classification::Public);
        assert!(!cond.evaluate(&ctx)); // Public <= Internal

        ctx.classification = Some(Classification::Internal);
        assert!(!cond.evaluate(&ctx)); // Internal == Internal (not above)

        ctx.classification = Some(Classification::Confidential);
        assert!(cond.evaluate(&ctx)); // Confidential > Internal
    }

    #[test]
    fn condition_entity_type_in() {
        let cond = GuardrailCondition::EntityTypeIn {
            entity_types: vec!["person".into(), "organization".into()],
        };
        let mut ctx = EvalContext::default();

        assert!(!cond.evaluate(&ctx)); // No entity type

        ctx.entity_type = Some("person".into());
        assert!(cond.evaluate(&ctx));

        ctx.entity_type = Some("Person".into()); // case-insensitive
        assert!(cond.evaluate(&ctx));

        ctx.entity_type = Some("product".into());
        assert!(!cond.evaluate(&ctx));
    }

    #[test]
    fn condition_edge_label_in() {
        let cond = GuardrailCondition::EdgeLabelIn {
            labels: vec!["salary".into(), "ssn".into()],
        };

        let ctx = EvalContext {
            edge_label: Some("salary".into()),
            ..Default::default()
        };
        assert!(cond.evaluate(&ctx));

        let ctx = EvalContext {
            edge_label: Some("SSN".into()), // case-insensitive
            ..Default::default()
        };
        assert!(cond.evaluate(&ctx));

        let ctx = EvalContext {
            edge_label: Some("works_at".into()),
            ..Default::default()
        };
        assert!(!cond.evaluate(&ctx));
    }

    #[test]
    fn condition_content_matches_regex() {
        let cond = GuardrailCondition::ContentMatchesRegex {
            pattern: r"\bSSN\b".into(),
        };

        let ctx = EvalContext {
            content: Some("My SSN is 123-45-6789".into()),
            ..Default::default()
        };
        assert!(cond.evaluate(&ctx));

        let ctx = EvalContext {
            content: Some("No sensitive data here".into()),
            ..Default::default()
        };
        assert!(!cond.evaluate(&ctx));

        // Invalid regex → non-match (safe default)
        let bad_cond = GuardrailCondition::ContentMatchesRegex {
            pattern: r"[invalid".into(),
        };
        let ctx = EvalContext {
            content: Some("anything".into()),
            ..Default::default()
        };
        assert!(!bad_cond.evaluate(&ctx));
    }

    #[test]
    fn condition_caller_role_below() {
        let cond = GuardrailCondition::CallerRoleBelow {
            role: ApiKeyRole::Admin,
        };

        let ctx = EvalContext {
            caller_role: Some(ApiKeyRole::Read),
            ..Default::default()
        };
        assert!(cond.evaluate(&ctx)); // Read < Admin

        let ctx = EvalContext {
            caller_role: Some(ApiKeyRole::Write),
            ..Default::default()
        };
        assert!(cond.evaluate(&ctx)); // Write < Admin

        let ctx = EvalContext {
            caller_role: Some(ApiKeyRole::Admin),
            ..Default::default()
        };
        assert!(!cond.evaluate(&ctx)); // Admin == Admin (not below)
    }

    #[test]
    fn condition_fact_age_above_days() {
        let cond = GuardrailCondition::FactAgeAboveDays { days: 90 };

        let ctx = EvalContext {
            fact_age_days: Some(30),
            ..Default::default()
        };
        assert!(!cond.evaluate(&ctx));

        let ctx = EvalContext {
            fact_age_days: Some(91),
            ..Default::default()
        };
        assert!(cond.evaluate(&ctx));

        let ctx = EvalContext {
            fact_age_days: Some(90),
            ..Default::default()
        };
        assert!(!cond.evaluate(&ctx)); // exactly 90 is not above 90
    }

    #[test]
    fn condition_confidence_below() {
        let cond = GuardrailCondition::ConfidenceBelow { confidence: 0.5 };

        let ctx = EvalContext {
            confidence: Some(0.3),
            ..Default::default()
        };
        assert!(cond.evaluate(&ctx));

        let ctx = EvalContext {
            confidence: Some(0.5),
            ..Default::default()
        };
        assert!(!cond.evaluate(&ctx)); // 0.5 is not below 0.5

        let ctx = EvalContext {
            confidence: Some(0.9),
            ..Default::default()
        };
        assert!(!cond.evaluate(&ctx));
    }

    #[test]
    fn condition_and_combinator() {
        let cond = GuardrailCondition::And {
            conditions: vec![
                GuardrailCondition::EdgeLabelIn {
                    labels: vec!["salary".into()],
                },
                GuardrailCondition::ClassificationAbove {
                    classification: Classification::Public,
                },
            ],
        };

        // Both match
        let ctx = EvalContext {
            edge_label: Some("salary".into()),
            classification: Some(Classification::Internal),
            ..Default::default()
        };
        assert!(cond.evaluate(&ctx));

        // Only one matches
        let ctx = EvalContext {
            edge_label: Some("works_at".into()),
            classification: Some(Classification::Internal),
            ..Default::default()
        };
        assert!(!cond.evaluate(&ctx));
    }

    #[test]
    fn condition_or_combinator() {
        let cond = GuardrailCondition::Or {
            conditions: vec![
                GuardrailCondition::EdgeLabelIn {
                    labels: vec!["salary".into()],
                },
                GuardrailCondition::EdgeLabelIn {
                    labels: vec!["ssn".into()],
                },
            ],
        };

        let ctx = EvalContext {
            edge_label: Some("salary".into()),
            ..Default::default()
        };
        assert!(cond.evaluate(&ctx));

        let ctx = EvalContext {
            edge_label: Some("ssn".into()),
            ..Default::default()
        };
        assert!(cond.evaluate(&ctx));

        let ctx = EvalContext {
            edge_label: Some("works_at".into()),
            ..Default::default()
        };
        assert!(!cond.evaluate(&ctx));
    }

    #[test]
    fn condition_not_combinator() {
        let cond = GuardrailCondition::Not {
            condition: Box::new(GuardrailCondition::EdgeLabelIn {
                labels: vec!["salary".into()],
            }),
        };

        let ctx = EvalContext {
            edge_label: Some("works_at".into()),
            ..Default::default()
        };
        assert!(cond.evaluate(&ctx)); // NOT salary → true

        let ctx = EvalContext {
            edge_label: Some("salary".into()),
            ..Default::default()
        };
        assert!(!cond.evaluate(&ctx)); // NOT salary → false
    }

    #[test]
    fn evaluate_rules_block_stops_evaluation() {
        let rules = vec![
            make_rule(
                1,
                "block_ssn",
                GuardrailTrigger::OnIngest,
                GuardrailCondition::ContentMatchesRegex {
                    pattern: r"\bSSN\b".into(),
                },
                GuardrailAction::Block {
                    reason: "SSN references not allowed".into(),
                },
                10,
            ),
            make_rule(
                2,
                "audit_all",
                GuardrailTrigger::OnIngest,
                GuardrailCondition::ClassificationAbove {
                    classification: Classification::Public,
                },
                GuardrailAction::AuditOnly {
                    severity: "info".into(),
                },
                20,
            ),
        ];

        let ctx = EvalContext {
            content: Some("My SSN is 123-45-6789".into()),
            classification: Some(Classification::Internal),
            ..Default::default()
        };

        let verdict = evaluate_rules(&rules, &GuardrailTrigger::OnIngest, &ctx);
        assert!(verdict.blocked);
        assert_eq!(
            verdict.block_reason.as_deref(),
            Some("SSN references not allowed")
        );
        // Second rule should NOT have been evaluated (block stops evaluation)
        assert_eq!(verdict.details.len(), 1);
    }

    #[test]
    fn evaluate_rules_priority_ordering() {
        // Two rules: warning at priority 20, block at priority 10.
        // Block should fire first because it has lower priority number.
        let rules = vec![
            make_rule(
                1,
                "block_first",
                GuardrailTrigger::OnRetrieval,
                GuardrailCondition::ClassificationAbove {
                    classification: Classification::Internal,
                },
                GuardrailAction::Block {
                    reason: "blocked".into(),
                },
                10,
            ),
            make_rule(
                2,
                "warn_second",
                GuardrailTrigger::OnRetrieval,
                GuardrailCondition::ClassificationAbove {
                    classification: Classification::Internal,
                },
                GuardrailAction::Warn {
                    message: "high classification".into(),
                },
                20,
            ),
        ];

        let ctx = EvalContext {
            classification: Some(Classification::Confidential),
            ..Default::default()
        };

        let verdict = evaluate_rules(&rules, &GuardrailTrigger::OnRetrieval, &ctx);
        assert!(verdict.blocked);
        assert!(verdict.warnings.is_empty()); // Block stopped before warn
    }

    #[test]
    fn evaluate_rules_reclassify_takes_highest() {
        let rules = vec![
            make_rule(
                1,
                "reclassify_to_confidential",
                GuardrailTrigger::OnFactCreation,
                GuardrailCondition::EdgeLabelIn {
                    labels: vec!["salary".into()],
                },
                GuardrailAction::Reclassify {
                    classification: Classification::Confidential,
                },
                10,
            ),
            make_rule(
                2,
                "reclassify_to_restricted",
                GuardrailTrigger::OnFactCreation,
                GuardrailCondition::EdgeLabelIn {
                    labels: vec!["salary".into()],
                },
                GuardrailAction::Reclassify {
                    classification: Classification::Restricted,
                },
                20,
            ),
        ];

        let ctx = EvalContext {
            edge_label: Some("salary".into()),
            ..Default::default()
        };

        let verdict = evaluate_rules(&rules, &GuardrailTrigger::OnFactCreation, &ctx);
        assert!(!verdict.blocked);
        assert_eq!(verdict.reclassify_to, Some(Classification::Restricted));
    }

    #[test]
    fn evaluate_rules_scope_filtering() {
        let user1 = Uuid::from_u128(100);
        let user2 = Uuid::from_u128(200);
        let now = Utc::now();

        let rules = vec![GuardrailRule {
            id: Uuid::from_u128(1),
            name: "user_specific".into(),
            description: "Only for user1".into(),
            trigger: GuardrailTrigger::OnRetrieval,
            condition: GuardrailCondition::ClassificationAbove {
                classification: Classification::Public,
            },
            action: GuardrailAction::Warn {
                message: "user1 warning".into(),
            },
            priority: 10,
            enabled: true,
            scope: GuardrailScope::User { user_id: user1 },
            created_at: now,
            updated_at: now,
        }];

        // User 1 should get the warning
        let ctx1 = EvalContext {
            classification: Some(Classification::Internal),
            user_id: Some(user1),
            ..Default::default()
        };
        let v1 = evaluate_rules(&rules, &GuardrailTrigger::OnRetrieval, &ctx1);
        assert_eq!(v1.warnings.len(), 1);

        // User 2 should NOT get the warning
        let ctx2 = EvalContext {
            classification: Some(Classification::Internal),
            user_id: Some(user2),
            ..Default::default()
        };
        let v2 = evaluate_rules(&rules, &GuardrailTrigger::OnRetrieval, &ctx2);
        assert!(v2.warnings.is_empty());
    }

    #[test]
    fn evaluate_rules_disabled_rule_skipped() {
        let now = Utc::now();
        let rules = vec![GuardrailRule {
            id: Uuid::from_u128(1),
            name: "disabled".into(),
            description: "disabled rule".into(),
            trigger: GuardrailTrigger::OnAny,
            condition: GuardrailCondition::ClassificationAbove {
                classification: Classification::Public,
            },
            action: GuardrailAction::Block {
                reason: "should not fire".into(),
            },
            priority: 1,
            enabled: false,
            scope: GuardrailScope::Global,
            created_at: now,
            updated_at: now,
        }];

        let ctx = EvalContext {
            classification: Some(Classification::Restricted),
            ..Default::default()
        };
        let verdict = evaluate_rules(&rules, &GuardrailTrigger::OnRetrieval, &ctx);
        assert!(!verdict.blocked);
    }

    #[test]
    fn trigger_matches() {
        assert!(GuardrailTrigger::OnAny.matches(&GuardrailTrigger::OnIngest));
        assert!(GuardrailTrigger::OnAny.matches(&GuardrailTrigger::OnRetrieval));
        assert!(GuardrailTrigger::OnIngest.matches(&GuardrailTrigger::OnIngest));
        assert!(!GuardrailTrigger::OnIngest.matches(&GuardrailTrigger::OnRetrieval));
    }

    #[test]
    fn guardrail_serde_roundtrip() {
        let rule = make_rule(
            42,
            "test_rule",
            GuardrailTrigger::OnRetrieval,
            GuardrailCondition::And {
                conditions: vec![
                    GuardrailCondition::ClassificationAbove {
                        classification: Classification::Internal,
                    },
                    GuardrailCondition::Not {
                        condition: Box::new(GuardrailCondition::EdgeLabelIn {
                            labels: vec!["public_info".into()],
                        }),
                    },
                ],
            },
            GuardrailAction::Redact,
            5,
        );

        let json = serde_json::to_value(&rule).unwrap();
        let parsed: GuardrailRule = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.name, "test_rule");
        assert_eq!(parsed.priority, 5);
        assert!(parsed.enabled);
    }

    #[test]
    fn create_guardrail_request_serde() {
        let json = serde_json::json!({
            "name": "block_pii",
            "description": "Block PII storage",
            "trigger": "on_ingest",
            "condition": {
                "type": "content_matches_regex",
                "pattern": "\\bSSN\\b"
            },
            "action": {
                "type": "block",
                "reason": "PII not allowed"
            }
        });
        let req: CreateGuardrailRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.name, "block_pii");
        assert!(req.enabled); // default true
        assert_eq!(req.priority, 0); // default 0
        assert_eq!(req.scope, GuardrailScope::Global); // default global
    }

    #[test]
    fn validate_guardrail_name_rules() {
        assert!(validate_guardrail_name("block_pii").is_ok());
        assert!(validate_guardrail_name("rule-123").is_ok());
        assert!(validate_guardrail_name("").is_err());
        assert!(validate_guardrail_name("a".repeat(65).as_str()).is_err());
        assert!(validate_guardrail_name("no spaces").is_err());
        assert!(validate_guardrail_name("no.dots").is_err());
    }

    #[test]
    fn validate_condition_regex_valid() {
        let cond = GuardrailCondition::ContentMatchesRegex {
            pattern: r"\bSSN\b".into(),
        };
        assert!(validate_condition_regexes(&cond).is_ok());
    }

    #[test]
    fn validate_condition_regex_invalid() {
        let cond = GuardrailCondition::ContentMatchesRegex {
            pattern: r"[invalid".into(),
        };
        assert!(validate_condition_regexes(&cond).is_err());
    }

    #[test]
    fn validate_condition_regex_nested() {
        let cond = GuardrailCondition::And {
            conditions: vec![
                GuardrailCondition::ContentMatchesRegex {
                    pattern: r"\bSSN\b".into(),
                },
                GuardrailCondition::ContentMatchesRegex {
                    pattern: r"[bad".into(),
                },
            ],
        };
        assert!(validate_condition_regexes(&cond).is_err());
    }

    #[test]
    fn evaluate_guardrails_request_into_context() {
        let req = EvaluateGuardrailsRequest {
            trigger: GuardrailTrigger::OnIngest,
            classification: Some(Classification::Confidential),
            entity_type: Some("person".into()),
            edge_label: None,
            content: Some("test content".into()),
            caller_role: Some(ApiKeyRole::Read),
            fact_age_days: Some(45),
            confidence: Some(0.8),
            user_id: Some(Uuid::from_u128(1)),
        };
        let ctx = req.into_eval_context();
        assert_eq!(ctx.classification, Some(Classification::Confidential));
        assert_eq!(ctx.entity_type.as_deref(), Some("person"));
        assert_eq!(ctx.content.as_deref(), Some("test content"));
    }

    #[test]
    fn verdict_from_response_conversion() {
        let verdict = GuardrailVerdict {
            blocked: false,
            block_reason: None,
            redact: true,
            reclassify_to: Some(Classification::Restricted),
            warnings: vec!["careful".into()],
            audit_entries: vec![("rule1".into(), "high".into())],
            details: vec![],
        };
        let resp = EvaluateGuardrailsResponse::from(verdict);
        assert!(!resp.blocked);
        assert!(resp.redact);
        assert_eq!(resp.reclassify_to, Some(Classification::Restricted));
        assert_eq!(resp.warnings.len(), 1);
        assert_eq!(resp.audit_entries.len(), 1);
        assert_eq!(resp.audit_entries[0].rule_name, "rule1");
    }
}
