//! Parsed heading boundaries shared by excerpts and opt-in anchor checking.
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use std::collections::BTreeSet;

pub const MAX_HEADINGS: usize = 4096;
#[derive(Debug)]
pub struct Heading {
    pub start: usize,
    pub end: usize,
    pub level: u8,
    pub anchor: String,
}
#[derive(Debug, Default)]
pub struct Sections {
    pub headings: Vec<Heading>,
    pub complete: bool,
    pub custom_html: bool,
    pub html_anchors: BTreeSet<String>,
}
impl Sections {
    pub fn parse(raw: &str, body_start: usize) -> Self {
        let mut result = Self {
            complete: true,
            ..Self::default()
        };
        let mut current = None;
        let mut title = String::new();
        let mut anchors = BTreeSet::new();
        for (event, range) in Parser::new(&raw[body_start..]).into_offset_iter() {
            match event {
                Event::Start(Tag::Heading { level, .. }) => {
                    if result.headings.len() == MAX_HEADINGS {
                        result.complete = false;
                        break;
                    }
                    current = Some((body_start + range.start, level as u8));
                    title.clear();
                }
                Event::Text(text) | Event::Code(text) if current.is_some() => title.push_str(&text),
                Event::SoftBreak | Event::HardBreak if current.is_some() => title.push(' '),
                Event::End(TagEnd::Heading(_)) => {
                    if let Some((start, level)) = current.take() {
                        // Explicit headings-v1 profile, not a promise about renderer-specific IDs.
                        let base: String = title
                            .to_lowercase()
                            .chars()
                            .filter_map(|c| {
                                if c.is_whitespace() {
                                    Some('-')
                                } else if c.is_alphanumeric() || matches!(c, '_' | '-') {
                                    Some(c)
                                } else {
                                    None
                                }
                            })
                            .collect();
                        let mut anchor = base.clone();
                        let mut suffix = 0;
                        while !anchors.insert(anchor.clone()) {
                            suffix += 1;
                            anchor = format!("{base}-{suffix}");
                        }
                        result.headings.push(Heading {
                            start,
                            end: raw.len(),
                            level,
                            anchor,
                        });
                    }
                }
                Event::Html(html) | Event::InlineHtml(html) => {
                    result.custom_html = true;
                    collect_html_anchors(&html, &mut result.html_anchors);
                }
                _ => {}
            }
        }
        let mut stack: Vec<usize> = vec![];
        for index in 0..result.headings.len() {
            while let Some(&previous) = stack.last() {
                if result.headings[previous].level < result.headings[index].level {
                    break;
                }
                result.headings[previous].end = result.headings[index].start;
                stack.pop();
            }
            stack.push(index);
        }
        result
    }
    pub fn containing(&self, offset: usize) -> Option<&Heading> {
        self.headings
            .iter()
            .rev()
            .find(|h| h.start <= offset && offset < h.end)
    }
    /// Unknown custom HTML anchors and truncated heading inventories stay inconclusive.
    pub fn contains_anchor(&self, fragment: &str) -> Option<bool> {
        let decoded = percent_encoding::percent_decode_str(fragment)
            .decode_utf8()
            .ok()?;
        if decoded.is_empty() {
            return Some(true);
        }
        if self.headings.iter().any(|h| h.anchor == decoded)
            || self.html_anchors.contains(decoded.as_ref())
        {
            Some(true)
        } else if self.complete && !self.custom_html {
            Some(false)
        } else {
            None
        }
    }
}

/// Extracts `id` and `name` attribute values from raw HTML fragments, so that
/// explicit HTML anchors can be resolved for the anchor check without a full
/// HTML parser. Values are matched for quoted and unquoted forms.
fn collect_html_anchors(html: &str, anchors: &mut BTreeSet<String>) {
    let mut rest = html;
    while let Some(pos) = rest.find(['i', 'n']) {
        let candidate = &rest[pos..];
        let name_len = if candidate.starts_with("id") {
            2
        } else if candidate.starts_with("name") {
            4
        } else {
            rest = &rest[pos + 1..];
            continue;
        };
        // Left boundary: the name must not continue an identifier.
        if pos > 0 {
            let prev = rest[..pos].chars().next_back().unwrap();
            if prev.is_alphanumeric() || matches!(prev, '-' | '_' | ':') {
                rest = &rest[pos + name_len..];
                continue;
            }
        }
        // Right boundary: the name must not continue an identifier.
        let after_name = &candidate[name_len..];
        if after_name.chars().next().is_some_and(char::is_alphanumeric) {
            rest = &rest[pos + name_len..];
            continue;
        }
        let mut after = after_name.trim_start();
        if !after.starts_with('=') {
            rest = after;
            continue;
        }
        after = after[1..].trim_start();
        let (value, next) = read_html_attr_value(after);
        if !value.is_empty() {
            anchors.insert(value);
        }
        rest = next.unwrap_or_default();
    }
}

fn read_html_attr_value(after: &str) -> (String, Option<&str>) {
    if let Some(inner) = after.strip_prefix('"') {
        match inner.find('"') {
            Some(end) => (inner[..end].to_owned(), Some(&inner[end + 1..])),
            None => (String::new(), None),
        }
    } else if let Some(inner) = after.strip_prefix('\'') {
        match inner.find('\'') {
            Some(end) => (inner[..end].to_owned(), Some(&inner[end + 1..])),
            None => (String::new(), None),
        }
    } else {
        let end = after
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(after.len());
        (after[..end].to_owned(), Some(&after[end..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn headings_preserve_boundaries_and_ignore_code() {
        let raw = "\u{feff}# ignored frontmatter\r\n# 中文\r\n## **Child** `code`\r\n```\n# fake\n```\n## Child code\ntext\n# End\n";
        let sections = Sections::parse(raw, raw.find("# 中文").unwrap());
        assert_eq!(sections.headings.len(), 4);
        assert_eq!(sections.headings[2].anchor, "child-code-1");
        let section = sections.containing(raw.find("# fake").unwrap()).unwrap();
        assert_eq!(section.anchor, "child-code");
        assert!(raw[section.start..section.end].ends_with("```\n"));
        assert_eq!(sections.contains_anchor("%E4%B8%AD%E6%96%87"), Some(true));
        assert_eq!(sections.contains_anchor("fake"), Some(false));
        assert_eq!(
            Sections::parse("# A\n<a id='custom'></a>", 0).contains_anchor("custom"),
            Some(true)
        );
        assert_eq!(
            Sections::parse("# A\n<a name='legacy'></a>", 0).contains_anchor("legacy"),
            Some(true)
        );
        assert_eq!(
            Sections::parse("# A\n<div id=\"pane\">", 0).contains_anchor("pane"),
            Some(true)
        );
        assert_eq!(
            Sections::parse("# A\n<a id='custom'></a>", 0).contains_anchor("missing"),
            None
        );
    }
}
