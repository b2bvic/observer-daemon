use regex::Regex;

use super::Issue;
use crate::config::RhetoricalCrutchConfig;

/// Check for rhetorical crutch patterns: "isn't just X, it's Y" and variants.
/// Active violation per rep-state.json.
pub fn validate(text: &str, config: &RhetoricalCrutchConfig) -> Vec<Issue> {
    let lower = text.to_lowercase();
    let mut issues = Vec::new();

    for pattern in &config.patterns {
        let re = match Regex::new(&format!("(?i){}", pattern)) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if re.is_match(&lower) {
            issues.push(Issue::new(
                "rhetorical_crutch",
                &format!("matched: {}", pattern),
                config.deduction,
            ));
            break; // One match is sufficient
        }
    }

    issues
}
