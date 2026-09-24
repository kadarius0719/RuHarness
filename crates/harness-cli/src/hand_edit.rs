//! `harness override <UNIT> <DIR>` — a hand edit's only way into the ledger
//! (docs/TUI-DESIGN.md §5.2): exactly `src/logic.rs` and `src/ffi.rs` of DIR,
//! judged by the migrate stage's one judge and recorded as a labelled
//! `human` attempt that the benchmark never counts as the pipeline's.

use crate::{lock_ledger, out, promote, report, require_sandbox, EXIT_ORACLE_RED};
use anyhow::{bail, Context, Result};
use harness_core::attempts;
use harness_core::ledger::Ledger;
use harness_core::{Facts, Plan, TargetContext, Unit};
use std::path::{Path, PathBuf};

/// Largest file of a hand edit that is read.
const MAX_EDIT_FILE_BYTES: u64 = 1024 * 1024;

/// `harness override`.
pub(crate) fn cmd_override(
    unit_id: String,
    dir: PathBuf,
    note: Option<String>,
    target: PathBuf,
    allow_unsandboxed: bool,
) -> Result<u8> {
    require_sandbox(allow_unsandboxed, "harness override")?;
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = lock_ledger(&ledger, &format!("override {unit_id}"))?;
    let plan_doc = Plan::load(&ledger.plan_path())?;
    plan_doc
        .execution_order()
        .context("plan.toml is structurally invalid; fix it first")?;
    let unit = plan_doc.unit(&unit_id)?;
    promote::recover_promotion(&ctx, &ledger, unit)?;
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    promote::migrate_preconditions(&ctx, &ledger, &facts, unit, "recording a hand edit")?;
    let (logic, ffi) = read_hand_edit(&ctx, unit, &dir)?;

    // A no-op edit would file a second attempt with an existing candidate's
    // digest and make the crate's provenance ambiguous (R-5).
    let (unit_source, driver) = attempts::current_binding(&ctx, &facts, unit)?;
    for rec in attempts::load_unit_attempts(&ledger, &unit_id)? {
        if rec.unit_source != unit_source || rec.driver != driver {
            continue;
        }
        // An unfinished HUMAN record is an override killed mid-judge (this
        // command holds the writer lock): `record_human_attempt` reclaims it
        // when it is this edit. An unfinished MODEL attempt still counts —
        // resumed, it would finish with this digest.
        if rec.provider_kind == attempts::HUMAN_KIND && rec.outcome == "in-progress" {
            continue;
        }
        let candidate = attempts::attempt_dir(&ledger, &unit_id, &rec.id).join("candidate");
        let same = |rel: &str, text: &str| {
            std::fs::read_to_string(candidate.join(rel)).is_ok_and(|on_disk| on_disk == text)
        };
        if same("src/logic.rs", &logic) && same("src/ffi.rs", &ffi) {
            bail!(
                "identical to attempt {} ({}); nothing to record",
                harness_llm::printable(&rec.id, 64),
                harness_llm::printable(&rec.outcome, 32)
            );
        }
    }

    let oracle = harness_oracle::CAbiDifferential;
    let edit = harness_llm::HumanEdit {
        logic: &logic,
        ffi: &ffi,
        note: note.as_deref(),
    };
    let outcome = harness_llm::record_human_attempt(&oracle, &ctx, &facts, &plan_doc, unit, &edit)?;
    let record = &outcome.record;
    if let Some(v) = &outcome.verdict {
        report::verdict(
            &unit_id,
            v,
            &outcome.attempt_dir.join("attempt-verdict.json"),
        );
    }
    // A red the judge stored no verdict for (the deny scan): say why.
    for line in outcome.failure_evidence.as_deref().unwrap_or("").lines() {
        out(format!(
            "override: deny scan: {}",
            harness_llm::printable(line.trim_start_matches("- "), 300)
        ));
    }
    let turn = record.turns.last().map_or("", |t| t.result.as_str());
    out(format!(
        "override: {unit_id} human attempt {} -> {} ({turn})",
        record.id,
        record.outcome.to_uppercase()
    ));
    report::event(&report::AttemptEvent {
        k: "attempt",
        unit: &unit_id,
        id: &record.id,
        outcome: &record.outcome,
        provider: &record.provider,
        model: &record.model,
        promoted: false,
        promotion: "not promoted: override never promotes",
    });
    if record.outcome == "green" {
        out(format!(
            "override: recorded, not promoted — `harness promote {unit_id} {}` promotes it",
            record.id
        ));
        Ok(0)
    } else {
        Ok(EXIT_ORACLE_RED)
    }
}

/// Read the two files of a hand edit, refusing anything else the harness
/// would otherwise silently drop or trust: DIR inside the target's ledger;
/// any `src/` entry but `logic.rs`, `ffi.rs` and a `lib.rs` equal to the
/// harness-owned one; a `Cargo.toml` that differs from the harness-owned
/// manifest; a symlink, a non-regular file, or a file over 1 MiB.
fn read_hand_edit(ctx: &TargetContext, unit: &Unit, dir: &Path) -> Result<(String, String)> {
    let dir = dir
        .canonicalize()
        .with_context(|| format!("reading {}", dir.display()))?;
    let ledger_dir = Ledger::new(&ctx.root).dir();
    if let Ok(ledger_dir) = ledger_dir.canonicalize() {
        if dir.starts_with(&ledger_dir) {
            bail!(
                "{} is inside the target's migration/ ledger; edit a copy elsewhere",
                dir.display()
            );
        }
    }
    let crate_name = unit
        .oracle_param_str("rust_crate")
        .context("unit has no rust_crate oracle param")?;
    let src = dir.join("src");
    let meta =
        std::fs::symlink_metadata(&src).with_context(|| format!("reading {}", src.display()))?;
    if !meta.file_type().is_dir() {
        bail!(
            "{} is not a directory (symlinks are refused)",
            src.display()
        );
    }
    let read = |path: &Path| -> Result<String> {
        let meta = std::fs::symlink_metadata(path)
            .with_context(|| format!("reading {}", path.display()))?;
        if !meta.file_type().is_file() {
            bail!(
                "{} is not a regular file (symlinks are refused)",
                path.display()
            );
        }
        if meta.len() > MAX_EDIT_FILE_BYTES {
            bail!(
                "{} is larger than {MAX_EDIT_FILE_BYTES} bytes",
                path.display()
            );
        }
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
    };
    for entry in std::fs::read_dir(&src).with_context(|| format!("reading {}", src.display()))? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        match name.as_str() {
            "logic.rs" | "ffi.rs" => {}
            "lib.rs" => {
                if read(&src.join("lib.rs"))? != harness_llm::CANDIDATE_LIB_RS {
                    bail!(
                        "src/lib.rs differs from the harness-owned one: a hand edit is exactly \
                         src/logic.rs and src/ffi.rs (the harness owns Cargo.toml and src/lib.rs)"
                    );
                }
            }
            other => bail!(
                "src/{} is not accepted: a hand edit is exactly src/logic.rs and src/ffi.rs",
                harness_llm::printable(other, 64)
            ),
        }
    }
    let manifest = dir.join("Cargo.toml");
    if manifest.exists() && read(&manifest)? != harness_llm::candidate_manifest(crate_name) {
        bail!(
            "Cargo.toml differs from the harness-owned manifest: a hand edit is exactly \
             src/logic.rs and src/ffi.rs (no dependencies, no build script)"
        );
    }
    Ok((read(&src.join("logic.rs"))?, read(&src.join("ffi.rs"))?))
}
