use crate::{BundleBinding, BundleReader, BundleRegistry, Cache, Cursor, ReadOptions, Response};
use chrono::{DateTime, Utc};
use okf_core::{
    decode_cursor, encode_cursor, logical_path, prefix_end, reserved, sha256, Diagnostic, Document,
    Error, Result,
};
use serde_json::{json, Value};
use std::collections::{BTreeSet, VecDeque};

const MAX_DOCUMENT: usize = 2 * 1024 * 1024;

pub struct App<'a> {
    pub registry: &'a dyn BundleRegistry,
    pub reader: &'a dyn BundleReader,
    pub cache: &'a dyn Cache,
    pub now: DateTime<Utc>,
}

impl App<'_> {
    pub fn binding(&self, name: &str) -> Result<BundleBinding> {
        let bindings: Vec<_> = self
            .registry
            .list()?
            .into_iter()
            .filter(|s| s.name == name)
            .collect();
        match bindings.len() {
            0 => Err(Error::new(
                "BUNDLE_NOT_FOUND",
                format!("unknown bundle: {name}"),
                3,
            )),
            1 => Ok(bindings[0].clone()),
            _ => Err(Error::new(
                "AMBIGUOUS_BUNDLE",
                "alias exists in project and user scopes; select --scope",
                2,
            )),
        }
    }

    fn resolve(&self, reference: &str) -> Result<(BundleBinding, String)> {
        let rest = reference
            .strip_prefix('@')
            .ok_or_else(|| Error::new("INVALID_REFERENCE", "expected @bundle/path", 2))?;
        let (name, path) = rest.split_once('/').unwrap_or((rest, ""));
        Ok((self.binding(name)?, logical_path(path)?))
    }

    fn card(&self, binding: &BundleBinding, path: &str) -> Result<Value> {
        let raw = self.reader.read(binding, path, MAX_DOCUMENT)?;
        let doc = Document::parse(&raw)?;
        let id = path.strip_suffix(".md").unwrap_or(path);
        let card = doc.card(id, self.now);
        let hash = sha256(raw.as_bytes());
        self.cache.remember(
            &binding.cache_key(),
            path,
            &hash,
            &card,
            &self.now.to_rfc3339(),
        )?;
        let mut value = serde_json::to_value(card).map_err(Error::io)?;
        value["kind"] = json!("concept");
        value["ref"] = json!(format!("@{}/{}", binding.name, id));
        value["bundle_id"] = json!(binding.id);
        value["content_sha256"] = json!(hash);
        Ok(value)
    }

    pub fn context(&self, selected: &[String], options: &ReadOptions) -> Result<Response> {
        let bindings = self.registry.list()?;
        if bindings.is_empty() {
            return Err(Error::new(
                "NO_BUNDLES",
                "register a knowledge bundle with bundle add",
                2,
            ));
        }
        for name in selected {
            self.binding(name)?;
        }
        let bindings: Vec<_> = bindings
            .into_iter()
            .filter(|b| selected.is_empty() || selected.contains(&b.name))
            .collect();
        for binding in &bindings {
            self.binding(&binding.name)?;
        }
        let mut response = Response::new("context", json!({"items": []}));
        let mut used = 0;
        let mut items = vec![];
        for binding in &bindings {
            if items.len() >= options.limit {
                response.partial(
                    "BUNDLE_LIMIT",
                    "more bundles are available; select --bundle",
                );
                break;
            }
            if let Err(error) = self.reader.probe(binding) {
                if bindings.len() == 1 {
                    return Err(error);
                }
                response.partial(&error.code, &format!("{}: {}", binding.name, error.message));
                continue;
            }
            let item = json!({"bundle_id": binding.id, "name": binding.name, "kind": binding.kind,
                "root": format!("@{}", binding.name), "scope": binding.scope});
            if !fits(&item, &mut used, options.budget) {
                response.partial(
                    "BUDGET_EXCEEDED",
                    "select fewer bundles or increase --budget-bytes",
                );
                break;
            }
            items.push(item);
            response.observe(binding, self.now, "", "unknown");
            response
                .next
                .push(json!({"operation": "browse", "target": format!("@{}", binding.name)}));
        }
        response.page["returned"] = json!(items.len());
        response.page["total"] = json!(bindings.len());
        response.data["items"] = json!(items);
        response.budget =
            json!({"content_bytes_limit": options.budget, "content_bytes_used": used});
        Ok(response)
    }

    pub fn browse(&self, reference: &str, options: &ReadOptions) -> Result<Response> {
        let (binding, directory) = self.resolve(reference)?;
        let entries = self.reader.list(&binding, &directory)?;
        let revision = sha256(&serde_json::to_vec(&entries).map_err(Error::io)?);
        let mut index = 0;
        if let Some(token) = &options.cursor {
            let cursor: Cursor = decode_cursor(token)?;
            check_cursor(&cursor, &binding, "browse", &directory, &revision)?;
            index = cursor.offset;
            if index > entries.len() {
                return Err(Error::new("INVALID_CURSOR", "offset outside directory", 2));
            }
        }
        let mut response = Response::new(
            "browse",
            json!({"directory": reference, "items": [], "index": {"origin": "synthesized"}}),
        );
        let mut items = vec![];
        let mut used = 0;
        let mut scanned = 0;
        // Index text has its own page through read; never consume an entire large index here.
        let index_path = if directory.is_empty() {
            "index.md".into()
        } else {
            format!("{directory}/index.md")
        };
        if entries.iter().any(|e| e.path == index_path) {
            response.data["index"] =
                json!({"origin": "authored", "ref": format!("@{}/{}", binding.name, index_path)});
            response.next.push(
                json!({"operation": "read", "target": format!("@{}/{}", binding.name, index_path)}),
            );
        }
        while index < entries.len() && items.len() < options.limit && scanned < options.max_scan {
            let entry = &entries[index];
            scanned += 1;
            let item = if entry.directory {
                json!({"kind": "directory", "id": entry.path, "ref": format!("@{}/{}", binding.name, entry.path)})
            } else if reserved(&entry.path) {
                index += 1;
                continue;
            } else {
                match self.card(&binding, &entry.path) {
                    Ok(card) => card,
                    Err(error) if error.code == "INVALID_OKF" => {
                        response.diagnostics.push(Diagnostic::new(
                            &error.code,
                            error.message,
                            Some(&entry.path),
                        ));
                        index += 1;
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            };
            if !fits(&item, &mut used, options.budget) {
                if items.is_empty() {
                    return Err(Error::new(
                        "BUDGET_TOO_SMALL",
                        "budget cannot fit the next card",
                        2,
                    ));
                }
                break;
            }
            items.push(item);
            index += 1;
        }
        let more = index < entries.len();
        let cursor = if more {
            Some(encode_cursor(&Cursor {
                bundle_id: binding.id.clone(),
                operation: "browse".into(),
                target: directory.clone(),
                revision,
                offset: index,
            })?)
        } else {
            None
        };
        response.page = json!({"returned": items.len(), "total": null, "has_more": more, "next_cursor": cursor});
        if more {
            response
                .next
                .push(json!({"operation": "browse", "target": reference, "cursor": cursor}));
        }
        response.data["items"] = json!(items);
        response.budget = json!({"content_bytes_limit": options.budget, "content_bytes_used": used, "scanned_files": scanned});
        response.observe(
            &binding,
            self.now,
            &directory,
            if more { "partial" } else { "complete" },
        );
        Ok(response)
    }

    pub fn read(&self, reference: &str, raw_mode: bool, options: &ReadOptions) -> Result<Response> {
        let (binding, id) = self.resolve(reference)?;
        if id.is_empty() {
            return Err(Error::new(
                "INVALID_REFERENCE",
                "read requires a concept or reserved file",
                2,
            ));
        }
        let path = if id.ends_with(".md") {
            id
        } else {
            format!("{id}.md")
        };
        let raw = self.reader.read(&binding, &path, MAX_DOCUMENT)?;
        let hash = sha256(raw.as_bytes());
        let concept_id = path.strip_suffix(".md").unwrap_or(&path);
        let mut response = Response::new("read", json!({}));
        if !reserved(&path) {
            match Document::parse(&raw) {
                Ok(doc) => {
                    let card = doc.card(concept_id, self.now);
                    if card.expiry == "unknown" {
                        response.diagnostics.push(Diagnostic::new(
                            "INVALID_TIMESTAMP",
                            "stale_after is not a timezone-aware timestamp",
                            Some(&path),
                        ));
                    }
                    self.cache.remember(
                        &binding.cache_key(),
                        &path,
                        &hash,
                        &card,
                        &self.now.to_rfc3339(),
                    )?;
                    response.data["card"] = serde_json::to_value(card).map_err(Error::io)?;
                }
                Err(e) if raw_mode => {
                    response
                        .diagnostics
                        .push(Diagnostic::new(&e.code, e.message, Some(&path)))
                }
                Err(e) => return Err(e),
            }
        }
        let start = if let Some(token) = &options.cursor {
            let cursor: Cursor = decode_cursor(token)?;
            check_cursor(&cursor, &binding, "read", &path, &hash)?;
            cursor.offset
        } else {
            0
        };
        let end = prefix_end(&raw, start, options.budget)?;
        let more = end < raw.len();
        let cursor = if more {
            Some(encode_cursor(&Cursor {
                bundle_id: binding.id.clone(),
                operation: "read".into(),
                target: path.clone(),
                revision: hash.clone(),
                offset: end,
            })?)
        } else {
            None
        };
        response.data["document"] = json!({"bundle_id": binding.id, "path": path,
            "concept_id": concept_id, "content_sha256": hash,
            "byte_start": start, "byte_end": end, "total_bytes": raw.len(), "text": &raw[start..end]});
        response.page = json!({"returned": 1, "total": 1, "has_more": more, "next_cursor": cursor});
        response.budget = json!({"content_bytes_limit": options.budget, "content_bytes_used": end - start, "truncated": more});
        if more {
            response
                .next
                .push(json!({"operation": "read", "target": reference, "cursor": cursor}));
        }
        response.observe(&binding, self.now, &path, "complete");
        Ok(response)
    }

    pub fn search(
        &self,
        query: &str,
        selected: &[String],
        options: &ReadOptions,
    ) -> Result<Response> {
        if query.trim().is_empty() {
            return Err(Error::new("EMPTY_QUERY", "query must not be empty", 2));
        }
        if options.cursor.is_some() {
            return Err(Error::new(
                "UNSUPPORTED_CAPABILITY",
                "search pagination is not implemented; narrow the bundle or query",
                2,
            ));
        }
        let all = self.registry.list()?;
        if all.is_empty() {
            return Err(Error::new("NO_BUNDLES", "register a bundle first", 2));
        }
        for name in selected {
            self.binding(name)?;
        }
        let bindings: Vec<_> = all
            .into_iter()
            .filter(|b| selected.is_empty() || selected.contains(&b.name))
            .collect();
        for binding in &bindings {
            self.binding(&binding.name)?;
        }
        let terms: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut response = Response::new(
            "search",
            json!({"query": query, "mode": "metadata", "items": []}),
        );
        let mut matches = vec![];
        let mut remaining = options.max_scan;
        let mut match_count = 0;
        for binding in &bindings {
            let mut queue = VecDeque::from([String::new()]);
            let mut seen = BTreeSet::new();
            let mut complete = true;
            'scan: while let Some(directory) = queue.pop_front() {
                if remaining == 0 {
                    complete = false;
                    break;
                }
                // Directory work counts too: empty trees cannot evade the scan budget.
                remaining -= 1;
                let entries = match self.reader.list(binding, &directory) {
                    Ok(entries) => entries,
                    Err(error) if bindings.len() > 1 => {
                        response
                            .partial(&error.code, &format!("{}: {}", binding.name, error.message));
                        complete = false;
                        break;
                    }
                    Err(error) => return Err(error),
                };
                for entry in entries {
                    if remaining == 0 {
                        complete = false;
                        break 'scan;
                    }
                    remaining -= 1;
                    if entry.directory {
                        if seen.insert(entry.path.clone()) {
                            queue.push_back(entry.path);
                        }
                    } else if !reserved(&entry.path) {
                        let card = match self.card(binding, &entry.path) {
                            Ok(card) => card,
                            Err(e) if e.code == "INVALID_OKF" => {
                                response.diagnostics.push(Diagnostic::new(
                                    &e.code,
                                    e.message,
                                    Some(&entry.path),
                                ));
                                continue;
                            }
                            Err(e) => return Err(e),
                        };
                        let haystack = format!(
                            "{} {} {} {} {}",
                            card["id"],
                            card["title"],
                            card["description"],
                            card["tags"],
                            card["type"]
                        )
                        .to_lowercase();
                        if terms.iter().all(|term| haystack.contains(term)) {
                            match_count += 1;
                            if card.to_string().len() > options.budget {
                                response.partial(
                                    "CARD_TOO_LARGE",
                                    "a matching card exceeds the content budget",
                                );
                            } else if matches.len() <= options.limit {
                                matches.push(card);
                            }
                        }
                    }
                }
            }
            if !complete {
                response.partial(
                    "INDEX_PARTIAL",
                    "metadata scan budget reached; increase --max-scan or select fewer bundles",
                );
            }
            response.observe(
                binding,
                self.now,
                "",
                if complete { "complete" } else { "partial" },
            );
        }
        matches.sort_by(|a, b| a["ref"].as_str().cmp(&b["ref"].as_str()));
        let total = match_count;
        let mut items = vec![];
        let mut used = 0;
        for card in matches {
            if items.len() >= options.limit || !fits(&card, &mut used, options.budget) {
                response.partial(
                    "RESULT_LIMIT",
                    "more matches exist; narrow the query or increase the result budget",
                );
                break;
            }
            items.push(card);
        }
        response.page["returned"] = json!(items.len());
        if response.status == "ok" {
            response.page["total"] = json!(total);
        }
        response.data["items"] = json!(items);
        response.budget = json!({"content_bytes_limit": options.budget, "content_bytes_used": used, "scanned_entries": options.max_scan - remaining});
        Ok(response)
    }

    pub fn validate(&self, name: &str, options: &ReadOptions) -> Result<Response> {
        let binding = self.binding(name)?;
        let mut response = Response::new("validate", json!({"concepts": 0, "checked": 0}));
        let mut queue = VecDeque::from([String::new()]);
        let mut scanned = 0;
        let mut failures = 0;
        'scan: while let Some(directory) = queue.pop_front() {
            if scanned >= options.max_scan {
                response.partial("INDEX_PARTIAL", "validation scan budget reached");
                break;
            }
            scanned += 1;
            for entry in self.reader.list(&binding, &directory)? {
                if scanned >= options.max_scan {
                    response.partial("INDEX_PARTIAL", "validation scan budget reached");
                    break 'scan;
                }
                scanned += 1;
                if entry.directory {
                    queue.push_back(entry.path);
                    continue;
                }
                let result = self
                    .reader
                    .read(&binding, &entry.path, MAX_DOCUMENT)
                    .and_then(|raw| {
                        if reserved(&entry.path) {
                            validate_reserved(&entry.path, &raw)
                        } else {
                            Document::parse(&raw).map(|_| ())
                        }
                    });
                if let Err(e) = result {
                    failures += 1;
                    response.diagnostics.push(Diagnostic::new(
                        &e.code,
                        e.message,
                        Some(&entry.path),
                    ));
                }
                response.data["checked"] =
                    json!(response.data["checked"].as_u64().unwrap_or(0) + 1);
                if !reserved(&entry.path) {
                    response.data["concepts"] =
                        json!(response.data["concepts"].as_u64().unwrap_or(0) + 1);
                }
            }
        }
        response.data["failures"] = json!(failures);
        if failures > 0 && response.exit_code == 0 {
            response.status = "error".into();
            response.exit_code = 1;
        }
        response.observe(
            &binding,
            self.now,
            "",
            if response.exit_code == 7 {
                "partial"
            } else {
                "complete"
            },
        );
        Ok(response)
    }
}

fn fits(value: &Value, used: &mut usize, limit: usize) -> bool {
    let bytes = value.to_string().len();
    if bytes > limit.saturating_sub(*used) {
        return false;
    }
    *used += bytes;
    true
}

fn check_cursor(
    cursor: &Cursor,
    binding: &BundleBinding,
    operation: &str,
    target: &str,
    revision: &str,
) -> Result<()> {
    if cursor.bundle_id != binding.id || cursor.operation != operation || cursor.target != target {
        return Err(Error::new(
            "INVALID_CURSOR",
            "cursor belongs to another request",
            2,
        ));
    }
    if cursor.revision != revision {
        return Err(Error::new(
            "REVISION_CHANGED",
            "bundle changed; restart pagination",
            6,
        ));
    }
    Ok(())
}

fn validate_reserved(path: &str, raw: &str) -> Result<()> {
    if path.ends_with("index.md") && raw.trim_start_matches('\u{feff}').starts_with("---") {
        let (fm, _) = okf_core::frontmatter(raw)?;
        if path != "index.md" || fm.keys().any(|k| k != "okf_version") {
            return Err(Error::new(
                "INVALID_OKF",
                "index frontmatter is only allowed at root with okf_version",
                1,
            ));
        }
    }
    if path.ends_with("log.md") {
        for line in raw.lines().filter_map(|s| s.strip_prefix("## ")) {
            if chrono::NaiveDate::parse_from_str(line.trim(), "%Y-%m-%d").is_err() {
                return Err(Error::new(
                    "INVALID_OKF",
                    "log date headings must be YYYY-MM-DD",
                    1,
                ));
            }
        }
    }
    Ok(())
}
