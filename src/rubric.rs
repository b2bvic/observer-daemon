use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Validation rubric taxonomy. The declaration order is not the priority order;
/// `classify` applies the explicit contract priority.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentClass {
    Legal,
    CareerApplication,
    ContentCopy,
    TechnicalBuild,
    Conversational,
    #[default]
    Generic,
}

impl ContentClass {
    pub const PRIORITY: [Self; 5] = [
        Self::Legal,
        Self::CareerApplication,
        Self::TechnicalBuild,
        Self::ContentCopy,
        Self::Conversational,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Legal => "legal",
            Self::CareerApplication => "career_application",
            Self::ContentCopy => "content_copy",
            Self::TechnicalBuild => "technical_build",
            Self::Conversational => "conversational",
            Self::Generic => "generic",
        }
    }
}

impl fmt::Display for ContentClass {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ContentClass {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "legal" => Ok(Self::Legal),
            "career_application" | "career" => Ok(Self::CareerApplication),
            "content_copy" | "content" | "copy" => Ok(Self::ContentCopy),
            "technical_build" | "technical" | "build" => Ok(Self::TechnicalBuild),
            "conversational" | "conversation" => Ok(Self::Conversational),
            "generic" | "" => Ok(Self::Generic),
            other => Err(format!("unknown content class: {other}")),
        }
    }
}

/// Explicit caller metadata. A class is a signal, not an override: conflicting
/// prompt and metadata matches still resolve through the fixed priority.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassificationMetadata {
    pub content_class: Option<ContentClass>,
    pub labels: Vec<String>,
}

const LEGAL_SIGNALS: &[&str] = &[
    "legal",
    "lawyer",
    "attorney",
    "court",
    "lawsuit",
    "litigation",
    "contract dispute",
    "opposing counsel",
    "liability",
    "settlement",
    "lease dispute",
];
const CAREER_SIGNALS: &[&str] = &[
    "career",
    "job application",
    "apply for",
    "resume",
    "résumé",
    "cover letter",
    "interview",
    "recruiter",
    "hiring manager",
    "job search",
    "candidate",
];
const TECHNICAL_SIGNALS: &[&str] = &[
    "build",
    "implement",
    "code",
    "debug",
    "bug",
    "deploy",
    "api",
    "rust",
    "typescript",
    "python",
    "architecture",
    "test suite",
    "database",
];
const CONTENT_SIGNALS: &[&str] = &[
    "content copy",
    "copywriting",
    "headline",
    "landing page",
    "article",
    "blog post",
    "social post",
    "email campaign",
    "content brief",
    "sales page",
];
const CONVERSATIONAL_SIGNALS: &[&str] = &[
    "let's talk",
    "lets talk",
    "chat with me",
    "how are you",
    "talk this through",
    "brainstorm with me",
];

fn signals_for(content_class: ContentClass) -> &'static [&'static str] {
    match content_class {
        ContentClass::Legal => LEGAL_SIGNALS,
        ContentClass::CareerApplication => CAREER_SIGNALS,
        ContentClass::TechnicalBuild => TECHNICAL_SIGNALS,
        ContentClass::ContentCopy => CONTENT_SIGNALS,
        ContentClass::Conversational => CONVERSATIONAL_SIGNALS,
        ContentClass::Generic => &[],
    }
}

/// Classify prompt text and explicit metadata with first-match priority:
/// legal > career_application > technical_build > content_copy > conversational.
pub fn classify(prompt: &str, metadata: &ClassificationMetadata) -> ContentClass {
    let mut input = prompt.to_ascii_lowercase();
    for label in &metadata.labels {
        input.push('\n');
        input.push_str(&label.to_ascii_lowercase());
    }

    for candidate in ContentClass::PRIORITY {
        let explicit_match = metadata.content_class == Some(candidate);
        let text_match = signals_for(candidate)
            .iter()
            .any(|signal| input.contains(signal));
        if explicit_match || text_match {
            return candidate;
        }
    }

    ContentClass::Generic
}

#[cfg(test)]
mod tests {
    use super::{ClassificationMetadata, ContentClass, classify};

    #[test]
    fn classifies_legal_fixture() {
        assert_eq!(
            classify(
                "Review the opposing counsel's settlement position.",
                &ClassificationMetadata::default(),
            ),
            ContentClass::Legal
        );
    }

    #[test]
    fn classifies_career_fixture() {
        assert_eq!(
            classify(
                "Help me plan a career change and prepare for an interview.",
                &ClassificationMetadata::default(),
            ),
            ContentClass::CareerApplication
        );
    }

    #[test]
    fn mixed_signal_obeys_fixed_priority() {
        assert_eq!(
            classify(
                "Implement a tool to review a contract dispute.",
                &ClassificationMetadata::default(),
            ),
            ContentClass::Legal
        );
    }

    #[test]
    fn explicit_metadata_participates_in_priority() {
        let metadata = ClassificationMetadata {
            content_class: Some(ContentClass::ContentCopy),
            labels: vec!["job application".to_string()],
        };
        assert_eq!(
            classify("Draft this.", &metadata),
            ContentClass::CareerApplication
        );
    }

    #[test]
    fn empty_prompt_falls_back_to_generic() {
        assert_eq!(
            classify("", &ClassificationMetadata::default()),
            ContentClass::Generic
        );
    }

    #[test]
    fn unknown_class_text_falls_back_to_generic() {
        assert_eq!(
            classify(
                "Sort these colored stones by size.",
                &ClassificationMetadata::default(),
            ),
            ContentClass::Generic
        );
    }
}
