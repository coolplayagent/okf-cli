use chrono::{DateTime, Utc};
use serde_json::Value;

/// Exact metadata filters; OR within a field, AND across fields.
#[derive(Debug, Default)]
pub struct SearchFilter {
    pub types: Vec<String>,
    pub tags: Vec<String>,
    pub statuses: Vec<String>,
    pub trusts: Vec<String>,
    pub expiries: Vec<String>,
    pub as_of: Option<DateTime<Utc>>,
}
impl SearchFilter {
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
