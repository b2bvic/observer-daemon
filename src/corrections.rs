use crate::rubric::ContentClass;
use chrono::{Local, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::Path;

/// A single correction entry in corrections.jsonl.
#[derive(Debug, Serialize, Deserialize)]
pub struct CorrectionEntry {
    pub ts: String,
    #[serde(rename = "type")]
    pub violation_type: String,
    pub pattern: String,
    pub session: String,
    pub score: u32,
    pub source: String,
    pub context: String,
    #[serde(default)]
    pub content_class: ContentClass,
}

/// A promoted rule in patterns.json.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PromotedRule {
    pub pattern: String,
    #[serde(rename = "type")]
    pub violation_type: String,
    pub promoted_date: String,
    pub occurrence_count: u32,
    pub action: String,
    pub deduction: u32,
    #[serde(default)]
    pub content_class: ContentClass,
}

/// The patterns.json file structure.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct PromotedPatterns {
    pub promoted_rules: Vec<PromotedRule>,
    pub last_aggregation: String,
}

/// Stable key for matching a promoted rule against validation issues.
pub fn promoted_rule_key(
    content_class: ContentClass,
    violation_type: &str,
    pattern: &str,
) -> String {
    format!("{}::{}::{}", content_class, violation_type, pattern)
}

/// Append a correction entry to the JSONL file.
pub fn write_correction(path: &Path, entry: &CorrectionEntry) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let json = serde_json::to_string(entry).map_err(std::io::Error::other)?;
    writeln!(file, "{}", json)?;
    Ok(())
}

/// Return true when any just-written (type, pattern) pair has reached the
/// promotion threshold inside the aggregation window.
pub fn should_promote_any(
    corrections_path: &Path,
    keys: &std::collections::HashSet<(ContentClass, String, String)>,
    promotion_threshold: u32,
    window_days: u32,
) -> bool {
    if keys.is_empty() {
        return false;
    }

    let cutoff = Utc::now() - chrono::Duration::days(window_days as i64);
    let mut counts: HashMap<(ContentClass, String, String), u32> = HashMap::new();

    let Ok(file) = std::fs::File::open(corrections_path) else {
        return false;
    };
    let reader = std::io::BufReader::new(file);

    for line in reader.lines().map_while(Result::ok) {
        let Ok(entry) = serde_json::from_str::<CorrectionEntry>(&line) else {
            continue;
        };
        if !keys.contains(&(
            entry.content_class,
            entry.violation_type.clone(),
            entry.pattern.clone(),
        )) {
            continue;
        }

        let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&entry.ts) else {
            continue;
        };
        if ts.with_timezone(&Utc) < cutoff {
            continue;
        }

        let key = (entry.content_class, entry.violation_type, entry.pattern);
        let count = counts.entry(key).or_insert(0);
        *count += 1;
        if *count >= promotion_threshold {
            return true;
        }
    }

    false
}

/// Aggregate corrections by (type, pattern) within the given window.
/// Returns patterns that crossed the promotion threshold.
pub fn aggregate_and_promote(
    corrections_path: &Path,
    patterns_path: &Path,
    promotion_threshold: u32,
    window_days: u32,
) -> Vec<PromotedRule> {
    let cutoff = Utc::now() - chrono::Duration::days(window_days as i64);

    // Count by (content class, type, pattern).
    let mut counts: HashMap<(ContentClass, String, String), u32> = HashMap::new();
    if let Ok(file) = std::fs::File::open(corrections_path) {
        let reader = std::io::BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            let Ok(entry) = serde_json::from_str::<CorrectionEntry>(&line) else {
                continue;
            };
            let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&entry.ts) else {
                continue;
            };
            if ts.with_timezone(&Utc) < cutoff {
                continue;
            }

            let key = (entry.content_class, entry.violation_type, entry.pattern);
            *counts.entry(key).or_insert(0) += 1;
        }
    }

    // Load existing promoted patterns
    let mut promoted = load_patterns(patterns_path);
    let existing: std::collections::HashSet<String> = promoted
        .promoted_rules
        .iter()
        .map(|r| promoted_rule_key(r.content_class, &r.violation_type, &r.pattern))
        .collect();

    // Find new promotions
    let mut new_promotions = Vec::new();
    let today = Local::now().format("%Y-%m-%d").to_string();

    for ((content_class, vtype, pattern), count) in &counts {
        let key = promoted_rule_key(*content_class, vtype, pattern);
        if *count >= promotion_threshold && !existing.contains(&key) {
            let rule = PromotedRule {
                pattern: pattern.clone(),
                violation_type: vtype.clone(),
                promoted_date: today.clone(),
                occurrence_count: *count,
                action: "reject".to_string(),
                deduction: 5,
                content_class: *content_class,
            };
            new_promotions.push(rule.clone());
            promoted.promoted_rules.push(rule);
        }
    }

    if !new_promotions.is_empty() {
        promoted.last_aggregation = Local::now().to_rfc3339();
        save_patterns(patterns_path, &promoted);
    }

    new_promotions
}

/// Load patterns.json.
pub fn load_patterns(path: &Path) -> PromotedPatterns {
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => PromotedPatterns::default(),
    }
}

/// Save patterns.json.
fn save_patterns(path: &Path, patterns: &PromotedPatterns) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(patterns) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::{CorrectionEntry, PromotedRule};
    use crate::rubric::ContentClass;

    #[test]
    fn classless_legacy_correction_deserializes_as_generic() {
        let entry: CorrectionEntry = serde_json::from_str(
            r#"{"ts":"2026-01-01T00:00:00Z","type":"tone","pattern":"x","session":"s","score":70,"source":"test","context":"fixture"}"#,
        )
        .expect("legacy correction");
        assert_eq!(entry.content_class, ContentClass::Generic);
    }

    #[test]
    fn classless_legacy_promotion_deserializes_as_generic() {
        let rule: PromotedRule = serde_json::from_str(
            r#"{"pattern":"x","type":"tone","promoted_date":"2026-01-01","occurrence_count":3,"action":"reject","deduction":5}"#,
        )
        .expect("legacy promotion");
        assert_eq!(rule.content_class, ContentClass::Generic);
    }
}
