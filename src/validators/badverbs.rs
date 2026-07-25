use regex::Regex;
use std::collections::HashSet;

use super::Issue;
use crate::config::BadverbsConfig;

/// Validate text for banned -ly adverbs.
/// Counts per ~500 words. Over threshold = flat deduction. Under = per-word.
pub fn validate(text: &str, config: &BadverbsConfig) -> Vec<Issue> {
    let mut issues = Vec::new();
    let badverb_set: HashSet<&str> = config.words.iter().map(|w| w.as_str()).collect();

    let re = Regex::new(r"\b\w+ly\b").expect("badverb regex");
    let hits: Vec<String> = re
        .find_iter(&text.to_lowercase())
        .map(|m| m.as_str().to_string())
        .filter(|w| badverb_set.contains(w.as_str()))
        .collect();

    if hits.is_empty() {
        return issues;
    }

    // Approximate word count (split on whitespace)
    let word_count = text.split_whitespace().count().max(1);
    let per_500 = (hits.len() as f64) / (word_count as f64 / 500.0);

    if per_500 > config.threshold_per_500_words as f64 {
        issues.push(Issue::new(
            "badverbs",
            &format!(
                "{} found ({:.1} per 500 words, threshold {})",
                hits.len(),
                per_500,
                config.threshold_per_500_words
            ),
            config.deduction_over_threshold,
        ));
    } else {
        for word in hits.iter().take(3) {
            issues.push(Issue::new("badverb", word, config.deduction_mild));
        }
    }

    issues
}
