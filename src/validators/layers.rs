use super::Issue;
use crate::config::LayersConfig;
use crate::rubric::ContentClass;

/// Check configured layer-alignment rules.
///
/// Architect mode detects when an assistant positions itself as the designer.
///
/// Adversarial mode detects when legal responses lack intent modeling.
pub fn validate(
    text: &str,
    prompt: &str,
    content_class: ContentClass,
    config: &LayersConfig,
) -> Vec<Issue> {
    let mut issues = Vec::new();
    let prompt_lower = prompt.to_lowercase();
    let response_lower = text.to_lowercase();

    // Architect mode: check for assistant-as-architect positioning.
    let is_building = content_class == ContentClass::TechnicalBuild
        && config
            .architect
            .prompt_signals
            .iter()
            .any(|s| prompt_lower.contains(s));

    if is_building {
        for tell in &config.architect.positioning_tells {
            if response_lower.contains(tell.as_str()) {
                issues.push(Issue::new(
                    "layer2",
                    &config.architect.issue_template.replace("{tell}", tell),
                    config.deduction_per_issue,
                ));
                break;
            }
        }
    }

    // Adversarial mode: check for missing intent modeling.
    let is_adversarial = content_class == ContentClass::Legal
        && config
            .adversarial
            .prompt_signals
            .iter()
            .any(|s| prompt_lower.contains(s));

    if is_adversarial {
        let has_adversarial = config
            .adversarial
            .required_markers
            .iter()
            .any(|m| response_lower.contains(m.as_str()));

        if !has_adversarial {
            issues.push(Issue::new(
                "layer1",
                &config.adversarial.missing_modeling_issue,
                config.deduction_per_issue,
            ));
        }
    }

    issues
}
