use crate::{Diagnostic, Error, Result};
use chrono::{DateTime, FixedOffset};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

// A malformed array must not allocate one diagnostic per element without bound.
const MAX_WARNINGS: usize = 64;

pub(crate) fn timestamp(value: &str) -> Option<DateTime<FixedOffset>> {
    if !timestamp_shape(value) {
        return None;
    }
    DateTime::parse_from_rfc3339(value).ok().or_else(|| {
        let utc = value
            .strip_suffix('Z')
            .or_else(|| value.strip_suffix('z'))
            .map(|value| format!("{value}+0000"));
        let value = utc.as_deref().unwrap_or(value);
        [
            "%Y-%m-%dT%H:%M:%S%.f%z",
            "%Y-%m-%dT%H:%M%:z",
            "%Y-%m-%dT%H:%M%z",
            "%Y%m%dT%H%M%S%.f%z",
        ]
        .iter()
        .find_map(|format| DateTime::parse_from_str(value, format).ok())
    })
}

fn digits(value: &str, width: usize) -> bool {
    value.len() == width && value.bytes().all(|byte| byte.is_ascii_digit())
}

// Chrono's strptime parser accepts unpadded components. Check the supported ISO
// shapes before parsing calendar values so malformed dates cannot imply freshness.
fn timestamp_shape(value: &str) -> bool {
    let Some((date, time)) = value.split_once(['T', 't']) else {
        return false;
    };
    let extended = date.len() == 10
        && date.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 4 | 7) {
                byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        });
    if !extended && !digits(date, 8) {
        return false;
    }
    let clock = if let Some(clock) = time.strip_suffix('Z').or_else(|| time.strip_suffix('z')) {
        clock
    } else {
        let Some(index) = time.rfind(['+', '-']) else {
            return false;
        };
        let offset = &time[index + 1..];
        let offset_valid = digits(offset, 4)
            || (offset.len() == 5
                && offset.bytes().enumerate().all(|(index, byte)| {
                    if index == 2 {
                        byte == b':'
                    } else {
                        byte.is_ascii_digit()
                    }
                }));
        if !offset_valid {
            return false;
        }
        &time[..index]
    };
    let clock = if let Some((clock, fraction)) = clock.split_once('.') {
        if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
        clock
    } else {
        clock
    };
    if extended {
        matches!(clock.len(), 5 | 8)
            && clock.bytes().enumerate().all(|(index, byte)| {
                if matches!(index, 2 | 5) {
                    byte == b':'
                } else {
                    byte.is_ascii_digit()
                }
            })
    } else {
        digits(clock, 6)
    }
}

pub(crate) fn valid_actor(value: &str) -> bool {
    ["human:", "process:"].iter().any(|prefix| {
        value
            .strip_prefix(prefix)
            .is_some_and(|id| !id.trim().is_empty())
    }) || value.split_once('/').is_some_and(|(producer, version)| {
        !producer.trim().is_empty() && !version.trim().is_empty()
    })
}

struct Check {
    warnings: Vec<Diagnostic>,
}

impl Check {
    fn computation(&mut self, frontmatter: &Map<String, Value>) {
        self.string(frontmatter.get("runtime"), "/runtime", true);
        if let Some(parameters) = frontmatter.get("parameters") {
            if let Some(parameters) = parameters.as_array() {
                let mut names = BTreeSet::new();
                for (index, parameter) in parameters.iter().enumerate() {
                    let field = format!("/parameters/{index}");
                    let Some(parameter) = parameter.as_object() else {
                        self.warn(
                            "INVALID_METADATA_TYPE",
                            &field,
                            "expected a parameter object",
                        );
                        continue;
                    };
                    for name in ["name", "type"] {
                        self.string(parameter.get(name), &format!("{field}/{name}"), true);
                    }
                    match parameter.get("required") {
                        None => self.warn(
                            "MISSING_METADATA_FIELD",
                            &format!("{field}/required"),
                            "parameter must declare whether it is required",
                        ),
                        Some(value) if !value.is_boolean() => self.warn(
                            "INVALID_METADATA_TYPE",
                            &format!("{field}/required"),
                            "expected a boolean",
                        ),
                        _ => {}
                    }
                    if let Some(name) = parameter
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|name| !name.trim().is_empty())
                    {
                        if !names.insert(name) {
                            self.warn(
                                "DUPLICATE_PARAMETER_NAME",
                                &format!("{field}/name"),
                                "parameter names should be unique within a computation",
                            );
                        }
                    }
                }
            } else {
                self.warn(
                    "INVALID_METADATA_TYPE",
                    "/parameters",
                    "expected a list of parameters",
                );
            }
        }
        if let Some(value) = frontmatter.get("computation") {
            self.string(Some(value), "/computation", true);
        }
        for name in ["executor", "attester"] {
            let Some(value) = frontmatter.get(name) else {
                continue;
            };
            let field = format!("/{name}");
            let Some(object) = value.as_object() else {
                self.warn(
                    "INVALID_METADATA_TYPE",
                    &field,
                    "expected a resource object",
                );
                continue;
            };
            self.string(object.get("resource"), &format!("{field}/resource"), true);
            if name == "executor" {
                if let Some(receipt) = object.get("receipt") {
                    if let Some(fields) = receipt.as_array() {
                        for (index, value) in fields.iter().enumerate() {
                            self.string(Some(value), &format!("{field}/receipt/{index}"), true);
                        }
                    } else {
                        self.warn(
                            "INVALID_METADATA_TYPE",
                            &format!("{field}/receipt"),
                            "expected a list of receipt field names",
                        );
                    }
                }
            }
        }
    }

    fn warn(&mut self, code: &str, field: &str, message: &str) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings
                .push(Diagnostic::warning(code, message, Some(field)));
        } else if self.warnings.len() == MAX_WARNINGS {
            self.warnings.push(Diagnostic::warning(
                "METADATA_DIAGNOSTICS_TRUNCATED",
                "additional metadata warnings omitted after 64 findings",
                None,
            ));
        }
    }

    fn string(&mut self, value: Option<&Value>, field: &str, required: bool) {
        match value {
            None if required => self.warn(
                "MISSING_METADATA_FIELD",
                field,
                "field is required within this metadata object",
            ),
            Some(Value::String(value)) if required && value.trim().is_empty() => self.warn(
                "INVALID_METADATA_VALUE",
                field,
                "expected a non-empty string",
            ),
            Some(value) if !value.is_string() => {
                self.warn("INVALID_METADATA_TYPE", field, "expected a string")
            }
            _ => {}
        }
    }

    fn datetime(&mut self, value: Option<&Value>, field: &str, required: bool) {
        match value {
            None if required => self.warn(
                "MISSING_METADATA_FIELD",
                field,
                "expected a datetime with an explicit UTC offset",
            ),
            Some(value) if value.as_str().and_then(timestamp).is_none() => self.warn(
                "INVALID_TIMESTAMP",
                field,
                "expected a datetime with an explicit UTC offset",
            ),
            _ => {}
        }
    }

    fn actor(&mut self, value: Option<&Value>, field: &str) {
        self.string(value, field, true);
        if let Some(value) = value
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            if !valid_actor(value) {
                self.warn(
                    "INVALID_ACTOR",
                    field,
                    "expected human:<id>, process:<id>, or <producer>/<version>",
                );
            }
        }
    }

    fn event(&mut self, value: &Value, field: &str, verified: bool) {
        let Some(object) = value.as_object() else {
            self.warn(
                "INVALID_METADATA_TYPE",
                field,
                "expected an actor event object",
            );
            return;
        };
        self.actor(object.get("by"), &format!("{field}/by"));
        self.datetime(object.get("at"), &format!("{field}/at"), verified);
    }

    fn window(&mut self, value: &Value, field: &str) {
        let Some(object) = value.as_object() else {
            self.warn(
                "INVALID_METADATA_TYPE",
                field,
                "expected a usage window object",
            );
            return;
        };
        self.datetime(object.get("from"), &format!("{field}/from"), true);
        self.datetime(object.get("to"), &format!("{field}/to"), true);
        let datetime = |key| object.get(key).and_then(Value::as_str).and_then(timestamp);
        if let (Some(from), Some(to)) = (datetime("from"), datetime("to")) {
            if from > to {
                self.warn(
                    "INVALID_TIME_RANGE",
                    field,
                    "usage window from must not follow to",
                );
            }
        }
    }

    fn sources(&mut self, value: &Value) {
        let Some(sources) = value.as_array() else {
            self.warn(
                "INVALID_METADATA_TYPE",
                "/sources",
                "expected a list of provenance materials",
            );
            return;
        };
        let mut ids = BTreeSet::new();
        for (index, source) in sources.iter().enumerate() {
            let field = format!("/sources/{index}");
            let Some(source) = source.as_object() else {
                self.warn("INVALID_METADATA_TYPE", &field, "expected a source object");
                continue;
            };
            // resource may be a URI, an internal path, or a population/scope descriptor.
            self.string(source.get("resource"), &format!("{field}/resource"), true);
            for name in ["id", "title", "author"] {
                self.string(source.get(name), &format!("{field}/{name}"), false);
            }
            if let Some(id) = source.get("id").and_then(Value::as_str) {
                if id.trim().is_empty() {
                    self.warn(
                        "INVALID_METADATA_VALUE",
                        &format!("{field}/id"),
                        "source id must be non-empty when present",
                    );
                } else if !ids.insert(id) {
                    self.warn(
                        "DUPLICATE_SOURCE_ID",
                        &format!("{field}/id"),
                        "source id must uniquely identify a provenance material within the concept",
                    );
                }
            }
            if source.get("usage_count").is_some_and(|value| {
                !value.is_number() || !value.to_string().bytes().all(|byte| byte.is_ascii_digit())
            }) {
                self.warn(
                    "INVALID_METADATA_VALUE",
                    &format!("{field}/usage_count"),
                    "expected a non-negative integer",
                );
            }
            self.datetime(
                source.get("last_modified"),
                &format!("{field}/last_modified"),
                false,
            );
            if let Some(window) = source.get("usage_window") {
                self.window(window, &format!("{field}/usage_window"));
            }
        }
    }
}

pub(crate) fn diagnostics(frontmatter: &Map<String, Value>, path: &str) -> Vec<Diagnostic> {
    let mut check = Check { warnings: vec![] };
    for name in ["title", "description", "resource"] {
        check.string(frontmatter.get(name), &format!("/{name}"), false);
    }
    if let Some(tags) = frontmatter.get("tags") {
        if let Some(tags) = tags.as_array() {
            for (index, tag) in tags.iter().enumerate() {
                check.string(Some(tag), &format!("/tags/{index}"), false);
            }
        } else {
            check.warn(
                "INVALID_METADATA_TYPE",
                "/tags",
                "expected a list of strings",
            );
        }
    }
    if let Some(generated) = frontmatter.get("generated") {
        check.event(generated, "/generated", false);
    }
    if let Some(verified) = frontmatter.get("verified") {
        match verified {
            Value::Object(_) => check.event(verified, "/verified", true),
            Value::Array(events) => {
                for (index, event) in events.iter().enumerate() {
                    check.event(event, &format!("/verified/{index}"), true);
                }
            }
            _ => check.warn(
                "INVALID_METADATA_TYPE",
                "/verified",
                "expected an event object or a list of event objects",
            ),
        }
    }
    if let Some(sources) = frontmatter.get("sources") {
        check.sources(sources);
    }
    if let Some(window) = frontmatter.get("usage_window") {
        check.window(window, "/usage_window");
    }
    if let Some(status) = frontmatter.get("status") {
        check.string(Some(status), "/status", false);
        if status
            .as_str()
            .is_some_and(|status| !matches!(status, "draft" | "stable" | "deprecated"))
        {
            check.warn(
                "INVALID_STATUS",
                "/status",
                "expected draft, stable, or deprecated",
            );
        }
    }
    check.datetime(frontmatter.get("stale_after"), "/stale_after", false);
    if crate::computation::is_attested(frontmatter) {
        check.computation(frontmatter);
    }
    for warning in &mut check.warnings {
        warning.path = Some(path.into());
    }
    check.warnings
}

/// Structural checks for OKF reserved resources; optional concept guidance is separate.
pub fn validate_reserved(path: &str, raw: &str) -> Result<()> {
    if path.ends_with("index.md") && raw.trim_start_matches('\u{feff}').starts_with("---") {
        let (fm, _) = crate::frontmatter(raw)?;
        if path != "index.md" || fm.keys().any(|key| key != "okf_version") {
            return Err(Error::new(
                "INVALID_OKF",
                "index frontmatter is only allowed at root with okf_version",
                1,
            ));
        }
    }
    if path.ends_with("log.md") {
        for line in raw.lines().filter_map(|line| line.strip_prefix("## ")) {
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
