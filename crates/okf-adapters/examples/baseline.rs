//! cargo run --release -p okf-adapters --example baseline -- fixtures/upstream
use okf_adapters::{Filesystem, SqliteCache, SqliteIndex};
use okf_application::{
    App, BundleBinding, BundleReader, BundleRegistry, Listing, ReadOptions, RequestControl,
    ScanBudget,
};
use okf_core::Result;
use serde_json::json;
use std::{
    cell::Cell,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};

struct Registry(Vec<BundleBinding>);
impl BundleRegistry for Registry {
    fn list(&self) -> Result<Vec<BundleBinding>> {
        Ok(self.0.clone())
    }
    fn add(&self, _: &BundleBinding) -> Result<()> {
        unreachable!()
    }
    fn remove(&self, _: &str, _: &str) -> Result<()> {
        unreachable!()
    }
}
struct Meter {
    lists: Cell<usize>,
    reads: Cell<usize>,
    bytes: Cell<usize>,
}
impl BundleReader for Meter {
    fn probe(&self, b: &BundleBinding) -> Result<()> {
        Filesystem.probe(b)
    }
    fn list(&self, b: &BundleBinding, p: &str, budget: &mut ScanBudget<'_>) -> Result<Listing> {
        self.lists.set(self.lists.get() + 1);
        Filesystem.list(b, p, budget)
    }
    fn read(&self, b: &BundleBinding, p: &str, max: usize, c: &RequestControl) -> Result<String> {
        self.reads.set(self.reads.get() + 1);
        let text = Filesystem.read(b, p, max, c)?;
        self.bytes.set(self.bytes.get() + text.len());
        Ok(text)
    }
}
fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("fixture root"));
    let registry = Registry(
        ["google", "skills", "ruby"]
            .into_iter()
            .map(|name| BundleBinding {
                id: name.into(),
                name: name.into(),
                kind: "fs".into(),
                scope: "project".into(),
                git: None,
                root: root
                    .join(name)
                    .join("bundle")
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            })
            .collect(),
    );
    let home = tempfile::tempdir().unwrap();
    let control = RequestControl::new(Duration::from_secs(120), Arc::new(AtomicBool::new(false)));
    let options = ReadOptions {
        limit: 100,
        budget: 65536,
        max_scan: 10000,
        cursor: None,
    };
    let mut runs = vec![];
    for phase in ["empty_observation_cache", "populated_observation_cache"] {
        for query in ["knowledge", "认证", "executor.resource"] {
            let cache_home = tempfile::tempdir().unwrap();
            let cache = SqliteCache::open(&cache_home.path().join("index.sqlite")).unwrap();
            let meter = Meter {
                lists: Cell::new(0),
                reads: Cell::new(0),
                bytes: Cell::new(0),
            };
            let app = App {
                registry: &registry,
                reader: &meter,
                cache: &cache,
                now: chrono::Utc::now(),
                control: &control,
            };
            if phase == "populated_observation_cache" {
                app.search(query, &[], &options).unwrap();
                meter.lists.set(0);
                meter.reads.set(0);
                meter.bytes.set(0);
            }
            let start = Instant::now();
            let result = app.search(query, &[], &options).unwrap();
            runs.push(json!({"phase":phase,"query":query,"elapsed_us":start.elapsed().as_micros(),
                "directory_lists":meter.lists.get(),"file_reads":meter.reads.get(),"read_bytes":meter.bytes.get(),
                "returned":result.page["returned"],"status":result.status,"scan":result.budget}));
        }
    }
    let cache = SqliteCache::open(&home.path().join("index.sqlite")).unwrap();
    let index = SqliteIndex::open(&home.path().join("content.sqlite")).unwrap();
    let meter = Meter {
        lists: Cell::new(0),
        reads: Cell::new(0),
        bytes: Cell::new(0),
    };
    let app = App {
        registry: &registry,
        reader: &meter,
        cache: &cache,
        now: chrono::Utc::now(),
        control: &control,
    };
    for name in ["google", "skills", "ruby"] {
        assert_eq!(app.sync(name, &options, &index).unwrap().status, "ok");
    }
    for query in ["knowledge", "认证", "executor.resource"] {
        meter.lists.set(0);
        meter.reads.set(0);
        meter.bytes.set(0);
        let start = Instant::now();
        let mut returned = 0;
        for name in ["google", "skills", "ruby"] {
            returned += app
                .indexed_search(query, name, &options, &index, false)
                .unwrap()
                .page["returned"]
                .as_u64()
                .unwrap();
        }
        runs.push(json!({"phase":"synchronized_index","query":query,"elapsed_us":start.elapsed().as_micros(),"directory_lists":meter.lists.get(),"file_reads":meter.reads.get(),"read_bytes":meter.bytes.get(),"returned":returned}));
    }
    println!("{}",serde_json::to_string_pretty(&json!({"os":std::env::consts::OS,"arch":std::env::consts::ARCH,
        "note":"OS caches are not flushed; phases describe only the observation database. Timings are informational.","runs":runs})).unwrap());
}
