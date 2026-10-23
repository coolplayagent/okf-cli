use okf_core::{reserved, sha256, validate_reserved, Document};
use serde_json::Value;
use std::{fs, path::Path};

#[test]
fn pinned_upstream_documents_preserve_original_bytes_and_remain_consumable() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/upstream");
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let mut concepts = 0;
    for source in manifest.as_array().unwrap() {
        for file in source["files"].as_array().unwrap() {
            let path = file["path"].as_str().unwrap();
            let bytes = fs::read(root.join(path)).unwrap();
            assert_eq!(sha256(&bytes), file["sha256"], "fixture changed: {path}");
            if !path.contains("/bundle/") {
                continue;
            }
            let text = std::str::from_utf8(&bytes).unwrap();
            let logical = path.split_once("/bundle/").unwrap().1;
            if reserved(logical) {
                validate_reserved(logical, text).unwrap_or_else(|e| panic!("{path}: {e:?}"));
            } else {
                let doc = Document::parse(text).unwrap_or_else(|e| panic!("{path}: {e:?}"));
                assert!(text.is_char_boundary(doc.body_start));
                concepts += 1;
            }
        }
    }
    assert!(concepts >= 20, "upstream corpus unexpectedly small");
}
