use regex::Regex;

use super::Issue;
use crate::config::TellsConfig;

fn clamp_char_boundary(s: &str, idx: usize) -> usize {
    let mut idx = idx.min(s.len());
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Check for AI opening and closing tell patterns.
pub fn validate(text: &str, config: &TellsConfig) -> Vec<Issue> {
    let mut issues = Vec::new();

    // Opening tells: check first line/paragraph
    let first_line = text.trim().lines().next().unwrap_or("");
    for pattern in &config.opening.patterns {
        let re = match Regex::new(&format!("(?i){}", pattern)) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if re.is_match(first_line) {
            issues.push(Issue::new(
                "opening_tell",
                &format!("AI opening pattern: {}", pattern),
                config.opening.deduction,
            ));
            break;
        }
    }

    // Closing tells: check last ~200 chars
    let tail_size = config
        .closing
        .scan_region
        .as_deref()
        .and_then(|s| s.strip_prefix("tail_"))
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(200);

    // Char-safe tail slice — avoid splitting multi-byte UTF-8 chars (em dashes, etc.)
    let tail = if text.len() > tail_size {
        let byte_start = text.len() - tail_size;
        let safe_start = clamp_char_boundary(text, byte_start);
        &text[safe_start..]
    } else {
        text
    };

    for pattern in &config.closing.patterns {
        let re = match Regex::new(&format!("(?i){}", pattern)) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if re.is_match(tail) {
            issues.push(Issue::new(
                "closing_tell",
                &format!("AI closing pattern: {}", pattern),
                config.closing.deduction,
            ));
            break;
        }
    }

    issues
}
