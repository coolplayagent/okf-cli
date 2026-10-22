use okf_core::{
    markdown::{resolve_link, LinkTarget, Markdown, MAX_REFERENCES},
    Document, Severity,
};

#[test]
fn bundle_paths_resolve_without_os_path_semantics() {
    for (destination, expected, fragment) in [
        ("../other.md", "other.md", None),
        ("/tables/a.md#columns", "tables/a.md", Some("columns")),
        ("./a%20b.md?q=1#part", "services/a b.md", Some("part")),
        ("#same", "services/note.md", Some("same")),
        ("", "services/note.md", None),
        (".././nested/../other.md", "other.md", None),
        ("/%E4%B8%AD%E6%96%87.md", "中文.md", None),
    ] {
        assert_eq!(
            resolve_link("services/note.md", destination).unwrap(),
            LinkTarget::Internal {
                path: expected.into(),
                fragment: fragment.map(str::to_owned)
            }
        );
    }
    for destination in [
        "https://example.test/a.md",
        "mailto:user@example.test",
        "//host/a.md",
        "urn:example:note",
    ] {
        assert_eq!(
            resolve_link("note.md", destination).unwrap(),
            LinkTarget::External
        );
    }
    for destination in [
        "../../outside.md",
        "%2e%2e/%2e%2e/a.md",
        "/../a.md",
        "a%5Cb.md",
        "%00.md",
        "%FF.md",
    ] {
        assert!(
            resolve_link("services/note.md", destination).is_err(),
            "{destination}"
        );
    }
}

#[test]
fn markdown_parser_handles_references_and_excludes_code_html_images_and_escapes() {
    let raw = concat!(
        "\u{feff}---\r\ntype: Note\r\nsources:\r\n  - id: source\r\n    resource: scope\r\n---\r\n",
        "中文 [inline](../a.md#part) [reference][ref] [shortcut]\r\n",
        "\r\n[ref]: /b.md \"title\"\r\n[shortcut]: <a b.md>\r\n\r\n",
        "Claim.[^source] Missing.[^absent] Undefined.[^dangling]\r\n\r\n",
        "[^source]: A source\r\n\r\n[^absent]: An absent source\r\n\r\n",
        "`[code](missing.md) [^inline]`\r\n\\[^escaped] ![image](missing.md)\r\n\r\n",
        "~~~rust\r\n[code](missing.md) [^fenced]\r\n~~~\r\n\r\n",
        "    [code](missing.md) [^indented]\r\n\r\n",
        "<div>\r\n[html](missing.md) [^html]\r\n</div>\r\n"
    );
    let doc = Document::parse(raw).unwrap();
    let parsed = Markdown::parse(
        raw,
        doc.body_start,
        "nested/note.md",
        Some(&doc.frontmatter),
    );
    assert_eq!(parsed.links.len(), 3, "{parsed:?}");
    assert_eq!(
        parsed.links[2].target,
        LinkTarget::Internal {
            path: "nested/a b.md".into(),
            fragment: None
        }
    );
    assert_eq!(
        parsed
            .diagnostics
            .iter()
            .map(|d| d.code.as_str())
            .collect::<Vec<_>>(),
        [
            "UNRESOLVED_SOURCE",
            "UNRESOLVED_SOURCE",
            "UNDEFINED_FOOTNOTE"
        ]
    );
    for diagnostic in &parsed.diagnostics {
        assert_eq!(diagnostic.severity, Severity::Warning);
        let span = diagnostic.location.as_ref().unwrap();
        assert!(matches!(
            &raw[span.byte_start..span.byte_end],
            "[^absent]" | "[^dangling]"
        ));
    }
    assert!(!parsed.truncated);
}

#[test]
fn provenance_join_uses_ids_not_positions_and_does_not_require_all_sources_to_be_cited() {
    for sources in [
        "[{id: a, resource: first}, {id: b, resource: second}]",
        "[{id: b, resource: second}, {id: a, resource: first}]",
    ] {
        let raw = format!("---\ntype: Note\nsources: {sources}\n---\nA.[^a]\n\n[^a]: Source A\n");
        let doc = Document::parse(&raw).unwrap();
        assert!(
            Markdown::parse(&raw, doc.body_start, "note.md", Some(&doc.frontmatter))
                .diagnostics
                .is_empty()
        );
    }
}

#[test]
fn markdown_definition_matching_does_not_change_the_source_join_key() {
    let raw = "---\ntype: Note\nsources: [{id: SOURCE, resource: scope}]\n---\nClaim.[^SOURCE]\n\n[^source]: Source prose\n";
    let doc = Document::parse(raw).unwrap();
    let parsed = Markdown::parse(raw, doc.body_start, "note.md", Some(&doc.frontmatter));
    assert!(parsed.diagnostics.is_empty(), "{parsed:?}");
}

#[test]
fn reference_and_diagnostic_limits_are_bounded() {
    let raw = format!(
        "---\ntype: Note\n---\n{}",
        "[missing](../outside.md) [^unknown]\n\n".repeat(MAX_REFERENCES + 10)
    );
    let doc = Document::parse(&raw).unwrap();
    let parsed = Markdown::parse(&raw, doc.body_start, "note.md", Some(&doc.frontmatter));
    assert!(parsed.truncated);
    assert_eq!(parsed.diagnostics.len(), 65);
    assert_eq!(
        parsed.diagnostics.last().unwrap().code,
        "BODY_DIAGNOSTICS_TRUNCATED"
    );
    let links = "[link](target.md) ".repeat(MAX_REFERENCES + 1);
    let parsed = Markdown::parse(&links, 0, "note.md", None);
    assert_eq!(parsed.links.len(), MAX_REFERENCES);
    assert!(parsed.truncated);
}

#[test]
fn encoded_directory_navigation_is_checked_as_a_bundle_target() {
    for destination in ["gone%2F", "gone%2F?q=1#part", "gone/."] {
        let raw = format!("[target]({destination})");
        let parsed = Markdown::parse(&raw, 0, "note.md", None);
        assert_eq!(parsed.links[0].bundle_path(), Some("gone"), "{destination}");
    }
}

#[test]
fn mixed_markdown_preserves_utf8_ranges_and_canonical_bundle_paths() {
    let pieces = [
        "[", "[^", "]", "(", ")", "\\", "`", "```", "~~~", "\r\n", "💡", "中文", "a.md", "%2e",
        "../", "#", "?", "<div>", "</div>", "    ", "*", "a", "!", "\0",
    ];
    let mut state = 0x6f6b665fu64;
    for case in 0..256 {
        let mut raw = String::from("\u{feff}---\r\ntype: Note\r\n---\r\n");
        for _ in 0..256 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            raw.push_str(pieces[state as usize % pieces.len()]);
        }
        let doc = Document::parse(&raw).unwrap();
        let parsed = Markdown::parse(
            &raw,
            doc.body_start,
            "nested/note.md",
            Some(&doc.frontmatter),
        );
        assert!(parsed.links.len() <= MAX_REFERENCES, "case {case}");
        assert!(parsed.diagnostics.len() <= 65, "case {case}");
        for span in parsed.links.iter().map(|link| &link.location).chain(
            parsed
                .diagnostics
                .iter()
                .filter_map(|d| d.location.as_ref()),
        ) {
            assert!(
                span.byte_start >= doc.body_start
                    && span.byte_start < span.byte_end
                    && span.byte_end <= raw.len(),
                "case {case}: {span:?}"
            );
            assert!(
                raw.is_char_boundary(span.byte_start) && raw.is_char_boundary(span.byte_end),
                "case {case}: {span:?}"
            );
        }
        for link in &parsed.links {
            if let LinkTarget::Internal { path, .. } = &link.target {
                assert!(
                    okf_core::logical_path(path).is_ok(),
                    "case {case}: {path:?}"
                );
            }
        }
    }
}
