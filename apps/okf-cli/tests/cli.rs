use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Fixture {
    work: TempDir,
    home: TempDir,
}
impl Fixture {
    fn new() -> Self {
        let f = Self {
            work: tempfile::tempdir().unwrap(),
            home: tempfile::tempdir().unwrap(),
        };
        fs::create_dir_all(f.work.path().join("knowledge/services")).unwrap();
        fs::write(
            f.work.path().join("knowledge/index.md"),
            "# Handbook\n* [Services](services/)\n",
        )
        .unwrap();
        fs::write(f.work.path().join("knowledge/services/auth.md"),
            "---\ntype: Service\ntitle: 登录服务\ndescription: rate limit and retry\nstale_after: 2000-01-01T00:00:00Z\n---\n# 登录\n每秒最多十次请求。🙂\n").unwrap();
        fs::write(
            f.work.path().join("knowledge/services/billing.md"),
            "---\ntype: Service\ntitle: Billing\n---\nPayment handling.\n",
        )
        .unwrap();
        f
    }
    fn run(&self, args: &[&str]) -> Output {
        self.run_in(self.work.path(), args)
    }
    fn run_in(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_okf"))
            .current_dir(cwd)
            .env("OKF_HOME", self.home.path())
            .args(["--format", "json"])
            .args(args)
            .output()
            .unwrap()
    }
    fn bind(&self) {
        let out = self.run(&[
            "bundle",
            "add",
            "book",
            "--kind",
            "fs",
            "--path",
            "knowledge",
        ]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}
fn value(output: &Output) -> Value {
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn local_progressive_read_and_subdirectory_resolution() {
    let f = Fixture::new();
    f.bind();
    let overview = value(&f.run(&["context"]));
    assert_eq!(overview["data"]["items"][0]["name"], "book");
    assert!(!overview.to_string().contains("每秒"));
    let root = value(&f.run(&["browse", "@book"]));
    assert_eq!(root["data"]["index"]["origin"], "authored");
    assert_eq!(root["data"]["items"][0]["kind"], "directory");
    let listing = value(&f.run_in(
        &f.work.path().join("knowledge/services"),
        &["browse", "@book/services"],
    ));
    assert_eq!(listing["data"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(listing["data"]["items"][0]["expiry"], "expired");
    let doc = value(&f.run(&["read", "@book/services/auth"]));
    let raw = fs::read_to_string(f.work.path().join("knowledge/services/auth.md")).unwrap();
    assert_eq!(doc["data"]["document"]["text"], raw);
    assert_eq!(doc["data"]["document"]["byte_end"], raw.len());
    assert_eq!(value(&f.run(&["validate", "book"]))["data"]["failures"], 0);
}

#[test]
fn unicode_read_pages_reconstruct_original_and_reject_changed_document() {
    let f = Fixture::new();
    f.bind();
    let mut cursor: Option<String> = None;
    let mut combined = String::new();
    loop {
        let mut args = vec!["read", "@book/services/auth", "--budget-bytes", "17"];
        if let Some(ref cursor) = cursor {
            args.extend(["--cursor", cursor]);
        }
        let out = f.run(&args);
        assert_eq!(out.status.code(), Some(0));
        let page = value(&out);
        combined.push_str(page["data"]["document"]["text"].as_str().unwrap());
        cursor = page["page"]["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        combined,
        fs::read_to_string(f.work.path().join("knowledge/services/auth.md")).unwrap()
    );
    let page = value(&f.run(&["read", "@book/services/auth", "--budget-bytes", "17"]));
    let cursor = page["page"]["next_cursor"].as_str().unwrap();
    fs::write(
        f.work.path().join("knowledge/services/auth.md"),
        "---\ntype: Note\n---\nchanged\n",
    )
    .unwrap();
    let out = f.run(&["read", "@book/services/auth", "--cursor", cursor]);
    assert_eq!(out.status.code(), Some(6));
    assert_eq!(value(&out)["data"]["error"]["code"], "REVISION_CHANGED");
}

#[test]
fn browse_cursor_cannot_cross_bundles_or_directories() {
    let f = Fixture::new();
    f.bind();
    let first = value(&f.run(&["browse", "@book/services", "--limit", "1"]));
    let cursor = first["page"]["next_cursor"].as_str().unwrap();
    let second = value(&f.run(&[
        "browse",
        "@book/services",
        "--limit",
        "1",
        "--cursor",
        cursor,
    ]));
    assert_ne!(
        first["data"]["items"][0]["id"],
        second["data"]["items"][0]["id"]
    );
    let wrong = f.run(&["browse", "@book", "--cursor", cursor]);
    assert_eq!(wrong.status.code(), Some(2));
}

#[test]
fn metadata_search_reports_incomplete_coverage() {
    let f = Fixture::new();
    f.bind();
    let result = f.run(&["search", "登录", "--max-scan", "1"]);
    assert_eq!(result.status.code(), Some(7));
    assert_eq!(
        value(&result)["bundles"][0]["coverage"]["metadata"],
        "partial"
    );
    let result = f.run(&["search", "登录"]);
    assert_eq!(result.status.code(), Some(0));
    assert_eq!(value(&result)["data"]["items"][0]["title"], "登录服务");
}

#[test]
fn malformed_concept_is_diagnosed_and_requires_raw_read() {
    let f = Fixture::new();
    f.bind();
    fs::write(
        f.work.path().join("knowledge/services/bad.md"),
        "# no frontmatter",
    )
    .unwrap();
    let browse = value(&f.run(&["browse", "@book/services"]));
    assert_eq!(browse["diagnostics"][0]["code"], "INVALID_OKF");
    assert_eq!(
        f.run(&["read", "@book/services/bad"]).status.code(),
        Some(1)
    );
    assert_eq!(
        f.run(&["read", "@book/services/bad", "--raw"])
            .status
            .code(),
        Some(0)
    );
    let validation = f.run(&["validate", "book"]);
    assert_eq!(validation.status.code(), Some(1));
    assert_eq!(value(&validation)["status"], "error");
}

#[test]
fn remove_preserves_bundle_and_cli_rejects_unimplemented_adapters() {
    let f = Fixture::new();
    f.bind();
    assert_eq!(f.run(&["bundle", "remove", "book"]).status.code(), Some(0));
    assert!(f.work.path().join("knowledge/services/auth.md").exists());
    let unsupported = f.run(&["bundle", "add", "remote", "--kind", "git"]);
    assert_eq!(unsupported.status.code(), Some(2));
    assert_eq!(
        value(&unsupported)["data"]["error"]["code"],
        "UNSUPPORTED_CAPABILITY"
    );
}

#[test]
fn argument_errors_and_budget_errors_are_bounded_json() {
    let f = Fixture::new();
    let invalid = f.run(&["browse", "@book", "--limit", "0"]);
    assert_eq!(invalid.status.code(), Some(2));
    assert_eq!(value(&invalid)["status"], "error");
    f.bind();
    let small = f.run(&["browse", "@book/services", "--budget-bytes", "1"]);
    assert_eq!(small.status.code(), Some(2));
    assert_eq!(value(&small)["data"]["error"]["code"], "BUDGET_TOO_SMALL");
    let bounded = f.run(&["read", "@book/services/auth", "--max-output-bytes", "1024"]);
    assert!(bounded.stdout.len() <= 1024);
    let _ = value(&bounded);
}

#[test]
fn traversal_and_arbitrary_cursors_are_rejected() {
    let f = Fixture::new();
    f.bind();
    for reference in [
        "@book/../outside",
        "@book/C:/outside",
        "@book/services\\auth",
    ] {
        assert_eq!(f.run(&["read", reference]).status.code(), Some(2));
    }
    assert_eq!(
        f.run(&["browse", "@book", "--cursor", "🙂"]).status.code(),
        Some(2)
    );
}

#[test]
fn argument_errors_honor_output_limit_and_preserve_error_code() {
    let f = Fixture::new();
    for content in ["认证", "\n", "\t", "\"", "\\"] {
        let long_argument = format!("unknown{}", content.repeat(1000));
        for limit in [
            vec!["--max-output-bytes", "1024"],
            vec!["--max-output-bytes=1024"],
        ] {
            let mut args = limit;
            args.push(&long_argument);
            let output = f.run(&args);
            assert_eq!(output.status.code(), Some(2));
            assert!(output.stdout.len() <= 1024);
            assert_eq!(value(&output)["data"]["error"]["code"], "INVALID_ARGUMENT");
        }
    }
}

#[test]
fn duplicate_aliases_require_scope_and_removal_preserves_other_scope() {
    let f = Fixture::new();
    f.bind();
    assert_eq!(
        f.run(&[
            "--scope",
            "user",
            "bundle",
            "add",
            "book",
            "--path",
            "knowledge"
        ])
        .status
        .code(),
        Some(0)
    );
    let ambiguous = f.run(&["read", "@book/services/auth"]);
    assert_eq!(ambiguous.status.code(), Some(2));
    assert_eq!(
        value(&ambiguous)["data"]["error"]["code"],
        "AMBIGUOUS_BUNDLE"
    );
    for scope in ["project", "user"] {
        assert_eq!(
            f.run(&["--scope", scope, "read", "@book/services/auth"])
                .status
                .code(),
            Some(0)
        );
    }
    assert_eq!(
        f.run(&["--scope", "user", "bundle", "remove", "book"])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(
        f.run(&["read", "@book/services/auth"]).status.code(),
        Some(0)
    );
}

#[test]
fn skill_install_preserves_existing_contents() {
    let f = Fixture::new();
    let destination = f.work.path().join("skill");
    let path = destination.to_str().unwrap();
    assert_eq!(f.run(&["skill", "install", path]).status.code(), Some(0));
    let skill = destination.join("SKILL.md");
    fs::write(&skill, "custom skill contents").unwrap();
    assert_eq!(f.run(&["skill", "install", path]).status.code(), Some(5));
    assert_eq!(fs::read_to_string(skill).unwrap(), "custom skill contents");
}

#[test]
fn read_preserves_repeated_markdown_suffix_in_concept_identity() {
    let f = Fixture::new();
    f.bind();
    fs::write(
        f.work.path().join("knowledge/services/guide.md.md"),
        "---\ntype: Note\n---\nbody\n",
    )
    .unwrap();
    let listing = value(&f.run(&["browse", "@book/services"]));
    let card = listing["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["id"] == "services/guide.md")
        .unwrap();
    let document = value(&f.run(&["read", "@book/services/guide.md.md"]));
    assert_eq!(document["data"]["document"]["concept_id"], card["id"]);
    assert_eq!(document["data"]["card"]["id"], card["id"]);
}

#[test]
fn bundle_registration_uses_bundle_configuration_and_response_fields() {
    let f = Fixture::new();
    let added = f.run(&["bundle", "add", "book", "--path", "knowledge"]);
    assert_eq!(added.status.code(), Some(0));
    let added = value(&added);
    assert_eq!(added["command"], "bundle add");
    assert_eq!(added["data"]["bundle"]["name"], "book");
    assert_eq!(added["schema_version"], "2.0");
    assert!(added.get("sources").is_none());
    let config = fs::read_to_string(f.work.path().join(".okf-cli/config.toml")).unwrap();
    assert!(config.contains("[[bundles]]"));
    let overview = value(&f.run(&["context", "--bundle", "book"]));
    assert_eq!(overview["bundles"][0]["alias"], "book");
    assert!(overview["bundles"][0].get("bundle_id").is_some());
    assert_eq!(
        f.run(&["search", "登录", "--bundle", "book"]).status.code(),
        Some(0)
    );
}

#[test]
fn unsupported_command_and_configuration_names_are_rejected() {
    let f = Fixture::new();
    assert_eq!(f.run(&["source", "list"]).status.code(), Some(2));
    assert_eq!(
        f.run(&["context", "--source", "book"]).status.code(),
        Some(2)
    );
    fs::create_dir_all(f.work.path().join(".okf-cli")).unwrap();
    fs::write(f.work.path().join(".okf-cli/config.toml"), "sources = []\n").unwrap();
    let output = f.run(&["bundle", "list"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(value(&output)["data"]["error"]["code"], "INVALID_CONFIG");
}

#[test]
fn commands_reject_options_that_do_not_apply() {
    let f = Fixture::new();
    for args in [
        vec!["read", "@book/note", "--limit", "1"],
        vec!["read", "@book/note", "--max-scan", "1"],
        vec!["validate", "book", "--limit", "1"],
        vec!["validate", "book", "--budget-bytes", "1024"],
        vec!["validate", "book", "--cursor", "token"],
        vec!["context", "--cursor", "token"],
        vec!["context", "--max-scan", "1"],
        vec!["context", "--query", "auth", "--cursor", "token"],
        vec!["search", "auth", "--cursor", "token"],
        vec!["bundle", "list", "--timeout-ms", "1"],
        vec!["browse", "@book", "--timeout-ms", "0"],
        vec!["read", "@book/note", "--timeout-ms", "3600001"],
    ] {
        let out = f.run(&args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(value(&out)["data"]["error"]["code"], "INVALID_ARGUMENT");
    }
    f.bind();
    assert_eq!(
        f.run(&[
            "context",
            "--query",
            "auth",
            "--max-scan",
            "100",
            "--timeout-ms",
            "30000"
        ])
        .status
        .code(),
        Some(0)
    );
}

#[test]
fn incomplete_directory_does_not_issue_or_validate_a_cursor() {
    let f = Fixture::new();
    f.bind();
    let full = value(&f.run(&["browse", "@book/services", "--limit", "1"]));
    let cursor = full["page"]["next_cursor"].as_str().unwrap();
    let out = f.run(&["browse", "@book/services", "--max-scan", "2"]);
    assert_eq!(out.status.code(), Some(7));
    let partial = value(&out);
    assert_eq!(partial["budget"]["scanned_entries"], 2);
    assert!(partial["page"]["next_cursor"].is_null());
    assert_eq!(partial["page"]["has_more"], false);
    assert_eq!(partial["bundles"][0]["coverage"]["metadata"], "partial");
    let out = f.run(&[
        "browse",
        "@book/services",
        "--max-scan",
        "2",
        "--cursor",
        cursor,
    ]);
    assert_eq!(out.status.code(), Some(7));
    assert_eq!(value(&out)["data"]["error"]["code"], "SCAN_LIMIT");
    assert_eq!(
        f.run(&["browse", "@book/services", "--cursor", cursor])
            .status
            .code(),
        Some(0)
    );
}

#[test]
fn multi_bundle_search_keeps_results_when_files_or_roots_fail() {
    let f = Fixture::new();
    f.bind();
    fs::create_dir(f.work.path().join("broken")).unwrap();
    fs::write(
        f.work.path().join("broken/large.md"),
        "x".repeat(2 * 1024 * 1024 + 1),
    )
    .unwrap();
    assert_eq!(
        f.run(&["bundle", "add", "broken", "--path", "broken"])
            .status
            .code(),
        Some(0)
    );
    let out = f.run(&["search", "登录"]);
    assert_eq!(out.status.code(), Some(7));
    let response = value(&out);
    assert_eq!(response["data"]["items"][0]["title"], "登录服务");
    assert!(response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == "DOCUMENT_TOO_LARGE" && d["path"] == "@broken/large.md"));
    assert!(!response.to_string().contains("INDEX_PARTIAL"));
    fs::remove_dir_all(f.work.path().join("broken")).unwrap();
    for command in [
        vec!["context"],
        vec!["search", "登录"],
        vec!["context", "--query", "登录"],
    ] {
        let out = f.run(&command);
        assert_eq!(out.status.code(), Some(7));
        let response = value(&out);
        assert_eq!(response["data"]["items"].as_array().unwrap().len(), 1);
        let failed = response["bundles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["alias"] == "broken")
            .unwrap();
        assert_eq!(failed["freshness"], "unknown");
        assert_eq!(failed["coverage"]["metadata"], "unknown");
        assert!(failed["checked_at"].is_null());
    }
    assert_eq!(
        f.run(&["search", "登录", "--bundle", "broken"])
            .status
            .code(),
        Some(3)
    );
}

#[test]
fn corrupt_cache_preserves_read_search_and_browse_results() {
    let f = Fixture::new();
    f.bind();
    fs::write(f.home.path().join("index.sqlite"), "not a SQLite database").unwrap();
    for args in [
        vec!["read", "@book/services/auth"],
        vec!["browse", "@book/services"],
        vec!["search", "登录"],
    ] {
        let out = f.run(&args);
        assert_eq!(out.status.code(), Some(7));
        let response = value(&out);
        assert_eq!(response["status"], "partial");
        assert!(response["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "CACHE_WRITE_FAILED"));
        if args[0] == "read" {
            assert!(response["data"]["document"]["text"]
                .as_str()
                .unwrap()
                .contains("每秒"));
        } else {
            assert!(!response["data"]["items"].as_array().unwrap().is_empty());
        }
    }
    assert_eq!(f.run(&["doctor"]).status.code(), Some(5));
}

#[test]
fn cache_failure_does_not_override_complete_validation_findings() {
    let f = Fixture::new();
    f.bind();
    fs::write(
        f.work.path().join("knowledge/services/bad.md"),
        "# no frontmatter",
    )
    .unwrap();
    fs::write(f.home.path().join("index.sqlite"), "corrupt cache").unwrap();
    let output = f.run(&["validate", "book"]);
    assert_eq!(output.status.code(), Some(1));
    let response = value(&output);
    assert_eq!(response["status"], "error");
    assert_eq!(response["data"]["failures"], 1);
    assert_eq!(response["bundles"][0]["coverage"]["metadata"], "complete");
    assert!(response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == "CACHE_UNAVAILABLE"));
    let output = f.run(&["validate", "book", "--max-scan", "1"]);
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(value(&output)["status"], "partial");
}
