//! Derived candidate data never replaces original document matching.
use crate::{RequestControl, SearchFilter, Snapshot};
use chrono::{DateTime, Utc};
use okf_core::{Document, Result};
use std::collections::BTreeSet;

pub const MAX_DOCUMENT_GRAMS: usize = 4096;
pub const MAX_POSTINGS: usize = 1_000_000;

pub struct CandidateRequest<'a> {
    pub generation: &'a str,
    pub terms: &'a [String],
    pub filter: &'a SearchFilter,
    pub evaluated_at: DateTime<Utc>,
    pub body_limit: usize,
    pub row_limit: usize,
    pub control: &'a RequestControl,
}

pub struct CandidateSelection {
    pub snapshot: Snapshot,
    pub complete: bool,
    pub rows_read: usize,
}

pub fn trigrams(text: &str) -> BTreeSet<String> {
    let chars: Vec<_> = text.chars().collect();
    chars.windows(3).map(|w| w.iter().collect()).collect()
}

/// None means this document must always be considered as a fallback candidate.
/// All possible expiry values preserve time-dependent card matching.
pub fn candidate_data(path: &str, raw: &str) -> Result<(String, Option<BTreeSet<String>>)> {
    let doc = Document::parse(raw)?;
    let metadata = serde_json::to_string(&doc.frontmatter).map_err(okf_core::Error::io)?;
    let id = path.strip_suffix(".md").unwrap_or(path);
    let mut card = doc.card(id, DateTime::UNIX_EPOCH);
    let mut grams = BTreeSet::new();
    let mut texts = vec![raw[doc.body_start..].to_lowercase()];
    for expiry in ["current", "expired", "unspecified", "unknown"] {
        card.expiry = expiry.into();
        texts.push(
            serde_json::to_string(&card)
                .map_err(okf_core::Error::io)?
                .to_lowercase(),
        );
    }
    for text in texts {
        let chars: Vec<_> = text.chars().collect();
        for window in chars.windows(3) {
            grams.insert(window.iter().collect::<String>());
            if grams.len() > MAX_DOCUMENT_GRAMS {
                return Ok((metadata, None));
            }
        }
    }
    Ok((metadata, Some(grams)))
}
