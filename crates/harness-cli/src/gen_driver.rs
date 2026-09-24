//! `harness gen-driver <UNIT>` — LLM driver generation with C-vs-C
//! self-validation (docs/SCHEMAS.md "M4 additions"; briefing §3.5 step 1).
//!
//! The model writes `driver.c` against the ORIGINAL C; the oracle's
//! `validate_driver` judges every candidate (build, shape, symbols called,
//! determinism, -O0/-O2 equality, sanitizers, mutation adequacy). A green
//! candidate is promoted to `migration/units/<id>/driver.c`, re-validated IN
//! PLACE, and only then recorded as `driver-validation.json`.

use anyhow::{bail, Context, Result};
use harness_core::driver::DriverValidation;
use harness_core::ledger::Ledger;
use harness_core::plan::{self as plan_mod, OracleValue};
use harness_core::{hash, Facts, Plan, TargetContext};
use std::path::{Path, PathBuf};

use crate::{lock_ledger, out, report, require_sandbox, safe_ledger_dir, EXIT_ORACLE_RED};

/// Arguments of `harness gen-driver`.
pub struct GenDriverArgs {
    pub unit: String,
    pub target: PathBuf,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub promote: bool,
    pub allow_unsandboxed: bool,
    pub retry: bool,
    pub attempt: Option<String>,
}

impl GenDriverArgs {
    /// The command line that resumes this run, as a human would type it:
    /// every value shell-quoted and attached (clap reads a separate `-…`
    /// word as a flag), every run flag kept; global flags (`--json`) are not
    /// repeated (a client re-runs the event's `args`).
    pub fn resume_command(&self) -> String {
        let q = report::shell_quote;
        let mut cmd = format!("harness gen-driver {}", q(&self.unit));
        cmd.push_str(&format!(" --target={}", q(&self.target.to_string_lossy())));
        if let Some(p) = &self.provider {
            cmd.push_str(&format!(" --provider={}", q(p)));
        }
        if let Some(m) = &self.model {
            cmd.push_str(&format!(" --model={}", q(m)));
        }
        if self.promote {
            cmd.push_str(" --promote");
        }
        if self.allow_unsandboxed {
            cmd.push_str(" --allow-unsandboxed");
        }
        if self.retry {
            cmd.push_str(" --retry");
        }
        if let Some(a) = &self.attempt {
            cmd.push_str(&format!(" --attempt={}", q(a)));
        }
        cmd
    }
}

/// The default `[unit.oracle]` table for a unit (shared with `bench init`):
/// the generated driver lives at `migration/units/<id>/driver.c`.
pub fn default_oracle_entries(unit_id: &str, files: &[String]) -> Vec<(&'static str, OracleValue)> {
    vec![
        ("kind", OracleValue::Str("c-abi-differential".into())),
        (
            "driver",
            OracleValue::Str(format!("migration/units/{unit_id}/driver.c")),
        ),
        (
            "rust_crate",
            OracleValue::Str(format!("{}_rs", unit_id.replace('-', "_"))),
        ),
        ("replaces", OracleValue::List(files.to_vec())),
    ]
}

pub fn cmd_gen_driver(args: GenDriverArgs) -> Result<u8> {
    let resume = args.resume_command();
    let GenDriverArgs {
        unit: unit_id,
        target,
        provider: provider_flag,
        model: model_flag,
        promote,
        allow_unsandboxed,
        retry,
        attempt,
    } = args;
    require_sandbox(allow_unsandboxed, "harness gen-driver")?;
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = lock_ledger(&ledger, &format!("gen-driver {unit_id}"))?;
    let plan_doc = Plan::load(&ledger.plan_path())?;
    plan_doc
        .execution_order()
        .context("plan.toml is structurally invalid; fix it first")?;
    let unit = plan_doc.unit(&unit_id)?;
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    let closure = facts.include_closure(&unit.files);
    if hash::file_set_hash_on_disk(&ctx.root, &closure)? != unit.source_hash {
        return Err(harness_core::Error::Stale {
            subject: format!("unit `{unit_id}`"),
            hint: "source changed since planning; run `harness scan`, then `harness plan`".into(),
        }
        .into());
    }
    // Where a promoted driver will live; refuse up front (before any model
    // call) when promotion could only ever clobber a human-written driver.
    let dest = ledger.driver_path(&unit_id);
    let dest_rel = format!("migration/units/{unit_id}/driver.c");
    if let Some(configured) = unit.oracle_param_str("driver") {
        if configured != dest_rel {
            bail!(
                "unit `{unit_id}` has a driver at `{configured}` (not `{dest_rel}`): generated \
                 drivers never replace a configured driver"
            );
        }
    }
    let validation_path = ledger.driver_validation_path(&unit_id);
    if dest.exists() && !validation_path.exists() {
        bail!(
            "unit `{unit_id}` has a driver with no driver-validation.json — a human-written \
             driver, which gen-driver never replaces"
        );
    }

    // Stage routing (§13.2): flag > [llm.driver] > [llm].
    let llm = &ctx.config.llm;
    let stage = llm.driver.as_ref();
    let provider_name = provider_flag
        .or_else(|| stage.and_then(|m| m.provider.clone()))
        .unwrap_or_else(|| llm.provider.clone());
    let model = model_flag
        .or_else(|| stage.and_then(|m| m.model.clone()))
        .unwrap_or_else(|| llm.model.clone());
    let max_tokens = stage.and_then(|m| m.max_tokens).unwrap_or(llm.max_tokens);
    let max_repairs = stage.and_then(|m| m.max_repairs).unwrap_or(3);

    // Driver-stage hand-offs/traces never mix with the migrate stage's.
    let traces = safe_ledger_dir(
        &ctx.root,
        &["migration", "units", &unit_id, "driver-traces"],
    )?;
    let resolved = harness_llm::providers::resolve(&provider_name, &traces)?;
    let params = harness_llm::migrate::MigrateParams {
        provider: &resolved,
        model: &model,
        max_tokens,
        max_repairs,
        traces_dir: &traces,
        retry,
        attempt: attempt.as_deref(),
        steer: None,
    };
    let judge = |candidate: &Path| harness_oracle::validate_driver(&ctx, unit, candidate);
    let outcome =
        match harness_llm::run_driver_generation(&params, &judge, &ctx, &facts, &plan_doc, unit) {
            Ok(o) => o,
            Err(e @ harness_core::Error::Awaiting { .. }) => {
                eprintln!("{e:#}");
                eprintln!(
                    "gen-driver: external provider mode — supply the response file under {} and \
                     re-run",
                    traces.display()
                );
                if let harness_core::Error::Awaiting { path, attempt } = &e {
                    report::event(&report::Awaiting {
                        k: "awaiting",
                        attempt: attempt.as_deref(),
                        path: path.display().to_string(),
                        resume: resume.clone(),
                        args: report::args(),
                    });
                }
                return Err(e.into());
            }
            Err(e) => return Err(e.into()),
        };
    let record = &outcome.record;
    for (i, t) in record.turns.iter().enumerate() {
        out(format!(
            "gen-driver: turn {} {} -> {}",
            i + 1,
            t.kind,
            t.result
        ));
    }
    out(format!(
        "gen-driver: {} attempt {} via `{}` ({}) model `{}` -> {}",
        unit_id,
        record.id,
        record.provider,
        record.provider_kind,
        record.model,
        record.outcome.to_uppercase()
    ));
    if let Some(drifted) = &outcome.drifted {
        out(format!(
            "gen-driver: verified from its recorded evidence; prompt: {}",
            harness_llm::conformance(drifted)
        ));
    }
    if record.outcome != "green" {
        return Ok(EXIT_ORACLE_RED);
    }
    let Some(candidate) = outcome.candidate_driver.as_ref() else {
        out("gen-driver: green attempt verified (replay run); nothing promoted".into());
        return Ok(0);
    };
    if resolved.kind == "replay" {
        out("gen-driver: replay run; nothing promoted".into());
        return Ok(0);
    }
    let bytes = std::fs::read(candidate).context("reading the green candidate driver")?;
    if hash::bytes_hash(&bytes) != record.candidate_digest {
        bail!("candidate driver digest does not match the attempt record; not promoting");
    }
    if dest.exists() {
        if std::fs::read(&dest).context("reading the current driver")? == bytes {
            out("gen-driver: this driver is already the unit's driver".into());
            return Ok(0);
        }
        if !promote {
            out(
                "gen-driver: green attempt recorded; not promoted (unit already has a \
                 generated driver — pass --promote to replace it; a verified unit's verdict \
                 then goes stale until re-verified)"
                    .to_string(),
            );
            return Ok(0);
        }
    }
    promote_driver(&ctx, &ledger, unit, &dest, &bytes)?;
    if unit.oracle.is_none() {
        plan_mod::set_oracle_table_if_absent(
            &ledger.plan_path(),
            &unit_id,
            &default_oracle_entries(&unit_id, &unit.files),
        )?;
    }
    out(format!(
        "gen-driver: promoted {} and recorded {}",
        dest_rel,
        validation_path.display()
    ));
    Ok(0)
}

/// Write the driver, validate it IN PLACE, and persist the validation only
/// when green; otherwise restore the previous driver (or none).
fn promote_driver(
    ctx: &TargetContext,
    ledger: &Ledger,
    unit: &harness_core::Unit,
    dest: &Path,
    bytes: &[u8],
) -> Result<()> {
    let previous = if dest.exists() {
        Some(std::fs::read(dest).context("reading the current driver")?)
    } else {
        None
    };
    harness_core::ledger::write_atomic(dest, bytes)?;
    let rollback = || -> Result<()> {
        match &previous {
            Some(old) => harness_core::ledger::write_atomic(dest, old)?,
            None => std::fs::remove_file(dest).context("removing unvalidated driver")?,
        }
        Ok(())
    };
    let validation: Result<DriverValidation, _> = harness_oracle::validate_driver(ctx, unit, dest);
    match validation {
        Ok(v) if v.green => {
            v.store(&ledger.driver_validation_path(&unit.id))?;
            Ok(())
        }
        Ok(v) => {
            rollback()?;
            let failed: Vec<&str> = v
                .checks
                .iter()
                .filter(|c| !c.passed)
                .map(|c| c.name.as_str())
                .collect();
            bail!(
                "promoted driver did not re-validate in place (failed: {}) — rolled back",
                failed.join(", ")
            )
        }
        Err(e) => {
            rollback()?;
            Err(e).context("in-place driver validation errored — rolled back")
        }
    }
}
