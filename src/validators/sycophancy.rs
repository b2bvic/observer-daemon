use regex::Regex;

use super::Issue;
use crate::config::SycophancyConfig;

/// Check if response opens with a sycophantic pattern.
///
/// Excludes correction acknowledgments: "You're right — correcting" is
/// Observer Protocol CORRECT stance, not sycophancy. The distinction:
/// sycophancy AGREES to please. Correction acknowledgment ACCEPTS to redirect.
pub fn validate(text: &str, config: &SycophancyConfig) -> Vec<Issue> {
    let first_line = text
        .trim()
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();

    if first_line.is_empty() {
        return Vec::new();
    }

    // Check against opener patterns
    let matched = config.opener_patterns.iter().any(|p| {
        Regex::new(&format!("(?i){}", p))
            .map(|re| re.is_match(&first_line))
            .unwrap_or(false)
    });

    if !matched {
        return Vec::new();
    }

    // Check correction exemptions: if first line contains correction signals,
    // this is a correction acknowledgment, not sycophancy
    let is_correction = config
        .correction_exemptions
        .iter()
        .any(|sig| first_line.contains(sig));

    // Special case: "you're right" followed by dash + action
    if first_line.starts_with("you're right") || first_line.starts_with("you're correct") {
        let rest = if first_line.starts_with("you're right") {
            &first_line[12..]
        } else {
            &first_line[14..]
        };

        if rest.contains('—') || rest.contains(" - ") {
            let after_dash = if rest.contains('—') {
                rest.split('—').last().unwrap_or("")
            } else {
                rest.split(" - ").last().unwrap_or("")
            };
            if config
                .correction_exemptions
                .iter()
                .any(|sig| after_dash.contains(sig))
            {
                return Vec::new();
            }
        }
    }

    if is_correction {
        return Vec::new();
    }

    vec![Issue::new(
        "sycophancy",
        "response opens with agreement/praise",
        config.deduction,
    )]
}
