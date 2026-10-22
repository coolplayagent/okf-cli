//! Application use cases depend on ports, never on an OS or storage implementation.
mod service;
use chrono::{DateTime, Utc};
use okf_core::{Card, Diagnostic, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
pub use service::App;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleBinding {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub root: String,
    pub scope: String,
}
impl BundleBinding {
    /// Configuration changes must not expose historical cards from another root.
    pub fn cache_key(&self) -> String {
        okf_core::sha256(
            format!("{}\0{}\0{}\0{}", self.id, self.kind, self.root, self.scope).as_bytes(),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub directory: bool,
    pub bytes: u64,
    pub revision: String,
}

pub trait BundleRegistry {
    fn list(&self) -> Result<Vec<BundleBinding>>;
    fn add(&self, binding: &BundleBinding) -> Result<()>;
    fn remove(&self, name: &str, scope: &str) -> Result<()>;
}

pub trait BundleReader {
    fn probe(&self, binding: &BundleBinding) -> Result<()>;
    fn list(&self, binding: &BundleBinding, directory: &str) -> Result<Vec<Entry>>;
    fn read(&self, binding: &BundleBinding, path: &str, max_bytes: usize) -> Result<String>;
}

pub trait Cache {
    fn remember(
        &self,
        bundle_id: &str,
        path: &str,
        hash: &str,
        card: &Card,
        checked_at: &str,
    ) -> Result<()>;
    fn status(&self, bundle_id: &str) -> Result<Value>;
}

#[derive(Debug, Clone)]
pub struct ReadOptions {
    pub limit: usize,
    pub budget: usize,
    pub max_scan: usize,
    pub cursor: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct Response {
    pub schema_version: &'static str,
    pub request_id: String,
    pub command: String,
    pub status: String,
    pub data: Value,
    pub bundles: Vec<Value>,
    pub page: Value,
    pub budget: Value,
    pub diagnostics: Vec<Diagnostic>,
    pub next: Vec<Value>,
    #[serde(skip)]
    pub exit_code: u8,
}
impl Response {
    pub fn new(command: &str, data: Value) -> Self {
        Self {
            schema_version: "2.0",
            request_id: String::new(),
            command: command.into(),
            status: "ok".into(),
            data,
            bundles: vec![],
            page: json!({"returned": 0, "total": null, "has_more": false, "next_cursor": null}),
            budget: json!({}),
            diagnostics: vec![],
            next: vec![],
            exit_code: 0,
        }
    }
    pub fn partial(&mut self, code: &str, message: &str) {
        self.status = "partial".into();
        self.exit_code = 7;
        self.diagnostics.push(Diagnostic::new(code, message, None));
    }
    pub fn observe(
        &mut self,
        binding: &BundleBinding,
        now: DateTime<Utc>,
        scope: &str,
        metadata: &str,
    ) {
        self.bundles.push(json!({
            "bundle_id": binding.id, "alias": binding.name, "view_kind": "observed",
            "upstream_revision": null, "checked_at": now.to_rfc3339(), "freshness": "checked",
            "coverage": {"scope": scope, "metadata": metadata, "body": "partial", "graph": "partial"}
        }));
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Cursor {
    pub bundle_id: String,
    pub operation: String,
    pub target: String,
    pub revision: String,
    pub offset: usize,
}
