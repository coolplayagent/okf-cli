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
