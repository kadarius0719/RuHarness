//! The attempts ledger (docs/SCHEMAS.md "M3 additions"): one committed,
//! content-identified record per executor attempt, rewritten atomically after
//! every turn so a crash — or the normal exit-and-resume rhythm of the
//! `external` provider — always leaves an accurate record.

use crate::error::Error;
use crate::ledger::Ledger;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Version of the attempt schema this build reads and writes.
pub const ATTEMPT_SCHEMA_VERSION: u64 = 1;
/// `schema` field of attempt.json.
pub const ATTEMPT_SCHEMA_NAME: &str = "ruharness-attempt";

/// One executor turn. Token fields are nullable: `None` = unknown (external
/// hand-off, or a provider that reports no usage) — never `0`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Turn {
    /// `translate` or `repair`.
    pub kind: String,
    /// Closed: `green | format | check | build | oracle | crash-timeout |
    /// truncated | blocked`.
    pub result: String,
    /// Trace key of the request (first 8 hex of its canonical-JSON hash).
    pub request_key: String,
    /// blake3 of the response text (checkable wherever traces are shared).
    pub response_hash: String,
    /// Input tokens, when the provider reported them.
    pub input_tokens: Option<u64>,
    /// Output tokens, when the provider reported them.
    pub output_tokens: Option<u64>,
}

/// A complete attempt record (`attempt.json`). Field order is canonical.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttemptRecord {
    /// Always [`ATTEMPT_SCHEMA_NAME`].
    pub schema: String,
    /// Schema version.
    pub schema_version: u64,
    /// Content-derived id (`a-<12hex>`), see [`attempt_id`].
    pub id: String,
    /// Unit id.
    pub unit: String,
    /// Provider profile name used.
    pub provider: String,
    /// Adapter kind behind the profile (`anthropic`, `openai-compat`, `external`).
    pub provider_kind: String,
    /// Model string sent.
    pub model: String,
    /// blake3(system ‖ NUL ‖ user) of the translate turn — equal digests across
    /// attempts prove the same migration was posed to different providers.
    pub prompt_digest: String,
    /// Unit source digest the attempt is bound to (file-set hash incl. includes).
    pub unit_source: String,
    /// Driver digest the attempt is bound to.
    pub driver: String,
    /// Toolchain + sandbox mode strings from the oracle (empty until a verdict ran).
    pub toolchain: Vec<String>,
    /// Closed: `in-progress | green | red | blocked | truncated | format`
    /// (reserved: `budget`, `thrash`).
    pub outcome: String,
    /// Turns so far.
    pub turns: Vec<Turn>,
    /// Location-independent digest of the last candidate written (empty if none).
    pub candidate_digest: String,
    /// Whether this attempt's candidate was promoted into the unit crate.
    pub promoted: bool,
}

/// Content-derived attempt id: `a-` + 12 hex of blake3(unit ‖ NUL ‖ unit_source
/// ‖ NUL ‖ driver ‖ NUL ‖ provider_kind ‖ NUL ‖ model ‖ NUL ‖ translate
/// request_key). Never a counter: parallel branches cannot collide, and a
/// re-run of the same attempt resumes the same directory.
pub fn attempt_id(
    unit: &str,
    unit_source: &str,
    driver: &str,
    provider_kind: &str,
    model: &str,
    translate_request_key: &str,
) -> String {
    let mut hasher = blake3::Hasher::new();
    for part in [unit, unit_source, driver, provider_kind, model] {
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(translate_request_key.as_bytes());
    format!("a-{}", &hasher.finalize().to_hex().to_string()[..12])
}

/// `migration/units/<unit>/attempts/<attempt-id>/`.
pub fn attempt_dir(ledger: &Ledger, unit: &str, attempt: &str) -> PathBuf {
    ledger.unit_dir(unit).join("attempts").join(attempt)
}

impl AttemptRecord {
    /// Atomic pretty-JSON write of `attempt.json` inside the attempt dir.
    pub fn store(&self, dir: &Path) -> Result<(), Error> {
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Invariant(format!("serialize attempt: {e}")))?;
        text.push('\n');
        crate::ledger::write_atomic(&dir.join("attempt.json"), text.as_bytes())
    }

    /// Load an attempt record, refusing newer schema versions.
    pub fn load(dir: &Path) -> Result<AttemptRecord, Error> {
        let path = dir.join("attempt.json");
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        let rec: AttemptRecord =
            serde_json::from_str(&text).map_err(|e| Error::parse(&path, e.to_string()))?;
        if rec.schema != ATTEMPT_SCHEMA_NAME {
            return Err(Error::parse(&path, "not a ruharness-attempt file"));
        }
        if rec.schema_version > ATTEMPT_SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                path,
                found: rec.schema_version,
                supported: ATTEMPT_SCHEMA_VERSION,
            });
        }
        Ok(rec)
    }
}

/// All attempt records for a unit, sorted by id (display order only).
pub fn load_unit_attempts(ledger: &Ledger, unit: &str) -> Result<Vec<AttemptRecord>, Error> {
    let dir = ledger.unit_dir(unit).join("attempts");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| Error::io(&dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(&dir, e))?;
        if entry.path().join("attempt.json").exists() {
            out.push(AttemptRecord::load(&entry.path())?);
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt_ids_are_content_derived() {
        let a = attempt_id("u1", "blake3:s", "blake3:d", "anthropic", "m", "abcd1234");
        let b = attempt_id("u1", "blake3:s", "blake3:d", "anthropic", "m", "abcd1234");
        let c = attempt_id(
            "u1",
            "blake3:s",
            "blake3:d",
            "openai-compat",
            "m",
            "abcd1234",
        );
        assert_eq!(a, b);
        assert_ne!(a, c, "provider kind must distinguish attempts");
        assert!(a.starts_with("a-") && a.len() == 14, "{a}");
    }

    #[test]
    fn unknown_usage_serializes_as_null_not_zero() {
        let t = Turn {
            kind: "translate".into(),
            result: "green".into(),
            request_key: "abcd1234".into(),
            response_hash: "blake3:x".into(),
            input_tokens: None,
            output_tokens: None,
        };
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("\"input_tokens\":null"), "{json}");
    }
}
