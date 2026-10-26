use chrono::{DateTime, Utc};
use serde_json::Value;

/// Directory selection and exact metadata filters; OR within a field, AND across fields.
#[derive(Debug, Default)]
pub struct SearchFilter {
    pub candidate_budget: Option<usize>,
    pub directory: Option<String>,
    pub types: Vec<String>,
    pub tags: Vec<String>,
    pub statuses: Vec<String>,
    pub trusts: Vec<String>,
    pub expiries: Vec<String>,
    pub as_of: Option<DateTime<Utc>>,
}
impl SearchFilter {
    pub fn validate(&self) -> okf_core::Result<()> {
        if self
            .candidate_budget
            .is_some_and(|n| n > crate::MAX_POSTINGS)
        {
            return Err(okf_core::Error::new(
                "INVALID_ARGUMENT",
                "candidate budget exceeds 1000000",
                2,
            ));
        }
        if let Some(directory) = &self.directory {
            if directory.is_empty() {
                return Err(okf_core::Error::new(
                    "INVALID_PATH",
                    "omit the directory filter for the bundle root",
                    2,
                ));
            }
            okf_core::logical_path(directory)?;
        }
        Ok(())
    }

    pub fn matches(&self, card: &Value) -> bool {
        (self.types.is_empty()
            || card["type"]
                .as_str()
                .is_some_and(|v| self.types.iter().any(|t| t == v)))
            && (self.statuses.is_empty()
                || card["status"]
                    .as_str()
                    .is_some_and(|v| self.statuses.iter().any(|s| s == v)))
            && (self.tags.is_empty()
                || card["tags"].as_array().is_some_and(|values| {
                    values
                        .iter()
                        .any(|v| v.as_str().is_some_and(|v| self.tags.iter().any(|t| t == v)))
                }))
            && (self.trusts.is_empty()
                || card["trust"]
                    .as_str()
                    .is_some_and(|v| self.trusts.iter().any(|t| t == v)))
            && (self.expiries.is_empty()
                || card["expiry"]
                    .as_str()
                    .is_some_and(|v| self.expiries.iter().any(|t| t == v)))
    }
}
