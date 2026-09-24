//! Promotion of a green migrate attempt into the unit's crate
//! (docs/SCHEMAS.md "Promotion protocol"; docs/CLI-HARDENING.md §2), its
//! crash recovery, the preconditions `migrate` and `promote` share, and the
//! `harness promote` command.
//!
//! ONE marker spans the whole protocol: `units/<id>/.promote-<attempt>/`.
//! It is created when the candidate is staged and removed LAST, after the
//! green tail (verdicts → status → `promoted: true` → `.prev` removed) has
//! completed; every step of the tail is idempotent, so a kill anywhere is
//! re-runnable, and [`recover_promotion`] resolves every marker it finds by
//! EVIDENCE (digests), never by guesswork.

use crate::{out, report, require_sandbox, EXIT_ORACLE_RED};
use anyhow::{bail, Context, Result};
use harness_core::attempts::{self, AttemptRecord};
use harness_core::ledger::Ledger;
use harness_core::traits::OracleStrategy;
use harness_core::{hash, plan, Error, Facts, Plan, TargetContext, Unit, UnitStatus, Verdict};
use std::path::{Path, PathBuf};

/// Prefix of the marker directory (`.promote-<attempt id>`).
const MARKER_PREFIX: &str = ".promote-";

/// How a promotion ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Promotion {
    /// Verified in place; the unit is `verified`.
    Verified,
    /// The in-place verdict was red; everything was rolled back.
    RolledBack,
}

/// The preconditions `migrate` establishes before running a unit and
/// `promote` re-establishes because the attempt may be older than the
/// tree: the plan's `source_hash` matches the tree, and (R6) a unit with
/// driver-generation history has a `validated` driver — deleting
/// `driver-validation.json` cannot relabel a generated driver as
/// human-written. `what` names the action for the hint.
pub(crate) fn migrate_preconditions(
    ctx: &TargetContext,
    ledger: &Ledger,
    facts: &Facts,
    unit: &Unit,
    what: &str,
) -> Result<()> {
    let closure = facts.include_closure(&unit.files);
    let current = hash::file_set_hash_on_disk(&ctx.root, &closure)?;
    if current != unit.source_hash {
        return Err(Error::Stale {
            subject: format!("unit `{}`", unit.id),
            hint: format!(
                "source changed since planning; run `harness scan`, then `harness plan`, review \
                 the diff, then {what}"
            ),
        }
        .into());
    }
    if ledger.unit_dir(&unit.id).join("driver-attempts").exists()
        || ledger.driver_validation_path(&unit.id).exists()
    {
        let state = crate::bench::driver_state(ctx, facts, unit)?;
        if state != "validated" {
            return Err(Error::Stale {
                subject: format!("unit `{}`", unit.id),
                hint: format!(
                    "its generated driver's validation is `{state}`; run `harness gen-driver {}` \
                     (or re-validate) before {what}",
                    unit.id
                ),
            }
            .into());
        }
    }
    Ok(())
}

/// The `(unit_source, driver)` digests an attempt of `unit` is bound to
/// when recorded against the CURRENT tree — derived exactly as
/// `run_migration` records them and as `bench` recognises provenance
/// (R-5): the include-closure file-set hash and the driver file hash.
pub(crate) fn current_binding(
    ctx: &TargetContext,
    facts: &Facts,
    unit: &Unit,
) -> Result<(String, String)> {
    let closure = facts.include_closure(&unit.files);
    let unit_source = hash::file_set_hash_on_disk(&ctx.root, &closure)?;
    let driver_rel = unit
        .oracle_param_str("driver")
        .context("unit has no [unit.oracle] driver")?;
    let driver = hash::file_hash(&ctx.root.join(driver_rel))?;
    Ok((unit_source, driver))
}

/// Copy the closed crate file list (Cargo.toml, Cargo.lock, src/**) — never
/// `target/` — from `from` to a fresh `to`.
pub(crate) fn copy_crate_sources(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to.join("src")).context("creating staged crate")?;
    for name in ["Cargo.toml", "Cargo.lock"] {
        let src = from.join(name);
        if src.exists() {
            std::fs::copy(&src, to.join(name)).with_context(|| format!("copying {name}"))?;
        }
    }
    fn copy_tree(from: &Path, to: &Path) -> Result<()> {
        for entry in std::fs::read_dir(from).context("reading candidate src")? {
            let entry = entry?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let (src, dst) = (entry.path(), to.join(&name));
            if src.is_dir() {
                std::fs::create_dir_all(&dst)?;
                copy_tree(&src, &dst)?;
            } else {
                std::fs::copy(&src, &dst)?;
            }
        }
        Ok(())
    }
    copy_tree(&from.join("src"), &to.join("src"))
}

/// The paths one promotion touches.
struct Paths {
    crate_dir: PathBuf,
    prev: PathBuf,
    marker: PathBuf,
}

fn paths(ledger: &Ledger, unit: &Unit, crate_name: &str, attempt: &str) -> Paths {
    let unit_dir = ledger.unit_dir(&unit.id);
    Paths {
        crate_dir: unit_dir.join(crate_name),
        prev: unit_dir.join(format!(".{crate_name}.prev")),
        marker: unit_dir.join(format!("{MARKER_PREFIX}{attempt}")),
    }
}

/// Undo a swap: remove the promoted crate, restore `.prev` when there is
/// one, remove the marker.
fn rollback(p: &Paths) -> Result<()> {
    if p.crate_dir.exists() {
        std::fs::remove_dir_all(&p.crate_dir).context("removing unverified promoted crate")?;
    }
    if p.prev.exists() {
        std::fs::rename(&p.prev, &p.crate_dir).context("restoring previous crate")?;
    }
    if p.marker.exists() {
        std::fs::remove_dir_all(&p.marker).context("removing promotion marker")?;
    }
    Ok(())
}

/// The green tail, every step idempotent, in the fixed order: verdicts →
/// status → `promoted: true` → `.prev` → the marker LAST.
fn finish(
    ledger: &Ledger,
    unit: &Unit,
    verdict: &Verdict,
    record: &AttemptRecord,
    p: &Paths,
) -> Result<()> {
    verdict.store(&ledger.verdict_latest_path(&unit.id))?;
    verdict.store(&ledger.verdict_last_green_path(&unit.id))?;
    harness_core::ledger::write_atomic(
        &ledger.verdict_md_path(&unit.id),
        verdict.render_md().as_bytes(),
    )?;
    plan::set_status(&ledger.plan_path(), &unit.id, UnitStatus::Verified)?;
    if !record.promoted {
        let mut promoted = record.clone();
        promoted.promoted = true;
        promoted.store(&attempts::attempt_dir(ledger, &unit.id, &record.id))?;
    }
    if p.prev.exists() {
        std::fs::remove_dir_all(&p.prev).context("removing previous crate backup")?;
    }
    if p.marker.exists() {
        std::fs::remove_dir_all(&p.marker).context("removing promotion marker")?;
    }
    Ok(())
}

/// Whether the crate on disk is `record`'s candidate (digest match).
fn swapped_in(crate_dir: &Path, record: &AttemptRecord) -> bool {
    crate_dir.exists()
        && hash::crate_content_hash(crate_dir).ok().as_deref()
            == Some(record.candidate_digest.as_str())
}

/// The committed green verdict, when it is bound to the crate on disk.
fn verified_in_place(
    ctx: &TargetContext,
    ledger: &Ledger,
    unit: &Unit,
    crate_dir: &Path,
) -> Option<Verdict> {
    let v = Verdict::load(&ledger.verdict_latest_path(&unit.id)).ok()?;
    if !v.green {
        return None;
    }
    let now = hash::unit_crate_file_set_hash(&ctx.root, crate_dir).ok()?;
    (now == v.inputs.rust_crate).then_some(v)
}

/// Resolve every promotion that was interrupted, by EVIDENCE. Runs in every
/// writing command right after the writer lock. For each marker
/// `.promote-<id>/`: the crate on disk is the attempt's candidate (digest)
/// or not; if not, the old crate is untouched or moved aside — restore
/// `.prev` when the crate is absent and drop the marker; if so, the
/// committed verdict is green and bound to it (the tail was under way —
/// finish it) or it is not (the in-place verify never completed — roll
/// back). A bare `.<crate>.prev` with no marker (the pre-marker protocol)
/// is resolved the same way: the verdict bound to the crate on disk means
/// the promotion had completed and only the backup is stale.
pub(crate) fn recover_promotion(ctx: &TargetContext, ledger: &Ledger, unit: &Unit) -> Result<()> {
    let Some(crate_name) = unit.oracle_param_str("rust_crate") else {
        return Ok(());
    };
    let unit_dir = ledger.unit_dir(&unit.id);
    let mut markers: Vec<String> = match std::fs::read_dir(&unit_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| n.strip_prefix(MARKER_PREFIX))
                    .map(str::to_string)
            })
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", unit_dir.display())),
    };
    markers.sort();
    for id in &markers {
        let p = paths(ledger, unit, crate_name, id);
        let record = attempts::load_pinned(ledger, &unit.id, id)?;
        match record {
            Some(record) if swapped_in(&p.crate_dir, &record) => {
                match verified_in_place(ctx, ledger, unit, &p.crate_dir) {
                    Some(verdict) => {
                        finish(ledger, unit, &verdict, &record, &p)?;
                        out(format!(
                            "recover: promotion of `{}` (attempt {id}) had verified in place; \
                             finished its bookkeeping",
                            unit.id
                        ));
                    }
                    None => {
                        rollback(&p)?;
                        out(format!(
                            "recover: rolled back an interrupted, unverified promotion of `{}` \
                             (attempt {id})",
                            unit.id
                        ));
                    }
                }
            }
            _ => {
                // Killed before or between the renames (or a stale marker
                // whose crate is another promotion's): nothing of this
                // attempt is at the promoted path.
                if !p.crate_dir.exists() && p.prev.exists() {
                    std::fs::rename(&p.prev, &p.crate_dir).context("restoring previous crate")?;
                }
                std::fs::remove_dir_all(&p.marker).context("removing promotion marker")?;
                out(format!(
                    "recover: cleared the staging of an interrupted promotion of `{}` (attempt \
                     {id}); the previous crate is in place",
                    unit.id
                ));
            }
        }
    }
    // Legacy: a backup with no marker.
    let p = paths(ledger, unit, crate_name, "");
    if p.prev.exists() {
        if p.crate_dir.exists() && verified_in_place(ctx, ledger, unit, &p.crate_dir).is_some() {
            std::fs::remove_dir_all(&p.prev).context("finishing promotion cleanup")?;
            out(format!(
                "recover: promotion of `{}` had completed (verdict bound to the crate on disk); \
                 removed the leftover backup",
                unit.id
            ));
        } else {
            if p.crate_dir.exists() {
                std::fs::remove_dir_all(&p.crate_dir)
                    .context("removing unverified promoted crate")?;
            }
            std::fs::rename(&p.prev, &p.crate_dir).context("restoring previous crate")?;
            out(format!(
                "recover: rolled back an interrupted, unverified promotion of `{}`",
                unit.id
            ));
        }
    }
    Ok(())
}

/// Promote `record`'s candidate (at `candidate`) into the unit's crate and
/// verify it IN PLACE; nothing is persisted unless the verdict is green.
/// The caller has taken the writer lock, run [`recover_promotion`], and
/// established the refusals of docs/CLI-HARDENING.md §2.
pub(crate) fn promote_attempt(
    ctx: &TargetContext,
    ledger: &Ledger,
    oracle: &dyn OracleStrategy,
    unit: &Unit,
    record: &AttemptRecord,
    candidate: &Path,
) -> Result<Promotion> {
    let crate_name = unit
        .oracle_param_str("rust_crate")
        .context("unit has no rust_crate oracle param")?;
    let p = paths(ledger, unit, crate_name, &record.id);
    if p.marker.exists() {
        bail!(
            "harness bug: promotion marker {} exists after recovery",
            p.marker.display()
        );
    }
    let staged = p.marker.join(crate_name);
    copy_crate_sources(candidate, &staged)?;
    if hash::crate_content_hash(&staged)? != record.candidate_digest {
        std::fs::remove_dir_all(&p.marker).ok();
        bail!("staged candidate digest does not match the attempt record; not promoting");
    }
    // Two renames; the marker stays until the tail is done (recovery
    // resolves a kill anywhere by evidence).
    if p.crate_dir.exists() {
        std::fs::rename(&p.crate_dir, &p.prev).context("moving current crate aside")?;
    }
    std::fs::rename(&staged, &p.crate_dir).context("swapping candidate in")?;

    let in_place = oracle.verify(ctx, unit);
    let verdict = match in_place {
        Ok(v) if v.green => v,
        other => {
            rollback(&p)?;
            // The red in-place verdict is NOT stored (SCHEMAS.md "Promotion
            // protocol" (4)): its checks are reported as courtesy — human
            // lines and `check` events — but never a `verdict` line, which
            // names a stored file.
            if let Ok(v) = &other {
                for c in &v.checks {
                    out(format!(
                        "promote: [{}] {} — {}",
                        if c.passed { "PASS" } else { "FAIL" },
                        c.name,
                        c.detail
                    ));
                }
                report::checks(&unit.id, v);
            }
            report::event(&report::PromoteEvent {
                k: "promote",
                unit: &unit.id,
                attempt: &record.id,
                result: "rolled-back",
            });
            other?; // a harness error surfaces as such; a red verdict falls through
            return Ok(Promotion::RolledBack);
        }
    };
    finish(ledger, unit, &verdict, record, &p)?;
    report::verdict(&unit.id, &verdict, &ledger.verdict_latest_path(&unit.id));
    report::event(&report::PromoteEvent {
        k: "promote",
        unit: &unit.id,
        attempt: &record.id,
        result: "verified",
    });
    Ok(Promotion::Verified)
}

/// `harness promote <UNIT> <ATTEMPT>`: the explicit act that Accept is.
pub(crate) fn cmd_promote(
    unit_id: String,
    attempt_id: String,
    target: PathBuf,
    replace: bool,
    allow_unsandboxed: bool,
) -> Result<u8> {
    require_sandbox(allow_unsandboxed, "harness promote")?;
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = crate::lock_ledger(&ledger, &format!("promote {unit_id} {attempt_id}"))?;
    let plan_doc = Plan::load(&ledger.plan_path())?;
    plan_doc
        .execution_order()
        .context("plan.toml is structurally invalid; fix it before promoting")?;
    let unit = plan_doc.unit(&unit_id)?;
    recover_promotion(&ctx, &ledger, unit)?;
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;

    // 1–3: the record, by id, and what it claims.
    let record = attempts::load_pinned(&ledger, &unit_id, &attempt_id)?
        .with_context(|| format!("unit `{unit_id}` has no attempt `{attempt_id}`"))?;
    if let Some(stage) = &record.stage {
        bail!(
            "attempt {attempt_id} is a `{stage}` attempt; `harness promote` takes migrate \
             attempts (drivers: `harness gen-driver --promote`)"
        );
    }
    if record.outcome != "green" {
        bail!(
            "attempt {attempt_id} is `{}`, not green; only a green attempt can be promoted",
            record.outcome
        );
    }
    if record.turns.last().is_none_or(|t| t.result != "green") || record.candidate_digest.is_empty()
    {
        bail!("attempt {attempt_id} claims green but its last turn or candidate digest says otherwise; refusing");
    }
    if record.promoted && !replace {
        bail!("attempt {attempt_id} is already promoted — pass --replace to promote it again");
    }
    // 4: the preconditions migrate had, re-established.
    migrate_preconditions(&ctx, &ledger, &facts, unit, "promoting")?;
    // 5: the record is bound to the CURRENT inputs (R-5 provenance).
    let (unit_source, driver) = current_binding(&ctx, &facts, unit)?;
    if record.unit_source != unit_source || record.driver != driver {
        let moved = if record.unit_source != unit_source {
            "unit source"
        } else {
            "driver"
        };
        return Err(Error::Stale {
            subject: format!("attempt {attempt_id}"),
            hint: format!(
                "it is bound to superseded inputs (its {moved} is not the current one); re-run \
                 `harness migrate {unit_id} --no-promote` to record an attempt against the \
                 current tree"
            ),
        }
        .into());
    }
    // 6: the candidate is intact.
    let attempt_dir = attempts::attempt_dir(&ledger, &unit_id, &attempt_id);
    let candidate = attempt_dir.join("candidate");
    if !candidate.is_dir() {
        bail!("attempt {attempt_id} has no candidate/ to promote (pruned?)");
    }
    if hash::crate_content_hash(&candidate)? != record.candidate_digest {
        bail!("attempt {attempt_id}: candidate/ does not match the record's digest; refusing");
    }
    // 7: a verified unit is replaced only on request.
    if matches!(unit.status, UnitStatus::Verified | UnitStatus::Merged) && !replace {
        bail!(
            "unit `{unit_id}` is already {} — pass --replace to replace its crate",
            unit.status.as_str()
        );
    }

    let oracle = harness_oracle::CAbiDifferential;
    match promote_attempt(&ctx, &ledger, &oracle, unit, &record, &candidate)? {
        Promotion::Verified => {
            out(format!(
                "promote: {unit_id} attempt {attempt_id} promoted and verified — status set to \
                 verified"
            ));
            Ok(0)
        }
        Promotion::RolledBack => {
            out(format!(
                "promote: {unit_id} attempt {attempt_id} did not verify in place — rolled back"
            ));
            Ok(EXIT_ORACLE_RED)
        }
    }
}
