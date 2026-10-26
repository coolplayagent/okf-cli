//! Application use cases depend on ports, never on an OS or storage implementation.
mod control;
mod filter;
mod index;
mod neighbors;
mod quality;
mod resources;
pub use filter::SearchFilter;
pub use index::{Index, IndexedDocument, Snapshot, MAX_INDEX_BYTES};
mod service;
use chrono::{DateTime, Utc};
pub use control::{interrupted, RequestControl, ScanBudget};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitRevision>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitRevision {
    pub reference: String,
    pub commit: String,
    pub bundle_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}
impl BundleBinding {
    /// Configuration changes must not expose historical cards from another root.
    pub fn cache_key(&self) -> String {
        okf_core::sha256(
            format!(
                "{}\0{}\0{}\0{}\0{}",
                self.id,
                self.kind,
                self.root,
                self.scope,
                self.git
                    .as_ref()
                    .map(|g| format!("{}:{}", g.commit, g.bundle_path))
                    .unwrap_or_default()
            )
            .as_bytes(),
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
    fn replace(&self, _old: &BundleBinding, _new: &BundleBinding) -> Result<()> {
        Err(okf_core::Error::new(
            "UNSUPPORTED_CAPABILITY",
            "registry update unavailable",
            2,
        ))
    }
}

pub trait BundleReader {
    /// Metadata only: never load or execute a declared resource.
    fn stat(
        &self,
        _binding: &BundleBinding,
        _path: &str,
        control: &RequestControl,
    ) -> Result<Option<ResourceMetadata>> {
        control.check()?;
        Err(okf_core::Error::new(
            "UNSUPPORTED_CAPABILITY",
            "resource metadata is unavailable",
            2,
        ))
    }
    fn probe(&self, binding: &BundleBinding) -> Result<()>;
    fn probe_controlled(&self, binding: &BundleBinding, control: &RequestControl) -> Result<()> {
        control.check()?;
        self.probe(binding)?;
        control.check()
    }
    fn list(
        &self,
        binding: &BundleBinding,
        directory: &str,
        budget: &mut ScanBudget<'_>,
    ) -> Result<Listing>;
    fn read(
        &self,
        binding: &BundleBinding,
        path: &str,
        max_bytes: usize,
        control: &RequestControl,
    ) -> Result<String>;
}

#[derive(Debug, Clone)]
pub struct ResourceMetadata {
    pub directory: bool,
    pub bytes: u64,
}

#[derive(Debug)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// An incomplete enumeration cannot support a revision-checked directory cursor.
    pub complete: bool,
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

/// Preserve readable bundle results when the observation cache cannot be opened.
pub struct UnavailableCache(pub okf_core::Error);
impl Cache for UnavailableCache {
    fn remember(&self, _: &str, _: &str, _: &str, _: &Card, _: &str) -> Result<()> {
        Err(self.0.clone())
    }
    fn status(&self, _: &str) -> Result<Value> {
        Err(self.0.clone())
    }
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
        self.partial_at(code, message, None);
    }
    fn partial_at(&mut self, code: &str, message: &str, path: Option<&str>) {
        self.status = "partial".into();
        self.exit_code = 7;
        self.diagnostics.push(Diagnostic::new(code, message, path));
    }
    pub fn issue(&mut self, error: &okf_core::Error, path: &str) {
        self.partial_at(&error.code, &error.message, Some(path));
    }
    pub fn unavailable(&mut self, binding: &BundleBinding, scope: &str) {
        self.bundles.push(json!({
            "bundle_id": binding.id, "alias": binding.name, "view_kind": if binding.git.is_some() { "snapshot" } else { "observed" },
            "upstream_revision": binding.git.as_ref().map(|g| &g.commit), "checked_at": null, "freshness": "unknown",
            "coverage": {"scope": scope, "metadata": "unknown", "body": "unknown", "graph": "unknown"}
        }));
    }
    pub fn observe(
        &mut self,
        binding: &BundleBinding,
        now: DateTime<Utc>,
        scope: &str,
        metadata: &str,
    ) {
        self.bundles.push(json!({
            "bundle_id": binding.id, "alias": binding.name, "view_kind": if binding.git.is_some() { "snapshot" } else { "observed" },
            "upstream_revision": binding.git.as_ref().map(|g| &g.commit), "checked_at": now.to_rfc3339(), "freshness": "checked",
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
