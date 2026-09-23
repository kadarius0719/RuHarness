//! Content-bound oracle verdicts (docs/SCHEMAS.md "Verdicts"): every verdict
//! records blake3 digests of what was actually tested — never timestamps —
//! so `verified` is a checkable claim, not an assertion.

use crate::error::Error;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Version of the verdict schema this build understands.
pub const VERDICT_SCHEMA_VERSION: u64 = 1;
/// Value of the `schema` envelope field.
pub const VERDICT_SCHEMA_NAME: &str = "ruharness-verdict";

/// One oracle check's outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// Check name (e.g. `differential-driver`).
    pub name: String,
    /// Whether it passed.
    pub passed: bool,
    /// Human-readable evidence (byte counts, first-diff offset, …).
    pub detail: String,
}

/// Digests of everything the oracle consumed, computed from the tree it
/// actually tested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerdictInputs {
    /// File-set hash of the unit's files plus transitive project includes.
    pub unit_source: String,
    /// Hash of the differential driver (empty when the kind has none).
    #[serde(default)]
    pub driver: String,
    /// File-set hash of the unit's Rust crate tree.
    #[serde(default)]
    pub rust_crate: String,
    /// Source files replaced at link time.
    #[serde(default)]
    pub replaces: Vec<String>,
    /// Toolchain identifiers (e.g. `rustc -V`, `cc --version` first lines).
    #[serde(default)]
    pub toolchain: Vec<String>,
}

/// A complete oracle verdict for one unit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    /// Envelope: always [`VERDICT_SCHEMA_NAME`].
    pub schema: String,
    /// Envelope version.
    pub schema_version: u64,
    /// Unit id this verdict is bound to.
    pub unit: String,
    /// True iff every check passed.
    pub green: bool,
    /// What was tested.
    pub inputs: VerdictInputs,
    /// Individual check outcomes.
    pub checks: Vec<Check>,
}

impl Verdict {
    /// Build a verdict; `green` is derived from the checks.
    pub fn new(unit: impl Into<String>, inputs: VerdictInputs, checks: Vec<Check>) -> Verdict {
        let green = !checks.is_empty() && checks.iter().all(|c| c.passed);
        Verdict {
            schema: VERDICT_SCHEMA_NAME.to_string(),
            schema_version: VERDICT_SCHEMA_VERSION,
            unit: unit.into(),
            green,
            inputs,
            checks,
        }
    }

    /// Write as pretty JSON (deterministic field order, trailing newline,
    /// atomic replace).
    pub fn store(&self, path: &Path) -> Result<(), Error> {
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Invariant(format!("serialize verdict: {e}")))?;
        text.push('\n');
        crate::ledger::write_atomic(path, text.as_bytes())
    }

    /// Load a verdict, refusing newer schema versions.
    pub fn load(path: &Path) -> Result<Verdict, Error> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        let v: Verdict =
            serde_json::from_str(&text).map_err(|e| Error::parse(path, e.to_string()))?;
        if v.schema != VERDICT_SCHEMA_NAME {
            return Err(Error::parse(path, "not a ruharness-verdict file"));
        }
        if v.schema_version > VERDICT_SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                path: path.into(),
                found: v.schema_version,
                supported: VERDICT_SCHEMA_VERSION,
            });
        }
        Ok(v)
    }

    /// Render the human-readable markdown view (no timestamps — run logs with
    /// wall-clock time belong in the gitignored build dir).
    pub fn render_md(&self) -> String {
        let mut md = format!(
            "# Oracle verdict — {}\n\nVerdict: **{}**\n\nInputs tested:\n- unit_source: `{}`\n- rust_crate: `{}`\n- driver: `{}`\n- toolchain: {}\n\nChecks:\n",
            self.unit,
            if self.green { "GREEN" } else { "RED" },
            self.inputs.unit_source,
            self.inputs.rust_crate,
            self.inputs.driver,
            self.inputs.toolchain.join("; "),
        );
        for c in &self.checks {
            md.push_str(&format!(
                "- **{}**: {} — {}\n",
                c.name,
                if c.passed { "PASS" } else { "FAIL" },
                c.detail
            ));
        }
        md
    }
}
