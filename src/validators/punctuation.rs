use regex::Regex;

use super::Issue;
use crate::config::PunctuationConfigs;

/// Check for em dash overuse and horizontal rule dividers.
pub fn validate(text: &str, config: &PunctuationConfigs) -> Vec<Issue> {
    let mut issues = Vec::new();

    // Em dash count vs threshold
    let em_dash_count = text.matches('—').count();
    let word_count = text.split_whitespace().count().max(1);
    let per_500 = (em_dash_count as f64) / (word_count as f64 / 500.0);

    if per_500 > config.em_dash.max_per_500_words as f64 && em_dash_count > 1 {
        issues.push(Issue::new(
            "em_dash",
            &format!(
                "{} em dashes ({:.1} per 500 words, max {})",
                em_dash_count, per_500, config.em_dash.max_per_500_words
            ),
            config.em_dash.deduction,
        ));
    }

    // Horizontal rule: --- on its own line, but NOT in first 10 lines (frontmatter)
    let hr_re = Regex::new(r"(?m)^---$").expect("horizontal rule regex");
    for mat in hr_re.find_iter(text) {
        // Count which line number this match is on
        let line_num = text[..mat.start()].matches('\n').count() + 1;
        if line_num > 10 {
            issues.push(Issue::new(
                "horizontal_rule",
                &format!(
                    "--- divider on line {} (only valid in frontmatter)",
                    line_num
                ),
                config.horizontal_rule.deduction,
            ));
            break; // One violation is enough
        }
    }

    issues
}
