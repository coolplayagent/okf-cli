//! Pins how `candidates-v1` spends its row budget. The intersection is ordered
//! by posting-list size and part of the budget stays reserved for document
//! materialization, so a query whose broad gram alone exceeds the budget still
//! returns the documents its narrow gram selected instead of nothing.
//!
//! These cases run in-process on a synthetic bundle large enough for the two
//! strategies to take different paths through the index. The end-to-end
//! comparison against the real CLI lives in `scripts/evaluate_index_candidates.py`.
use okf_adapters::{Filesystem, SqliteCache, SqliteIndex};
use okf_application::{
    App, BundleBinding, BundleReader, BundleRegistry, Listing, ReadOptions, RequestControl,
    Response, ScanBudget, SearchFilter,
};
use std::{
    fs,
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

const DOCUMENTS: usize = 240;

struct Registry(Vec<BundleBinding>);
impl BundleRegistry for Registry {
    fn list(&self) -> okf_core::Result<Vec<BundleBinding>> {
        Ok(self.0.clone())
    }
    fn add(&self, _: &BundleBinding) -> okf_core::Result<()> {
        unreachable!()
    }
    fn remove(&self, _: &str, _: &str) -> okf_core::Result<()> {
        unreachable!()
    }
}

struct Reader;
impl BundleReader for Reader {
    fn probe(&self, binding: &BundleBinding) -> okf_core::Result<()> {
        Filesystem.probe(binding)
    }
    fn list(
        &self,
        binding: &BundleBinding,
        directory: &str,
        budget: &mut ScanBudget<'_>,
    ) -> okf_core::Result<Listing> {
        Filesystem.list(binding, directory, budget)
    }
    fn read(
        &self,
        binding: &BundleBinding,
        path: &str,
        max_bytes: usize,
        control: &RequestControl,
    ) -> okf_core::Result<String> {
        Filesystem.read(binding, path, max_bytes, control)
    }
}

fn corpus(root: &Path, name: &str, first: usize, documents: usize) {
    let bundle = root.join(name);
    fs::create_dir_all(bundle.join("services")).unwrap();
    fs::write(
        bundle.join("index.md"),
        "---\nokf_version: \"0.2\"\n---\n\n# 合成知识库\n\n* [服务](services/)\n",
    )
    .unwrap();
    for index in first..first + documents {
        // `consistency` is in every document, so its trigram postings alone
        // outgrow a small budget. `throughput` appears in a sixth of the
        // documents and must win the intersection by being read first.
        let body = if index % 6 == 0 {
            "endpoint mentions throughput."
        } else {
            "endpoint depicts ordinary narration."
        };
        fs::write(
            bundle.join("services").join(format!("s{index:04}.md")),
            format!(
                "---\ntype: Service\ntitle: 服务 {index}\ndescription: consistency contract\ntags: [synth]\nstatus: stable\n---\n\n# 行为\n\n{body} consistency.\n"
            ),
        )
        .unwrap();
    }
}

struct Harness {
    /// Kept alive for the temporary bundle and index files.
    _home: tempfile::TempDir,
    registry: Registry,
    reader: Reader,
    cache: SqliteCache,
    control: RequestControl,
    index: SqliteIndex,
}

impl Harness {
    fn new(home: tempfile::TempDir, names: &[&str]) -> Self {
        let registry = Registry(
            names
                .iter()
                .map(|name| BundleBinding {
                    id: name.to_string(),
                    name: name.to_string(),
                    kind: "fs".into(),
                    scope: "project".into(),
                    git: None,
                    root: home
                        .path()
                        .join(name)
                        .canonicalize()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                })
                .collect(),
        );
        let cache = SqliteCache::open(&home.path().join("cache.sqlite")).unwrap();
        let index = SqliteIndex::open(&home.path().join("content.sqlite")).unwrap();
        let harness = Self {
            _home: home,
            registry,
            reader: Reader,
            cache,
            control: RequestControl::new(
                Duration::from_secs(120),
                Arc::new(AtomicBool::new(false)),
            ),
            index,
        };
        for name in names {
            let response = harness.app().sync(name, &harness.options(), &harness.index);
            assert_eq!(response.unwrap().status, "ok", "sync failed for {name}");
        }
        harness
    }
    fn app(&self) -> App<'_> {
        App {
            registry: &self.registry,
            reader: &self.reader,
            cache: &self.cache,
            now: "2030-01-01T00:00:00Z".parse().unwrap(),
            control: &self.control,
        }
    }
    fn options(&self) -> ReadOptions {
        ReadOptions {
            limit: 100,
            budget: 1 << 20,
            max_scan: 10_000,
            cursor: None,
        }
    }
    fn search(&self, query: &str, selected: &[String], filter: &SearchFilter) -> Response {
        self.app()
            .indexed_search_bundles_filtered(
                query,
                selected,
                &self.options(),
                &self.index,
                false,
                filter,
            )
            .unwrap()
    }
}

fn counter(response: &Response, key: &str) -> u64 {
    response
        .budget
        .get(key)
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

fn ids(response: &Response) -> Vec<String> {
    response.data["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect()
}

fn candidates(rows: usize) -> SearchFilter {
    SearchFilter {
        candidate_budget: Some(rows),
        ..SearchFilter::default()
    }
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| v.to_string()).collect()
}

#[test]
fn candidates_return_the_same_evidence_as_a_full_scan() {
    let home = tempfile::tempdir().unwrap();
    corpus(home.path(), "book", 0, DOCUMENTS);
    let harness = Harness::new(home, &["book"]);
    let book = names(&["book"]);
    for query in ["consistency", "throughput", "throughput consistency"] {
        let full = harness.search(query, &book, &SearchFilter::default());
        let selected = harness.search(query, &book, &candidates(100_000));
        // Items carry score, snippet byte range and content hash, so equal values
        // prove the strategies agree on membership, ordering and evidence.
        assert_eq!(
            full.data["items"], selected.data["items"],
            "candidates-v1 diverged from the full scan for {query:?}"
        );
        assert_eq!(selected.data["retrieval_strategy"], "candidates-v1");
        assert!(
            counter(&selected, "candidate_rows_read") <= counter(&selected, "candidate_rows_limit"),
            "a generous budget must not be exceeded"
        );
    }
}

#[test]
fn a_broad_gram_does_not_starve_document_materialization() {
    let home = tempfile::tempdir().unwrap();
    corpus(home.path(), "book", 0, DOCUMENTS);
    let harness = Harness::new(home, &["book"]);
    let book = names(&["book"]);
    let query = "throughput consistency";
    let full = harness.search(query, &book, &SearchFilter::default());
    let complete = ids(&full);
    assert_eq!(complete.len(), DOCUMENTS / 6);
    // The shared gram's postings already exceed this budget. Ordering the
    // intersection by selectivity keeps the narrow gram's documents reachable,
    // and the reserved rows let them be loaded instead of being cut off after
    // the postings consumed everything.
    let tight = harness.search(query, &book, &candidates(500));
    assert!(
        counter(&tight, "candidate_rows_read") <= 500,
        "candidate reads must stay within the requested budget"
    );
    assert!(
        !ids(&tight).is_empty(),
        "a budget that cannot read every posting list must still return the documents the narrow gram selected"
    );
    assert!(ids(&tight).iter().all(|id| complete.contains(id)));
    assert!(tight
        .diagnostics
        .iter()
        .any(|d| d.code == "CANDIDATE_LIMIT"));
    assert!(tight.page["total"].is_null());
}

#[test]
fn an_incomplete_candidate_budget_never_invents_evidence_or_a_total() {
    let home = tempfile::tempdir().unwrap();
    corpus(home.path(), "book", 0, DOCUMENTS);
    let harness = Harness::new(home, &["book"]);
    let book = names(&["book"]);
    let complete = ids(&harness.search("throughput", &book, &SearchFilter::default()));
    assert!(!complete.is_empty());
    for rows in [0, 4, 16] {
        let tight = harness.search("throughput", &book, &candidates(rows));
        assert!(counter(&tight, "candidate_rows_read") <= rows as u64);
        // Whatever survives must be part of the complete answer, and reduced
        // coverage must be stated rather than reported as a clean result.
        assert!(ids(&tight).iter().all(|id| complete.contains(id)));
        if tight.status != "ok" {
            assert!(tight
                .diagnostics
                .iter()
                .any(|d| d.code == "CANDIDATE_LIMIT"));
            assert!(tight.page["total"].is_null());
        }
    }
}

#[test]
fn candidate_rows_are_split_across_selected_bundles() {
    let home = tempfile::tempdir().unwrap();
    corpus(home.path(), "alpha", 0, DOCUMENTS);
    corpus(home.path(), "beta", DOCUMENTS, DOCUMENTS / 4);
    let harness = Harness::new(home, &["alpha", "beta"]);
    let both = names(&["alpha", "beta"]);
    let complete = ids(&harness.search("throughput", &both, &SearchFilter::default()));
    assert!(complete.len() > 2);
    // The budget is divided between bundles and the remainder goes to the first
    // ones, so the shares must sum to the shared limit and no bundle may read
    // past its own share, including the part reserved for materialization.
    for rows in [3, 7, 100] {
        let tight = harness.search("throughput", &both, &candidates(rows));
        let limits: Vec<u64> = tight
            .bundles
            .iter()
            .map(|bundle| bundle["candidate_row_limit"].as_u64().unwrap())
            .collect();
        assert_eq!(limits.len(), 2);
        assert_eq!(
            limits.iter().sum::<u64>(),
            rows as u64,
            "shares must sum to the shared budget"
        );
        for (position, bundle) in tight.bundles.iter().enumerate() {
            assert!(
                bundle["candidate_rows_read"].as_u64().unwrap() <= limits[position],
                "bundle {position} read past its share"
            );
        }
        assert!(ids(&tight).iter().all(|id| complete.contains(id)));
    }
}
