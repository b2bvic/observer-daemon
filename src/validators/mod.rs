pub mod badverbs;
pub mod context_time;
pub mod crutch;
pub mod filler;
pub mod floate;
pub mod layers;
pub mod lexical;
pub mod policy_pack;
pub mod punctuation;
pub mod rhythm;
pub mod sycophancy;
pub mod tells;

/// A single validation issue with its deduction cost.
#[derive(Debug, Clone)]
pub struct Issue {
    pub category: String,
    pub detail: String,
    pub deduction: u32,
}

impl Issue {
    pub fn new(category: &str, detail: &str, deduction: u32) -> Self {
        Self {
            category: category.to_string(),
            detail: detail.to_string(),
            deduction,
        }
    }

    /// Format as "category: detail" for logs.
    pub fn display(&self) -> String {
        format!("{}: {}", self.category, self.detail)
    }
}
