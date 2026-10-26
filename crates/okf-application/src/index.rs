//! Explicit, bounded snapshot synchronization and offline consumption.
use crate::{App, BundleBinding, ReadOptions, Response, ScanBudget};
use okf_core::{reserved, sha256, validate_reserved, Document, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};

pub const MAX_INDEX_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_INDEX_DOCUMENTS: usize = 100_000;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedDocument {
    pub path: String,
    pub hash: String,
    pub raw: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub generation: String,
    pub upstream_revision: Option<String>,
    pub checked_at: String,
    pub documents: Vec<IndexedDocument>,
}
#[derive(Debug, Clone)]
pub struct IndexStats {
    pub generation: String,
    pub upstream_revision: Option<String>,
    pub checked_at: String,
    pub documents: usize,
    pub concepts: usize,
    pub bytes: usize,
}
#[derive(Debug)]
pub struct IndexVerification {
    pub generation: String,
    pub documents_total: usize,
    pub documents_checked: usize,
    pub bytes_checked: usize,
    pub complete: bool,
    pub failures: usize,
    pub findings: Vec<(String, Error)>,
    pub interruption: Option<Error>,
}

pub fn index_generation<'a>(
    revision: Option<&str>,
    entries: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<String> {
    Ok(sha256(
        &serde_json::to_vec(&(revision, entries.into_iter().collect::<Vec<_>>()))
            .map_err(Error::io)?,
    ))
}

pub trait Index {
    fn load(&self, key: &str) -> Result<Option<Snapshot>>;
    /// Metadata and counts only; does not load or verify document bodies.
    fn stats(&self, key: &str) -> Result<Option<IndexStats>>;
    /// Header and requested document are read in one transaction.
    fn read(&self, key: &str, path: &str) -> Result<Option<Snapshot>>;
    /// Read at most `limit` concepts from the expected generation, ordered by path.
    fn search(&self, key: &str, generation: &str, limit: usize) -> Result<Option<Snapshot>>;
    /// Inspect a bounded inventory in one snapshot, retaining findings and progress on interruption.
    fn verify(
        &self,
        key: &str,
        limit: usize,
        control: &crate::RequestControl,
    ) -> Result<Option<IndexVerification>>;
    /// Compare-and-swap publication. Incomplete scans must never call this method.
    fn publish(&self, key: &str, expected: Option<&str>, snapshot: &Snapshot) -> Result<()>;
    fn clear(&self, key: &str) -> Result<()>;
}
impl BundleBinding {
    pub fn index_key(&self) -> String {
        sha256(
            format!(
                "{}\0{}\0{}\0{}\0{}",
                self.id,
                self.kind,
                self.root,
                self.scope,
                self.git
                    .as_ref()
                    .map(|g| g.bundle_path.as_str())
                    .unwrap_or("")
            )
            .as_bytes(),
        )
    }
}
impl App<'_> {
    pub fn sync(&self, name: &str, options: &ReadOptions, index: &dyn Index) -> Result<Response> {
        let binding = self.binding(name)?;
        let key = binding.index_key();
        let previous = index.stats(&key)?;
        let mut response = Response::new(
            "sync",
            json!({"published":false,"previous_generation":previous.as_ref().map(|s| &s.generation)}),
        );
        let mut scan = ScanBudget::new(options.max_scan, self.control);
        let mut queue = VecDeque::from([String::new()]);
        let mut documents = BTreeMap::new();
        let mut bytes = 0;
        let mut complete = true;
        'scan: while let Some(directory) = queue.pop_front() {
            let listing = match self.reader.list(&binding, &directory, &mut scan) {
                Ok(listing) => listing,
                Err(error) => {
                    response.issue(&error, &format!("@{name}/{directory}"));
                    complete = false;
                    break;
                }
            };
            if !listing.complete {
                complete = false;
                response.partial("SCAN_LIMIT", "incomplete scan; previous index retained");
            }
            for entry in listing.entries {
                if entry.directory {
                    queue.push_back(entry.path);
                    continue;
                }
                match self
                    .reader
                    .read(&binding, &entry.path, 2 * 1024 * 1024, self.control)
                    .and_then(|raw| {
                        if reserved(&entry.path) {
                            validate_reserved(&entry.path, &raw)?;
                        } else {
                            Document::parse(&raw)?;
                        }
                        Ok(raw)
                    }) {
                    Ok(raw) => {
                        if raw.len() > MAX_INDEX_BYTES - bytes {
                            complete = false;
                            response.partial(
                                "INDEX_LIMIT",
                                "index exceeds 64 MiB; previous index retained",
                            );
                            break 'scan;
                        }
                        bytes += raw.len();
                        documents.insert(
                            entry.path.clone(),
                            IndexedDocument {
                                path: entry.path,
                                hash: sha256(raw.as_bytes()),
                                raw,
                            },
                        );
                    }
                    Err(error) => {
                        complete = false;
                        response.issue(&error, &format!("@{name}/{}", entry.path));
                        if crate::interrupted(&error) {
                            break 'scan;
                        }
                    }
                }
            }
        }
        if let Err(error) = self.control.check() {
            complete = false;
            if !response.diagnostics.iter().any(|d| d.code == error.code) {
                response.partial(&error.code, &error.message);
            }
        }
        response.budget = json!({"scan_limit":options.max_scan,"scanned_entries":scan.used(),"index_bytes_limit":MAX_INDEX_BYTES,"index_bytes_used":bytes});
        response.observe(
            &binding,
            self.now,
            "",
            if complete { "complete" } else { "partial" },
        );
        response.data["documents"] = json!(documents.len());
        if complete {
            let documents: Vec<_> = documents.into_values().collect();
            let revision = binding.git.as_ref().map(|g| g.commit.clone());
            let generation = index_generation(
                revision.as_deref(),
                documents.iter().map(|d| (d.path.as_str(), d.hash.as_str())),
            )?;
            let snapshot = Snapshot {
                generation,
                upstream_revision: revision,
                checked_at: self.now.to_rfc3339(),
                documents,
            };
            self.control.check()?;
            index.publish(
                &key,
                previous.as_ref().map(|s| s.generation.as_str()),
                &snapshot,
            )?;
            response.data["published"] = json!(true);
            response.data["generation"] = json!(snapshot.generation);
        }
        Ok(response)
    }

    pub(crate) fn snapshot(
        &self,
        name: &str,
        index: &dyn Index,
    ) -> Result<(BundleBinding, Snapshot)> {
        self.control.check()?;
        let binding = self.binding(name)?;
        let snapshot = index
            .load(&binding.index_key())?
            .ok_or_else(|| Error::new("INDEX_MISSING", "run sync for this bundle first", 3))?;
        self.control.check()?;
        Ok((binding, snapshot))
    }

    pub fn index_status(&self, name: &str, index: &dyn Index) -> Result<Response> {
        self.control.check()?;
        let binding = self.binding(name)?;
        let snapshot = index
            .stats(&binding.index_key())?
            .ok_or_else(|| Error::new("INDEX_MISSING", "run sync for this bundle first", 3))?;
        self.control.check()?;
        Ok(Response::new(
            "index status",
            json!({"generation":snapshot.generation,"documents":snapshot.documents,
            "bytes":snapshot.bytes,"checked_at":snapshot.checked_at,
            "upstream_revision":snapshot.upstream_revision,"matches_binding_revision":snapshot.upstream_revision==binding.git.as_ref().map(|g|g.commit.clone()),
            "view_kind":if binding.git.is_some() {"snapshot"} else {"observed"}}),
        ))
    }

    pub fn index_verify(&self, name: &str, limit: usize, index: &dyn Index) -> Result<Response> {
        self.control.check()?;
        let binding = self.binding(name)?;
        let report = index
            .verify(&binding.index_key(), limit, self.control)?
            .ok_or_else(|| Error::new("INDEX_MISSING", "run sync for this bundle first", 3))?;
        let valid = if report.failures > 0 {
            Some(false)
        } else if report.complete {
            Some(true)
        } else {
            None
        };
        let mut response = Response::new(
            "index verify",
            json!({
                "generation": report.generation, "documents_total": report.documents_total,
                "documents_checked": report.documents_checked, "complete": report.complete,
                "valid": valid, "failures": report.failures,
                "findings_truncated": report.failures > report.findings.len(), "live_files_checked": false
            }),
        );
        for (path, error) in &report.findings {
            response.issue(
                error,
                &if path.is_empty() {
                    format!("@{name}")
                } else {
                    format!("@{name}/{path}")
                },
            );
        }
        if let Some(error) = &report.interruption {
            response.issue(error, &format!("@{name}"));
        }
        if report.complete && report.failures > 0 {
            response.status = "error".into();
            response.exit_code = 5;
        }
        response.budget = json!({"scan_limit":limit,"indexed_documents_scanned":report.documents_checked,
            "index_bytes_limit":MAX_INDEX_BYTES,"index_bytes_checked":report.bytes_checked});
        if report.failures > 0 {
            response
                .next
                .push(json!({"operation":"sync","target":name}));
        }
        Ok(response)
    }

    pub fn indexed_search(
        &self,
        query: &str,
        name: &str,
        options: &ReadOptions,
        index: &dyn Index,
        context: bool,
    ) -> Result<Response> {
        self.indexed_search_filtered(
            query,
            name,
            options,
            index,
            context,
            &crate::SearchFilter::default(),
        )
    }

    pub fn indexed_search_filtered(
        &self,
        query: &str,
        name: &str,
        options: &ReadOptions,
        index: &dyn Index,
        context: bool,
        filter: &crate::SearchFilter,
    ) -> Result<Response> {
        self.indexed_search_bundles_filtered(
            query,
            &[name.to_owned()],
            options,
            index,
            context,
            filter,
        )
    }

    pub fn indexed_search_bundles_filtered(
        &self,
        query: &str,
        selected: &[String],
        options: &ReadOptions,
        index: &dyn Index,
        context: bool,
        filter: &crate::SearchFilter,
    ) -> Result<Response> {
        let terms: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
        if terms.is_empty() {
            return Err(Error::new("EMPTY_QUERY", "query must not be empty", 2));
        }
        if selected.is_empty() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "indexed queries require at least one --bundle",
                2,
            ));
        }
        let mut bindings = self.selected_bindings(selected)?;
        bindings.sort_by(|a, b| a.name.cmp(&b.name));
        let single = bindings.len() == 1;
        let evaluated_at = filter.as_of.unwrap_or(self.now);
        let mut response = Response::new(
            if context { "context" } else { "search" },
            json!({"query":query,"mode":"full_text","items":[],"evaluated_at":evaluated_at.to_rfc3339(),
                "disclosure":if context {"selected_excerpts"} else {"indexed_snippets"}}),
        );
        if !single {
            response.data["generations"] = json!({});
        }
        let mut complete = true;
        let mut available = vec![];
        for binding in bindings {
            let result = self
                .control
                .check()
                .and_then(|()| index.stats(&binding.index_key()))
                .and_then(|stats| {
                    stats.ok_or_else(|| {
                        Error::new("INDEX_MISSING", "run sync for this bundle first", 3)
                    })
                });
            match result {
                Ok(stats) => available.push((binding, stats)),
                Err(error) if single => return Err(error),
                Err(error) => {
                    response.issue(&error, &format!("@{}", binding.name));
                    response.unavailable(&binding, "");
                    complete = false;
                }
            }
        }
        // Round-robin allocation redistributes unused shares from small bundles.
        let mut quotas = vec![0; available.len()];
        let mut remaining = options.max_scan;
        while remaining > 0 {
            let mut allocated = false;
            for ((_, stats), quota) in available.iter().zip(&mut quotas) {
                if remaining > 0 && *quota < stats.concepts {
                    *quota += 1;
                    remaining -= 1;
                    allocated = true;
                }
            }
            if !allocated {
                break;
            }
        }
        let mut candidates = vec![];
        let mut scanned = 0;
        for ((binding, stats), quota) in available.into_iter().zip(quotas) {
            let name = &binding.name;
            let result = self
                .control
                .check()
                .and_then(|()| index.search(&binding.index_key(), &stats.generation, quota))
                .and_then(|snapshot| {
                    snapshot.ok_or_else(|| {
                        Error::new("INDEX_MISSING", "run sync for this bundle first", 3)
                    })
                });
            let snapshot = match result {
                Ok(snapshot) => snapshot,
                Err(error) if single => return Err(error),
                Err(error) => {
                    response.issue(&error, &format!("@{name}"));
                    response.unavailable(&binding, "");
                    complete = false;
                    continue;
                }
            };
            if single {
                response.data["generation"] = json!(snapshot.generation);
            } else {
                response.data["generations"][name] = json!(snapshot.generation);
            }
            let mut bundle_complete = quota >= stats.concepts;
            if !bundle_complete {
                response.issue(
                    &Error::new(
                        "SCAN_LIMIT",
                        "indexed document evaluation budget reached",
                        7,
                    ),
                    &format!("@{name}"),
                );
            }
            let bundle_start = scanned;
            for document in &snapshot.documents {
                if let Err(error) = self.control.check() {
                    bundle_complete = false;
                    response.issue(&error, &format!("@{name}"));
                    break;
                }
                scanned += 1;
                let doc = match Document::parse(&document.raw) {
                    Ok(doc) => doc,
                    Err(error) if single => return Err(error),
                    Err(error) => {
                        response.issue(&error, &format!("@{name}/{}", document.path));
                        bundle_complete = false;
                        continue;
                    }
                };
                let id = document.path.strip_suffix(".md").unwrap_or(&document.path);
                let card = doc.card(id, evaluated_at);
                if !filter.matches(&serde_json::to_value(&card).map_err(Error::io)?) {
                    continue;
                }
                let metadata = serde_json::to_string(&card)
                    .map_err(Error::io)?
                    .to_lowercase();
                let body = &document.raw[doc.body_start..];
                let lower = body.to_lowercase();
                if !terms
                    .iter()
                    .all(|t| metadata.contains(t) || lower.contains(t))
                {
                    continue;
                }
                response.diagnostics.extend(self.document_diagnostics(
                    &binding,
                    &document.path,
                    &document.raw,
                    &doc,
                ));
                let score = terms
                    .iter()
                    .map(|t| if metadata.contains(t) { 5 } else { 1 })
                    .sum::<usize>();
                let (start, end, boundaries_complete) = excerpt(
                    &document.raw,
                    doc.body_start,
                    &terms,
                    if context { 4096 } else { 512 },
                    context,
                );
                if !boundaries_complete
                    && !response
                        .diagnostics
                        .iter()
                        .any(|d| d.code == "SECTION_LIMIT")
                {
                    response.partial(
                        "SECTION_LIMIT",
                        "heading inventory truncated; excerpt boundaries may be incomplete",
                    );
                }
                let mut item = serde_json::to_value(card).map_err(Error::io)?;
                item["bundle_id"] = json!(binding.id);
                item["ref"] = json!(format!("@{name}/{id}"));
                item["content_sha256"] = json!(document.hash);
                item["upstream_revision"] = json!(snapshot.upstream_revision);
                item["generation"] = json!(snapshot.generation);
                item["score"] = json!(score);
                item["selection_reason"] =
                    json!("all query terms matched; metadata matches rank above body-only matches");
                item["snippet"] = json!({"path":document.path,"byte_start":start,"byte_end":end,"text":&document.raw[start..end]});
                candidates.push((score, item));
            }
            complete &= bundle_complete;
            indexed_observation(&mut response, &binding, &snapshot, bundle_complete);
            let observation = response.bundles.last_mut().unwrap();
            observation["indexed_documents_scanned"] = json!(scanned - bundle_start);
            observation["scan_limit"] = json!(quota);
        }

        candidates.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1["ref"].as_str().cmp(&b.1["ref"].as_str()))
        });
        let total = candidates.len();
        let mut used = 0;
        let mut items = vec![];
        for (_, item) in candidates {
            let size = serde_json::to_vec(&item).map_err(Error::io)?.len();
            if items.len() >= options.limit || size > options.budget.saturating_sub(used) {
                continue;
            }
            used += size;
            response.next.push(
                json!({"operation":"read","target":item["ref"],"indexed":true,"generation":item["generation"]}),
            );
            items.push(item);
        }
        if items.len() < total {
            response.partial(
                "RESULT_LIMIT",
                "selection does not include all matches; increase budget or narrow query",
            );
        }
        response.page["returned"] = json!(items.len());
        response.page["total"] = if complete { json!(total) } else { Value::Null };
        response.data["items"] = json!(items);
        response.data["coverage_gaps"] = json!({"unselected_matches":total-response.data["items"].as_array().unwrap().len(),"live_files_checked":false});
        response.budget = json!({"content_bytes_limit":options.budget,"content_bytes_used":used,"scan_limit":options.max_scan,"indexed_documents_scanned":scanned});
        if !single {
            response.data["coverage_gaps"]["unavailable_bundles"] = json!(response
                .bundles
                .iter()
                .filter(|b| b["freshness"] == "unknown")
                .map(|b| b["alias"].clone())
                .collect::<Vec<_>>());
            response.budget["scope"] = json!("request");
        }
        response
            .bundles
            .sort_by(|a, b| a["alias"].as_str().cmp(&b["alias"].as_str()));
        Ok(response)
    }

    pub fn indexed_read(
        &self,
        reference: &str,
        options: &ReadOptions,
        index: &dyn Index,
    ) -> Result<Response> {
        let (name, id) = reference
            .strip_prefix('@')
            .and_then(|s| s.split_once('/'))
            .ok_or_else(|| Error::new("INVALID_REFERENCE", "expected @bundle/path", 2))?;
        let path = okf_core::logical_path(id)?;
        let path = if path.ends_with(".md") {
            path
        } else {
            format!("{path}.md")
        };
        self.control.check()?;
        let binding = self.binding(name)?;
        let snapshot = index
            .read(&binding.index_key(), &path)?
            .ok_or_else(|| Error::new("INDEX_MISSING", "run sync for this bundle first", 3))?;
        self.control.check()?;
        let doc = snapshot
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| Error::new("NOT_FOUND", "file absent from index", 3))?;
        let mut start = 0;
        if let Some(token) = &options.cursor {
            let cursor: crate::Cursor = okf_core::decode_cursor(token)?;
            if cursor.bundle_id != binding.id
                || cursor.operation != "indexed_read"
                || cursor.target != path
            {
                return Err(Error::new(
                    "INVALID_CURSOR",
                    "cursor belongs to another request",
                    2,
                ));
            }
            if cursor.revision != snapshot.generation {
                return Err(Error::new(
                    "REVISION_CHANGED",
                    "index was replaced; restart read",
                    6,
                ));
            }
            start = cursor.offset;
        }
        let end = okf_core::prefix_end(&doc.raw, start, options.budget)?;
        let more = end < doc.raw.len();
        let cursor = if more {
            Some(okf_core::encode_cursor(&crate::Cursor {
                bundle_id: binding.id.clone(),
                operation: "indexed_read".into(),
                target: path.clone(),
                revision: snapshot.generation.clone(),
                offset: end,
            })?)
        } else {
            None
        };
        let mut response = Response::new(
            "read",
            json!({"document":{"bundle_id":binding.id,"path":path,"concept_id":if reserved(&path) {None} else {path.strip_suffix(".md")},"content_sha256":doc.hash,"generation":snapshot.generation,
            "upstream_revision":snapshot.upstream_revision,"byte_start":start,"byte_end":end,"total_bytes":doc.raw.len(),"text":&doc.raw[start..end]}}),
        );
        if !reserved(&path) {
            let parsed = Document::parse(&doc.raw)?;
            response
                .diagnostics
                .extend(self.document_diagnostics(&binding, &path, &doc.raw, &parsed));
        }
        response.page = json!({"returned":1,"total":1,"has_more":more,"next_cursor":cursor});
        response.budget =
            json!({"content_bytes_limit":options.budget,"content_bytes_used":end-start});
        if more {
            response.next.push(
                json!({"operation":"read","target":reference,"indexed":true,"cursor":cursor}),
            );
        }
        indexed_observation(&mut response, &binding, &snapshot, true);
        Ok(response)
    }
}

pub(crate) fn indexed_observation(
    response: &mut Response,
    binding: &BundleBinding,
    snapshot: &Snapshot,
    complete: bool,
) {
    response.bundles.push(json!({"bundle_id":binding.id,"alias":binding.name,"view_kind":if binding.git.is_some() {"snapshot"} else {"observed"},
        "upstream_revision":snapshot.upstream_revision,"index_generation":snapshot.generation,"checked_at":snapshot.checked_at,"freshness":"cached",
        "matches_binding_revision":snapshot.upstream_revision==binding.git.as_ref().map(|g|g.commit.clone()),
        "coverage":{"scope":"","metadata":if complete {"complete"} else {"partial"},"body":"complete","graph":"unknown"}}));
}

fn excerpt(
    raw: &str,
    body_start: usize,
    terms: &[String],
    limit: usize,
    section: bool,
) -> (usize, usize, bool) {
    let mut offset = body_start;
    let sections = okf_core::sections::Sections::parse(raw, body_start);
    let mut hit = None;
    for line in raw[body_start..].split_inclusive('\n') {
        let lower = line.to_lowercase();
        if let Some(normalized) = terms.iter().filter_map(|term| lower.find(term)).min() {
            // Lowercasing can change UTF-8 length (for example U+0130). Map the
            // normalized match back to original bytes before selecting a window.
            let mut consumed = 0;
            let original = line
                .char_indices()
                .find_map(|(index, ch)| {
                    let width = ch.to_lowercase().map(char::len_utf8).sum::<usize>();
                    let contains = normalized < consumed + width;
                    consumed += width;
                    contains.then_some(index)
                })
                .unwrap_or(0);
            let heading = sections
                .containing(offset + original)
                .map_or(body_start, |h| h.start);
            hit = Some((if section { heading } else { offset }, offset + original));
            break;
        }
        offset += line.len();
    }
    let (mut start, hit) = hit.unwrap_or((body_start, body_start));
    if hit - start >= limit {
        start = hit;
    }
    let section_end = sections.containing(hit).map_or_else(
        || {
            sections
                .headings
                .iter()
                .find(|h| h.start > hit)
                .map_or(raw.len(), |h| h.start)
        },
        |h| h.end,
    );
    let mut end = (start + limit).min(section_end);
    while !raw.is_char_boundary(end) {
        end -= 1;
    }
    (start, end, sections.complete)
}

#[cfg(test)]
mod tests {
    use super::excerpt;
    #[test]
    fn long_unicode_lines_keep_the_match_and_original_offsets() {
        let raw = format!("# Heading\n{}İ认证 executor.resource 🙂", "前".repeat(1000));
        for section in [false, true] {
            let (start, end, complete) = excerpt(&raw, 0, &["认证".into()], 128, section);
            assert!(complete);
            assert!(raw[start..end].contains("认证"));
            assert!(end - start <= 128);
        }
        let (start, end, _) = excerpt(&raw, 0, &["i\u{307}".into()], 128, false);
        assert!(raw[start..end].starts_with('İ'));
    }
}
