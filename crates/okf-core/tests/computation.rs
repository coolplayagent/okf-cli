use okf_core::{markdown::Markdown, Document, Severity};
use serde_json::{json, Value};

#[test]
fn pinned_computation_contract_cases_preserve_fields_and_original_positions() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/computation-cases.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        for bom_crlf in [false, true] {
            let raw = format!(
                "---\n{}---\n{}",
                case["yaml"].as_str().unwrap(),
                case["body"].as_str().unwrap()
            );
            let raw = if bom_crlf {
                format!("\u{feff}{}", raw.replace('\n', "\r\n"))
            } else {
                raw
            };
            let doc = Document::parse(&raw).unwrap();
            let original = doc.frontmatter.clone();
            let metadata = doc.diagnostics("@book/note.md");
            assert_eq!(
                json!(metadata
                    .iter()
                    .map(|d| json!([d.code, d.field]))
                    .collect::<Vec<_>>()),
                case["metadata"],
                "{}",
                case["name"]
            );
            let parsed = Markdown::parse(&raw, doc.body_start, "note.md", Some(&doc.frontmatter));
            assert_eq!(
                json!(parsed
                    .diagnostics
                    .iter()
                    .map(|d| &d.code)
                    .collect::<Vec<_>>()),
                case["body_codes"],
                "{}",
                case["name"]
            );
            for diagnostic in metadata.iter().chain(&parsed.diagnostics) {
                assert_eq!(diagnostic.severity, Severity::Warning);
            }
            for diagnostic in &parsed.diagnostics {
                assert!(diagnostic.field.is_none());
                let span = diagnostic.location.as_ref().unwrap();
                assert!(
                    span.byte_start >= doc.body_start
                        && span.byte_start <= span.byte_end
                        && span.byte_end <= raw.len()
                );
                assert!(
                    raw.is_char_boundary(span.byte_start) && raw.is_char_boundary(span.byte_end)
                );
            }
            assert_eq!(doc.frontmatter, original);
        }
    }
}

#[test]
fn computation_checks_share_metadata_and_body_warning_limits() {
    let raw = format!(
        "---\ntype: Attested Computation\nruntime: custom\nparameters: [{}]\n---\n[^unknown]\n",
        vec!["{}"; 100].join(",")
    );
    let doc = Document::parse(&raw).unwrap();
    let metadata = doc.diagnostics("@book/note.md");
    assert_eq!(metadata.len(), 65);
    assert_eq!(
        metadata.last().unwrap().code,
        "METADATA_DIAGNOSTICS_TRUNCATED"
    );
    let raw = format!(
        "---\ntype: Attested Computation\nruntime: custom\n---\n{}",
        "[^unknown] ".repeat(100)
    );
    let doc = Document::parse(&raw).unwrap();
    let parsed = Markdown::parse(&raw, doc.body_start, "note.md", Some(&doc.frontmatter));
    assert_eq!(parsed.diagnostics.len(), 65);
    assert_eq!(
        parsed.diagnostics.last().unwrap().code,
        "BODY_DIAGNOSTICS_TRUNCATED"
    );
}
