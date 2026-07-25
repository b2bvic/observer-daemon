use aho_corasick::AhoCorasick;
use std::collections::HashSet;

use super::Issue;
use crate::config::FloateConfig;

/// Validate text against FLOATE word blacklist and phrase patterns.
/// Uses Aho-Corasick for single-pass multi-pattern matching.
pub fn validate(text: &str, config: &FloateConfig) -> Vec<Issue> {
    let mut issues = Vec::new();
    let lower = text.to_lowercase();

    // Word-level: extract unique words, intersect with blacklist
    let effective = config.effective_words();
    let words: HashSet<String> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect();

    let mut word_hits: Vec<String> = words.intersection(&effective).cloned().collect();
    word_hits.sort();

    let mut remaining = config.max_deduction;
    for word in word_hits.iter().take(5) {
        if remaining == 0 {
            break;
        }
        let deduction = config.deduction_per_hit.min(remaining);
        issues.push(Issue::new("FLOATE", &format!("word:{}", word), deduction));
        remaining -= deduction;
    }

    // Phrase-level: Aho-Corasick scan
    if remaining > 0 && !config.phrases.is_empty() {
        let ac = AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(&config.phrases)
            .expect("Failed to build Aho-Corasick automaton");

        let mut phrase_hits = Vec::new();
        for mat in ac.find_iter(&lower) {
            let phrase = &config.phrases[mat.pattern().as_usize()];
            if !phrase_hits.contains(phrase) {
                phrase_hits.push(phrase.clone());
            }
        }

        for phrase in phrase_hits.iter().take(5) {
            if remaining == 0 {
                break;
            }
            let deduction = config.deduction_per_hit.min(remaining);
            issues.push(Issue::new(
                "FLOATE",
                &format!("phrase:{}", phrase),
                deduction,
            ));
            remaining -= deduction;
        }
    }

    issues
}
