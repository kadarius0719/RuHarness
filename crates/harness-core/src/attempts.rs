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
/// `provider_kind` (and `provider`) of a labelled human attempt recorded by
/// `harness override` (docs/TUI-DESIGN.md §5.2): judged like any other,
/// never counted as the pipeline's (see [`provenance`]).
pub const HUMAN_KIND: &str = "human";
/// `Turn.kind` of the first turn of a steer attempt (docs/TUI-DESIGN.md §5.1).
pub const STEER_KIND: &str = "steer";

/// One executor turn. Token fields are nullable: `None` = unknown (external
/// hand-off, or a provider that reports no usage) — never `0`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Turn {
    /// OPEN, display-only (no behavior keys on it): `translate` (first migrate
    /// turn), `steer` (first turn of a seeded attempt), `human` (the one turn
    /// of a hand edit), `generate` (first driver turn) or `repair`.
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
    /// blake3(system ‖ NUL ‖ user) of the FIRST turn (translate, or the steer
    /// turn of a seeded attempt; empty for a human attempt) — equal digests
    /// across attempts prove the same question was posed to different
    /// providers.
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
    /// A steer attempt's seed: the finished attempt whose candidate and stored
    /// verdict its first turn shows (additive; omitted otherwise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seeded_from: Option<String>,
    /// A steer attempt's guidance note, verbatim — so its first turn renders
    /// from the ledger alone (additive; omitted otherwise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steer_note: Option<String>,
    /// A human attempt's note (printable, bounded; additive, optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
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

/// One migrate attempt of `unit` by id, for callers that take the id from
/// outside (`--attempt`, `harness promote`, a client): `id` must be a clean
/// path segment (never joined otherwise), the record filed under it must
/// carry that id (the attempts ledger is target-owned input — a record
/// posing under another directory's name is refused, not trusted) and
/// belong to `unit`. `None` when there is no such attempt.
pub fn load_pinned(ledger: &Ledger, unit: &str, id: &str) -> Result<Option<AttemptRecord>, Error> {
    if !crate::plan::is_clean_segment(id) {
        return Err(Error::Invariant(format!(
            "{:?} is not an attempt id",
            printable(id, 64)
        )));
    }
    let dir = attempt_dir(ledger, unit, id);
    let record = match AttemptRecord::load(&dir) {
        Ok(record) => record,
        Err(e) if e.is_not_found() => return Ok(None),
        Err(e) => return Err(e),
    };
    if record.id != id {
        return Err(Error::Invariant(format!(
            "{} holds a record with id {:?}, not `{id}` — the attempts ledger is inconsistent; \
             refusing to touch it",
            dir.join("attempt.json").display(),
            printable(&record.id, 64)
        )));
    }
    if record.unit != unit {
        return Err(Error::Invariant(format!(
            "attempt {id} belongs to unit `{}`, not `{unit}`",
            printable(&record.unit, 64)
        )));
    }
    Ok(Some(record))
}

/// The `(unit_source, driver)` digests an attempt of `unit` is bound to when
/// recorded against the CURRENT tree — derived exactly as the executor
/// records them: the include-closure file-set hash and the driver file hash
/// (`[unit.oracle] driver`, else the unit's default `driver.c`; `""` when
/// the file does not exist). The binding half of R-5
/// (docs/REPLAY-DESIGN.md §R): `bench`, `harness promote`, `harness
/// override`, a steer seed and the review cockpit all use this one function.
pub fn current_binding(
    ctx: &crate::TargetContext,
    facts: &crate::Facts,
    unit: &crate::Unit,
) -> Result<(String, String), Error> {
    let ledger = Ledger::new(&ctx.root);
    let closure = facts.include_closure(&unit.files);
    let unit_source = crate::hash::file_set_hash_on_disk(&ctx.root, &closure)?;
    let driver_path = match unit.oracle_param_str("driver") {
        Some(rel) => ctx.root.join(rel),
        None => ledger.driver_path(&unit.id),
    };
    let driver = if driver_path.exists() {
        crate::hash::file_hash(&driver_path)?
    } else {
        String::new()
    };
    Ok((unit_source, driver))
}

/// The content hash of the unit's crate on disk (`units/<id>/<rust_crate>/`,
/// crate-relative paths — what every `candidate_digest` is), `None` when the
/// unit has no `rust_crate` or the crate has no `Cargo.toml`.
pub fn unit_crate_digest(ledger: &Ledger, unit: &crate::Unit) -> Result<Option<String>, Error> {
    unit.oracle_param_str("rust_crate")
        .map(|name| ledger.unit_dir(&unit.id).join(name))
        .filter(|dir| dir.join("Cargo.toml").is_file())
        .map(|dir| crate::hash::crate_content_hash(&dir))
        .transpose()
}

/// Which recorded attempt produced the unit's crate on disk (R-5,
/// docs/REPLAY-DESIGN.md §R, as extended by docs/TUI-DESIGN.md §5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provenance<'a> {
    /// No green attempt bound to the current inputs has the crate's digest
    /// (on a verified unit: "provenance unknown").
    None,
    /// Exactly one MODEL attempt did — the pipeline produced the crate.
    Pipeline(&'a AttemptRecord),
    /// Several model attempts did (ambiguous provenance), in id order.
    Ambiguous(Vec<&'a AttemptRecord>),
    /// Only a labelled human attempt did — a hand edit, never counted as the
    /// pipeline's.
    Human(&'a AttemptRecord),
}

/// The ONE implementation of R-5: among `records`, the green attempts bound
/// to the current `unit_source` AND `driver` whose `candidate_digest` equals
/// `crate_digest`; a steer attempt that reproduced its seed's candidate is
/// collapsed into its seed (the seed is the provenance); then exactly one
/// model attempt → [`Provenance::Pipeline`], several →
/// [`Provenance::Ambiguous`], only human attempts → [`Provenance::Human`],
/// none → [`Provenance::None`]. Not the `promoted` flag: an older attempt
/// keeps it after a newer one replaced its crate.
pub fn provenance<'a>(
    records: &'a [AttemptRecord],
    unit_source: &str,
    driver: &str,
    crate_digest: Option<&str>,
) -> Provenance<'a> {
    let Some(digest) = crate_digest else {
        return Provenance::None;
    };
    let matches: Vec<&AttemptRecord> = records
        .iter()
        .filter(|r| {
            r.stage.is_none()
                && r.outcome == "green"
                && r.unit_source == unit_source
                && r.driver == driver
                && !r.candidate_digest.is_empty()
                && r.candidate_digest == digest
        })
        .collect();
    let seeded_by_a_match = |r: &AttemptRecord| {
        r.seeded_from
            .as_deref()
            .is_some_and(|seed| matches.iter().any(|m| m.id == seed))
    };
    let mut model: Vec<&AttemptRecord> = Vec::new();
    let mut human: Vec<&AttemptRecord> = Vec::new();
    for r in &matches {
        if seeded_by_a_match(r) {
            continue;
        }
        if r.provider_kind == HUMAN_KIND {
            human.push(r);
        } else {
            model.push(r);
        }
    }
    model.sort_by(|a, b| a.id.cmp(&b.id));
    human.sort_by(|a, b| a.id.cmp(&b.id));
    match (model.len(), human.first()) {
        (1, _) => Provenance::Pipeline(model[0]),
        (0, Some(h)) => Provenance::Human(h),
        (0, None) => Provenance::None,
        _ => Provenance::Ambiguous(model),
    }
}

/// `text` reduced to printable ASCII and cut to `max_bytes`.
fn printable(text: &str, max_bytes: usize) -> String {
    text.chars()
        .map(|c| if (' '..='~').contains(&c) { c } else { '?' })
        .take(max_bytes)
        .collect()
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
            let record = AttemptRecord::load(&entry.path())?;
            // A record filed under another directory would let one attempt
            // pose as another (provenance, supersession): refused.
            if entry.file_name().to_str() != Some(record.id.as_str()) {
                return Err(Error::parse(
                    entry.path().join("attempt.json"),
                    "the record's id is not its directory name",
                ));
            }
            out.push(record);
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// `superseded.jsonl` schema name (docs/REPLAY-DESIGN.md §R R-7).
pub const SUPERSEDED_SCHEMA_NAME: &str = "ruharness-superseded";
/// `superseded.jsonl` schema version.
pub const SUPERSEDED_SCHEMA_VERSION: u64 = 1;
/// Longest `reason` kept (it is echoed in reports).
const MAX_REASON_BYTES: usize = 400;
/// Largest `superseded.jsonl` read.
const MAX_SUPERSEDED_BYTES: u64 = 1024 * 1024;

/// One line of `migration/units/<unit>/superseded.jsonl`: a finished attempt
/// that is EXPECTED not to reproduce any more, and the attempt that replaced
/// it. Hand-written, committed, reviewed; `bench check --replay` verifies
/// every entry (docs/REPLAY-DESIGN.md §R R-7) — a line never excuses an
/// integrity failure, and an entry for an attempt that still reproduces is
/// itself a problem.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Supersession {
    /// Always [`SUPERSEDED_SCHEMA_NAME`].
    pub schema: String,
    /// Schema version.
    pub schema_version: u64,
    /// The superseded attempt's id.
    pub attempt: String,
    /// `migrate` | `driver`.
    pub stage: String,
    /// Why (free text, reviewed; echoed printable and bounded).
    pub reason: String,
    /// The finished attempt that replaced it (same unit, stage and inputs).
    pub superseded_by: String,
    /// Set when the superseded attempt was NOT green: the judge accepts now
    /// what it rejected then — always listed, never implied.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub loosening: bool,
}

/// The unit's supersession entries, the LATEST line per `(stage, attempt)`
/// winning, in file order of those lines. Absent file = none. The file is
/// target-owned input: a symlink, a malformed line, an unknown schema, a
/// stage other than `migrate`/`driver`, or an id that is not a clean
/// segment is an error naming the line.
pub fn load_supersessions(ledger: &Ledger, unit: &str) -> Result<Vec<Supersession>, Error> {
    let path = ledger.unit_dir(unit).join("superseded.jsonl");
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(&path, e)),
    };
    if !meta.file_type().is_file() {
        return Err(Error::parse(
            &path,
            "not a regular file (symlinks are refused)",
        ));
    }
    if meta.len() > MAX_SUPERSEDED_BYTES {
        return Err(Error::parse(
            &path,
            format!("larger than {MAX_SUPERSEDED_BYTES} bytes"),
        ));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
    let mut out: Vec<Supersession> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let bad = |why: String| Error::parse(&path, format!("line {}: {why}", n + 1));
        let entry: Supersession = serde_json::from_str(line).map_err(|e| bad(e.to_string()))?;
        if entry.schema != SUPERSEDED_SCHEMA_NAME {
            return Err(bad("not a ruharness-superseded line".into()));
        }
        if entry.schema_version > SUPERSEDED_SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                path: path.clone(),
                found: entry.schema_version,
                supported: SUPERSEDED_SCHEMA_VERSION,
            });
        }
        if entry.stage != "migrate" && entry.stage != DRIVER_STAGE {
            return Err(bad("stage must be `migrate` or `driver`".into()));
        }
        for id in [&entry.attempt, &entry.superseded_by] {
            if !crate::plan::is_clean_segment(id) {
                return Err(bad("attempt ids must be clean path segments".into()));
            }
        }
        if entry.attempt == entry.superseded_by {
            return Err(bad("an attempt cannot supersede itself".into()));
        }
        if entry.reason.trim().is_empty() || entry.reason.len() > MAX_REASON_BYTES {
            return Err(bad(format!("reason must be 1..={MAX_REASON_BYTES} bytes")));
        }
        out.retain(|e| !(e.stage == entry.stage && e.attempt == entry.attempt));
        out.push(entry);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supersessions_load_latest_wins_and_refuse_bad_lines() {
        let root = std::env::temp_dir().join(format!("ruharness-supersede-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ledger = Ledger::new(root.clone());
        let dir = ledger.unit_dir("u");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_supersessions(&ledger, "u").unwrap().is_empty());
        let line = |attempt: &str, reason: &str| {
            format!(
                "{{\"schema\":\"ruharness-superseded\",\"schema_version\":1,\"attempt\":\"{attempt}\",\
                 \"stage\":\"migrate\",\"reason\":\"{reason}\",\"superseded_by\":\"a-b\"}}\n"
            )
        };
        let path = dir.join("superseded.jsonl");
        std::fs::write(
            &path,
            line("a-a", "first") + &line("a-a", "second") + &line("a-c", "x"),
        )
        .unwrap();
        let got = load_supersessions(&ledger, "u").unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(
            (got[0].attempt.as_str(), got[0].reason.as_str()),
            ("a-a", "second")
        );
        assert_eq!(
            (got[1].attempt.as_str(), got[1].reason.as_str()),
            ("a-c", "x")
        );
        assert!(!got[0].loosening);
        for (bad, expect) in [
            (line("../x", "r"), "clean path segments"),
            (line("a-b", "self"), "cannot supersede itself"),
            (line("a-a", ""), "reason must be"),
            (
                line("a-a", "r").replace("ruharness-superseded", "other"),
                "not a ruharness-superseded line",
            ),
            (
                line("a-a", "r").replace("\"migrate\"", "\"verify\""),
                "stage must be",
            ),
            (
                line("a-a", "r").replace("\"schema_version\":1", "\"schema_version\":2"),
                "schema_version 2",
            ),
        ] {
            std::fs::write(&path, &bad).unwrap();
            let err = load_supersessions(&ledger, "u").unwrap_err();
            assert!(err.to_string().contains(expect), "{bad} -> {err}");
        }
        #[cfg(unix)]
        {
            std::fs::remove_file(&path).unwrap();
            let real = root.join("elsewhere.jsonl");
            std::fs::write(&real, line("a-a", "r")).unwrap();
            std::os::unix::fs::symlink(&real, &path).unwrap();
            let err = load_supersessions(&ledger, "u").unwrap_err();
            assert!(err.to_string().contains("symlinks are refused"), "{err}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

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
            seeded_from: None,
            steer_note: None,
            note: None,
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

    fn prov_rec(id: &str, kind: &str, outcome: &str, digest: &str) -> AttemptRecord {
        AttemptRecord {
            schema: ATTEMPT_SCHEMA_NAME.into(),
            schema_version: 1,
            id: id.into(),
            unit: "u".into(),
            stage: None,
            provider: kind.into(),
            provider_kind: kind.into(),
            model: "m".into(),
            prompt_digest: String::new(),
            unit_source: "blake3:src".into(),
            driver: "blake3:drv".into(),
            toolchain: vec![],
            outcome: outcome.into(),
            turns: vec![],
            candidate_digest: digest.into(),
            promoted: false,
            seeded_from: None,
            steer_note: None,
            note: None,
        }
    }

    #[test]
    fn provenance_is_the_one_r5_rule() {
        let p = |recs: &[AttemptRecord], crate_digest: Option<&str>| match provenance(
            recs,
            "blake3:src",
            "blake3:drv",
            crate_digest,
        ) {
            Provenance::None => "none".to_string(),
            Provenance::Pipeline(r) => format!("pipeline:{}", r.id),
            Provenance::Human(r) => format!("human:{}", r.id),
            Provenance::Ambiguous(rs) => format!(
                "ambiguous:{}",
                rs.iter()
                    .map(|r| r.id.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        };
        let model = prov_rec("a-1", "external", "green", "blake3:c");
        // One model attempt: pipeline provenance.
        assert_eq!(
            p(std::slice::from_ref(&model), Some("blake3:c")),
            "pipeline:a-1"
        );
        // No crate, or a crate no attempt produced: none.
        assert_eq!(p(std::slice::from_ref(&model), None), "none");
        assert_eq!(p(std::slice::from_ref(&model), Some("blake3:x")), "none");
        // A RED attempt whose digest equals the crate is not provenance.
        let red = prov_rec("a-2", "external", "red", "blake3:c");
        assert_eq!(p(std::slice::from_ref(&red), Some("blake3:c")), "none");
        // Bound to superseded inputs: not provenance.
        let stale = AttemptRecord {
            driver: "blake3:old".into(),
            ..prov_rec("a-3", "external", "green", "blake3:c")
        };
        assert_eq!(p(std::slice::from_ref(&stale), Some("blake3:c")), "none");
        // A driver-stage record never is.
        let driver = AttemptRecord {
            stage: Some(DRIVER_STAGE.into()),
            ..prov_rec("d-1", "external", "green", "blake3:c")
        };
        assert_eq!(p(std::slice::from_ref(&driver), Some("blake3:c")), "none");
        // Two model attempts: ambiguous, in id order.
        let other = prov_rec("a-0", "anthropic", "green", "blake3:c");
        assert_eq!(
            p(&[model.clone(), other.clone()], Some("blake3:c")),
            "ambiguous:a-0,a-1"
        );
        // A steer attempt that reproduced its seed collapses into the seed.
        let steer = AttemptRecord {
            seeded_from: Some("a-1".into()),
            ..prov_rec("a-9", "external", "green", "blake3:c")
        };
        assert_eq!(
            p(&[model.clone(), steer.clone()], Some("blake3:c")),
            "pipeline:a-1"
        );
        // A human attempt alone: human provenance, never the pipeline's.
        let human = prov_rec("a-h", HUMAN_KIND, "green", "blake3:c");
        assert_eq!(
            p(std::slice::from_ref(&human), Some("blake3:c")),
            "human:a-h"
        );
        // A model attempt with the same candidate outranks the human one.
        assert_eq!(
            p(&[human.clone(), model.clone()], Some("blake3:c")),
            "pipeline:a-1"
        );
        // A model steer that reproduced a human seed: the human is the source.
        let steer_on_human = AttemptRecord {
            seeded_from: Some("a-h".into()),
            ..prov_rec("a-8", "external", "green", "blake3:c")
        };
        assert_eq!(p(&[human, steer_on_human], Some("blake3:c")), "human:a-h");
    }

    #[test]
    fn additive_fields_are_omitted_when_absent() {
        let rec = prov_rec("a-1", "external", "green", "blake3:c");
        let json = serde_json::to_string(&rec).unwrap();
        for key in ["seeded_from", "steer_note", "\"note\""] {
            assert!(!json.contains(key), "{key} in {json}");
        }
        let steer = AttemptRecord {
            seeded_from: Some("a-0".into()),
            steer_note: Some("use an iterator".into()),
            ..rec
        };
        let back: AttemptRecord =
            serde_json::from_str(&serde_json::to_string(&steer).unwrap()).unwrap();
        assert_eq!(back, steer);
    }
}
