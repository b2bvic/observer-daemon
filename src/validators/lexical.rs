use std::collections::HashMap;

use super::Issue;
use crate::config::{LexicalConfig, RepetitionConfig};

/// Check for modal verb monotony (same verb 3+ times).
pub fn validate_monotony(text: &str, config: &LexicalConfig) -> Vec<Issue> {
    let mut issues = Vec::new();
    let modal_set: std::collections::HashSet<&str> =
        config.modal_verbs.iter().map(|v| v.as_str()).collect();

    // Count occurrences — need owned strings since we're working with lowercase
    let lower = text.to_lowercase();
    let mut counts: HashMap<String, u32> = HashMap::new();
    for word in lower.split(|c: char| !c.is_alphanumeric()) {
        if !word.is_empty() && modal_set.contains(word) {
            *counts.entry(word.to_string()).or_insert(0) += 1;
        }
    }

    let mut total_deduction = 0u32;
    let mut monotone: Vec<(String, u32)> = counts
        .into_iter()
        .filter(|(_, count)| *count >= config.verb_monotony_threshold)
        .collect();
    monotone.sort_by(|a, b| b.1.cmp(&a.1));

    for (verb, count) in monotone.iter().take(3) {
        if total_deduction >= config.max_deduction {
            break;
        }
        let deduction = config
            .deduction_per_monotone
            .min(config.max_deduction - total_deduction);
        issues.push(Issue::new(
            "verb_monotony",
            &format!("'{}' appears {}x", verb, count),
            deduction,
        ));
        total_deduction += deduction;
    }

    issues
}

/// Check for cross-turn phrase repetition (lexical entropy collapse).
/// Compares 4+ word phrases from current response against prior responses.
pub fn validate_repetition(
    response: &str,
    prior_responses: &[String],
    config: &RepetitionConfig,
) -> Vec<Issue> {
    if prior_responses.is_empty() {
        return Vec::new();
    }

    // Extract 4-word phrases from current response
    let words: Vec<&str> = response.split_whitespace().collect();
    let mut current_phrases = std::collections::HashSet::new();

    for window in words.windows(4) {
        let phrase = window.join(" ").to_lowercase();
        // Strip trailing punctuation
        let cleaned: String = phrase
            .trim_end_matches(|c: char| ".,!?;:".contains(c))
            .to_string();
        if cleaned.len() >= config.min_phrase_length {
            current_phrases.insert(cleaned);
        }
    }

    // Check against prior responses (most recent N)
    let lookback = prior_responses.len().min(config.lookback_turns);
    let recent = &prior_responses[prior_responses.len() - lookback..];

    let mut repeated = Vec::new();
    for prior in recent {
        let prior_lower = prior.to_lowercase();
        for phrase in &current_phrases {
            if prior_lower.contains(phrase.as_str()) && !repeated.contains(phrase) {
                repeated.push(phrase.clone());
            }
        }
    }

    let mut issues = Vec::new();
    if !repeated.is_empty() {
        let count = repeated.len().min(config.max_reported);
        let deduction = (count as u32 * config.deduction_per_repeat).min(config.max_deduction);
        issues.push(Issue::new(
            "repetition",
            &format!("{} phrases repeated from prior turns", count),
            deduction,
        ));
    }

    issues
}
