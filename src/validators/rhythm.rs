use regex::Regex;

use super::Issue;
use crate::config::RhythmConfigs;

/// Validate text for rhythm pattern violations:
/// - Bullet-then-explanation (3+ matches)
/// - Parallel construction (3+ identical sentence structures)
/// - Triple beat (X, Y, and Z)
pub fn validate(text: &str, config: &RhythmConfigs) -> Vec<Issue> {
    let mut issues = Vec::new();

    // Bullet-then-explanation
    let bullet_re = if config.bullet_explain.multiline {
        Regex::new(&format!("(?m){}", config.bullet_explain.pattern))
    } else {
        Regex::new(&config.bullet_explain.pattern)
    }
    .expect("bullet_explain regex");

    let bullet_count = bullet_re.find_iter(text).count();
    if bullet_count >= config.bullet_explain.threshold as usize {
        issues.push(Issue::new(
            "bullet_rhythm",
            &format!("{} bullet-then-explanation patterns", bullet_count),
            config.bullet_explain.deduction,
        ));
    }

    // Parallel construction
    let parallel_re =
        Regex::new(&config.parallel_construction.pattern).expect("parallel_construction regex");
    let parallel_count = parallel_re.find_iter(text).count();
    if parallel_count >= config.parallel_construction.threshold as usize {
        issues.push(Issue::new(
            "parallel_construction",
            &format!("{} parallel sentence groups", parallel_count),
            config.parallel_construction.deduction,
        ));
    }

    // Triple beat
    let triple_re = Regex::new(&config.triple_beat.pattern).expect("triple_beat regex");
    let triple_count = triple_re.find_iter(text).count();
    if triple_count >= config.triple_beat.threshold as usize {
        issues.push(Issue::new(
            "triple_beat",
            &format!("{} triple-beat patterns", triple_count),
            config.triple_beat.deduction,
        ));
    }

    issues
}
