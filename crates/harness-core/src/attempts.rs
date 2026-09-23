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
/// `stage` of a driver-generation attempt (M4). Migrate attempts carry no
/// `stage` field at all, so every pre-M4 record stays byte-identical.
pub const DRIVER_STAGE: &str = "driver";

/// One executor turn. Token fields are nullable: `None` = unknown (external
/// hand-off, or a provider that reports no usage) — never `0`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Turn {
    /// OPEN, display-only (no behavior keys on it): `translate` (first migrate
    /// turn), `generate` (first driver turn) or `repair`.
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
    /// Pipeline stage: `None` = migrate (the field is then omitted, keeping
    /// pre-M4 records byte-identical); `Some("driver")` = driver generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
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
    /// Driver digest the attempt is bound to (`""` for a driver attempt).
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

/// Content-derived driver-attempt id: `d-` + 12 hex of blake3(`driver` ‖ NUL ‖
/// unit ‖ NUL ‖ unit_source ‖ NUL ‖ provider_kind ‖ NUL ‖ model ‖ NUL ‖
/// generate request_key). A separate derivation from [`attempt_id`], which
/// never mixes in a stage (migrate ids are frozen).
pub fn driver_attempt_id(
    unit: &str,
    unit_source: &str,
    provider_kind: &str,
    model: &str,
    generate_request_key: &str,
) -> String {
    let mut hasher = blake3::Hasher::new();
    for part in [DRIVER_STAGE, unit, unit_source, provider_kind, model] {
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(generate_request_key.as_bytes());
    format!("d-{}", &hasher.finalize().to_hex().to_string()[..12])
}

/// `migration/units/<unit>/driver-attempts/<attempt-id>/`.
pub fn driver_attempt_dir(ledger: &Ledger, unit: &str, attempt: &str) -> PathBuf {
    ledger.unit_dir(unit).join("driver-attempts").join(attempt)
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

/// All migrate attempt records for a unit, sorted by id (display order only).
pub fn load_unit_attempts(ledger: &Ledger, unit: &str) -> Result<Vec<AttemptRecord>, Error> {
    load_records(&ledger.unit_dir(unit).join("attempts"))
}

/// All driver-generation attempt records for a unit, sorted by id.
pub fn load_unit_driver_attempts(ledger: &Ledger, unit: &str) -> Result<Vec<AttemptRecord>, Error> {
    load_records(&ledger.unit_dir(unit).join("driver-attempts"))
}

fn load_records(dir: &Path) -> Result<Vec<AttemptRecord>, Error> {
    let dir = dir.to_path_buf();
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
        let d = driver_attempt_id("u1", "blake3:s", "anthropic", "m", "abcd1234");
        assert!(d.starts_with("d-") && d.len() == 14, "{d}");
        assert_ne!(&d[2..], &a[2..]);
    }

    const GOLDEN_M3_ID: &str = "a-7189df7f664d";

    #[test]
    fn migrate_ids_are_frozen() {
        // Golden: the M3 derivation must never change (recorded attempt dirs
        // are keyed by it). Value computed by the M3 implementation.
        let a = attempt_id("u1", "blake3:s", "blake3:d", "anthropic", "m", "abcd1234");
        assert_eq!(a, GOLDEN_M3_ID);
    }

    #[test]
    fn migrate_records_omit_stage() {
        let rec = AttemptRecord {
            schema: ATTEMPT_SCHEMA_NAME.into(),
            schema_version: 1,
            id: "a-000000000000".into(),
            unit: "u".into(),
            stage: None,
            provider: "p".into(),
            provider_kind: "k".into(),
            model: "m".into(),
            prompt_digest: String::new(),
            unit_source: String::new(),
            driver: String::new(),
            toolchain: vec![],
            outcome: "green".into(),
            turns: vec![],
            candidate_digest: String::new(),
            promoted: false,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(!json.contains("stage"), "{json}");
        let driver = AttemptRecord {
            stage: Some(DRIVER_STAGE.into()),
            ..rec
        };
        let json = serde_json::to_string(&driver).unwrap();
        assert!(
            json.contains(r#""unit":"u","stage":"driver","provider""#),
            "{json}"
        );
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
