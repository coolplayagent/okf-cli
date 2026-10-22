//! Markdown relationships and provenance joins, without storage access.
use crate::{logical_path, ByteRange, Diagnostic, Error, Result};
use percent_encoding::percent_decode_str;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde_json::{Map, Value};
use std::{collections::BTreeSet, ops::Range};

pub const MAX_REFERENCES: usize = 256;
pub const MAX_BODY_WARNINGS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    External,
    Internal {
        path: String,
        fragment: Option<String>,
    },
}

/// Resolve Markdown paths separately from CLI paths: ../ is valid inside a bundle.
pub fn resolve_link(document: &str, destination: &str) -> Result<LinkTarget> {
    resolve_destination(document, destination).map(|(target, _)| target)
}

fn resolve_destination(document: &str, destination: &str) -> Result<(LinkTarget, bool)> {
    logical_path(document)?;
    if destination.starts_with("//") || uri_scheme(destination) {
        return Ok((LinkTarget::External, false));
    }
    let (address, fragment) = destination
        .split_once('#')
        .map_or((destination, None), |(path, fragment)| {
            (path, Some(fragment.to_owned()))
        });
    let address = address.split_once('?').map_or(address, |(path, _)| path);
    let decoded = percent_decode_str(address)
        .decode_utf8()
        .map_err(|_| Error::new("INVALID_LINK_PATH", "link path is not UTF-8", 2))?;
    if decoded
        .chars()
        .any(|c| c.is_control() || matches!(c, '\\' | ':'))
    {
        return Err(Error::new(
            "INVALID_LINK_PATH",
            "invalid bundle link path",
            2,
        ));
    }
    let directory =
        decoded.ends_with('/') || matches!(decoded.rsplit('/').next(), Some("." | ".."));
    let mut parts: Vec<&str> = if decoded.starts_with('/') {
        vec![]
    } else if decoded.is_empty() {
        document.split('/').collect()
    } else {
        document
            .rsplit_once('/')
            .map_or(vec![], |(parent, _)| parent.split('/').collect())
    };
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(Error::new(
                        "LINK_PATH_ESCAPE",
                        "link escapes bundle root",
                        4,
                    ));
                }
            }
            _ => parts.push(part),
        }
    }
    Ok((
        LinkTarget::Internal {
            path: parts.join("/"),
            fragment,
        },
        directory,
    ))
}

fn uri_scheme(value: &str) -> bool {
    value.split_once(':').is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"+.-".contains(&c))
    })
}

#[derive(Debug)]
pub struct BodyLink {
    pub destination: String,
    pub target: LinkTarget,
    pub directory: bool,
    pub location: ByteRange,
}

impl BodyLink {
    /// Only Markdown documents, explicit directories, and same-document links
    /// participate in the bundle navigation check. Asset URIs remain opaque.
    pub fn bundle_path(&self) -> Option<&str> {
        match &self.target {
            LinkTarget::Internal { path, .. }
                if path.ends_with(".md") || path.is_empty() || self.directory =>
            {
                Some(path)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct Markdown {
    pub links: Vec<BodyLink>,
    pub diagnostics: Vec<Diagnostic>,
    pub truncated: bool,
}

struct Footnote {
    label: String,
    range: Range<usize>,
    defined: bool,
}

pub fn body_warning(
    code: &str,
    message: impl Into<String>,
    path: &str,
    location: ByteRange,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::warning(code, message, None);
    diagnostic.path = Some(path.to_owned());
    diagnostic.location = Some(location);
    diagnostic
}

pub fn body_truncation_warning(path: &str) -> Diagnostic {
    let mut diagnostic = Diagnostic::warning(
        "BODY_DIAGNOSTICS_TRUNCATED",
        "body reference or diagnostic limit reached",
        None,
    );
    diagnostic.path = Some(path.to_owned());
    diagnostic
}

impl Markdown {
    /// `body_start` and returned ranges address the unmodified complete file.
    pub fn parse(
        raw: &str,
        body_start: usize,
        document: &str,
        frontmatter: Option<&Map<String, Value>>,
    ) -> Self {
        let mut result = Self::default();
        let mut references = vec![];
        let mut text: Option<Range<usize>> = None;
        let mut image_depth = 0;
        let mut code_block = false;
        let mut link_count = 0;
        let sources: BTreeSet<&str> = frontmatter
            .and_then(|fm| fm.get("sources"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|source| source.get("id").and_then(Value::as_str))
            .collect();
        let body = &raw[body_start..];
        for (event, range) in
            Parser::new_ext(body, Options::ENABLE_FOOTNOTES | Options::ENABLE_TABLES)
                .into_offset_iter()
        {
            // Undefined footnotes are literal text in GFM. Inspect only contiguous text
            // events so code, HTML, escaped brackets, and images remain excluded.
            if matches!(event, Event::Text(_)) && image_depth == 0 && !code_block {
                match &mut text {
                    Some(previous) if previous.end == range.start => previous.end = range.end,
                    _ => {
                        if let Some(previous) = text.replace(range) {
                            collect_undefined(
                                body,
                                previous,
                                &mut references,
                                &mut result.truncated,
                            );
                        }
                    }
                }
                continue;
            }
            if let Some(previous) = text.take() {
                collect_undefined(body, previous, &mut references, &mut result.truncated);
            }
            match event {
                Event::Start(Tag::CodeBlock(_)) => code_block = true,
                Event::End(TagEnd::CodeBlock) => code_block = false,
                Event::Start(Tag::Image { .. }) => image_depth += 1,
                Event::End(TagEnd::Image) => image_depth -= 1,
                _ if image_depth > 0 => {}
                Event::Start(Tag::Link { dest_url, .. }) => {
                    if link_count >= MAX_REFERENCES {
                        result.truncated = true;
                        continue;
                    }
                    link_count += 1;
                    let location = byte_range(range, body_start);
                    match resolve_destination(document, &dest_url) {
                        Ok((target, directory)) => {
                            result.links.push(BodyLink {
                                destination: dest_url.into_string(),
                                target,
                                directory,
                                location,
                            });
                        }
                        Err(error) => result.warn(body_warning(
                            &error.code,
                            error.message,
                            document,
                            location,
                        )),
                    }
                }
                Event::FootnoteReference(label) => {
                    if references.len() < MAX_REFERENCES {
                        // GFM emits this event only for references with definitions,
                        // including case-insensitive matches handled by the parser.
                        references.push(Footnote {
                            label: label.into_string(),
                            range,
                            defined: true,
                        });
                    } else {
                        result.truncated = true;
                    }
                }
                _ => {}
            }
        }
        if let Some(previous) = text.take() {
            collect_undefined(body, previous, &mut references, &mut result.truncated);
        }
        if frontmatter.is_some() {
            for Footnote {
                label,
                range,
                defined,
            } in references
            {
                let location = byte_range(range, body_start);
                if !sources.contains(label.as_str()) {
                    result.warn(body_warning(
                        "UNRESOLVED_SOURCE",
                        format!("footnote [^{label}] matches no sources[].id"),
                        document,
                        location.clone(),
                    ));
                }
                if !defined {
                    result.warn(body_warning(
                        "UNDEFINED_FOOTNOTE",
                        format!("footnote [^{label}] has no definition"),
                        document,
                        location,
                    ));
                }
            }
        }
        if result.truncated {
            result.diagnostics.push(body_truncation_warning(document));
        }
        result
    }

    fn warn(&mut self, diagnostic: Diagnostic) {
        if self.diagnostics.len() < MAX_BODY_WARNINGS {
            self.diagnostics.push(diagnostic);
        } else {
            self.truncated = true;
        }
    }
}

fn byte_range(range: Range<usize>, base: usize) -> ByteRange {
    ByteRange {
        byte_start: base + range.start,
        byte_end: base + range.end,
    }
}

fn collect_undefined(
    body: &str,
    range: Range<usize>,
    references: &mut Vec<Footnote>,
    truncated: &mut bool,
) {
    let mut cursor = range.start;
    while let Some(offset) = body[cursor..range.end].find("[^") {
        let start = cursor + offset;
        cursor = start + 2;
        let Some(end) = body[cursor..range.end].find(']') else {
            break;
        };
        let label = &body[cursor..cursor + end];
        let escaped = body[..start]
            .bytes()
            .rev()
            .take_while(|&b| b == b'\\')
            .count()
            % 2
            == 1;
        if !escaped
            && !label.is_empty()
            && !label
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, '[' | '\\'))
        {
            if references.len() >= MAX_REFERENCES {
                *truncated = true;
                break;
            }
            references.push(Footnote {
                label: label.to_owned(),
                range: start..cursor + end + 1,
                defined: false,
            });
        }
        cursor += end + 1;
    }
}
