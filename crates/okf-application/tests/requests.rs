use chrono::Utc;
use okf_application::{
    App, BundleBinding, BundleReader, BundleRegistry, Cache, Entry, Listing, ReadOptions,
    RequestControl, ScanBudget,
};
use okf_core::{Card, Error, Result};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

struct Registry;
impl BundleRegistry for Registry {
    fn list(&self) -> Result<Vec<BundleBinding>> {
        Ok(vec![BundleBinding {
            id: "book".into(),
            name: "book".into(),
            root: "unused".into(),
            kind: "test".into(),
            scope: "project".into(),
            git: None,
        }])
    }
    fn add(&self, _: &BundleBinding) -> Result<()> {
        unreachable!()
    }
    fn remove(&self, _: &str, _: &str) -> Result<()> {
        unreachable!()
    }
}

struct Reader {
    cancelled: Arc<AtomicBool>,
    calls: AtomicUsize,
    stop_on_second: bool,
}
impl BundleReader for Reader {
    fn probe(&self, _: &BundleBinding) -> Result<()> {
        Ok(())
    }
    fn list(&self, _: &BundleBinding, _: &str, budget: &mut ScanBudget<'_>) -> Result<Listing> {
        budget.charge()?;
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(Listing {
            complete: true,
            entries: ["first.md", "second.md"]
                .into_iter()
                .map(|path| Entry {
                    path: path.into(),
                    directory: false,
                    bytes: 30,
                    revision: "1".into(),
                })
                .collect(),
        })
    }
    fn read(
        &self,
        _: &BundleBinding,
        path: &str,
        _: usize,
        control: &RequestControl,
    ) -> Result<String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.stop_on_second && path == "second.md" {
            self.cancelled.store(true, Ordering::Relaxed);
        }
        control.check()?;
        Ok("---\ntype: Note\ntitle: example\n---\nbody\n".into())
    }
}

struct ObservationCache {
    fail: bool,
}
impl Cache for ObservationCache {
    fn remember(&self, _: &str, _: &str, _: &str, _: &Card, _: &str) -> Result<()> {
        if self.fail {
            Err(Error::new("IO_ERROR", "injected cache failure", 5))
        } else {
            Ok(())
        }
    }
    fn status(&self, _: &str) -> Result<Value> {
        Ok(json!({}))
    }
}

fn options() -> ReadOptions {
    ReadOptions {
        limit: 20,
        budget: 16 * 1024,
        max_scan: 100,
        cursor: None,
    }
}

struct MultiRegistry;
impl BundleRegistry for MultiRegistry {
    fn list(&self) -> Result<Vec<BundleBinding>> {
        let template = Registry.list()?.remove(0);
        Ok(["first", "second", "third"]
            .into_iter()
            .map(|name| BundleBinding {
                id: name.into(),
                name: name.into(),
                ..template.clone()
            })
            .collect())
    }
    fn add(&self, _: &BundleBinding) -> Result<()> {
        unreachable!()
    }
    fn remove(&self, _: &str, _: &str) -> Result<()> {
        unreachable!()
    }
}

struct InterruptingIndex {
    cancelled: Arc<AtomicBool>,
    calls: AtomicUsize,
}
impl okf_application::Index for InterruptingIndex {
    fn verify(
        &self,
        _: &str,
        _: usize,
        _: &RequestControl,
    ) -> Result<Option<okf_application::IndexVerification>> {
        self.cancelled.store(true, Ordering::Relaxed);
        Ok(Some(okf_application::IndexVerification {
            generation: "v1".into(),
            documents_total: 3,
            documents_checked: 1,
            bytes_checked: 30,
            complete: false,
            failures: 0,
            findings: vec![],
            interruption: Some(Error::new("CANCELLED", "cancelled during verification", 7)),
        }))
    }
    fn load(&self, _: &str) -> Result<Option<okf_application::Snapshot>> {
        panic!("full snapshot load is forbidden")
    }
    fn stats(&self, _: &str) -> Result<Option<okf_application::IndexStats>> {
        Ok(Some(okf_application::IndexStats {
            generation: "v1".into(),
            upstream_revision: None,
            checked_at: "2030-01-01T00:00:00Z".into(),
            documents: 1,
            concepts: 1,
            bytes: 30,
        }))
    }
    fn read(&self, _: &str, _: &str) -> Result<Option<okf_application::Snapshot>> {
        unreachable!()
    }
    fn search(&self, _: &str, _: &str, limit: usize) -> Result<Option<okf_application::Snapshot>> {
        assert_eq!(limit, 1);
        if self.calls.fetch_add(1, Ordering::Relaxed) == 1 {
            self.cancelled.store(true, Ordering::Relaxed);
            return Err(Error::new(
                "CANCELLED",
                "cancelled during second index read",
                7,
            ));
        }
        let raw = "---\ntype: Note\n---\nexample".to_string();
        Ok(Some(okf_application::Snapshot {
            generation: "v1".into(),
            upstream_revision: None,
            checked_at: "2030-01-01T00:00:00Z".into(),
            documents: vec![okf_application::IndexedDocument {
                path: "note.md".into(),
                hash: okf_core::sha256(raw.as_bytes()),
                raw,
            }],
        }))
    }
    fn publish(&self, _: &str, _: Option<&str>, _: &okf_application::Snapshot) -> Result<()> {
        unreachable!()
    }
    fn clear(&self, _: &str) -> Result<()> {
        unreachable!()
    }
}

#[test]
fn index_verification_preserves_partial_progress_on_cancellation() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let control = RequestControl::new(Duration::from_secs(30), cancelled.clone());
    let reader = Reader {
        cancelled: cancelled.clone(),
        calls: AtomicUsize::new(0),
        stop_on_second: false,
    };
    let index = InterruptingIndex {
        cancelled,
        calls: AtomicUsize::new(0),
    };
    let app = App {
        registry: &Registry,
        reader: &reader,
        cache: &ObservationCache { fail: false },
        now: Utc::now(),
        control: &control,
    };
    let response = app.index_verify("book", 100, &index).unwrap();
    assert_eq!(response.exit_code, 7);
    assert_eq!(response.data["documents_checked"], 1);
    assert_eq!(response.data["complete"], false);
    assert!(response.data["valid"].is_null());
    assert_eq!(response.diagnostics[0].code, "CANCELLED");
    assert_eq!(reader.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn multi_bundle_cancellation_preserves_finished_evidence_and_stops_index_io() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let control = RequestControl::new(Duration::from_secs(30), cancelled.clone());
    let reader = Reader {
        cancelled: cancelled.clone(),
        calls: AtomicUsize::new(0),
        stop_on_second: false,
    };
    let index = InterruptingIndex {
        cancelled,
        calls: AtomicUsize::new(0),
    };
    let app = App {
        registry: &MultiRegistry,
        reader: &reader,
        cache: &ObservationCache { fail: false },
        now: Utc::now(),
        control: &control,
    };
    let response = app
        .indexed_search_bundles_filtered(
            "example",
            &["third".into(), "first".into(), "second".into()],
            &options(),
            &index,
            false,
            &Default::default(),
        )
        .unwrap();
    assert_eq!(response.exit_code, 7);
    assert_eq!(response.data["items"].as_array().unwrap().len(), 1);
    assert_eq!(response.data["items"][0]["ref"], "@first/note");
    assert_eq!(
        response.data["coverage_gaps"]["unavailable_bundles"],
        json!(["second", "third"])
    );
    assert!(response.page["total"].is_null());
    assert_eq!(index.calls.load(Ordering::Relaxed), 2);
    assert_eq!(reader.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn cancellation_retains_completed_search_and_browse_cards() {
    for browse in [false, true] {
        let cancelled = Arc::new(AtomicBool::new(false));
        let control = RequestControl::new(Duration::from_secs(30), cancelled.clone());
        let reader = Reader {
            cancelled,
            calls: AtomicUsize::new(0),
            stop_on_second: true,
        };
        let app = App {
            registry: &Registry,
            reader: &reader,
            cache: &ObservationCache { fail: false },
            now: Utc::now(),
            control: &control,
        };
        let response = if browse {
            app.browse("@book", &options())
        } else {
            app.search("example", &[], &options())
        }
        .unwrap();
        assert_eq!(response.status, "partial");
        assert_eq!(response.exit_code, 7);
        assert_eq!(response.data["items"].as_array().unwrap().len(), 1);
        assert_eq!(response.data["items"][0]["id"], "first");
        assert!(response.diagnostics.iter().any(|d| d.code == "CANCELLED"));
        assert!(!response.diagnostics.iter().any(|d| d.code == "SCAN_LIMIT"));
        assert_eq!(response.bundles[0]["coverage"]["metadata"], "partial");
        assert_eq!(reader.calls.load(Ordering::Relaxed), 3);
    }
}

#[test]
fn deadline_returns_unknown_coverage_without_reader_io() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let control = RequestControl::new(Duration::ZERO, cancelled.clone());
    let reader = Reader {
        cancelled,
        calls: AtomicUsize::new(0),
        stop_on_second: false,
    };
    let app = App {
        registry: &Registry,
        reader: &reader,
        cache: &ObservationCache { fail: false },
        now: Utc::now(),
        control: &control,
    };
    for response in [
        app.context(&[], &options()).unwrap(),
        app.search("example", &[], &options()).unwrap(),
        app.validate("book", &options()).unwrap(),
    ] {
        assert_eq!(response.exit_code, 7);
        assert_eq!(response.diagnostics[0].code, "DEADLINE_EXCEEDED");
        assert_eq!(response.bundles[0]["coverage"]["metadata"], "unknown");
    }
    assert_eq!(reader.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn cache_failure_preserves_all_cards_and_raw_document() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let control = RequestControl::new(Duration::from_secs(30), cancelled.clone());
    let reader = Reader {
        cancelled,
        calls: AtomicUsize::new(0),
        stop_on_second: false,
    };
    let app = App {
        registry: &Registry,
        reader: &reader,
        cache: &ObservationCache { fail: true },
        now: Utc::now(),
        control: &control,
    };
    let response = app.search("example", &[], &options()).unwrap();
    assert_eq!(response.exit_code, 7);
    assert_eq!(response.data["items"].as_array().unwrap().len(), 2);
    assert_eq!(response.bundles[0]["coverage"]["metadata"], "complete");
    assert!(response
        .diagnostics
        .iter()
        .all(|d| d.code == "CACHE_WRITE_FAILED"));
    let response = app.browse("@book", &options()).unwrap();
    assert_eq!(response.exit_code, 7);
    assert_eq!(response.data["items"].as_array().unwrap().len(), 2);
    assert_eq!(response.bundles[0]["coverage"]["metadata"], "complete");
    let response = app.read("@book/first", false, &options()).unwrap();
    assert_eq!(response.exit_code, 7);
    assert_eq!(
        response.data["document"]["text"],
        "---\ntype: Note\ntitle: example\n---\nbody\n"
    );
}

#[test]
fn cancelled_validation_keeps_counts_and_reports_incomplete_coverage() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let control = RequestControl::new(Duration::from_secs(30), cancelled.clone());
    let reader = Reader {
        cancelled,
        calls: AtomicUsize::new(0),
        stop_on_second: true,
    };
    let app = App {
        registry: &Registry,
        reader: &reader,
        cache: &ObservationCache { fail: false },
        now: Utc::now(),
        control: &control,
    };
    let response = app.validate("book", &options()).unwrap();
    assert_eq!(response.exit_code, 7);
    assert_eq!(response.data["checked"], 1);
    assert_eq!(response.data["failures"], 0);
    assert_eq!(response.diagnostics[0].code, "CANCELLED");
    assert_eq!(response.bundles[0]["coverage"]["metadata"], "partial");
}

#[test]
fn cancellation_during_last_cache_write_is_reported() {
    struct CancellingCache(Arc<AtomicBool>);
    impl Cache for CancellingCache {
        fn remember(&self, _: &str, path: &str, _: &str, _: &Card, _: &str) -> Result<()> {
            if path == "second.md" {
                self.0.store(true, Ordering::Relaxed);
            }
            Ok(())
        }
        fn status(&self, _: &str) -> Result<Value> {
            unreachable!()
        }
    }
    for browse in [false, true] {
        let cancelled = Arc::new(AtomicBool::new(false));
        let control = RequestControl::new(Duration::from_secs(30), cancelled.clone());
        let reader = Reader {
            cancelled: cancelled.clone(),
            calls: AtomicUsize::new(0),
            stop_on_second: false,
        };
        let cache = CancellingCache(cancelled);
        let app = App {
            registry: &Registry,
            reader: &reader,
            cache: &cache,
            now: Utc::now(),
            control: &control,
        };
        let response = if browse {
            app.browse("@book", &options())
        } else {
            app.search("example", &[], &options())
        }
        .unwrap();
        assert_eq!(response.exit_code, 7);
        assert_eq!(response.data["items"].as_array().unwrap().len(), 2);
        assert_eq!(response.diagnostics[0].code, "CANCELLED");
        assert_eq!(response.bundles[0]["coverage"]["metadata"], "partial");
    }
}

#[test]
fn cancellation_before_link_checks_preserves_completed_metadata_coverage() {
    struct LateCancelReader(Reader);
    impl BundleReader for LateCancelReader {
        fn probe(&self, binding: &BundleBinding) -> Result<()> {
            self.0.probe(binding)
        }
        fn list(
            &self,
            binding: &BundleBinding,
            directory: &str,
            budget: &mut ScanBudget<'_>,
        ) -> Result<Listing> {
            self.0.list(binding, directory, budget)
        }
        fn read(
            &self,
            binding: &BundleBinding,
            path: &str,
            max_bytes: usize,
            control: &RequestControl,
        ) -> Result<String> {
            let raw = self.0.read(binding, path, max_bytes, control)?;
            if path == "second.md" {
                self.0.cancelled.store(true, Ordering::Relaxed);
            }
            Ok(raw.replace("body", "[target](first.md)"))
        }
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let control = RequestControl::new(Duration::from_secs(30), cancelled.clone());
    let reader = LateCancelReader(Reader {
        cancelled,
        calls: AtomicUsize::new(0),
        stop_on_second: false,
    });
    let app = App {
        registry: &Registry,
        reader: &reader,
        cache: &ObservationCache { fail: false },
        now: Utc::now(),
        control: &control,
    };
    let response = app.validate("book", &options()).unwrap();
    assert_eq!(response.exit_code, 7);
    assert_eq!(response.data["checked"], 2);
    assert_eq!(response.data["link_check"]["complete"], false);
    assert_eq!(response.bundles[0]["coverage"]["metadata"], "complete");
    assert_eq!(response.diagnostics.len(), 1);
    assert_eq!(response.diagnostics[0].code, "CANCELLED");
}

#[test]
fn denied_resource_is_not_reported_as_missing_or_incomplete_metadata() {
    struct ResourceReader;
    impl BundleReader for ResourceReader {
        fn probe(&self, _: &BundleBinding) -> Result<()> {
            Ok(())
        }
        fn list(&self, _: &BundleBinding, _: &str, budget: &mut ScanBudget<'_>) -> Result<Listing> {
            budget.charge()?;
            Ok(Listing {
                complete: true,
                entries: vec![Entry {
                    path: "compute.md".into(),
                    directory: false,
                    bytes: 100,
                    revision: "1".into(),
                }],
            })
        }
        fn read(
            &self,
            _: &BundleBinding,
            _: &str,
            _: usize,
            control: &RequestControl,
        ) -> Result<String> {
            control.check()?;
            Ok(
                "---\ntype: Attested Computation\nruntime: python\ncomputation: code.py\n---\n"
                    .into(),
            )
        }
        fn stat(
            &self,
            _: &BundleBinding,
            _: &str,
            _: &RequestControl,
        ) -> Result<Option<okf_application::ResourceMetadata>> {
            Err(Error::new("ACCESS_DENIED", "injected permission denial", 4))
        }
    }
    let control = RequestControl::new(Duration::from_secs(30), Arc::new(AtomicBool::new(false)));
    let app = App {
        registry: &Registry,
        reader: &ResourceReader,
        cache: &ObservationCache { fail: false },
        now: Utc::now(),
        control: &control,
    };
    let response = app.validate("book", &options()).unwrap();
    assert_eq!(response.exit_code, 7);
    assert_eq!(response.data["resource_check"]["complete"], false);
    assert_eq!(response.bundles[0]["coverage"]["metadata"], "complete");
    assert!(response
        .diagnostics
        .iter()
        .any(|d| d.code == "ACCESS_DENIED"));
    assert!(!response
        .diagnostics
        .iter()
        .any(|d| d.code == "MISSING_RESOURCE"));
}
