//! Generated differential drivers (docs/SCHEMAS.md "M4 additions"): the
//! content-bound self-validation record, the mutant model, stratified mutant
//! sampling and the mutation-adequacy gate.
//!
//! Everything here is pure (no process, no clock): the oracle runs the builds
//! and hands the per-mutant outcomes to [`evaluate_mutation`], so the gate's
//! arithmetic is unit-testable and identical on every machine.

use crate::config::DriverPolicy;
use crate::error::Error;
use crate::verdict::Check;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Version of the driver-validation schema this build reads and writes.
pub const DRIVER_VALIDATION_SCHEMA_VERSION: u64 = 1;
/// `schema` field of `driver-validation.json`.
pub const DRIVER_VALIDATION_SCHEMA_NAME: &str = "ruharness-driver-validation";
/// Below this many compiled mutants the ratio rule is replaced by
/// "all but at most one killed" (a single equivalent mutant must not make a
/// tiny unit unvalidatable).
pub const RATIO_RULE_MIN_COMPILED: u32 = 10;

/// One mutation of the unit's C source: replace bytes `start..end` of `file`
/// with `replacement`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mutant {
    /// Repo-relative path of the mutated file.
    pub file: String,
    /// Byte offset where the replaced span starts.
    pub start: usize,
    /// Byte offset one past the replaced span.
    pub end: usize,
    /// Replacement bytes (UTF-8 text).
    pub replacement: String,
    /// Operator class, kebab-case (e.g. `relational`, `arith`, `literal`,
    /// `cast-delete`, `signedness`, `table-element`, `string-literal`).
    pub operator: String,
    /// 1-based line of `start` (display).
    pub line: u32,
    /// Enclosing function name; `""` for a file-scope site (static tables).
    pub function: String,
}

impl Mutant {
    /// Stable ordering key: blake3(file ‖ NUL ‖ start ‖ NUL ‖ operator ‖ NUL ‖
    /// replacement). No RNG anywhere in sampling.
    pub fn order_key(&self) -> String {
        let mut h = blake3::Hasher::new();
        h.update(self.file.as_bytes());
        h.update(b"\0");
        h.update(self.start.to_string().as_bytes());
        h.update(b"\0");
        h.update(self.operator.as_bytes());
        h.update(b"\0");
        h.update(self.replacement.as_bytes());
        h.finalize().to_hex().to_string()
    }

    /// Apply to the original file bytes. `None` if the span does not fit.
    pub fn apply(&self, original: &[u8]) -> Option<Vec<u8>> {
        if self.start > self.end || self.end > original.len() {
            return None;
        }
        let mut out = Vec::with_capacity(original.len() + self.replacement.len());
        out.extend_from_slice(&original[..self.start]);
        out.extend_from_slice(self.replacement.as_bytes());
        out.extend_from_slice(&original[self.end..]);
        Some(out)
    }
}

/// Deterministic stratified sample of at most `max` mutants: every unit
/// symbol first gets up to `ceil(max / |symbols|)` of the mutants inside its
/// own body (in [`Mutant::order_key`] order), then the remaining slots are
/// filled from all other mutants in key order. The result is sorted by key.
pub fn sample_mutants(all: &[Mutant], symbols: &[String], max: usize) -> Vec<Mutant> {
    let mut keyed: Vec<(String, &Mutant)> = all.iter().map(|m| (m.order_key(), m)).collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    keyed.dedup_by(|a, b| a.0 == b.0);
    if max == 0 {
        return Vec::new();
    }
    let per_symbol = if symbols.is_empty() {
        0
    } else {
        max.div_ceil(symbols.len())
    };
    let mut chosen: BTreeMap<String, &Mutant> = BTreeMap::new();
    for symbol in symbols {
        for (key, m) in keyed
            .iter()
            .filter(|(_, m)| &m.function == symbol)
            .take(per_symbol)
        {
            if chosen.len() < max {
                chosen.insert(key.clone(), m);
            }
        }
    }
    for (key, m) in &keyed {
        if chosen.len() >= max {
            break;
        }
        chosen.entry(key.clone()).or_insert(m);
    }
    chosen.into_values().cloned().collect()
}

/// What running one sampled mutant produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutantOutcome {
    /// The mutated unit did not compile (discarded, never counted).
    NotCompiled,
    /// The driver's output changed, or it crashed / exited non-zero / timed out.
    Killed,
    /// Byte-identical output: the driver cannot tell this mutant apart.
    Survived,
}

/// A surviving mutant, as reported (harness-generated text only — never the
/// mutated source bytes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Survivor {
    /// Repo-relative file.
    pub file: String,
    /// 1-based line.
    pub line: u32,
    /// Enclosing function (`""` at file scope).
    pub function: String,
    /// Operator class.
    pub operator: String,
}

/// Mutation-adequacy statistics recorded in the validation record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationStats {
    /// Mutation sites found in the unit's `.c` files.
    pub sites: u32,
    /// Mutants sampled (≤ policy `max_mutants`).
    pub sampled: u32,
    /// Sampled mutants that compiled.
    pub compiled: u32,
    /// Compiled mutants the driver killed.
    pub killed: u32,
    /// Compiled mutants that survived, sorted (file, line, operator).
    pub survivors: Vec<Survivor>,
}

/// Apply the mutation gate (docs/SCHEMAS.md "M4 additions"):
/// - `sites == 0` → passes, detail `n/a (0 sites)` (flagged by the scorer);
/// - sites exist but no sampled mutant compiled → `Err` (a HARNESS failure —
///   never fed to a model as driver feedback);
/// - `compiled ≥ 10` → `killed * 1000 ≥ min_kill_permille * compiled`;
/// - `1 ≤ compiled < 10` → `killed ≥ compiled − 1`;
/// - and every unit symbol with ≥ 2 compiled mutants in its own body has ≥ 1
///   kill.
pub fn evaluate_mutation(
    sites: u32,
    results: &[(Mutant, MutantOutcome)],
    symbols: &[String],
    policy: &DriverPolicy,
) -> Result<(MutationStats, Check), Error> {
    let sampled = u32::try_from(results.len()).unwrap_or(u32::MAX);
    let compiled_iter = results
        .iter()
        .filter(|(_, o)| *o != MutantOutcome::NotCompiled);
    let compiled = u32::try_from(compiled_iter.clone().count()).unwrap_or(u32::MAX);
    let killed = u32::try_from(
        compiled_iter
            .clone()
            .filter(|(_, o)| *o == MutantOutcome::Killed)
            .count(),
    )
    .unwrap_or(u32::MAX);
    let mut survivors: Vec<Survivor> = results
        .iter()
        .filter(|(_, o)| *o == MutantOutcome::Survived)
        .map(|(m, _)| Survivor {
            file: m.file.clone(),
            line: m.line,
            function: m.function.clone(),
            operator: m.operator.clone(),
        })
        .collect();
    survivors.sort_by(|a, b| {
        (&a.file, a.line, &a.operator, &a.function).cmp(&(
            &b.file,
            b.line,
            &b.operator,
            &b.function,
        ))
    });
    let stats = MutationStats {
        sites,
        sampled,
        compiled,
        killed,
        survivors,
    };
    if sites == 0 {
        return Ok((
            stats,
            Check {
                name: "mutation".into(),
                passed: true,
                detail: "n/a (0 sites)".into(),
            },
        ));
    }
    if compiled == 0 {
        return Err(Error::Invariant(format!(
            "mutation: {sites} site(s) but none of the {sampled} sampled mutant(s) compiled — \
             a harness limitation (include paths?), not a driver failure"
        )));
    }
    let ratio_ok = if compiled >= RATIO_RULE_MIN_COMPILED {
        u64::from(killed) * 1000 >= u64::from(policy.min_kill_permille) * u64::from(compiled)
    } else {
        killed + 1 >= compiled
    };
    let mut unkilled_symbols: Vec<&str> = Vec::new();
    for symbol in symbols {
        let own: Vec<&MutantOutcome> = results
            .iter()
            .filter(|(m, o)| &m.function == symbol && *o != MutantOutcome::NotCompiled)
            .map(|(_, o)| o)
            .collect();
        if own.len() >= 2 && !own.iter().any(|o| **o == MutantOutcome::Killed) {
            unkilled_symbols.push(symbol);
        }
    }
    let rule = if compiled >= RATIO_RULE_MIN_COMPILED {
        format!(
            "needs ≥ {}.{:03}",
            policy.min_kill_permille / 1000,
            policy.min_kill_permille % 1000
        )
    } else {
        format!("needs ≥ {} (small-n rule)", compiled - 1)
    };
    let mut detail =
        format!("killed {killed}/{compiled} compiled ({sampled} sampled of {sites} sites; {rule})");
    if !unkilled_symbols.is_empty() {
        detail.push_str(&format!(
            "; no mutant killed in: {}",
            unkilled_symbols.join(", ")
        ));
    }
    let passed = ratio_ok && unkilled_symbols.is_empty();
    Ok((
        stats,
        Check {
            name: "mutation".into(),
            passed,
            detail,
        },
    ))
}

/// What a validation was bound to — digests, never timestamps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverValidationInputs {
    /// File-set hash of the unit files + include closure (as in verdicts).
    pub unit_source: String,
    /// Hash of the validated `driver.c`.
    pub driver: String,
    /// Toolchain + sandbox + cflags identity strings.
    pub toolchain: Vec<String>,
}

/// `units/<id>/driver-validation.json` (and `validation.json` per driver
/// attempt): the C-vs-C self-validation of a generated driver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverValidation {
    /// Always [`DRIVER_VALIDATION_SCHEMA_NAME`].
    pub schema: String,
    /// Schema version.
    pub schema_version: u64,
    /// Unit id.
    pub unit: String,
    /// True iff every check passed.
    pub green: bool,
    /// What was validated.
    pub inputs: DriverValidationInputs,
    /// The effective policy the driver was held to.
    pub policy: DriverPolicy,
    /// Checks in run order: `driver-build`, `driver-shape`, `symbols-called`,
    /// `determinism`, `opt-levels`, `sanitizers`, `mutation` (a run stops at
    /// the first failure of the first three).
    pub checks: Vec<Check>,
    /// Mutation statistics, when the mutation check ran.
    pub mutation: Option<MutationStats>,
}

impl DriverValidation {
    /// Build a record; `green` is derived (non-empty, all passed).
    pub fn new(
        unit: impl Into<String>,
        inputs: DriverValidationInputs,
        policy: DriverPolicy,
        checks: Vec<Check>,
        mutation: Option<MutationStats>,
    ) -> DriverValidation {
        let green = !checks.is_empty() && checks.iter().all(|c| c.passed);
        DriverValidation {
            schema: DRIVER_VALIDATION_SCHEMA_NAME.to_string(),
            schema_version: DRIVER_VALIDATION_SCHEMA_VERSION,
            unit: unit.into(),
            green,
            inputs,
            policy,
            checks,
            mutation,
        }
    }

    /// Whether this record is green AND bound to exactly these digests.
    pub fn is_fresh_green(&self, unit_source: &str, driver: &str) -> bool {
        self.green && self.inputs.unit_source == unit_source && self.inputs.driver == driver
    }

    /// Atomic pretty-JSON write (trailing newline).
    pub fn store(&self, path: &Path) -> Result<(), Error> {
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Invariant(format!("serialize driver validation: {e}")))?;
        text.push('\n');
        crate::ledger::write_atomic(path, text.as_bytes())
    }

    /// Load, refusing a foreign or newer schema.
    pub fn load(path: &Path) -> Result<DriverValidation, Error> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        let v: DriverValidation =
            serde_json::from_str(&text).map_err(|e| Error::parse(path, e.to_string()))?;
        if v.schema != DRIVER_VALIDATION_SCHEMA_NAME {
            return Err(Error::parse(path, "not a ruharness-driver-validation file"));
        }
        if v.schema_version > DRIVER_VALIDATION_SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                path: path.into(),
                found: v.schema_version,
                supported: DRIVER_VALIDATION_SCHEMA_VERSION,
            });
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(function: &str, start: usize, op: &str) -> Mutant {
        Mutant {
            file: "src/a.c".into(),
            start,
            end: start + 1,
            replacement: "-".into(),
            operator: op.into(),
            line: 1,
            function: function.into(),
        }
    }

    const POLICY: DriverPolicy = DriverPolicy {
        max_mutants: 24,
        min_kill_permille: 600,
    };

    #[test]
    fn sampling_is_stratified_deterministic_and_bounded() {
        let mut all: Vec<Mutant> = (0..40).map(|i| m("big", i, "arith")).collect();
        all.extend((100..103).map(|i| m("small", i, "arith")));
        let symbols = vec!["big".to_string(), "small".to_string()];
        let s1 = sample_mutants(&all, &symbols, 16);
        let mut rev = all.clone();
        rev.reverse();
        let s2 = sample_mutants(&rev, &symbols, 16);
        assert_eq!(s1, s2, "order of discovery must not matter");
        assert_eq!(s1.len(), 16);
        assert_eq!(s1.iter().filter(|m| m.function == "small").count(), 3);
        assert!(sample_mutants(&all, &symbols, 0).is_empty());
    }

    #[test]
    fn apply_replaces_span() {
        let mutant = Mutant {
            file: "f.c".into(),
            start: 2,
            end: 3,
            replacement: "-".into(),
            operator: "arith".into(),
            line: 1,
            function: "f".into(),
        };
        assert_eq!(mutant.apply(b"a + b").unwrap(), b"a - b");
        assert!(Mutant { end: 99, ..mutant }.apply(b"x").is_none());
    }

    fn results(
        killed: usize,
        survived: usize,
        not_compiled: usize,
    ) -> Vec<(Mutant, MutantOutcome)> {
        let mut out = Vec::new();
        for i in 0..killed {
            out.push((m("f", i, "arith"), MutantOutcome::Killed));
        }
        for i in 0..survived {
            out.push((m("f", 100 + i, "relational"), MutantOutcome::Survived));
        }
        for i in 0..not_compiled {
            out.push((m("f", 200 + i, "cast-delete"), MutantOutcome::NotCompiled));
        }
        out
    }

    #[test]
    fn gate_ratio_rule_and_small_n_rule() {
        let sym = vec!["f".to_string()];
        let (_, c) = evaluate_mutation(30, &results(6, 4, 3), &sym, &POLICY).unwrap();
        assert!(c.passed, "6/10 meets 0.6: {}", c.detail);
        let (_, c) = evaluate_mutation(30, &results(5, 5, 0), &sym, &POLICY).unwrap();
        assert!(!c.passed, "5/10 < 0.6");
        let (_, c) = evaluate_mutation(3, &results(2, 1, 0), &sym, &POLICY).unwrap();
        assert!(c.passed, "small n tolerates one equivalent mutant");
        let (_, c) = evaluate_mutation(3, &results(1, 2, 0), &sym, &POLICY).unwrap();
        assert!(!c.passed);
    }

    #[test]
    fn gate_requires_a_kill_per_symbol_with_two_or_more_mutants() {
        let sym = vec!["f".to_string(), "g".to_string()];
        let mut r = results(12, 0, 0);
        r.push((m("g", 300, "arith"), MutantOutcome::Survived));
        r.push((m("g", 301, "arith"), MutantOutcome::Survived));
        let (stats, c) = evaluate_mutation(40, &r, &sym, &POLICY).unwrap();
        assert!(!c.passed, "{}", c.detail);
        assert!(c.detail.contains("no mutant killed in: g"), "{}", c.detail);
        assert_eq!(stats.survivors.len(), 2);
    }

    #[test]
    fn zero_sites_passes_flagged_and_zero_compiled_is_harness_error() {
        let (_, c) = evaluate_mutation(0, &[], &[], &POLICY).unwrap();
        assert!(c.passed && c.detail.starts_with("n/a"));
        assert!(evaluate_mutation(5, &results(0, 0, 5), &[], &POLICY).is_err());
    }
}
