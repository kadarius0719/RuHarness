//! Deterministic per-unit risk scoring (docs/SCHEMAS.md "Risk score").
//!
//! Computed at render time as a pure function — never committed as records.
//! v1 formula: capped weighted sum with FIXED ABSOLUTE caps (re-normalization
//! churn is forbidden); weights are the M4 calibration hypothesis, recorded in
//! DECISIONS.md. A `blocker` finding pins the unit score to ≥ 90.

use crate::facts::Facts;
use crate::observer::{affected_units, counts_for_risk, Finding, Review, TriageFile};
use crate::plan::Plan;
use std::collections::BTreeMap;

/// A unit's risk score and its signal breakdown.
#[derive(Debug, Clone)]
pub struct UnitRisk {
    /// Unit id.
    pub unit: String,
    /// 0–100.
    pub score: u32,
    /// Signal name → raw (uncapped) value, for the rendered breakdown.
    pub signals: BTreeMap<&'static str, u32>,
    /// Blocker categories affecting this unit (score pinned ≥ 90 when any).
    pub blockers: Vec<String>,
}

// Fixed absolute normalization caps (v1 — changing any is a schema-visible
// scoring change, record it in DECISIONS.md).
const CAP_SIG_STARS: f64 = 40.0;
const CAP_ALLOC: f64 = 10.0;
const CAP_LOC: f64 = 2000.0;
const CAP_SCC: f64 = 5.0;
const CAP_FAN_IN: f64 = 10.0;
const CAP_DEPTH: f64 = 5.0;
const CAP_UB: f64 = 5.0;
const CAP_MACRO_GLOBAL: f64 = 10.0;

fn capped(value: u32, cap: f64, weight: f64) -> f64 {
    (f64::from(value) / cap).min(1.0) * weight
}

/// Score every plan unit. Findings include annotations; dismissed findings
/// count until a human upholds the dismissal ([`counts_for_risk`]).
/// Result sorted by descending score, unit-id tiebreak.
pub fn score_units(
    facts: &Facts,
    plan: &Plan,
    findings: &[Finding],
    triage: &TriageFile,
    reviews: &[Review],
) -> Vec<UnitRisk> {
    // Dependency depth per unit (longest chain), from the validated order.
    let mut depth: BTreeMap<&str, u32> = BTreeMap::new();
    if let Ok(order) = plan.execution_order() {
        for unit in order {
            let d = unit
                .depends_on
                .iter()
                .filter_map(|dep| depth.get(dep.as_str()))
                .max()
                .map(|m| m + 1)
                .unwrap_or(0);
            depth.insert(unit.id.as_str(), d);
        }
    }
    // Fan-in: units that depend on this unit.
    let mut fan_in: BTreeMap<&str, u32> = BTreeMap::new();
    for u in &plan.units {
        for dep in &u.depends_on {
            *fan_in.entry(dep.as_str()).or_insert(0) += 1;
        }
    }

    let mut out: Vec<UnitRisk> = Vec::with_capacity(plan.units.len());
    for unit in &plan.units {
        // Signature pointer density: '*' occurrences in the unit's public
        // symbol signatures (documented proxy until a typed frontend).
        let sig_stars: u32 = facts
            .symbols
            .iter()
            .filter(|s| unit.files.contains(&s.file) && s.visibility == "public")
            .map(|s| s.signature.matches('*').count() as u32)
            .sum();
        let loc: u32 = facts
            .symbols
            .iter()
            .filter(|s| unit.files.contains(&s.file))
            .map(|s| s.span.1.saturating_sub(s.span.0) + 1)
            .sum();

        let mut alloc = 0u32;
        let mut ub = 0u32;
        let mut macro_global = 0u32;
        let mut blockers: Vec<String> = Vec::new();
        for f in findings {
            if !counts_for_risk(f, triage, reviews) {
                continue;
            }
            if !affected_units(&f.file, plan, facts).contains(&unit.id.as_str()) {
                continue;
            }
            if f.blocker && !blockers.contains(&f.category) {
                blockers.push(f.category.clone());
            }
            // Human/oracle annotations are by definition the UB/impl-defined
            // hazards the detectors cannot see: they always feed the UB signal.
            if f.detector == "human" || f.detector == "oracle" {
                ub += 1;
                continue;
            }
            match f.category.as_str() {
                "alloc-ownership" => alloc += 1,
                "bitfield" | "union-decl" | "ub-reliance" | "impl-defined" | "impl-contract" => {
                    ub += 1
                }
                c if c.starts_with("macro-") => macro_global += 1,
                "global-mutable" => macro_global += 1,
                _ => {}
            }
        }
        blockers.sort();

        let scc = unit.files.len() as u32;
        let fi = *fan_in.get(unit.id.as_str()).unwrap_or(&0);
        let dd = *depth.get(unit.id.as_str()).unwrap_or(&0);

        // Weights (v1, sum = 100): ptr 25 · size×coupling 30 · ub 20 ·
        // macro+global 15 · alloc 10. Each signal appears in exactly one term.
        let ptr = capped(sig_stars, CAP_SIG_STARS, 25.0);
        let size = capped(loc, CAP_LOC, 15.0)
            + capped(scc, CAP_SCC, 5.0)
            + capped(fi, CAP_FAN_IN, 5.0)
            + capped(dd, CAP_DEPTH, 5.0);
        let ub_w = capped(ub, CAP_UB, 20.0);
        let mg = capped(macro_global, CAP_MACRO_GLOBAL, 15.0);
        let alloc_w = capped(alloc, CAP_ALLOC, 10.0);
        let mut score = (ptr + size + ub_w + mg + alloc_w).round() as u32;
        if !blockers.is_empty() {
            score = score.max(90);
        }
        let score = score.min(100);

        let mut signals: BTreeMap<&'static str, u32> = BTreeMap::new();
        signals.insert("sig_stars", sig_stars);
        signals.insert("loc", loc);
        signals.insert("scc_files", scc);
        signals.insert("fan_in", fi);
        signals.insert("dep_depth", dd);
        signals.insert("alloc", alloc);
        signals.insert("ub", ub);
        signals.insert("macro_global", macro_global);

        out.push(UnitRisk {
            unit: unit.id.clone(),
            score,
            signals,
            blockers,
        });
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.unit.cmp(&b.unit)));
    out
}
