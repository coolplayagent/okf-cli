//! Explicit, bounded snapshot synchronization and offline consumption.
use crate::{App, BundleBinding, ReadOptions, Response, ScanBudget};
use okf_core::{reserved, sha256, validate_reserved, Document, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};

pub const MAX_INDEX_BYTES: usize = 64 * 1024 * 1024;
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
pub trait Index {
    fn load(&self, key: &str) -> Result<Option<Snapshot>>;
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
        let previous = index.load(&key)?;
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
            let generation = sha256(
                &serde_json::to_vec(&(
                    revision.clone(),
                    documents
                        .iter()
                        .map(|d| (&d.path, &d.hash))
                        .collect::<Vec<_>>(),
                ))
                .map_err(Error::io)?,
            );
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

    fn snapshot(&self, name: &str, index: &dyn Index) -> Result<(BundleBinding, Snapshot)> {
        self.control.check()?;
        let binding = self.binding(name)?;
        let snapshot = index
            .load(&binding.index_key())?
            .ok_or_else(|| Error::new("INDEX_MISSING", "run sync for this bundle first", 3))?;
        self.control.check()?;
        Ok((binding, snapshot))
    }

    pub fn index_status(&self, name: &str, index: &dyn Index) -> Result<Response> {
        let (binding, snapshot) = self.snapshot(name, index)?;
        Ok(Response::new(
            "index status",
            json!({"generation":snapshot.generation,"documents":snapshot.documents.len(),
            "bytes":snapshot.documents.iter().map(|d| d.raw.len()).sum::<usize>(),"checked_at":snapshot.checked_at,
            "upstream_revision":snapshot.upstream_revision,"matches_binding_revision":snapshot.upstream_revision==binding.git.as_ref().map(|g|g.commit.clone()),
            "view_kind":if binding.git.is_some() {"snapshot"} else {"observed"}}),
        ))
    }

    pub fn indexed_search(
        &self,
        query: &str,
        name: &str,
        options: &ReadOptions,
        index: &dyn Index,
        context: bool,
    ) -> Result<Response> {
        let terms: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
        if terms.is_empty() {
            return Err(Error::new("EMPTY_QUERY", "query must not be empty", 2));
        }
        let (binding, snapshot) = self.snapshot(name, index)?;
        let mut candidates = vec![];
        let mut complete = true;
        let mut response = Response::new(
            if context { "context" } else { "search" },
            json!({"query":query,"mode":"full_text","items":[],"generation":snapshot.generation,
            "disclosure":if context {"selected_excerpts"} else {"indexed_snippets"}}),
        );
        let mut scanned = 0;
        for document in &snapshot.documents {
            if let Err(error) = self.control.check() {
                complete = false;
                response.partial(&error.code, &error.message);
                break;
            }
            if reserved(&document.path) {
                continue;
            }
            if scanned >= options.max_scan {
                complete = false;
                response.partial("SCAN_LIMIT", "indexed document evaluation budget reached");
                break;
            }
            scanned += 1;
            let doc = Document::parse(&document.raw)?;
            let id = document.path.strip_suffix(".md").unwrap_or(&document.path);
            let card = doc.card(id, self.now);
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
            let (start, end) = excerpt(
                &document.raw,
                doc.body_start,
                &terms,
                if context { 4096 } else { 512 },
                context,
            );
            let mut item = serde_json::to_value(card).map_err(Error::io)?;
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
                json!({"operation":"read","target":item["ref"],"indexed":true,"generation":snapshot.generation}),
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
        indexed_observation(&mut response, &binding, &snapshot, complete);
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
        let (binding, snapshot) = self.snapshot(name, index)?;
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

fn indexed_observation(
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
) -> (usize, usize) {
    let mut offset = body_start;
    let mut heading = body_start;
    let mut hit = None;
    for line in raw[body_start..].split_inclusive('\n') {
        if line.starts_with('#') {
            heading = offset;
        }
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
            hit = Some((if section { heading } else { offset }, offset + original));
            break;
        }
        offset += line.len();
    }
    let (mut start, hit) = hit.unwrap_or((body_start, body_start));
    if hit - start >= limit {
        start = hit;
    }
    let mut end = (start + limit).min(raw.len());
    while !raw.is_char_boundary(end) {
        end -= 1;
    }
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::excerpt;
    #[test]
    fn long_unicode_lines_keep_the_match_and_original_offsets() {
        let raw = format!("# Heading\n{}İ认证 executor.resource 🙂", "前".repeat(1000));
        for section in [false, true] {
            let (start, end) = excerpt(&raw, 0, &["认证".into()], 128, section);
            assert!(raw[start..end].contains("认证"));
            assert!(end - start <= 128);
        }
        let (start, end) = excerpt(&raw, 0, &["i\u{307}".into()], 128, false);
        assert!(raw[start..end].starts_with('İ'));
    }
}
