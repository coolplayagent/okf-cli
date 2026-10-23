//! Static Attested Computation contracts. No code or resources are executed.
use crate::{markdown::body_warning, ByteRange, Diagnostic};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Tag, TagEnd};
use serde_json::{Map, Value};
use std::ops::Range;

pub(crate) fn is_attested(frontmatter: &Map<String, Value>) -> bool {
    frontmatter.get("type").and_then(Value::as_str) == Some("Attested Computation")
}

#[derive(Default)]
pub(crate) struct ComputationBody {
    file_declared: bool,
    container_depth: usize,
    heading: Option<(String, Range<usize>)>,
    section: bool,
    section_location: Option<Range<usize>>,
    fence: Option<(Range<usize>, bool)>,
    second_fence: Option<Range<usize>>,
    empty_fence: Option<Range<usize>>,
    first_fence: Option<Range<usize>>,
}

impl ComputationBody {
    pub(crate) fn new(frontmatter: &Map<String, Value>) -> Self {
        Self {
            file_declared: frontmatter.contains_key("computation"),
            ..Self::default()
        }
    }

    pub(crate) fn event(&mut self, event: &Event<'_>, range: Range<usize>) {
        match event {
            Event::Start(Tag::BlockQuote(_) | Tag::List(_) | Tag::FootnoteDefinition(_)) => {
                self.container_depth += 1;
                return;
            }
            Event::End(TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::FootnoteDefinition) => {
                self.container_depth -= 1;
                return;
            }
            _ if self.container_depth > 0 => return,
            _ => {}
        }
        match event {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            }) => {
                self.heading = Some((String::new(), range));
                self.section = false;
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some((title, _)) = &mut self.heading {
                    title.push_str(text);
                }
                if let Some((_, nonempty)) = &mut self.fence {
                    *nonempty |= text.chars().any(|character| !character.is_whitespace());
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((title, _)) = &mut self.heading {
                    title.push(' ');
                }
            }
            Event::End(TagEnd::Heading(HeadingLevel::H1)) => {
                if let Some((title, location)) = self.heading.take() {
                    self.section = title.trim() == "Computation";
                    if self.section && self.section_location.is_none() {
                        self.section_location = Some(location);
                    }
                }
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) if self.section => {
                self.fence = Some((range, false));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((location, nonempty)) = self.fence.take() {
                    if self.first_fence.is_none() {
                        self.first_fence = Some(location.clone());
                    } else if self.second_fence.is_none() {
                        self.second_fence = Some(location.clone());
                    }
                    if !nonempty && self.empty_fence.is_none() {
                        self.empty_fence = Some(location);
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn finish(self, path: &str, body_start: usize) -> Vec<Diagnostic> {
        let mut diagnostics = vec![];
        let warning = |code: &str, message: &str, range: Range<usize>| {
            body_warning(
                code,
                message,
                path,
                ByteRange {
                    byte_start: body_start + range.start,
                    byte_end: body_start + range.end,
                },
            )
        };
        if !self.file_declared && self.first_fence.is_none() {
            diagnostics.push(warning("MISSING_COMPUTATION", "expected one fenced computation under # Computation or a computation file declaration", self.section_location.unwrap_or(0..0)));
        }
        if self.file_declared {
            if let Some(range) = self.first_fence {
                diagnostics.push(warning(
                    "CONFLICTING_COMPUTATION",
                    "file declaration and inline computation are both present",
                    range,
                ));
            }
        }
        if let Some(range) = self.second_fence {
            diagnostics.push(warning(
                "MULTIPLE_COMPUTATIONS",
                "expected a single inline computation",
                range,
            ));
        }
        if let Some(range) = self.empty_fence {
            diagnostics.push(warning(
                "EMPTY_COMPUTATION",
                "inline computation is empty",
                range,
            ));
        }
        diagnostics
    }
}
