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
fn multi_bundle_queries_share_budgets_and_isolate_missing_indexes() {
    let f = Fixture::new();
    for (name, count) in [("large", 5), ("small", 1), ("unsynced", 0)] {
        let directory = f.work.path().join(name);
        fs::create_dir(&directory).unwrap();
        for i in 0..count {
            fs::write(
                directory.join(format!("note{i}.md")),
                format!("---\ntype: Note\ntitle: needle\n---\n{name} evidence {i}\n"),
            )
            .unwrap();
        }
        assert_eq!(
            f.run(&["bundle", "add", name, "--path", name])
                .status
                .code(),
            Some(0)
        );
        if name != "unsynced" {
            assert_eq!(f.run(&["sync", name]).status.code(), Some(0));
            fs::rename(&directory, f.work.path().join(format!("{name}-offline"))).unwrap();
        }
    }
    for command in [
        vec!["search", "needle", "--full-text"],
        vec!["context", "--query", "needle", "--content"],
    ] {
        let mut args = command.clone();
        args.extend(["--bundle", "large", "--bundle", "small", "--max-scan", "2"]);
        let out = f.run(&args);
        assert_eq!(out.status.code(), Some(7));
        let response = value(&out);
        assert_eq!(response["budget"]["indexed_documents_scanned"], 2);
        assert_eq!(response["budget"]["scope"], "request");
        assert_eq!(response["data"]["items"][0]["ref"], "@large/note0");
        assert_eq!(response["data"]["items"][1]["ref"], "@small/note0");
        assert!(response["page"]["total"].is_null());
        assert_eq!(response["bundles"][0]["coverage"]["metadata"], "partial");
        assert_eq!(response["bundles"][1]["coverage"]["metadata"], "complete");
        for item in response["data"]["items"].as_array().unwrap() {
            let read = f.run(&[
                "read",
                item["ref"].as_str().unwrap(),
                "--indexed",
                "--generation",
                item["generation"].as_str().unwrap(),
            ]);
            assert_eq!(read.status.code(), Some(0));
            assert_eq!(
                value(&read)["data"]["document"]["bundle_id"],
                item["bundle_id"]
            );
        }
        let mut reversed = command.clone();
        reversed.extend([
            "--bundle",
            "small",
            "--bundle",
            "large",
            "--bundle",
            "large",
            "--max-scan",
            "2",
        ]);
        assert_eq!(
            value(&f.run(&reversed))["data"]["items"],
            response["data"]["items"]
        );
        args.extend(["--bundle", "unsynced"]);
        let partial = value(&f.run(&args));
        assert_eq!(partial["data"]["items"], response["data"]["items"]);
        assert_eq!(
            partial["data"]["coverage_gaps"]["unavailable_bundles"],
            serde_json::json!(["unsynced"])
        );
        let mut full = command.clone();
        full.extend(["--bundle", "large", "--bundle", "small", "--max-scan", "6"]);
        let complete = f.run(&full);
        assert_eq!(complete.status.code(), Some(0));
        assert_eq!(value(&complete)["page"]["total"], 6);
        full.extend(["--limit", "1", "--budget-bytes", "1"]);
        let empty = f.run(&full);
        assert_eq!(empty.status.code(), Some(7));
        assert_eq!(value(&empty)["page"]["returned"], 0);
        assert_eq!(value(&empty)["budget"]["content_bytes_used"], 0);
        let mut missing = command.clone();
        missing.extend(["--bundle", "unsynced"]);
        assert_eq!(f.run(&missing).status.code(), Some(3));
        assert_eq!(f.run(&command).status.code(), Some(2));
    }
}

#[test]
fn lifecycle_filters_use_request_time_in_live_and_offline_queries() {
    let f = Fixture::new();
    f.bind();
    for (name, fields) in [
        ("human", "verified: {by: 'human:alice', at: '2000-01-01T00:00:00Z'}\nstale_after: '2030-01-01T00:00:00Z'\n"),
        ("machine", "verified: {by: 'process:ci', at: '2000-01-01T00:00:00Z'}\nstale_after: '2030-01-01T00:00:00Z'\n"),
        ("unknown", "stale_after: invalid\n"),
        ("unspecified", ""),
    ] {
        fs::write(f.work.path().join(format!("knowledge/{name}.md")),
            format!("---\ntype: Note\ntitle: policy\n{fields}---\npolicy evidence\n")).unwrap();
    }
    assert_eq!(f.run(&["sync", "book"]).status.code(), Some(0));
    for base in [
        vec!["search", "policy"],
        vec!["search", "policy", "--full-text"],
        vec!["context", "--query", "policy"],
        vec!["context", "--query", "policy", "--content"],
    ] {
        let mut args = base.clone();
        args.extend([
            "--bundle",
            "book",
            "--as-of",
            "2029-12-31T19:00:00-05:00",
            "--trust",
            "human-reviewed",
            "--expiry",
            "expired",
        ]);
        let output = f.run(&args);
        assert_eq!(output.status.code(), Some(0));
        let response = value(&output);
        assert_eq!(
            response["data"]["evaluated_at"],
            "2030-01-01T00:00:00+00:00"
        );
        assert_eq!(response["page"]["returned"], 1);
        assert_eq!(response["data"]["items"][0]["id"], "human");
        assert_ne!(
            response["bundles"][0]["checked_at"],
            response["data"]["evaluated_at"]
        );
        let mut args = base;
        args.extend([
            "--bundle",
            "book",
            "--trust",
            "unverified",
            "--expiry",
            "unknown",
            "--expiry",
            "unspecified",
        ]);
        assert_eq!(value(&f.run(&args))["page"]["returned"], 2);
    }
    fs::rename(
        f.work.path().join("knowledge"),
        f.work.path().join("offline"),
    )
    .unwrap();
    let query = |time| {
        value(&f.run(&[
            "search",
            "policy",
            "--full-text",
            "--bundle",
            "book",
            "--trust",
            "machine-confirmed",
            "--expiry",
            "current",
            "--as-of",
            time,
        ]))
    };
    let before = query("2029-12-31T23:59:59Z");
    let after = query("2030-01-01T00:00:00Z");
    assert_eq!(before["page"]["returned"], 1);
    assert_eq!(after["page"]["returned"], 0);
    assert_eq!(before["data"]["generation"], after["data"]["generation"]);
    for args in [
        vec!["search", "policy", "--as-of", "2030-01-01"],
        vec!["context", "--as-of", "2030-01-01T00:00:00Z"],
        vec!["search", "policy", "--trust", "trusted"],
    ] {
        assert_eq!(f.run(&args).status.code(), Some(2));
    }
}

#[test]
fn warning_gate_distinguishes_complete_counts_from_truncation_and_scan_limits() {
    let f = Fixture::new();
    f.bind();
    let path = f.work.path().join("knowledge/warnings.md");
    fs::write(&path, "---\ntype: Note\ndescription: 123\n---\n").unwrap();
    assert_eq!(f.run(&["validate", "book"]).status.code(), Some(0));
    let rejected = f.run(&["validate", "book", "--max-warnings", "0"]);
    assert_eq!(rejected.status.code(), Some(1));
    let gate = &value(&rejected)["data"]["quality_gate"];
    assert_eq!(gate["passed"], false);
    assert_eq!(gate["warnings_total"], 1);
    assert_eq!(gate["complete"], true);
    assert_eq!(
        f.run(&["validate", "book", "--max-warnings", "1"])
            .status
            .code(),
        Some(0)
    );
    let limited = f.run(&["validate", "book", "--max-warnings", "0", "--max-scan", "1"]);
    assert_eq!(limited.status.code(), Some(7));
    assert!(value(&limited)["data"]["quality_gate"]["passed"].is_null());
    fs::write(
        &path,
        format!(
            "---\ntype: Note\ntags: [{}]\n---\n",
            vec!["123"; 100].join(",")
        ),
    )
    .unwrap();
    for (limit, expected) in [("64", Value::Null), ("0", Value::Bool(false))] {
        let result = f.run(&["validate", "book", "--max-warnings", limit]);
        assert_eq!(result.status.code(), Some(7));
        let gate = &value(&result)["data"]["quality_gate"];
        assert_eq!(gate["warnings_observed"], 64);
        assert!(gate["warnings_total"].is_null());
        assert_eq!(gate["passed"], expected);
    }
    fs::write(&path, "not a concept\n").unwrap();
    let invalid = f.run(&["validate", "book", "--max-warnings", "0"]);
    assert_eq!(invalid.status.code(), Some(1));
    assert!(value(&invalid)["data"]["quality_gate"]["passed"].is_null());
}

#[test]
fn build_info_does_not_open_runtime_state() {
    let f = Fixture::new();
    let absent = f.work.path().join("absent");
    let output = f.run(&["--home", absent.to_str().unwrap(), "build-info"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(!absent.exists());
    let info = &value(&output)["data"];
    assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
    assert!(info["target"].as_str().unwrap().contains('-'));
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
    let unsupported = f.run(&["bundle", "add", "remote", "--kind", "s3"]);
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
    assert_eq!(small.status.code(), Some(7));
    assert_eq!(value(&small)["diagnostics"][0]["code"], "CARD_TOO_LARGE");
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

#[test]
fn metadata_warnings_are_consistent_and_do_not_reduce_coverage() {
    let f = Fixture::new();
    f.bind();
    let raw = "---\ntype: Future Concept\ntitle: Warning fixture\nverified: {by: 'human:reviewer', at: 'no timezone'}\nsources: [{id: scope, resource: 'all queries in project X', usage_count: -2}]\nstale_after: '2026-10-22'\nx-custom: {nested: [a, b]}\n---\nExact original body.\n";
    fs::write(f.work.path().join("knowledge/warning.md"), raw).unwrap();
    let mut expected = None;
    for args in [
        vec!["read", "@book/warning"],
        vec!["browse", "@book"],
        vec!["search", "Warning fixture"],
        vec!["context", "--query", "Warning fixture"],
        vec!["validate", "book"],
    ] {
        let output = f.run(&args);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let response = value(&output);
        assert_eq!(response["status"], "ok");
        assert_eq!(response["bundles"][0]["coverage"]["metadata"], "complete");
        let warnings = response["diagnostics"].as_array().unwrap();
        assert_eq!(warnings.len(), 3);
        for warning in warnings {
            assert_eq!(warning["severity"], "warning");
            assert_eq!(warning["path"], "@book/warning.md");
            assert!(warning["field"].as_str().unwrap().starts_with('/'));
        }
        if let Some(expected) = &expected {
            assert_eq!(warnings, expected, "{args:?}");
        } else {
            expected = Some(warnings.clone());
        }
        if args[0] == "read" {
            assert_eq!(response["data"]["document"]["text"], raw);
            assert_eq!(response["data"]["card"]["expiry"], "unknown");
        }
        if args[0] == "validate" {
            assert_eq!(response["data"]["warnings"], 3);
            assert_eq!(response["data"]["failures"], 0);
        }
    }
}

#[test]
fn metadata_warnings_coexist_with_structural_errors_and_partial_scans() {
    let f = Fixture::new();
    f.bind();
    fs::write(
        f.work.path().join("knowledge/warning.md"),
        "---\ntype: Note\nstatus: experimental\n---\nbody\n",
    )
    .unwrap();
    fs::write(
        f.work.path().join("knowledge/bad.md"),
        "# missing frontmatter",
    )
    .unwrap();
    let output = f.run(&["validate", "book"]);
    assert_eq!(output.status.code(), Some(1));
    let response = value(&output);
    assert_eq!(response["data"]["failures"], 1);
    assert_eq!(response["data"]["warnings"], 1);
    assert!(
        response["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "INVALID_OKF"
                && diagnostic["severity"] == "error")
    );
    let output = f.run(&["search", "not found", "--max-scan", "1"]);
    assert_eq!(output.status.code(), Some(7));
    let response = value(&output);
    assert_eq!(response["bundles"][0]["coverage"]["metadata"], "partial");
    assert!(response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == "SCAN_LIMIT" && diagnostic["severity"] == "error"));
}

#[test]
fn minimal_concepts_and_unknown_extensions_remain_consumable() {
    let f = Fixture::new();
    f.bind();
    fs::write(f.work.path().join("knowledge/minimal.md"), "---\ntype: User Defined Type\nx-extra: {anything: [1, 2]}\n---\n[future link](/not-created.md)\n").unwrap();
    for args in [
        vec!["read", "@book/minimal"],
        vec!["search", "minimal"],
        vec!["validate", "book"],
    ] {
        let output = f.run(&args);
        assert_eq!(output.status.code(), Some(0));
        let response = value(&output);
        let diagnostics = response["diagnostics"].as_array().unwrap();
        if args[0] == "validate" {
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(diagnostics[0]["code"], "BROKEN_LINK");
            assert_eq!(diagnostics[0]["severity"], "warning");
        } else {
            assert!(diagnostics.is_empty());
        }
    }
}

#[test]
fn body_diagnostics_preserve_bytes_and_are_consistent_across_consumption_commands() {
    let f = Fixture::new();
    f.bind();
    let raw = "\u{feff}---\r\ntype: Note\r\nsources: [{id: known, resource: scope}]\r\n---\r\n中文.[^known] Missing.[^missing]\r\n\r\n[^known]: Source\r\n\r\n[escape](../../outside.md) [future](future.md#part)\r\n`[^code]`\r\n";
    fs::write(f.work.path().join("knowledge/services/note.md"), raw).unwrap();
    let mut expected = None;
    for args in [
        vec!["read", "@book/services/note"],
        vec!["browse", "@book/services"],
        vec!["search", "note"],
        vec!["context", "--query", "note"],
    ] {
        let output = f.run(&args);
        assert_eq!(output.status.code(), Some(0));
        let response = value(&output);
        let diagnostics = &response["diagnostics"];
        assert_eq!(diagnostics.as_array().unwrap().len(), 3);
        if let Some(expected) = &expected {
            assert_eq!(diagnostics, expected);
        } else {
            expected = Some(diagnostics.clone());
        }
        for diagnostic in diagnostics.as_array().unwrap() {
            assert_eq!(diagnostic["path"], "@book/services/note.md");
            assert_eq!(diagnostic["severity"], "warning");
            assert!(diagnostic["field"].is_null());
            let start = diagnostic["location"]["byte_start"].as_u64().unwrap() as usize;
            let end = diagnostic["location"]["byte_end"].as_u64().unwrap() as usize;
            assert!(matches!(
                &raw[start..end],
                "[^missing]" | "[escape](../../outside.md)"
            ));
        }
        if args[0] == "read" {
            assert_eq!(response["data"]["document"]["text"], raw);
        }
    }
    let output = f.run(&["validate", "book"]);
    assert_eq!(output.status.code(), Some(0));
    let response = value(&output);
    assert_eq!(response["data"]["warnings"], 4);
    assert_eq!(response["data"]["failures"], 0);
    assert_eq!(response["data"]["link_check"]["complete"], true);
    assert_eq!(response["bundles"][0]["coverage"]["metadata"], "complete");
    assert!(response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == "BROKEN_LINK"));
    assert_eq!(
        fs::read(f.work.path().join("knowledge/services/note.md")).unwrap(),
        raw.as_bytes()
    );
}

#[test]
fn validation_checks_authored_indexes_and_does_not_infer_missing_targets_from_partial_scans() {
    let f = Fixture::new();
    f.bind();
    fs::write(f.work.path().join("knowledge/index.md"), "# Index\n[missing](missing.md) [valid][ref] [dir](services/#part)\n\n[ref]: services/auth.md#heading\n").unwrap();
    let complete = f.run(&["validate", "book"]);
    assert_eq!(complete.status.code(), Some(0));
    let response = value(&complete);
    assert_eq!(response["data"]["warnings"], 1);
    assert_eq!(response["diagnostics"][0]["path"], "@book/index.md");
    assert_eq!(response["diagnostics"][0]["code"], "BROKEN_LINK");
    let partial = f.run(&["validate", "book", "--max-scan", "4"]);
    assert_eq!(partial.status.code(), Some(7));
    let response = value(&partial);
    assert_eq!(response["data"]["link_check"]["complete"], false);
    assert_eq!(response["budget"]["links_checked"], 0);
    assert!(!response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == "BROKEN_LINK"));
}

#[test]
fn validation_link_limit_does_not_downgrade_completed_metadata_coverage() {
    let f = Fixture::new();
    f.bind();
    let raw = format!(
        "---\ntype: Note\n---\n{}",
        "[link](/services/auth.md) ".repeat(256)
    );
    for index in 0..17 {
        fs::write(
            f.work.path().join(format!("knowledge/note-{index}.md")),
            &raw,
        )
        .unwrap();
    }
    let output = f.run(&["validate", "book"]);
    assert_eq!(output.status.code(), Some(7));
    let response = value(&output);
    assert_eq!(response["budget"]["links_checked"], 4096);
    assert_eq!(response["data"]["link_check"]["complete"], false);
    assert_eq!(response["bundles"][0]["coverage"]["metadata"], "complete");
    assert_eq!(response["diagnostics"][0]["code"], "LINK_LIMIT");
}

#[test]
fn validation_bounds_pending_link_bytes_independently_of_link_count() {
    let f = Fixture::new();
    f.bind();
    let raw = format!(
        "---\ntype: Note\n---\n[link](/services/auth.md#{})",
        "a".repeat(600_000)
    );
    for index in 0..2 {
        fs::write(
            f.work.path().join(format!("knowledge/note-{index}.md")),
            &raw,
        )
        .unwrap();
    }
    let output = f.run(&["validate", "book"]);
    assert_eq!(output.status.code(), Some(7));
    let response = value(&output);
    assert!(response["budget"]["link_bytes_used"].as_u64().unwrap() <= 2 * 1024 * 1024);
    assert!(response["budget"]["links_checked"].as_u64().unwrap() < 4096);
    assert_eq!(response["data"]["link_check"]["complete"], false);
    assert_eq!(response["bundles"][0]["coverage"]["metadata"], "complete");
    assert_eq!(response["diagnostics"][0]["code"], "LINK_LIMIT");
}

#[test]
fn validation_checks_encoded_directories_and_distinguishes_files_from_directories() {
    let f = Fixture::new();
    f.bind();
    fs::create_dir(f.work.path().join("knowledge/folder.md")).unwrap();
    fs::write(f.work.path().join("knowledge/links.md"), "---\ntype: Note\n---\n[missing](gone%2F?q=1#part) [file](/folder.md) [directory](/services/auth.md/) [valid](/folder.md%2F) [dot](/folder.md/.)\n").unwrap();
    let output = f.run(&["validate", "book"]);
    assert_eq!(output.status.code(), Some(0));
    let response = value(&output);
    assert_eq!(response["data"]["warnings"], 3);
    assert_eq!(response["data"]["link_check"]["complete"], true);
    assert!(response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .all(|d| d["code"] == "BROKEN_LINK" && d["severity"] == "warning"));
}

#[test]
fn broken_link_findings_share_the_per_file_body_warning_limit() {
    let f = Fixture::new();
    f.bind();
    for (footnotes, broken) in [(0, 64), (10, 44), (40, 0)] {
        let raw = format!(
            "---\ntype: Note\n---\n{}\n\n{}",
            "[^unknown] ".repeat(footnotes),
            "[missing](/missing.md) ".repeat(100)
        );
        fs::write(f.work.path().join("knowledge/links.md"), raw).unwrap();
        let output = f.run(&["validate", "book"]);
        assert_eq!(output.status.code(), Some(0));
        let response = value(&output);
        let diagnostics = response["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 65);
        assert_eq!(
            diagnostics
                .iter()
                .filter(|d| d["code"] == "BROKEN_LINK")
                .count(),
            broken
        );
        assert_eq!(
            diagnostics
                .iter()
                .filter(|d| d["code"] == "BODY_DIAGNOSTICS_TRUNCATED")
                .count(),
            1
        );
        assert_eq!(response["data"]["warnings"], 65);
        assert_eq!(response["data"]["link_check"]["complete"], false);
        assert_eq!(response["bundles"][0]["coverage"]["metadata"], "complete");
    }
}

#[test]
fn computation_warnings_are_consistent_and_keep_contracts_consumable() {
    let f = Fixture::new();
    f.bind();
    let raw = "\u{feff}---\r\ntype: Attested Computation\r\nruntime: custom\r\nparameters: [{name: x, type: custom, required: true}, {name: x, type: custom, required: false}]\r\ncomputation: missing/code.py\r\nexecutor: {resource: missing/run.md, receipt: [7]}\r\n---\r\n# Computation\r\n```python\r\nraise RuntimeError('must not execute')\r\n```\r\n\r\n```python\r\nreturn 2\r\n```\r\n";
    fs::write(f.work.path().join("knowledge/compute.md"), raw).unwrap();
    let mut expected = None;
    for args in [
        vec!["read", "@book/compute"],
        vec!["browse", "@book"],
        vec!["search", "compute"],
        vec!["context", "--query", "compute"],
        vec!["validate", "book"],
    ] {
        let output = f.run(&args);
        assert_eq!(output.status.code(), Some(0));
        let response = value(&output);
        assert_eq!(response["status"], "ok");
        assert_eq!(response["bundles"][0]["coverage"]["metadata"], "complete");
        let diagnostics = serde_json::json!(response["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["code"] != "MISSING_RESOURCE")
            .collect::<Vec<_>>());
        assert_eq!(diagnostics.as_array().unwrap().len(), 4);
        if let Some(expected) = &expected {
            assert_eq!(&diagnostics, expected);
        } else {
            expected = Some(diagnostics.clone());
        }
        for diagnostic in diagnostics.as_array().unwrap() {
            assert_eq!(diagnostic["severity"], "warning");
            assert_eq!(diagnostic["path"], "@book/compute.md");
        }
        assert_eq!(diagnostics[0]["code"], "DUPLICATE_PARAMETER_NAME");
        assert_eq!(diagnostics[0]["field"], "/parameters/1/name");
        assert_eq!(diagnostics[1]["field"], "/executor/receipt/0");
        for diagnostic in diagnostics.as_array().unwrap().iter().skip(2) {
            let location = &diagnostic["location"];
            let start = location["byte_start"].as_u64().unwrap() as usize;
            let end = location["byte_end"].as_u64().unwrap() as usize;
            assert!(raw[start..end].starts_with("```python"));
        }
        if args[0] == "read" {
            assert_eq!(response["data"]["document"]["text"], raw);
        }
        if args[0] == "validate" {
            assert_eq!(response["data"]["warnings"], 6);
            assert_eq!(response["data"]["failures"], 0);
        }
    }
    assert_eq!(
        fs::read(f.work.path().join("knowledge/compute.md")).unwrap(),
        raw.as_bytes()
    );
}

#[test]
fn computation_contracts_check_local_resources_without_fetching_uris_or_restricting_unknown_types()
{
    let f = Fixture::new();
    f.bind();
    for raw in [
        "---\ntype: Attested Computation\nruntime: custom\ncomputation: missing/code.sql\nexecutor: {resource: missing/run.md}\nattester: {resource: https://example.invalid/attest.py}\n---\n",
        "---\ntype: Unknown Computation\nruntime: []\nparameters: false\nexecutor: null\n---\n# Computation\n```\n```\n",
    ] {
        fs::write(f.work.path().join("knowledge/compute.md"), raw).unwrap();
        let output = f.run(&["validate", "book"]);
        assert_eq!(output.status.code(), Some(0));
        let response = value(&output);
        if raw.contains("type: Attested Computation") {
            assert_eq!(response["diagnostics"].as_array().unwrap().len(), 2);
            assert!(response["diagnostics"].as_array().unwrap().iter().all(|d| d["code"] == "MISSING_RESOURCE"));
            assert_eq!(response["data"]["resource_check"]["external_skipped"], 1);
        } else {
            assert!(response["diagnostics"].as_array().unwrap().is_empty());
        }
        assert_eq!(response["data"]["failures"], 0);
    }
}

#[test]
fn resource_checks_are_root_relative_bounded_and_do_not_execute_or_decode_assets() {
    let f = Fixture::new();
    f.bind();
    fs::create_dir_all(f.work.path().join("knowledge/assets")).unwrap();
    fs::write(f.work.path().join("knowledge/assets/code.py"), [0xff, 0x00]).unwrap();
    fs::write(f.work.path().join("knowledge/services/compute.md"),"---\ntype: Attested Computation\nruntime: python\ncomputation: assets/code.py\nexecutor: {resource: assets}\nattester: {resource: missing.py}\n---\n").unwrap();
    let output = f.run(&["validate", "book"]);
    assert_eq!(output.status.code(), Some(0));
    let response = value(&output);
    assert_eq!(response["data"]["resource_check"]["complete"], true);
    assert_eq!(response["budget"]["resources_checked"], 3);
    let diagnostics = response["diagnostics"].as_array().unwrap();
    assert!(diagnostics
        .iter()
        .any(|d| d["code"] == "RESOURCE_NOT_A_FILE" && d["field"] == "/executor/resource"));
    assert!(diagnostics
        .iter()
        .any(|d| d["code"] == "MISSING_RESOURCE" && d["field"] == "/attester/resource"));
    let output = f.run(&["validate", "book", "--max-resources", "1"]);
    assert_eq!(output.status.code(), Some(7));
    let response = value(&output);
    assert_eq!(response["data"]["resource_check"]["complete"], false);
    assert_eq!(response["bundles"][0]["coverage"]["metadata"], "complete");
    assert_eq!(response["budget"]["resources_checked"], 1);
    assert!(response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == "RESOURCE_LIMIT"));
}

#[test]
fn oversized_cards_do_not_hide_later_results_and_have_consistent_diagnostics() {
    let f = Fixture::new();
    f.bind();
    fs::write(
        f.work.path().join("knowledge/services/aaa.md"),
        format!(
            "---\ntype: Service\ntitle: {}\n---\n",
            "oversize".repeat(1000)
        ),
    )
    .unwrap();
    for args in [
        vec!["browse", "@book/services", "--budget-bytes", "2048"],
        vec!["search", "Service", "--budget-bytes", "2048"],
        vec!["context", "--query", "Service", "--budget-bytes", "2048"],
    ] {
        let output = f.run(&args);
        assert_eq!(output.status.code(), Some(7));
        let response = value(&output);
        assert!(response["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "services/billing"));
        assert!(response["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "CARD_TOO_LARGE" && d["path"] == "@book/services/aaa.md"));
    }
}

#[test]
fn cache_recovery_command_works_when_database_cannot_be_opened() {
    let f = Fixture::new();
    f.bind();
    fs::write(f.home.path().join("index.sqlite"), b"broken").unwrap();
    let output = f.run(&["cache", "reset"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(
        Path::new(value(&output)["data"]["archive"].as_str().unwrap())
            .join("index.sqlite")
            .exists()
    );
    assert_eq!(
        f.run(&["read", "@book/services/auth"]).status.code(),
        Some(0)
    );
    assert!(f.work.path().join("knowledge/services/auth.md").exists());
}

#[test]
fn indexed_search_and_context_preserve_citations_offline_and_failed_sync_preserves_old_generation()
{
    let f = Fixture::new();
    f.bind();
    let path = f.work.path().join("knowledge/services/auth.md");
    let original = fs::read_to_string(&path).unwrap();
    let synced = value(&f.run(&["sync", "book"]));
    assert_eq!(synced["data"]["published"], true);
    let generation = synced["data"]["generation"].clone();
    let live = value(&f.run(&["search", "每秒"]));
    assert_eq!(live["page"]["returned"], 0);
    for args in [
        vec!["search", "每秒", "--bundle", "book", "--full-text"],
        vec![
            "context",
            "--query",
            "每秒",
            "--bundle",
            "book",
            "--content",
        ],
    ] {
        let output = f.run(&args);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let response = value(&output);
        let item = &response["data"]["items"][0];
        assert_eq!(item["expiry"], "expired");
        assert_eq!(item["generation"], generation);
        let snippet = &item["snippet"];
        let start = snippet["byte_start"].as_u64().unwrap() as usize;
        let end = snippet["byte_end"].as_u64().unwrap() as usize;
        assert_eq!(snippet["text"], &original[start..end]);
        assert_eq!(
            item["content_sha256"],
            okf_core::sha256(original.as_bytes())
        );
        assert_eq!(response["bundles"][0]["freshness"], "cached");
    }
    fs::write(&path, "---\ntype: Service\n---\nchanged").unwrap();
    let output = f.run(&["sync", "book", "--max-scan", "1"]);
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(value(&output)["data"]["published"], false);
    assert_eq!(
        value(&f.run(&["index", "status", "book"]))["data"]["generation"],
        generation
    );
    fs::rename(
        f.work.path().join("knowledge"),
        f.work.path().join("offline"),
    )
    .unwrap();
    fs::write(
        f.home.path().join("index.sqlite"),
        b"corrupt unrelated observation cache",
    )
    .unwrap();
    let output = f.run(&["read", "@book/services/auth", "--indexed"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(value(&output)["data"]["document"]["text"], original);
    assert_eq!(
        f.run(&["search", "每秒", "--bundle", "book", "--full-text"])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(f.run(&["index", "clear", "book"]).status.code(), Some(0));
    assert_eq!(
        f.run(&["read", "@book/services/auth", "--indexed"])
            .status
            .code(),
        Some(3)
    );
}

#[test]
fn indexed_cursor_rejects_new_generation_and_sync_removes_deleted_documents() {
    let f = Fixture::new();
    f.bind();
    assert_eq!(f.run(&["sync", "book"]).status.code(), Some(0));
    let first = value(&f.run(&[
        "read",
        "@book/services/auth",
        "--indexed",
        "--budget-bytes",
        "17",
    ]));
    let cursor = first["page"]["next_cursor"].as_str().unwrap();
    let generation = first["data"]["document"]["generation"].as_str().unwrap();
    fs::remove_file(f.work.path().join("knowledge/services/billing.md")).unwrap();
    assert_eq!(f.run(&["sync", "book"]).status.code(), Some(0));
    assert_eq!(
        f.run(&[
            "read",
            "@book/services/auth",
            "--indexed",
            "--cursor",
            cursor
        ])
        .status
        .code(),
        Some(6)
    );
    assert_eq!(
        f.run(&["read", "@book/services/billing", "--indexed"])
            .status
            .code(),
        Some(3)
    );
    assert_eq!(
        f.run(&[
            "read",
            "@book/services/auth",
            "--indexed",
            "--generation",
            generation
        ])
        .status
        .code(),
        Some(6)
    );
}

#[test]
fn git_cli_refresh_keeps_old_pages_until_explicit_refresh_and_failed_refresh_preserves_binding() {
    let f = Fixture::new();
    let git = |args: &[&str]| {
        let output = Command::new(std::env::var_os("OKF_GIT").unwrap_or_else(|| "git".into()))
            .current_dir(f.work.path())
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "--initial-branch=main"]);
    git(&["add", "knowledge"]);
    git(&["commit", "-m", "first"]);
    let output = f.run(&[
        "bundle",
        "add",
        "book",
        "--kind",
        "git",
        "--path",
        ".",
        "--ref",
        "main",
        "--bundle-path",
        "knowledge",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let pinned = value(&output)["data"]["bundle"]["git"]["commit"].clone();
    let first = value(&f.run(&["read", "@book/services/auth", "--budget-bytes", "17"]));
    let cursor = first["page"]["next_cursor"].as_str().unwrap();
    fs::write(
        f.work.path().join("knowledge/services/auth.md"),
        "---\ntype: Service\n---\nnew",
    )
    .unwrap();
    git(&["add", "knowledge"]);
    git(&["commit", "-m", "second"]);
    let continued = f.run(&["read", "@book/services/auth", "--cursor", cursor]);
    assert_eq!(continued.status.code(), Some(0));
    assert_eq!(value(&continued)["bundles"][0]["upstream_revision"], pinned);
    assert_eq!(value(&continued)["bundles"][0]["view_kind"], "snapshot");
    assert_eq!(f.run(&["bundle", "refresh", "book"]).status.code(), Some(0));
    assert_eq!(
        f.run(&["read", "@book/services/auth", "--cursor", cursor])
            .status
            .code(),
        Some(6)
    );
    git(&["branch", "-m", "renamed"]);
    let before = value(&f.run(&["bundle", "show", "book"]));
    assert_ne!(f.run(&["bundle", "refresh", "book"]).status.code(), Some(0));
    assert_eq!(
        value(&f.run(&["bundle", "show", "book"]))["data"],
        before["data"]
    );
}

#[test]
fn filtered_context_stops_at_peer_sections_and_keeps_code_headings() {
    let f = Fixture::new();
    f.bind();
    let raw = "---\ntype: Runbook\ntitle: recovery\ntags: [ops]\nstatus: stable\n---\n# Parent\n## Recovery\n```python\n# fake heading\nrestart_worker()\n```\n## Unrelated\nnot evidence\n";
    fs::write(f.work.path().join("knowledge/recovery.md"), raw).unwrap();
    assert_eq!(f.run(&["sync", "book"]).status.code(), Some(0));
    let response = value(&f.run(&[
        "context",
        "--query",
        "restart_worker",
        "--bundle",
        "book",
        "--content",
        "--type",
        "Runbook",
        "--tag",
        "ops",
        "--status",
        "stable",
    ]));
    assert_eq!(response["page"]["returned"], 1);
    let snippet = &response["data"]["items"][0]["snippet"];
    let text = snippet["text"].as_str().unwrap();
    assert!(text.starts_with("## Recovery"));
    assert!(!text.contains("Unrelated"));
    assert_eq!(
        text,
        &raw[snippet["byte_start"].as_u64().unwrap() as usize
            ..snippet["byte_end"].as_u64().unwrap() as usize]
    );
    for mode in [vec![], vec!["--full-text"]] {
        let mut args = vec![
            "search", "recovery", "--bundle", "book", "--type", "Service",
        ];
        args.extend(mode);
        assert_eq!(value(&f.run(&args))["page"]["returned"], 0);
    }
    assert_eq!(f.run(&["context", "--tag", "ops"]).status.code(), Some(2));
}

#[test]
fn indexed_neighbors_report_evidence_cycles_missing_targets_and_limits() {
    let f = Fixture::new();
    f.bind();
    let a="---\ntype: Note\nsources:\n  - resource: b.md\n---\n[B](b.md#heading) [missing](gone.md)\n";
    fs::write(f.work.path().join("knowledge/a.md"), a).unwrap();
    fs::write(
        f.work.path().join("knowledge/b.md"),
        "---\ntype: Note\n---\n# Heading\n[A](a.md)\n",
    )
    .unwrap();
    assert_eq!(f.run(&["sync", "book"]).status.code(), Some(0));
    fs::rename(
        f.work.path().join("knowledge"),
        f.work.path().join("offline"),
    )
    .unwrap();
    let response = value(&f.run(&["neighbors", "@book/a"]));
    assert_eq!(response["page"]["returned"], 4);
    let edges = response["data"]["items"].as_array().unwrap();
    assert!(edges.iter().any(|e| e["target_exists"] == false));
    assert!(edges
        .iter()
        .any(|e| e["evidence"]["reference"]["field"] == "/sources/0/resource"));
    let edge = edges
        .iter()
        .find(|e| {
            e["from"] == "@book/a.md" && e["evidence"]["reference"]["kind"] == "markdown_link"
        })
        .unwrap();
    let location = &edge["evidence"]["reference"]["location"];
    assert!(a[location["byte_start"].as_u64().unwrap() as usize
        ..location["byte_end"].as_u64().unwrap() as usize]
        .contains("b.md"));
    assert_eq!(
        value(&f.run(&["neighbors", "@book/a", "--direction", "incoming"]))["page"]["returned"],
        1
    );
    assert_eq!(
        f.run(&["neighbors", "@book/a", "--limit", "1"])
            .status
            .code(),
        Some(7)
    );
    let limited = value(&f.run(&["neighbors", "@book/a", "--max-scan", "1"]));
    assert_eq!(limited["data"]["relationship_check"]["complete"], false);
    assert!(limited["page"]["total"].is_null());
}

#[test]
fn attachment_and_anchor_checks_are_opt_in_and_custom_html_resolves() {
    let f = Fixture::new();
    f.bind();
    fs::write(f.work.path().join("knowledge/links.md"),"---\ntype: Note\n---\n# 中文\n# 中文\n[valid](#%E4%B8%AD%E6%96%87-1) [bad](#absent) [file](asset.bin) [missing](missing.bin) [custom](custom.md#custom)\n").unwrap();
    fs::write(
        f.work.path().join("knowledge/custom.md"),
        "---\ntype: Note\n---\n<a id='custom'></a>\n",
    )
    .unwrap();
    fs::write(f.work.path().join("knowledge/asset.bin"), [0, 255, 128]).unwrap();
    let plain = value(&f.run(&["validate", "book"]));
    assert!(plain["data"]["attachment_check"].is_null());
    let output = f.run(&[
        "validate",
        "book",
        "--attachments",
        "--anchor-style",
        "headings-v1",
    ]);
    assert_eq!(output.status.code(), Some(0));
    let response = value(&output);
    assert_eq!(response["data"]["attachment_check"]["complete"], true);
    assert_eq!(response["data"]["attachment_check"]["checked"], 2);
    assert_eq!(response["data"]["anchor_check"]["skipped"], 0);
    let diagnostics = response["diagnostics"].as_array().unwrap();
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d["code"] == "MISSING_ANCHOR")
            .count(),
        1
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d["code"] == "MISSING_ATTACHMENT")
            .count(),
        1
    );
    assert_eq!(
        value(&f.run(&["validate", "book", "--attachments", "--max-scan", "1"]))["data"]
            ["attachment_check"]["complete"],
        false
    );
}

#[test]
fn image_references_are_checked_when_attachments_are_enabled() {
    let f = Fixture::new();
    f.bind();
    fs::write(
        f.work.path().join("knowledge/images.md"),
        "---\ntype: Note\n---\n![ok](present.png) ![missing](gone.png)\n",
    )
    .unwrap();
    fs::write(f.work.path().join("knowledge/present.png"), [0, 255, 128]).unwrap();
    let plain = value(&f.run(&["validate", "book"]));
    assert!(plain["data"]["attachment_check"].is_null());
    let response = value(&f.run(&["validate", "book", "--attachments"]));
    assert_eq!(
        response["data"]["attachment_check"]["images_included"],
        true
    );
    assert_eq!(response["data"]["attachment_check"]["images_checked"], 2);
    assert_eq!(
        response["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["code"] == "MISSING_IMAGE")
            .count(),
        1
    );
}
