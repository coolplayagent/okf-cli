use chrono::{DateTime, Utc};
use okf_core::{validate_reserved, Document, Severity};
use serde_json::{json, Value};

#[test]
fn pinned_specification_metadata_cases() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/metadata-cases.json")).unwrap();
    let now = DateTime::parse_from_rfc3339("2026-10-22T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    for case in fixture["cases"].as_array().unwrap() {
        let raw = format!(
            "---\n{}---\noriginal body\n",
            case["yaml"].as_str().unwrap()
        );
        let doc = Document::parse(&raw).unwrap();
        let original = doc.frontmatter.clone();
        let diagnostics = doc.diagnostics("@book/note.md");
        let findings: Vec<_> = diagnostics
            .iter()
            .map(|diagnostic| json!([diagnostic.code, diagnostic.field]))
            .collect();
        assert_eq!(json!(findings), case["warnings"], "{}", case["name"]);
        assert!(diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity == Severity::Warning
                && diagnostic.path.as_deref() == Some("@book/note.md")));
        let card = serde_json::to_value(doc.card("note", now)).unwrap();
        for field in ["trust", "status", "expiry"] {
            if let Some(expected) = case.get(field) {
                assert_eq!(&card[field], expected, "{}: {field}", case["name"]);
            }
        }
        assert_eq!(doc.frontmatter, original);
        assert_eq!(&raw[doc.body_start..], "original body\n");
    }
}

#[test]
fn diagnostics_are_bounded_for_large_malformed_arrays() {
    let raw = format!(
        "---\ntype: Note\ntags: [{}]\n---\n",
        vec!["1"; 10_000].join(",")
    );
    let doc = Document::parse(&raw).unwrap();
    let diagnostics = doc.diagnostics("@book/note.md");
    assert_eq!(diagnostics.len(), 65);
    assert_eq!(
        diagnostics.last().unwrap().code,
        "METADATA_DIAGNOSTICS_TRUNCATED"
    );
    assert!(diagnostics.last().unwrap().field.is_none());
}

#[test]
fn structural_errors_remain_separate_from_optional_metadata() {
    for raw in [
        "---\ntitle: Missing type\n---\n",
        "---\ntype: []\n---\n",
        "---\ntype: Note\nverified: [\n---\n",
    ] {
        assert_eq!(Document::parse(raw).unwrap_err().code, "INVALID_OKF");
    }
    assert!(validate_reserved("index.md", "---\nokf_version: '0.2'\n---\n# Index\n").is_ok());
    assert!(validate_reserved("group/index.md", "---\nokf_version: '0.2'\n---\n").is_err());
    assert!(validate_reserved("log.md", "# Log\n## 2026-10-22\n* update\n").is_ok());
    assert!(validate_reserved("log.md", "# Log\n## yesterday\n").is_err());
}

#[test]
fn wide_integers_and_json_internal_marker_keys_preserve_yaml_values() {
    let doc = Document::parse("---\ntype: Note\ncustom:\n  $serde_json::private::Number: literal metadata\n  other: true\nlarge: 340282366920938463463374607431768211455\nnegative: -170141183460469231731687303715884105728\n---\noriginal\n").unwrap();
    assert_eq!(
        doc.frontmatter["large"].to_string(),
        "340282366920938463463374607431768211455"
    );
    assert_eq!(
        doc.frontmatter["negative"].to_string(),
        "-170141183460469231731687303715884105728"
    );
    assert_eq!(
        doc.frontmatter["custom"]["$serde_json::private::Number"],
        "literal metadata"
    );
    assert_eq!(doc.frontmatter["custom"]["other"], true);
    assert!(doc.diagnostics("@book/note.md").is_empty());
}
