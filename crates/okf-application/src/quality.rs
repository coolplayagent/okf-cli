use crate::Response;
use okf_core::{Diagnostic, Severity};
use serde_json::{json, Value};

impl Response {
    /// Apply a caller-selected policy after validation and operational diagnostics.
    /// A bounded diagnostics list is a lower bound, never an invented total.
    pub fn apply_warning_limit(&mut self, limit: u64) {
        let truncated = |code: &str| {
            matches!(
                code,
                "METADATA_DIAGNOSTICS_TRUNCATED" | "BODY_DIAGNOSTICS_TRUNCATED"
            )
        };
        let observed = self
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning && !truncated(&d.code))
            .count() as u64;
        let complete = self.data["failures"] == 0
            && self.data["link_check"]["complete"] == true
            && self.data["resource_check"]["complete"] == true
            && ["anchor_check", "attachment_check"].iter().all(|key| {
                self.data
                    .get(key)
                    .is_none_or(|check| check["complete"] == true)
            })
            && !self.diagnostics.iter().any(|d| {
                truncated(&d.code) || matches!(d.code.as_str(), "CANCELLED" | "DEADLINE_EXCEEDED")
            });
        let exceeded = observed > limit;
        let passed = if exceeded {
            json!(false)
        } else if complete {
            json!(true)
        } else {
            Value::Null
        };
        self.data["quality_gate"] = json!({
            "max_warnings": limit, "warnings_observed": observed,
            "warnings_total": if complete { json!(observed) } else { Value::Null },
            "complete": complete, "passed": passed
        });
        if !complete && self.exit_code == 0 {
            self.partial(
                "QUALITY_GATE_INCOMPLETE",
                "warning count is incomplete; quality gate cannot pass",
            );
        }
        if exceeded {
            self.diagnostics.push(Diagnostic::new(
                "WARNING_LIMIT_EXCEEDED",
                "observed warnings exceed --max-warnings",
                None,
            ));
            // Partial scans and operational errors retain their existing exit code.
            if self.exit_code == 0 {
                self.status = "error".into();
                self.exit_code = 1;
            }
        }
    }
}
