use chrono::{Local, Timelike};

use super::Issue;
use crate::config::ContextTimeConfig;

/// Check if response structure matches Victor's physical context.
///
/// Commute (6-8am, 5-7pm ET): actionable answer must be in first paragraph.
/// Office (8am-5pm ET): responses should be concise (under max chars).
pub fn validate(text: &str, config: &ContextTimeConfig) -> Vec<Issue> {
    let mut issues = Vec::new();

    // Use local time (system should be ET)
    let hour = Local::now().hour();

    let paragraphs: Vec<&str> = text
        .split("\n\n")
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();

    // Commute check
    let is_commute = config
        .commute_hours
        .iter()
        .any(|range| range.len() == 2 && hour >= range[0] && hour < range[1]);

    if is_commute && paragraphs.len() > config.commute_max_paragraphs_before_action + 2 {
        let first_para = paragraphs.first().unwrap_or(&"").to_lowercase();
        let action_words = [
            "don't", "do", "file", "wait", "hold", "send", "ignore", "respond", "stop", "start",
            "call", "yes", "no", "skip", "proceed",
        ];
        let has_action = action_words.iter().any(|w| first_para.contains(w));
        if !has_action {
            issues.push(Issue::new(
                "context_time",
                "commute: actionable answer buried past first paragraph",
                config.deduction_per_issue,
            ));
        }
    }

    // Office check
    let is_office = config.office_hours.len() == 2
        && hour >= config.office_hours[0]
        && hour < config.office_hours[1];

    if is_office && text.len() > config.office_max_chars {
        issues.push(Issue::new(
            "context_time",
            &format!(
                "office_hours: response {} chars (max {})",
                text.len(),
                config.office_max_chars
            ),
            config.deduction_per_issue,
        ));
    }

    issues
}
