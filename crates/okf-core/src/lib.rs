//! Pure OKF parsing, identities and disclosure primitives. No filesystem or clock I/O.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize)]
pub struct Error {
    pub code: String,
    pub message: String,
    pub exit_code: u8,
}

impl Error {
    pub fn new(code: &str, message: impl Into<String>, exit_code: u8) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            exit_code,
        }
    }
    pub fn io(error: impl fmt::Display) -> Self {
        Self::new("IO_ERROR", error.to_string(), 5)
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub path: Option<String>,
}
impl Diagnostic {
    pub fn new(code: &str, message: impl Into<String>, path: Option<&str>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            path: path.map(str::to_owned),
        }
    }
}

/// Validate a logical bundle path before any OS-specific interpretation.
pub fn logical_path(path: &str) -> Result<String> {
    if path.starts_with('/')
        || path.chars().any(|c| matches!(c, '\\' | ':' | '\0'))
        || path.split('/').any(|p| p == ".." || p == ".")
        || (!path.is_empty() && path.split('/').any(str::is_empty))
    {
        return Err(Error::new(
            "INVALID_PATH",
            "expected a bundle-relative path without traversal",
            2,
        ));
    }
    Ok(path.to_owned())
}

pub fn reserved(path: &str) -> bool {
    matches!(path.rsplit('/').next(), Some("index.md" | "log.md"))
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The original UTF-8 text is kept separately. Offsets always address that text.
#[derive(Debug, Clone)]
pub struct Document {
    pub frontmatter: Map<String, Value>,
    pub body_start: usize,
}

pub fn frontmatter(raw: &str) -> Result<(Map<String, Value>, usize)> {
    let bom = raw.strip_prefix('\u{feff}').map_or(0, |_| 3);
    let mut lines = raw[bom..].split_inclusive('\n');
    let first = lines.next().unwrap_or("");
    if first.trim_end() != "---" {
        return Err(Error::new("INVALID_OKF", "missing YAML frontmatter", 1));
    }
    let start = bom + first.len();
    let mut offset = start;
    for line in lines {
        if line.trim_end() == "---" {
            let value: Value = serde_yaml_ng::from_str(&raw[start..offset])
                .map_err(|e| Error::new("INVALID_OKF", format!("invalid YAML: {e}"), 1))?;
            let map = value
                .as_object()
                .cloned()
                .ok_or_else(|| Error::new("INVALID_OKF", "frontmatter must be a mapping", 1))?;
            return Ok((map, offset + line.len()));
        }
        offset += line.len();
    }
    Err(Error::new(
        "INVALID_OKF",
        "unterminated YAML frontmatter",
        1,
    ))
}

impl Document {
    pub fn parse(raw: &str) -> Result<Self> {
        let (frontmatter, body_start) = frontmatter(raw)?;
        if frontmatter
            .get("type")
            .and_then(Value::as_str)
            .is_none_or(|s| s.trim().is_empty())
        {
            return Err(Error::new(
                "INVALID_OKF",
                "type must be a non-empty string",
                1,
            ));
        }
        Ok(Self {
            frontmatter,
            body_start,
        })
    }

    pub fn card(&self, id: &str, now: DateTime<Utc>) -> Card {
        let text = |key: &str| {
            self.frontmatter
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let tags = self
            .frontmatter
            .get("tags")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let events: Vec<&Value> = match self.frontmatter.get("verified") {
            Some(Value::Array(a)) => a.iter().collect(),
            Some(v @ Value::Object(_)) => vec![v],
            _ => vec![],
        };
        let actors: Vec<&str> = events
            .iter()
            .filter_map(|v| v.get("by").and_then(Value::as_str))
            .filter(|s| !s.trim().is_empty())
            .collect();
        let trust = if actors
            .iter()
            .any(|a| a.starts_with("human:") && a.len() > 6)
        {
            "human-reviewed"
        } else if actors.is_empty() {
            "unverified"
        } else {
            "machine-confirmed"
        };
        let expiry = match self.frontmatter.get("stale_after") {
            None => "unspecified",
            Some(v) => match v
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            {
                Some(t) if now >= t.with_timezone(&Utc) => "expired",
                Some(_) => "current",
                None => "unknown",
            },
        };
        Card {
            id: id.into(),
            kind: text("type").unwrap_or_default(),
            title: text("title").unwrap_or_else(|| id.rsplit('/').next().unwrap_or(id).into()),
            description: text("description"),
            tags,
            trust: trust.into(),
            status: text("status").unwrap_or_else(|| "stable".into()),
            expiry: expiry.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub trust: String,
    pub status: String,
    pub expiry: String,
}

/// A byte budget never cuts inside a UTF-8 scalar value.
pub fn prefix_end(text: &str, start: usize, budget: usize) -> Result<usize> {
    if start > text.len() || !text.is_char_boundary(start) {
        return Err(Error::new(
            "INVALID_CURSOR",
            "offset is not a UTF-8 boundary",
            2,
        ));
    }
    let mut end = start.saturating_add(budget).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == start && start < text.len() {
        return Err(Error::new(
            "BUDGET_TOO_SMALL",
            "budget cannot hold the next Unicode character",
            2,
        ));
    }
    Ok(end)
}

/// Opaque hex-encoded JSON; callers must revalidate scope and current revision.
pub fn encode_cursor<T: Serialize>(value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value).map_err(|e| Error::new("INTERNAL", e.to_string(), 5))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn decode_cursor<T: for<'a> Deserialize<'a>>(token: &str) -> Result<T> {
    let invalid = || Error::new("INVALID_CURSOR", "invalid or oversized cursor", 2);
    if token.len() > 8192 || !token.is_ascii() || token.len() & 1 != 0 {
        return Err(invalid());
    }
    let bytes = (0..token.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&token[i..i + 2], 16).map_err(|_| invalid()))
        .collect::<Result<Vec<_>>>()?;
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_preserve_bom_crlf_and_unknown_fields() {
        let raw = "\u{feff}---\r\ntype: Service\r\ncustom: [a, b]\r\n---\r\n# 你好\r\n";
        let doc = Document::parse(raw).unwrap();
        assert_eq!(&raw[doc.body_start..], "# 你好\r\n");
        assert_eq!(doc.frontmatter["custom"][1], "b");
    }

    #[test]
    fn broken_yaml_and_missing_type_are_errors() {
        for raw in [
            "no header",
            "---\ntype: x",
            "---\ntype: [\n---\n",
            "---\ntitle: X\n---\n",
        ] {
            assert!(Document::parse(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn invalid_expiry_is_unknown_not_current() {
        let doc = Document::parse(
            "---\ntype: Note\nstale_after: 2026-10-01\nverified: {by: 'human:dana'}\n---\n",
        )
        .unwrap();
        let now = DateTime::parse_from_rfc3339("2026-10-21T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let card = doc.card("note", now);
        assert_eq!(card.expiry, "unknown");
        assert_eq!(card.trust, "human-reviewed");
    }

    #[test]
    fn paths_reject_both_platforms_traversal() {
        for path in ["../x", "/x", "C:/x", "a\\x", "a/../x", "a//x"] {
            assert!(logical_path(path).is_err());
        }
        assert!(logical_path("服务/身份 验证.md").is_ok());
    }

    #[test]
    fn byte_pages_reconstruct_unicode() {
        let text = "甲🙂乙abc";
        let mut start = 0;
        let mut pieces = String::new();
        while start < text.len() {
            let end = prefix_end(text, start, 4).unwrap();
            pieces.push_str(&text[start..end]);
            start = end;
        }
        assert_eq!(pieces, text);
        assert!(prefix_end(text, 0, 1).is_err());
    }
}
