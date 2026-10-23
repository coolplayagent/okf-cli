//! Direct, evidenced relationships from one published index generation.
use crate::{App, Index, ReadOptions, Response};
use okf_core::{
    markdown::{resolve_link, LinkTarget, Markdown},
    Document, Error, Result,
};
use serde_json::json;
use std::collections::BTreeSet;

impl App<'_> {
    pub fn neighbors(
        &self,
        reference: &str,
        direction: &str,
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
        let paths: BTreeSet<_> = snapshot.documents.iter().map(|d| d.path.as_str()).collect();
        if !paths.contains(path.as_str()) {
            return Err(Error::new("NOT_FOUND", "file absent from index", 3));
        }
        let mut response = Response::new(
            "neighbors",
            json!({"target":reference,"direction":direction,"generation":snapshot.generation,"items":[]}),
        );
        let mut items = vec![];
        let mut used = 0;
        let mut scanned = 0;
        let mut total = 0;
        let mut complete = true;
        for document in &snapshot.documents {
            if direction == "outgoing" && document.path != path {
                continue;
            }
            if let Err(e) = self.control.check() {
                response.partial(&e.code, &e.message);
                complete = false;
                break;
            }
            if scanned >= options.max_scan {
                response.partial("SCAN_LIMIT", "relationship scan budget reached");
                complete = false;
                break;
            }
            scanned += 1;
            let doc = if okf_core::reserved(&document.path) {
                None
            } else {
                Some(Document::parse(&document.raw)?)
            };
            let body_start = doc.as_ref().map(|d| d.body_start).unwrap_or_else(|| {
                okf_core::frontmatter(&document.raw).map_or(0, |(_, start)| start)
            });
            let markdown = Markdown::parse(
                &document.raw,
                body_start,
                &document.path,
                doc.as_ref().map(|d| &d.frontmatter),
            );
            if markdown.truncated {
                complete = false;
            }
            for diagnostic in &markdown.diagnostics {
                if matches!(
                    diagnostic.code.as_str(),
                    "INVALID_LINK_PATH" | "LINK_PATH_ESCAPE"
                ) {
                    complete = false;
                    response.issue(
                        &Error::new(&diagnostic.code, &diagnostic.message, 7),
                        &format!("@{name}/{}", document.path),
                    );
                }
            }
            let mut edges = vec![];
            for link in markdown.links {
                if let LinkTarget::Internal {
                    path: target,
                    fragment,
                } = link.target
                {
                    if !link.directory && target.ends_with(".md") {
                        edges.push((target, json!({"kind":"markdown_link","destination":link.destination,"fragment":fragment,"location":link.location})));
                    }
                }
            }
            if let Some(sources) = doc
                .as_ref()
                .and_then(|d| d.frontmatter.get("sources"))
                .and_then(|v| v.as_array())
            {
                if sources.len() > 256 {
                    complete = false;
                }
                for (i, source) in sources.iter().take(256).enumerate() {
                    if let Some(resource) = source.get("resource").and_then(|v| v.as_str()) {
                        match resolve_link("index.md", resource) {
                            Ok(LinkTarget::Internal {
                                path: target,
                                fragment,
                            }) if target.ends_with(".md") => {
                                edges.push((target, json!({"kind":"source","destination":resource,"fragment":fragment,"field":format!("/sources/{i}/resource")})));
                            }
                            Err(error) => {
                                complete = false;
                                response.issue(&error, &format!("@{name}/{}", document.path));
                            }
                            _ => {}
                        }
                    }
                }
            }
            for (target, evidence) in edges {
                let outgoing = document.path == path;
                let incoming = target == path;
                if !(outgoing && direction != "incoming" || incoming && direction != "outgoing") {
                    continue;
                }
                total += 1;
                let item = json!({"from":format!("@{name}/{}",document.path),"to":format!("@{name}/{target}"),
                    "direction":if outgoing {"outgoing"} else {"incoming"},"target_exists":paths.contains(target.as_str()),
                    "generation":snapshot.generation,"upstream_revision":snapshot.upstream_revision,
                    "evidence":{"path":document.path,"content_sha256":document.hash,"reference":evidence}});
                let size = serde_json::to_vec(&item).map_err(Error::io)?.len();
                if items.len() < options.limit && size <= options.budget.saturating_sub(used) {
                    used += size;
                    items.push(item);
                }
            }
        }
        if !complete && response.status == "ok" {
            response.partial("REFERENCE_LIMIT", "relationship references were truncated");
        }
        if items.len() < total {
            response.partial(
                "RESULT_LIMIT",
                "not all relationships fit the result budget",
            );
        }
        response.page["returned"] = json!(items.len());
        response.page["total"] = if complete {
            json!(total)
        } else {
            serde_json::Value::Null
        };
        let mut next_targets = BTreeSet::new();
        for item in &items {
            let target = if item["direction"] == "incoming" {
                item["from"].as_str()
            } else if item["target_exists"] == true {
                item["to"].as_str()
            } else {
                None
            };
            if let Some(target) = target {
                if next_targets.insert(target.to_owned()) {
                    response.next.push(json!({"operation":"read","target":target,"indexed":true,"generation":snapshot.generation}));
                }
            }
        }
        response.data["items"] = json!(items);
        response.data["relationship_check"] = json!({"complete":complete,"scope":if direction == "outgoing" {"document"} else {"bundle"},"source_paths":"bundle_root_relative"});
        response.budget = json!({"scan_limit":options.max_scan,"indexed_documents_scanned":scanned,"content_bytes_limit":options.budget,"content_bytes_used":used});
        crate::index::indexed_observation(&mut response, &binding, &snapshot, complete);
        response.bundles[0]["coverage"]["graph"] =
            json!(if complete { "complete" } else { "partial" });
        response.bundles[0]["coverage"]["scope"] = json!(if direction == "outgoing" {
            path.as_str()
        } else {
            ""
        });
        Ok(response)
    }
}
