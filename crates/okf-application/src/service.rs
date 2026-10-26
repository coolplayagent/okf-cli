use crate::{
    interrupted, BundleBinding, BundleReader, BundleRegistry, Cache, Cursor, ReadOptions,
    RequestControl, Response, ScanBudget,
};
use chrono::{DateTime, Utc};
use okf_core::markdown::{
    body_truncation_warning, body_warning, BodyLink, LinkTarget, Markdown, MAX_BODY_WARNINGS,
};
use okf_core::{
    decode_cursor, encode_cursor, logical_path, prefix_end, reserved, sha256, validate_reserved,
    Card, Diagnostic, Document, Error, Result, Severity,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_DOCUMENT: usize = 2 * 1024 * 1024;
const MAX_VALIDATION_LINKS: usize = 4096;
const MAX_VALIDATION_LINK_BYTES: usize = 2 * 1024 * 1024;

struct CardObservation {
    value: Value,
    diagnostics: Vec<Diagnostic>,
    cache_error: Option<Error>,
}

#[derive(Default)]
struct BodyWarningState {
    count: usize,
    truncated: bool,
}

pub struct App<'a> {
    pub registry: &'a dyn BundleRegistry,
    pub reader: &'a dyn BundleReader,
    pub cache: &'a dyn Cache,
    pub now: DateTime<Utc>,
    pub control: &'a RequestControl,
}

impl App<'_> {
    fn markdown(
        &self,
        binding: &BundleBinding,
        path: &str,
        raw: &str,
        doc: Option<&Document>,
    ) -> Markdown {
        let start = doc.map_or_else(
            || okf_core::frontmatter(raw).map_or(0, |(_, start)| start),
            |doc| doc.body_start,
        );
        let mut markdown = Markdown::parse(raw, start, path, doc.map(|doc| &doc.frontmatter));
        for diagnostic in &mut markdown.diagnostics {
            diagnostic.path = Some(format!("@{}/{}", binding.name, path));
        }
        markdown
    }

    pub(crate) fn document_diagnostics(
        &self,
        binding: &BundleBinding,
        path: &str,
        raw: &str,
        doc: &Document,
    ) -> Vec<Diagnostic> {
        let mut diagnostics = doc.diagnostics(&format!("@{}/{}", binding.name, path));
        diagnostics.extend(self.markdown(binding, path, raw, Some(doc)).diagnostics);
        diagnostics
    }

    pub fn binding(&self, name: &str) -> Result<BundleBinding> {
        find_binding(&self.registry.list()?, name).cloned()
    }

    pub(crate) fn selected_bindings(&self, selected: &[String]) -> Result<Vec<BundleBinding>> {
        let bindings = self.registry.list()?;
        if bindings.is_empty() {
            return Err(Error::new(
                "NO_BUNDLES",
                "register a knowledge bundle with bundle add",
                2,
            ));
        }
        for name in selected {
            find_binding(&bindings, name)?;
        }
        bindings
            .iter()
            .filter(|binding| selected.is_empty() || selected.contains(&binding.name))
            .map(|binding| find_binding(&bindings, &binding.name).cloned())
            .collect()
    }

    fn resolve(&self, reference: &str) -> Result<(BundleBinding, String)> {
        let rest = reference
            .strip_prefix('@')
            .ok_or_else(|| Error::new("INVALID_REFERENCE", "expected @bundle/path", 2))?;
        let (name, path) = rest.split_once('/').unwrap_or((rest, ""));
        Ok((self.binding(name)?, logical_path(path)?))
    }

    fn remember_card(
        &self,
        binding: &BundleBinding,
        path: &str,
        hash: &str,
        card: &Card,
    ) -> Result<Option<Error>> {
        self.control.check()?;
        Ok(self
            .cache
            .remember(
                &binding.cache_key(),
                path,
                hash,
                card,
                &self.now.to_rfc3339(),
            )
            .err())
    }

    fn card(&self, binding: &BundleBinding, path: &str) -> Result<CardObservation> {
        self.card_at(binding, path, self.now)
    }

    fn card_at(
        &self,
        binding: &BundleBinding,
        path: &str,
        evaluated_at: DateTime<Utc>,
    ) -> Result<CardObservation> {
        self.control.check()?;
        let raw = self
            .reader
            .read(binding, path, MAX_DOCUMENT, self.control)?;
        let doc = Document::parse(&raw)?;
        let diagnostics = self.document_diagnostics(binding, path, &raw, &doc);
        let id = path.strip_suffix(".md").unwrap_or(path);
        let card = doc.card(id, self.now);
        let hash = sha256(raw.as_bytes());
        let cache_error = self.remember_card(binding, path, &hash, &card)?;
        let mut value = serde_json::to_value(doc.card(id, evaluated_at)).map_err(Error::io)?;
        value["kind"] = json!("concept");
        value["ref"] = json!(format!("@{}/{}", binding.name, id));
        value["bundle_id"] = json!(binding.id);
        value["content_sha256"] = json!(hash);
        value["upstream_revision"] = json!(binding.git.as_ref().map(|g| &g.commit));
        Ok(CardObservation {
            value,
            diagnostics,
            cache_error,
        })
    }

    pub fn context(&self, selected: &[String], options: &ReadOptions) -> Result<Response> {
        let bindings = self.selected_bindings(selected)?;
        let mut response = Response::new("context", json!({"items": []}));
        let mut used = 0;
        let mut items = vec![];
        for binding in &bindings {
            if let Err(error) = self.control.check() {
                response.issue(&error, &format!("@{}", binding.name));
                response.unavailable(binding, "");
                continue;
            }
            if items.len() >= options.limit {
                response.partial(
                    "BUNDLE_LIMIT",
                    "more bundles are available; select --bundle",
                );
                break;
            }
            if let Err(error) = self.reader.probe_controlled(binding, self.control) {
                if bindings.len() == 1 {
                    return Err(error);
                }
                response.partial(&error.code, &format!("{}: {}", binding.name, error.message));
                response.unavailable(binding, "");
                continue;
            }
            if let Err(error) = self.control.check() {
                response.issue(&error, &format!("@{}", binding.name));
                response.unavailable(binding, "");
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
        let mut scan = ScanBudget::new(options.max_scan, self.control);
        let listing = self.reader.list(&binding, &directory, &mut scan)?;
        let mut complete = listing.complete;
        if !listing.complete && options.cursor.is_some() {
            return Err(Error::new(
                "SCAN_LIMIT",
                "cannot verify directory cursor within scan budget; increase --max-scan",
                7,
            ));
        }
        let entries = listing.entries;
        let revision = cursor_revision(
            &binding,
            &sha256(&serde_json::to_vec(&entries).map_err(Error::io)?),
        );
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
            json!({"directory": reference, "items": [], "index": {"origin": if listing.complete { "synthesized" } else { "unknown" }}}),
        );
        let mut items = vec![];
        let mut used = 0;
        if !listing.complete {
            response.partial(
                "SCAN_LIMIT",
                "directory enumeration is incomplete; increase --max-scan and restart browse",
            );
        }
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
        while index < entries.len() && items.len() < options.limit {
            if let Err(error) = self.control.check() {
                complete = false;
                response.partial(&error.code, &error.message);
                break;
            }
            let entry = &entries[index];
            let item = if entry.directory {
                json!({"kind": "directory", "id": entry.path, "ref": format!("@{}/{}", binding.name, entry.path)})
            } else if reserved(&entry.path) {
                index += 1;
                continue;
            } else {
                match self.card(&binding, &entry.path) {
                    Ok(CardObservation {
                        value: card,
                        diagnostics,
                        cache_error,
                    }) => {
                        response.diagnostics.extend(diagnostics);
                        if let Some(error) = cache_error {
                            response.partial("CACHE_WRITE_FAILED", &error.message);
                        }
                        card
                    }
                    Err(error) if interrupted(&error) => {
                        complete = false;
                        response.partial(&error.code, &error.message);
                        break;
                    }
                    Err(error) => {
                        complete = false;
                        response.issue(&error, &format!("@{}/{}", binding.name, entry.path));
                        index += 1;
                        continue;
                    }
                }
            };
            if item.to_string().len() > options.budget {
                complete = false;
                response.issue(&Error::new("CARD_TOO_LARGE", "card exceeds the content budget; increase --budget-bytes or read the file directly", 7), &format!("@{}/{}", binding.name, entry.path));
                index += 1;
                continue;
            }
            if !fits(&item, &mut used, options.budget) {
                break;
            }
            items.push(item);
            index += 1;
        }
        if let Err(error) = self.control.check() {
            complete = false;
            if !response.diagnostics.iter().any(|d| d.code == error.code) {
                response.partial(&error.code, &error.message);
            }
        }
        let more = index < entries.len() && listing.complete;
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
        response.budget = json!({"content_bytes_limit": options.budget, "content_bytes_used": used, "scan_limit": options.max_scan, "scanned_entries": scan.used()});
        response.observe(
            &binding,
            self.now,
            &directory,
            if more || !complete {
                "partial"
            } else {
                "complete"
            },
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
        let raw = self
            .reader
            .read(&binding, &path, MAX_DOCUMENT, self.control)?;
        let hash = sha256(raw.as_bytes());
        let concept_id = path.strip_suffix(".md").unwrap_or(&path);
        let mut response = Response::new("read", json!({}));
        if !reserved(&path) {
            match Document::parse(&raw) {
                Ok(doc) => {
                    let card = doc.card(concept_id, self.now);
                    response
                        .diagnostics
                        .extend(self.document_diagnostics(&binding, &path, &raw, &doc));
                    if let Some(error) = self.remember_card(&binding, &path, &hash, &card)? {
                        response.partial("CACHE_WRITE_FAILED", &error.message);
                    }
                    response.data["card"] = serde_json::to_value(card).map_err(Error::io)?;
                }
                Err(e) if raw_mode => {
                    response
                        .diagnostics
                        .push(Diagnostic::new(&e.code, e.message, Some(&path)))
                }
                Err(e) => return Err(e),
            }
        } else {
            response
                .diagnostics
                .extend(self.markdown(&binding, &path, &raw, None).diagnostics);
        }
        let start = if let Some(token) = &options.cursor {
            let cursor: Cursor = decode_cursor(token)?;
            check_cursor(
                &cursor,
                &binding,
                "read",
                &path,
                &cursor_revision(&binding, &hash),
            )?;
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
                revision: cursor_revision(&binding, &hash),
                offset: end,
            })?)
        } else {
            None
        };
        response.data["document"] = json!({"bundle_id": binding.id, "path": path,
            "concept_id": concept_id, "content_sha256": hash, "upstream_revision": binding.git.as_ref().map(|g| &g.commit),
            "byte_start": start, "byte_end": end, "total_bytes": raw.len(), "text": &raw[start..end]});
        self.control.check()?;
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
        self.search_filtered(query, selected, options, &crate::SearchFilter::default())
    }

    pub fn search_filtered(
        &self,
        query: &str,
        selected: &[String],
        options: &ReadOptions,
        filter: &crate::SearchFilter,
    ) -> Result<Response> {
        filter.validate()?;
        let directory = filter.directory.as_deref().unwrap_or("");
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
        let bindings = self.selected_bindings(selected)?;
        let terms: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
        let evaluated_at = filter.as_of.unwrap_or(self.now);
        let mut response = Response::new(
            "search",
            json!({"query": query, "mode": "metadata", "items": [], "evaluated_at": evaluated_at.to_rfc3339()}),
        );
        if filter.directory.is_some() {
            response.data["directory"] = json!(directory);
        }
        let mut matches = vec![];
        let mut scan = ScanBudget::new(options.max_scan, self.control);
        let mut match_count = 0;
        for binding in &bindings {
            let mut queue = VecDeque::from([directory.to_owned()]);
            let mut seen = BTreeSet::new();
            let mut complete = true;
            let mut observed = false;
            'scan: while let Some(directory) = queue.pop_front() {
                let listing = match self.reader.list(binding, &directory, &mut scan) {
                    Ok(listing) => listing,
                    Err(error)
                        if interrupted(&error)
                            || error.code == "SCAN_LIMIT"
                            || bindings.len() > 1
                            || observed =>
                    {
                        response.issue(&error, &format!("@{}/{}", binding.name, directory));
                        complete = false;
                        break;
                    }
                    Err(error) => return Err(error),
                };
                observed = true;
                if !listing.complete {
                    complete = false;
                    response.issue(&Error::new("SCAN_LIMIT", "metadata scan budget reached; increase --max-scan or select fewer bundles", 7), &format!("@{}/{}", binding.name, directory));
                }
                for entry in listing.entries {
                    if let Err(error) = self.control.check() {
                        response.issue(&error, &format!("@{}", binding.name));
                        complete = false;
                        break 'scan;
                    }
                    if entry.directory {
                        if seen.insert(entry.path.clone()) {
                            queue.push_back(entry.path);
                        }
                    } else if !reserved(&entry.path) {
                        let card = match self.card_at(binding, &entry.path, evaluated_at) {
                            Ok(CardObservation {
                                value: card,
                                diagnostics,
                                cache_error,
                            }) => {
                                response.diagnostics.extend(diagnostics);
                                if let Some(error) = cache_error {
                                    response.issue(
                                        &Error::new("CACHE_WRITE_FAILED", error.message, 7),
                                        &format!("@{}/{}", binding.name, entry.path),
                                    );
                                }
                                card
                            }
                            Err(error) => {
                                response
                                    .issue(&error, &format!("@{}/{}", binding.name, entry.path));
                                complete = false;
                                if interrupted(&error) {
                                    break 'scan;
                                }
                                continue;
                            }
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
                        if filter.matches(&card) && terms.iter().all(|term| haystack.contains(term))
                        {
                            match_count += 1;
                            if card.to_string().len() > options.budget {
                                complete = false;
                                response.issue(&Error::new("CARD_TOO_LARGE", "card exceeds the content budget; increase --budget-bytes or read the file directly", 7), &format!("@{}/{}", binding.name, entry.path));
                            } else if matches.len() <= options.limit {
                                matches.push(card);
                            }
                        }
                    }
                }
            }
            if let Err(error) = self.control.check() {
                complete = false;
                if !response.diagnostics.iter().any(|d| d.code == error.code) {
                    response.issue(&error, &format!("@{}", binding.name));
                }
            }
            if observed {
                response.observe(
                    binding,
                    self.now,
                    directory,
                    if complete { "complete" } else { "partial" },
                );
            } else {
                response.unavailable(binding, directory);
            }
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
        response.budget = json!({"content_bytes_limit": options.budget, "content_bytes_used": used, "scan_limit": options.max_scan, "scanned_entries": scan.used()});
        Ok(response)
    }

    pub fn validate(&self, name: &str, options: &ReadOptions) -> Result<Response> {
        self.validate_with_checks(name, options, 4096, false, false)
    }

    pub fn validate_with_checks(
        &self,
        name: &str,
        options: &ReadOptions,
        resource_limit: usize,
        attachments: bool,
        anchors: bool,
    ) -> Result<Response> {
        let binding = self.binding(name)?;
        let mut response = Response::new("validate", json!({"concepts": 0, "checked": 0}));
        let mut queue = VecDeque::from([String::new()]);
        let mut scan = ScanBudget::new(options.max_scan, self.control);
        let mut failures = 0;
        let mut observed = false;
        let mut complete = true;
        let mut existing = BTreeMap::new();
        let mut links: Vec<(String, BodyLink)> = vec![];
        let mut links_truncated = false;
        let mut link_bytes = 0;
        let mut body_warnings = BTreeMap::<String, BodyWarningState>::new();
        let mut resources = crate::resources::Resources::new(resource_limit);
        let mut sections = BTreeMap::new();
        let mut section_bytes = 0;
        let mut anchors_complete = true;
        let mut anchors_limited = false;
        let mut attachments_complete = true;
        let mut anchors_checked = 0;
        let mut anchors_skipped = 0;
        let mut attachments_checked = 0;
        'scan: while let Some(directory) = queue.pop_front() {
            let listing = match self.reader.list(&binding, &directory, &mut scan) {
                Ok(listing) => listing,
                Err(error) if interrupted(&error) || error.code == "SCAN_LIMIT" => {
                    complete = false;
                    response.issue(&error, &format!("@{}/{}", binding.name, directory));
                    break;
                }
                Err(error) => return Err(error),
            };
            observed = true;
            if !listing.complete {
                complete = false;
                response.issue(
                    &Error::new("SCAN_LIMIT", "validation scan budget reached", 7),
                    &format!("@{}/{}", binding.name, directory),
                );
            }
            for entry in listing.entries {
                if let Err(error) = self.control.check() {
                    complete = false;
                    response.issue(&error, &format!("@{}", binding.name));
                    break 'scan;
                }
                if entry.directory {
                    existing.insert(entry.path.clone(), true);
                    queue.push_back(entry.path);
                    continue;
                }
                existing.insert(entry.path.clone(), false);
                let result = self
                    .reader
                    .read(&binding, &entry.path, MAX_DOCUMENT, self.control)
                    .and_then(|raw| {
                        let (mut diagnostics, markdown) = if reserved(&entry.path) {
                            validate_reserved(&entry.path, &raw)?;
                            (vec![], self.markdown(&binding, &entry.path, &raw, None))
                        } else {
                            let doc = Document::parse(&raw)?;
                            resources.collect(&entry.path, &doc);
                            (
                                doc.diagnostics(&format!("@{}/{}", binding.name, entry.path)),
                                self.markdown(&binding, &entry.path, &raw, Some(&doc)),
                            )
                        };
                        if anchors {
                            let body_start =
                                okf_core::frontmatter(&raw).map_or(0, |(_, start)| start);
                            let parsed = okf_core::sections::Sections::parse(&raw, body_start);
                            let bytes = entry.path.len()
                                + parsed
                                    .headings
                                    .iter()
                                    .map(|h| h.anchor.len() + 64)
                                    .sum::<usize>();
                            if bytes <= MAX_VALIDATION_LINK_BYTES - section_bytes {
                                section_bytes += bytes;
                                anchors_complete &= parsed.complete;
                                anchors_limited |= !parsed.complete;
                                sections.insert(entry.path.clone(), parsed);
                            } else {
                                anchors_complete = false;
                                anchors_limited = true;
                            }
                        }
                        let count = markdown
                            .diagnostics
                            .iter()
                            .filter(|d| d.location.is_some())
                            .count();
                        if count > 0 || markdown.truncated {
                            body_warnings.insert(
                                entry.path.clone(),
                                BodyWarningState {
                                    count,
                                    truncated: markdown.truncated,
                                },
                            );
                        }
                        diagnostics.extend(markdown.diagnostics);
                        for link in markdown.links {
                            let (path, fragment) = match &link.target {
                                LinkTarget::Internal { path, fragment }
                                    if link.bundle_path().is_some() || attachments =>
                                {
                                    (path, fragment)
                                }
                                _ => continue,
                            };
                            let bytes = entry.path.len()
                                + link.destination.len()
                                + path.len()
                                + fragment.as_ref().map_or(0, String::len);
                            if links.len() < MAX_VALIDATION_LINKS
                                && bytes <= MAX_VALIDATION_LINK_BYTES - link_bytes
                            {
                                link_bytes += bytes;
                                links.push((entry.path.clone(), link));
                            } else {
                                links_truncated = true;
                            }
                        }
                        Ok(diagnostics)
                    });
                match result {
                    Ok(diagnostics) => response.diagnostics.extend(diagnostics),
                    Err(e) => {
                        if interrupted(&e) {
                            complete = false;
                            response.issue(&e, &format!("@{}/{}", binding.name, entry.path));
                            break 'scan;
                        }
                        failures += 1;
                        response.diagnostics.push(Diagnostic::new(
                            &e.code,
                            e.message,
                            Some(&entry.path),
                        ));
                    }
                }
                response.data["checked"] =
                    json!(response.data["checked"].as_u64().unwrap_or(0) + 1);
                if !reserved(&entry.path) {
                    response.data["concepts"] =
                        json!(response.data["concepts"].as_u64().unwrap_or(0) + 1);
                }
            }
        }
        // A missing target can only be established after complete enumeration.
        // No target reads or network requests are needed for this check.
        let metadata_complete = complete;
        let mut links_checked = 0;
        if complete {
            for (path, link) in &links {
                if let Err(error) = self.control.check() {
                    complete = false;
                    response.issue(&error, &format!("@{}", binding.name));
                    break;
                }
                if let LinkTarget::Internal {
                    path: target,
                    fragment,
                } = &link.target
                {
                    let mut finding = None;
                    if link.bundle_path().is_some() {
                        links_checked += 1;
                        let exists = if target.is_empty() {
                            link.directory
                        } else {
                            existing.get(target) == Some(&link.directory)
                        };
                        if !exists {
                            finding = Some((
                                "BROKEN_LINK",
                                format!(
                                    "bundle link target missing or wrong file type: {}",
                                    link.destination
                                ),
                            ));
                        } else if anchors && !link.directory {
                            if let Some(fragment) = fragment {
                                match sections
                                    .get(target)
                                    .and_then(|s| s.contains_anchor(fragment))
                                {
                                    Some(true) => anchors_checked += 1,
                                    Some(false) => {
                                        anchors_checked += 1;
                                        finding = Some((
                                            "MISSING_ANCHOR",
                                            format!(
                                                "heading anchor absent under headings-v1: {}",
                                                link.destination
                                            ),
                                        ));
                                    }
                                    None => {
                                        anchors_skipped += 1;
                                        anchors_complete = false;
                                    }
                                }
                            }
                        }
                    } else if attachments {
                        attachments_checked += 1;
                        match self.reader.stat(&binding, target, self.control) {
                            Ok(Some(meta)) if !meta.directory => {}
                            Ok(_) => {
                                finding = Some((
                                    "MISSING_ATTACHMENT",
                                    format!(
                                        "attachment missing or not a file: {}",
                                        link.destination
                                    ),
                                ))
                            }
                            Err(error) => {
                                attachments_complete = false;
                                response.issue(&error, &format!("@{}/{}", binding.name, path));
                                if interrupted(&error) {
                                    break;
                                }
                            }
                        }
                    }
                    if let Some((code, message)) = finding {
                        let state = body_warnings.entry(path.clone()).or_default();
                        if state.count < MAX_BODY_WARNINGS {
                            state.count += 1;
                            response.diagnostics.push(body_warning(
                                code,
                                message,
                                &format!("@{}/{}", binding.name, path),
                                link.location.clone(),
                            ));
                        } else if !state.truncated {
                            state.truncated = true;
                            response.diagnostics.push(body_truncation_warning(&format!(
                                "@{}/{}",
                                binding.name, path
                            )));
                        }
                    }
                }
            }
        }
        if links_truncated {
            response.partial(
                "LINK_LIMIT",
                "validation link count or byte limit reached; select a smaller bundle",
            );
        }
        if anchors && anchors_limited {
            response.partial("ANCHOR_LIMIT", "heading inventory limit reached");
        }
        let checks_complete = complete
            && failures == 0
            && !links_truncated
            && !body_warnings.values().any(|state| state.truncated);
        if anchors {
            response.data["anchor_check"] = json!({"profile":"headings-v1","complete":checks_complete && anchors_complete,"checked":anchors_checked,"skipped":anchors_skipped});
        }
        if attachments {
            response.data["attachment_check"] = json!({"scope":"bundle_body_links","complete":checks_complete && attachments_complete,"checked":attachments_checked,"images_included":false});
        }
        response.data["failures"] = json!(failures);
        if let Err(error) = self.control.check() {
            complete = false;
            if !response.diagnostics.iter().any(|d| d.code == error.code) {
                response.issue(&error, &format!("@{}", binding.name));
            }
        }
        response.budget = json!({"scan_limit": options.max_scan, "scanned_entries": scan.used(),
            "link_limit": MAX_VALIDATION_LINKS, "links_checked": links_checked,
            "link_bytes_limit": MAX_VALIDATION_LINK_BYTES, "link_bytes_used": link_bytes,
            "anchor_bytes_limit": MAX_VALIDATION_LINK_BYTES, "anchor_bytes_used":section_bytes});
        response.data["link_check"] = json!({
            "scope": "bundle_markdown_targets",
            "complete": complete && failures == 0 && !links_truncated && !body_warnings.values().any(|state| state.truncated)
        });
        resources.check(
            &binding,
            self.reader,
            self.control,
            &mut response,
            metadata_complete && failures == 0,
        );
        response.data["warnings"] = json!(response
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count());
        if failures > 0 && response.exit_code == 0 {
            response.status = "error".into();
            response.exit_code = 1;
        }
        if observed {
            response.observe(
                &binding,
                self.now,
                "",
                if !metadata_complete {
                    "partial"
                } else {
                    "complete"
                },
            );
        } else {
            response.unavailable(&binding, "");
        }
        Ok(response)
    }
}

fn find_binding<'a>(bindings: &'a [BundleBinding], name: &str) -> Result<&'a BundleBinding> {
    let mut matches = bindings.iter().filter(|binding| binding.name == name);
    let binding = matches
        .next()
        .ok_or_else(|| Error::new("BUNDLE_NOT_FOUND", format!("unknown bundle: {name}"), 3))?;
    if matches.next().is_some() {
        return Err(Error::new(
            "AMBIGUOUS_BUNDLE",
            "alias exists in project and user scopes; select --scope",
            2,
        ));
    }
    Ok(binding)
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

fn cursor_revision(binding: &BundleBinding, hash: &str) -> String {
    binding.git.as_ref().map_or_else(
        || hash.into(),
        |git| format!("{}:{}:{hash}", git.commit, git.bundle_path),
    )
}
