//! Observer surfaces (docs/SCHEMAS.md "M2 additions"): findings, annotations,
//! triage verdicts, human reviews, and the observations.md renderer.
//!
//! Writer model: findings are detector-output-only (pure function of tree +
//! detector suite); annotations are human/oracle-owned; triage is written only
//! by `harness observe`; reviews only via `harness review`; observations.md is
//! always rendered, never hand-edited.

use crate::error::Error;
use crate::facts::Facts;
use crate::plan::Plan;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

/// Version of the findings schema this build reads and writes.
pub const FINDINGS_SCHEMA_VERSION: u64 = 1;
/// `schema` field of findings.jsonl.
pub const FINDINGS_SCHEMA_NAME: &str = "ruharness-findings";
/// Version of the triage schema this build reads and writes.
pub const TRIAGE_SCHEMA_VERSION: u64 = 1;
/// `schema` field of triage.jsonl.
pub const TRIAGE_SCHEMA_NAME: &str = "ruharness-triage";

/// A hazard finding (docs/SCHEMAS.md). Field order is canonical.
///
/// `id` is content-keyed (see [`finding_id`]); `span` is display data;
/// behavior travels on `blocker`/`human_mandatory`, never on category lists.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Finding {
    /// Content-keyed id (`f-<16hex>`).
    pub id: String,
    /// Producing detector (`macros`, …) — or `human`/`oracle` for annotations.
    pub detector: String,
    /// Hazard category (open enum).
    pub category: String,
    /// `high | medium | low | info` (open enum).
    pub severity: String,
    /// Pins the affected unit's risk score to ≥ 90.
    pub blocker: bool,
    /// Dismissals require human review to discharge.
    pub human_mandatory: bool,
    /// Repo-relative file.
    pub file: String,
    /// The file's content hash at detect time (freshness binding).
    pub file_hash: String,
    /// 1-based (start_line, end_line) — display data, not identity.
    pub span: (u32, u32),
    /// 0-based index among same-(detector, category, spanned-bytes) findings
    /// in this file (disambiguates identical constructs).
    pub occurrence: u32,
    /// Human-readable description (never sent inside a trusted prompt region).
    pub message: String,
    /// Source excerpt, ≤ 200 chars (same trust rules as `message`).
    pub evidence: String,
}

/// Content-keyed finding id: `f-` + first 16 hex of
/// blake3(detector ‖ NUL ‖ category ‖ NUL ‖ file ‖ NUL ‖ blake3-hex of the
/// exact spanned source bytes ‖ NUL ‖ occurrence). Stable across line shifts
/// and message rewording.
pub fn finding_id(
    detector: &str,
    category: &str,
    file: &str,
    spanned_bytes: &[u8],
    occurrence: u32,
) -> String {
    let span_hash = blake3::hash(spanned_bytes).to_hex().to_string();
    let mut hasher = blake3::Hasher::new();
    for part in [detector, category, file, &span_hash] {
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(occurrence.to_string().as_bytes());
    let hex = hasher.finalize().to_hex().to_string();
    format!("f-{}", &hex[..16])
}

/// The findings file: header metadata + records.
#[derive(Debug, Clone, Default)]
pub struct FindingsFile {
    /// Detector suite identifier (e.g. `c-treesitter-v1`).
    pub detector_suite: String,
    /// File-set hash over the facts file records at detect time.
    pub facts_hash: String,
    /// Findings, canonically sorted on store.
    pub findings: Vec<Finding>,
}

#[derive(Serialize)]
struct FindingsHeaderOut<'a> {
    k: &'static str,
    schema: &'static str,
    schema_version: u64,
    detector_suite: &'a str,
    facts_hash: &'a str,
}

#[derive(Serialize)]
struct RecordOut<'a, T: Serialize> {
    k: &'static str,
    #[serde(flatten)]
    record: &'a T,
}

fn jsonl_line<T: Serialize>(kind: &'static str, record: &T) -> Result<String, Error> {
    serde_json::to_string(&RecordOut { k: kind, record })
        .map_err(|e| Error::Invariant(format!("serialize {kind}: {e}")))
}

impl FindingsFile {
    /// Canonical bytes: header, then findings sorted by (file, id) with the
    /// full line as tiebreak. Trailing newline.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, Error> {
        let header = serde_json::to_string(&FindingsHeaderOut {
            k: "header",
            schema: FINDINGS_SCHEMA_NAME,
            schema_version: FINDINGS_SCHEMA_VERSION,
            detector_suite: &self.detector_suite,
            facts_hash: &self.facts_hash,
        })
        .map_err(|e| Error::Invariant(format!("serialize header: {e}")))?;
        let mut lines: Vec<((String, String), String)> = Vec::with_capacity(self.findings.len());
        for f in &self.findings {
            lines.push(((f.file.clone(), f.id.clone()), jsonl_line("finding", f)?));
        }
        lines.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let mut all = vec![header];
        all.extend(lines.into_iter().map(|(_, l)| l));
        let mut bytes = all.join("\n").into_bytes();
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Atomic canonical write.
    pub fn store(&self, path: &Path) -> Result<(), Error> {
        crate::ledger::write_atomic(path, &self.to_canonical_bytes()?)
    }

    /// Load, refusing newer schema versions; unknown record kinds skipped.
    pub fn load(path: &Path) -> Result<FindingsFile, Error> {
        let (header, records) = load_jsonl(
            path,
            FINDINGS_SCHEMA_NAME,
            FINDINGS_SCHEMA_VERSION,
            "finding",
        )?;
        Ok(FindingsFile {
            detector_suite: header
                .get("detector_suite")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            facts_hash: header
                .get("facts_hash")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            findings: records,
        })
    }
}

/// Load annotations (human/oracle findings; same record shape, no canonical
/// guarantee since humans append). Missing file = empty.
pub fn load_annotations(path: &Path) -> Result<Vec<Finding>, Error> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| Error::parse(path, format!("line {}: {e}", i + 1)))?;
        // headers/unknown kinds pass through
        if value.get("k").and_then(|v| v.as_str()) == Some("finding") {
            out.push(
                serde_json::from_value(value)
                    .map_err(|e| Error::parse(path, format!("line {}: {e}", i + 1)))?,
            );
        }
    }
    Ok(out)
}

/// Triage verdict value (closed enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TriageVerdict {
    /// The finding is a real migration risk.
    Confirm,
    /// The finding is judged not a real risk (human review still applies).
    Dismiss,
    /// The model could not adjudicate.
    Uncertain,
}

/// One triage verdict record (docs/SCHEMAS.md). Field order canonical.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerdictRecord {
    /// Finding id the verdict is for.
    pub finding: String,
    /// Harness-computed content hash binding verdict to batch + slice.
    pub content_hash: String,
    /// The verdict.
    pub verdict: TriageVerdict,
    /// `high | medium | low` (closed).
    pub confidence: String,
    /// Model rationale (untrusted text; displayed, never executed).
    pub rationale: String,
    /// `file:start-end` citations.
    pub evidence: Vec<String>,
}

/// The triage file (verdicts only — run metadata lives in gitignored traces).
#[derive(Debug, Clone, Default)]
pub struct TriageFile {
    /// Verdicts, canonically sorted on store.
    pub verdicts: Vec<VerdictRecord>,
}

#[derive(Serialize)]
struct TriageHeaderOut {
    k: &'static str,
    schema: &'static str,
    schema_version: u64,
}

impl TriageFile {
    /// Canonical bytes: header, then verdicts sorted by finding id.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, Error> {
        let header = serde_json::to_string(&TriageHeaderOut {
            k: "header",
            schema: TRIAGE_SCHEMA_NAME,
            schema_version: TRIAGE_SCHEMA_VERSION,
        })
        .map_err(|e| Error::Invariant(format!("serialize header: {e}")))?;
        let mut lines: Vec<(String, String)> = Vec::with_capacity(self.verdicts.len());
        for v in &self.verdicts {
            lines.push((v.finding.clone(), jsonl_line("verdict", v)?));
        }
        lines.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let mut all = vec![header];
        all.extend(lines.into_iter().map(|(_, l)| l));
        let mut bytes = all.join("\n").into_bytes();
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Atomic canonical write.
    pub fn store(&self, path: &Path) -> Result<(), Error> {
        crate::ledger::write_atomic(path, &self.to_canonical_bytes()?)
    }

    /// Load, refusing newer schema versions. Missing file = empty.
    pub fn load(path: &Path) -> Result<TriageFile, Error> {
        if !path.exists() {
            return Ok(TriageFile::default());
        }
        let (_, records) = load_jsonl(path, TRIAGE_SCHEMA_NAME, TRIAGE_SCHEMA_VERSION, "verdict")?;
        Ok(TriageFile { verdicts: records })
    }
}

/// A human review record (docs/SCHEMAS.md; written via `harness review`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Review {
    /// Finding id under review.
    pub finding: String,
    /// `uphold-dismiss` or `reinstate`.
    pub action: String,
    /// Free-text note.
    #[serde(default)]
    pub note: String,
}

/// Load reviews (missing file = empty).
pub fn load_reviews(path: &Path) -> Result<Vec<Review>, Error> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| Error::parse(path, format!("line {}: {e}", i + 1)))?;
        if value.get("k").and_then(|v| v.as_str()) == Some("review") {
            out.push(
                serde_json::from_value(value)
                    .map_err(|e| Error::parse(path, format!("line {}: {e}", i + 1)))?,
            );
        }
    }
    Ok(out)
}

/// Append a review record (canonical single line).
pub fn append_review(path: &Path, review: &Review) -> Result<(), Error> {
    let mut text = if path.exists() {
        std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?
    } else {
        String::new()
    };
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&jsonl_line("review", review)?);
    text.push('\n');
    crate::ledger::write_atomic(path, text.as_bytes())
}

fn load_jsonl<T: serde::de::DeserializeOwned>(
    path: &Path,
    schema: &str,
    supported: u64,
    kind: &str,
) -> Result<(serde_json::Value, Vec<T>), Error> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    let mut lines = text.lines();
    let header_line = lines
        .next()
        .ok_or_else(|| Error::parse(path, "empty file"))?;
    let header: serde_json::Value = serde_json::from_str(header_line)
        .map_err(|e| Error::parse(path, format!("header: {e}")))?;
    if header.get("schema").and_then(|v| v.as_str()) != Some(schema) {
        return Err(Error::parse(path, format!("not a {schema} file")));
    }
    let version = header
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| Error::parse(path, "header missing schema_version"))?;
    if version > supported {
        return Err(Error::SchemaTooNew {
            path: path.into(),
            found: version,
            supported,
        });
    }
    let mut records = Vec::new();
    for (i, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| Error::parse(path, format!("line {}: {e}", i + 2)))?;
        if value.get("k").and_then(|v| v.as_str()) == Some(kind) {
            records.push(
                serde_json::from_value(value)
                    .map_err(|e| Error::parse(path, format!("line {}: {e}", i + 2)))?,
            );
        }
    }
    Ok((header, records))
}

/// Units affected by a finding in `file`: every plan unit whose include
/// closure (per facts) contains it. Computed at observe/render time — never
/// stored in findings.jsonl.
pub fn affected_units<'a>(file: &str, plan: &'a Plan, facts: &Facts) -> Vec<&'a str> {
    plan.units
        .iter()
        .filter(|u| facts.include_closure(&u.files).iter().any(|p| p == file))
        .map(|u| u.id.as_str())
        .collect()
}

/// The effective verdict state of a finding after triage + human review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingState {
    /// Confirmed risk (by model, or implicitly for annotations).
    Confirmed,
    /// Dismissed by the model, awaiting human review (keeps risk weight).
    DismissedPendingReview,
    /// Dismissed and human-upheld (excluded from risk).
    DismissedAcknowledged,
    /// Dismissed by the model but human-reinstated (counts as confirmed).
    Reinstated,
    /// Model could not adjudicate (keeps risk weight).
    Uncertain,
    /// No verdict yet.
    Untriaged,
}

/// Resolve a finding's state from triage + reviews. Annotations (detector
/// `human`/`oracle`) are implicitly confirmed and exempt from triage.
pub fn finding_state(finding: &Finding, triage: &TriageFile, reviews: &[Review]) -> FindingState {
    if finding.detector == "human" || finding.detector == "oracle" {
        return FindingState::Confirmed;
    }
    let verdict = triage.verdicts.iter().find(|v| v.finding == finding.id);
    let review = reviews.iter().rev().find(|r| r.finding == finding.id);
    match verdict.map(|v| v.verdict) {
        None => FindingState::Untriaged,
        Some(TriageVerdict::Confirm) => FindingState::Confirmed,
        Some(TriageVerdict::Uncertain) => FindingState::Uncertain,
        Some(TriageVerdict::Dismiss) => match review.map(|r| r.action.as_str()) {
            Some("uphold-dismiss") => FindingState::DismissedAcknowledged,
            Some("reinstate") => FindingState::Reinstated,
            _ => FindingState::DismissedPendingReview,
        },
    }
}

/// Inputs to the observations renderer.
pub struct ObservationsInput<'a> {
    /// Detector findings.
    pub findings: &'a FindingsFile,
    /// Human/oracle annotations.
    pub annotations: &'a [Finding],
    /// Triage verdicts.
    pub triage: &'a TriageFile,
    /// Human reviews.
    pub reviews: &'a [Review],
    /// The plan.
    pub plan: &'a Plan,
    /// The facts.
    pub facts: &'a Facts,
    /// Per-unit risk, pre-sorted descending (from [`crate::risk`]).
    pub risk: &'a [crate::risk::UnitRisk],
}

/// Render observations.md (docs/SCHEMAS.md): deterministic, no timestamps.
/// Fails unless every detector finding has a verdict (stage-2 done-criterion).
pub fn render_observations(input: &ObservationsInput) -> Result<String, Error> {
    let missing: Vec<&str> = input
        .findings
        .findings
        .iter()
        .filter(|f| finding_state(f, input.triage, input.reviews) == FindingState::Untriaged)
        .map(|f| f.id.as_str())
        .collect();
    if !missing.is_empty() {
        return Err(Error::Invariant(format!(
            "stage-2 done-criterion unmet: {} finding(s) untriaged: {}",
            missing.len(),
            missing.join(", ")
        )));
    }

    let mut all: Vec<&Finding> = input.findings.findings.iter().collect();
    all.extend(input.annotations.iter());

    let mut md = String::from("# Observations\n\nRendered by `harness observe` — do not edit.\n");
    md.push_str(&format!(
        "\nDetector suite: `{}` · findings: {} · annotations: {}\n",
        input.findings.detector_suite,
        input.findings.findings.len(),
        input.annotations.len()
    ));

    md.push_str("\n## Units by risk\n\n| unit | status | score | blockers | findings (confirmed/dismissed/uncertain) |\n|---|---|---|---|---|\n");
    for r in input.risk {
        let unit = input.plan.unit(&r.unit)?;
        let affecting: Vec<&&Finding> = all
            .iter()
            .filter(|f| affected_units(&f.file, input.plan, input.facts).contains(&r.unit.as_str()))
            .collect();
        let mut confirmed = 0;
        let mut dismissed = 0;
        let mut uncertain = 0;
        for f in &affecting {
            match finding_state(f, input.triage, input.reviews) {
                FindingState::Confirmed | FindingState::Reinstated => confirmed += 1,
                FindingState::DismissedAcknowledged | FindingState::DismissedPendingReview => {
                    dismissed += 1
                }
                _ => uncertain += 1,
            }
        }
        md.push_str(&format!(
            "| {} | {} | {} | {} | {}/{}/{} |\n",
            r.unit,
            unit.status.as_str(),
            r.score,
            if r.blockers.is_empty() {
                "—".to_string()
            } else {
                r.blockers.join(", ")
            },
            confirmed,
            dismissed,
            uncertain
        ));
    }

    md.push_str("\n## Findings\n\n");
    for f in &all {
        let state = finding_state(f, input.triage, input.reviews);
        let verdict = input.triage.verdicts.iter().find(|v| v.finding == f.id);
        let units = affected_units(&f.file, input.plan, input.facts).join(", ");
        md.push_str(&format!(
            "### {} — {} `{}` ({})\n\n- location: `{}:{}–{}` · severity: {} · affects: {}\n",
            f.id,
            f.detector,
            f.category,
            state_label(state),
            f.file,
            f.span.0,
            f.span.1,
            f.severity,
            if units.is_empty() {
                "(no unit)".to_string()
            } else {
                units
            }
        ));
        md.push_str(&format!("- {}\n", f.message));
        if let Some(v) = verdict {
            // Model-authored text is rendered inline-only: control characters
            // collapse to spaces so it can never open a new markdown block.
            let rationale: String = v
                .rationale
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            md.push_str(&format!(
                "- triage ({}, {}): {}\n",
                verdict_label(v.verdict),
                v.confidence,
                rationale.trim()
            ));
        }
        if f.human_mandatory && state == FindingState::DismissedPendingReview {
            md.push_str("- **HUMAN REVIEW REQUIRED** — dismissal of a human-mandatory category; run `harness review`\n");
        }
        md.push('\n');
    }

    // Stale verdicts (finding regenerated away).
    let current_ids: BTreeSet<&str> = input
        .findings
        .findings
        .iter()
        .map(|f| f.id.as_str())
        .collect();
    let stale: Vec<&VerdictRecord> = input
        .triage
        .verdicts
        .iter()
        .filter(|v| !current_ids.contains(v.finding.as_str()))
        .collect();
    if !stale.is_empty() {
        md.push_str("## Stale verdicts (re-triage)\n\n");
        for v in stale {
            md.push_str(&format!("- {} — finding no longer present\n", v.finding));
        }
        md.push('\n');
    }

    md.push_str("## Standing caveats\n\nThe c-treesitter-v1 suite cannot detect (type information required — libclang frontend material): pointer arithmetic, type punning/cast chains, aliasing assumptions, indirect-call target resolution. Reliance on undefined/implementation-defined behavior is oracle/annotation territory (see annotations).\n");
    Ok(md)
}

fn state_label(s: FindingState) -> &'static str {
    match s {
        FindingState::Confirmed => "confirmed",
        FindingState::DismissedPendingReview => "dismissed — pending human review",
        FindingState::DismissedAcknowledged => "dismissed — human upheld",
        FindingState::Reinstated => "reinstated by human",
        FindingState::Uncertain => "uncertain",
        FindingState::Untriaged => "untriaged",
    }
}

fn verdict_label(v: TriageVerdict) -> &'static str {
    match v {
        TriageVerdict::Confirm => "confirm",
        TriageVerdict::Dismiss => "dismiss",
        TriageVerdict::Uncertain => "uncertain",
    }
}

/// Observer ledger paths under `migration/observer/`.
pub struct ObserverPaths;

impl ObserverPaths {
    /// findings.jsonl path.
    pub fn findings(ledger: &crate::ledger::Ledger) -> std::path::PathBuf {
        ledger.dir().join("observer").join("findings.jsonl")
    }
    /// annotations.jsonl path.
    pub fn annotations(ledger: &crate::ledger::Ledger) -> std::path::PathBuf {
        ledger.dir().join("observer").join("annotations.jsonl")
    }
    /// triage.jsonl path.
    pub fn triage(ledger: &crate::ledger::Ledger) -> std::path::PathBuf {
        ledger.dir().join("observer").join("triage.jsonl")
    }
    /// reviews.jsonl path.
    pub fn reviews(ledger: &crate::ledger::Ledger) -> std::path::PathBuf {
        ledger.dir().join("observer").join("reviews.jsonl")
    }
    /// observations.md path.
    pub fn observations(ledger: &crate::ledger::Ledger) -> std::path::PathBuf {
        ledger.dir().join("observer").join("observations.md")
    }
    /// Gitignored traces dir.
    pub fn traces(ledger: &crate::ledger::Ledger) -> std::path::PathBuf {
        ledger.dir().join("observer").join("traces")
    }
}

/// Effective finding weight map for risk scoring: true when the finding still
/// counts (everything except human-acknowledged dismissals).
pub fn counts_for_risk(f: &Finding, triage: &TriageFile, reviews: &[Review]) -> bool {
    finding_state(f, triage, reviews) != FindingState::DismissedAcknowledged
}

#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finding_ids_are_position_independent() {
        let a = finding_id(
            "macros",
            "macro-statement-body",
            "src/u.h",
            b"#define X {..}",
            0,
        );
        let b = finding_id(
            "macros",
            "macro-statement-body",
            "src/u.h",
            b"#define X {..}",
            0,
        );
        assert_eq!(a, b);
        assert!(a.starts_with("f-") && a.len() == 18, "{a}");
        // occurrence disambiguates identical twins
        let c = finding_id(
            "macros",
            "macro-statement-body",
            "src/u.h",
            b"#define X {..}",
            1,
        );
        assert_ne!(a, c);
    }

    #[test]
    fn dismissal_requires_human_ack_to_stop_counting() {
        let f = Finding {
            id: "f-1".into(),
            detector: "macros".into(),
            category: "macro-statement-body".into(),
            severity: "high".into(),
            blocker: false,
            human_mandatory: false,
            file: "src/u.h".into(),
            file_hash: "blake3:x".into(),
            span: (1, 2),
            occurrence: 0,
            message: "m".into(),
            evidence: "e".into(),
        };
        let triage = TriageFile {
            verdicts: vec![VerdictRecord {
                finding: "f-1".into(),
                content_hash: "blake3:h".into(),
                verdict: TriageVerdict::Dismiss,
                confidence: "high".into(),
                rationale: "not real".into(),
                evidence: vec![],
            }],
        };
        assert!(counts_for_risk(&f, &triage, &[]));
        let reviews = vec![Review {
            finding: "f-1".into(),
            action: "uphold-dismiss".into(),
            note: String::new(),
        }];
        assert!(!counts_for_risk(&f, &triage, &reviews));
        let reviews = vec![Review {
            finding: "f-1".into(),
            action: "reinstate".into(),
            note: String::new(),
        }];
        assert_eq!(
            finding_state(&f, &triage, &reviews),
            FindingState::Reinstated
        );
    }
}
