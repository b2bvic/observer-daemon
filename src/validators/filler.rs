use super::Issue;
use crate::config::FillerConfig;

/// Check for Observer Protocol filler phrases.
pub fn validate(text: &str, config: &FillerConfig) -> Vec<Issue> {
    let lower = text.to_lowercase();
    let mut issues = Vec::new();
    let mut total_deduction = 0u32;

    for phrase in &config.phrases {
        if lower.contains(phrase.as_str()) && total_deduction < config.max_deduction {
            let deduction = config
                .deduction_per_phrase
                .min(config.max_deduction - total_deduction);
            issues.push(Issue::new("filler", &format!("'{}'", phrase), deduction));
            total_deduction += deduction;
        }
    }

    issues
}
