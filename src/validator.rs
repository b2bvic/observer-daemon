use crate::config::{CorrectionsConfig, Spec};
use crate::corrections::{PromotedRule, promoted_rule_key};
use crate::rubric::{ClassificationMetadata, ContentClass, classify};
use crate::validators::*;

fn clamp_char_boundary(s: &str, idx: usize) -> usize {
    let mut idx = idx.min(s.len());
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn issue_is_enabled(
    spec: &Spec,
    issue: &Issue,
    content_class: ContentClass,
    promoted_rules: &[PromotedRule],
) -> bool {
    if promoted_rules.iter().any(|rule| {
        promoted_rule_key(rule.content_class, &rule.violation_type, &rule.pattern)
            == promoted_rule_key(content_class, &issue.category, &issue.detail)
    }) {
        return true;
    }

    match (issue.category.as_str(), issue.detail.as_str()) {
        ("insight_bow", _) => spec.violations.active.iter().any(|v| v == "insight_bow"),
        ("rhetorical_crutch", _) => spec
            .violations
            .active
            .iter()
            .any(|v| v == "rhetorical_crutch_isnt_just_x"),
        ("layer1", _) => spec
            .violations
            .active
            .iter()
            .any(|v| v == "layer1_missing_adversarial"),
        ("layer2", _) => spec
            .violations
            .active
            .iter()
            .any(|v| v == "layer2_bot_as_architect"),
        ("FLOATE", "word:landscape") => spec
            .violations
            .active
            .iter()
            .any(|v| v == "floate_landscape"),
        ("FLOATE", "word:significant") => spec
            .violations
            .active
            .iter()
            .any(|v| v == "floate_significant"),
        ("policy_pack", _) => {
            spec.policy_pack.enabled || spec.violations.active.iter().any(|v| v == "policy_pack")
        }
        _ => true,
    }
}

fn matching_promoted_rule<'a>(
    issue: &Issue,
    content_class: ContentClass,
    promoted_rules: &'a [PromotedRule],
) -> Option<&'a PromotedRule> {
    promoted_rules.iter().find(|rule| {
        promoted_rule_key(rule.content_class, &rule.violation_type, &rule.pattern)
            == promoted_rule_key(content_class, &issue.category, &issue.detail)
    })
}

/// Result of validating a single response.
#[derive(Debug)]
pub struct ValidationResult {
    pub score: u32,
    pub issues: Vec<Issue>,
    pub response_chars: usize,
    pub content_class: ContentClass,
}

impl ValidationResult {
    pub fn correction_context(&self, config: &CorrectionsConfig) -> Option<String> {
        if self.score >= 75 || self.issues.is_empty() {
            return None;
        }

        let top_issues: Vec<String> = self.issues.iter().take(4).map(|i| i.display()).collect();
        Some(
            config
                .context_template
                .replace("{score}", &self.score.to_string())
                .replace("{issues}", &top_issues.join("; ")),
        )
    }

    pub fn to_json(&self, session_id: &str, source: &str) -> serde_json::Value {
        serde_json::json!({
            "score": self.score,
            "issues": self.issues.iter().map(|i| i.display()).collect::<Vec<_>>(),
            "session": session_id,
            "source": source,
            "response_chars": self.response_chars,
            "content_class": self.content_class,
        })
    }
}

/// Run all 16 validators against a response.
pub fn validate_response(
    response: &str,
    prompt: &str,
    prior_responses: &[String],
    promoted_rules: &[PromotedRule],
    spec: &Spec,
) -> ValidationResult {
    validate_response_with_metadata(
        response,
        prompt,
        &ClassificationMetadata::default(),
        prior_responses,
        promoted_rules,
        spec,
    )
}

pub fn validate_response_with_metadata(
    response: &str,
    prompt: &str,
    metadata: &ClassificationMetadata,
    prior_responses: &[String],
    promoted_rules: &[PromotedRule],
    spec: &Spec,
) -> ValidationResult {
    let content_class = classify(prompt, metadata);
    let mut all_issues = Vec::new();

    // 1+2. FLOATE words + phrases
    all_issues.extend(floate::validate(response, &spec.filters.floate));

    // 3. Badverbs
    all_issues.extend(badverbs::validate(response, &spec.filters.badverbs));

    // 4. Sycophancy
    all_issues.extend(sycophancy::validate(response, &spec.sycophancy));

    // 5. Insight bows
    {
        let tail_size = spec
            .insight_bow
            .scan_region
            .strip_prefix("tail_")
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(500);

        let tail = if response.len() > tail_size {
            let byte_start = response.len() - tail_size;
            let safe_start = clamp_char_boundary(response, byte_start);
            &response[safe_start..]
        } else {
            response
        };

        let lower = tail.to_lowercase();
        for pattern in &spec.insight_bow.patterns {
            let re = match regex::Regex::new(&format!("(?i){}", pattern)) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if re.is_match(&lower) {
                all_issues.push(Issue::new(
                    "insight_bow",
                    "neat conclusion wrapping the thread",
                    spec.insight_bow.deduction,
                ));
                break;
            }
        }
    }

    // 6. Filler phrases
    all_issues.extend(filler::validate(response, &spec.filler));

    // 7. Rhythm patterns (bullet, parallel, triple)
    all_issues.extend(rhythm::validate(response, &spec.rhythm));

    // 8. Verb monotony
    all_issues.extend(lexical::validate_monotony(response, &spec.lexical));

    // 9. Cross-turn repetition
    all_issues.extend(lexical::validate_repetition(
        response,
        prior_responses,
        &spec.repetition,
    ));

    // 10. Context-time alignment
    all_issues.extend(context_time::validate(response, &spec.context_time));

    // 10b. Cross-surface policy pack
    all_issues.extend(policy_pack::validate(response, prompt, &spec.policy_pack));

    // 11. Layer alignment
    all_issues.extend(layers::validate(
        response,
        prompt,
        content_class,
        &spec.layers,
    ));

    // 14. Rhetorical crutch
    all_issues.extend(crutch::validate(response, &spec.rhetorical_crutch));

    // 15+16. Punctuation (em dash + horizontal rule)
    all_issues.extend(punctuation::validate(response, &spec.punctuation));

    // 17+18. Opening/closing tells
    all_issues.extend(tells::validate(response, &spec.tells));

    all_issues.retain(|issue| issue_is_enabled(spec, issue, content_class, promoted_rules));
    let mut reject_promoted_rule_hit = false;
    for issue in &mut all_issues {
        if let Some(rule) = matching_promoted_rule(issue, content_class, promoted_rules) {
            issue.deduction = rule.deduction.max(issue.deduction);
            if rule.action.eq_ignore_ascii_case("reject") {
                reject_promoted_rule_hit = true;
            }
        }
    }

    // Calculate score
    let total_deduction: u32 = all_issues.iter().map(|i| i.deduction).sum();
    let mut score = 100u32.saturating_sub(total_deduction.min(spec.scoring.max_deductions));
    if reject_promoted_rule_hit {
        score = 0;
    }

    ValidationResult {
        score,
        issues: all_issues,
        response_chars: response.len(),
        content_class,
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_response, validate_response_with_metadata};
    use crate::config::load_spec;
    use crate::corrections::PromotedRule;
    use std::path::Path;

    #[test]
    fn promoted_reject_rule_forces_failure_and_uses_rule_deduction() {
        let spec = load_spec(Path::new("spec.toml.example")).expect("spec");
        let promoted = vec![PromotedRule {
            pattern: "word:landscape".to_string(),
            violation_type: "FLOATE".to_string(),
            promoted_date: "2026-04-09".to_string(),
            occurrence_count: 3,
            action: "reject".to_string(),
            deduction: 9,
            content_class: crate::rubric::ContentClass::Generic,
        }];

        let result = validate_response("A landscape shift matters.", "", &[], &promoted, &spec);

        assert_eq!(result.score, 0);
        assert!(result.issues.iter().any(|issue| issue.category == "FLOATE"
            && issue.detail == "word:landscape"
            && issue.deduction == 9));
        let context = result
            .correction_context(&spec.corrections)
            .expect("correction context");
        assert!(context.contains("prior response scored 0/100"));
        assert!(context.ends_with("configured writing and layer rules.]"));
    }

    #[test]
    fn promoted_rules_are_class_scoped() {
        let spec = load_spec(Path::new("spec.toml.example")).expect("spec");
        let promoted = vec![PromotedRule {
            pattern: "word:landscape".to_string(),
            violation_type: "FLOATE".to_string(),
            promoted_date: "2026-07-24".to_string(),
            occurrence_count: 3,
            action: "reject".to_string(),
            deduction: 9,
            content_class: crate::rubric::ContentClass::Legal,
        }];
        let metadata = crate::rubric::ClassificationMetadata {
            content_class: Some(crate::rubric::ContentClass::CareerApplication),
            labels: vec![],
        };

        let result = validate_response_with_metadata(
            "A landscape shift matters.",
            "Prepare my job application.",
            &metadata,
            &[],
            &promoted,
            &spec,
        );

        assert_ne!(result.score, 0);
        assert_eq!(
            result.content_class,
            crate::rubric::ContentClass::CareerApplication
        );
    }

    #[test]
    fn regression_2026_07_24_career_strategy_briefing_does_not_use_legal_rubric() {
        let spec = load_spec(Path::new("spec.toml.example")).expect("spec");
        let result = validate_response(
            "Lead with the candidate's operating experience and measurable outcomes.",
            "Prepare a career-strategy briefing for a candidate applying to a compliance role.",
            &[],
            &[],
            &spec,
        );

        assert_eq!(
            result.content_class,
            crate::rubric::ContentClass::CareerApplication
        );
        assert!(result.issues.iter().all(|issue| issue.category != "layer1"));
    }
}
